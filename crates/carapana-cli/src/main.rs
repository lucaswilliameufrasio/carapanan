use carapana_core::provider::{
    CancellationToken, ModelCapabilities, ProviderAdapter, ProviderEvent, ProviderMessage,
    ProviderMessageRole, ProviderRequest, ResolvedModelSelection,
};
use carapana_openai::{
    ChatGptProvider, CredentialStore, authenticate, enable_plan_usage, load_fresh_account,
};
use clap::{Parser, Subcommand};
use futures_util::StreamExt;
use std::{
    error::Error,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::ExitCode,
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
mod tui;

#[cfg(unix)]
use carapana_daemon::{DaemonService, IpcAttachment, ipc_request};
#[cfg(unix)]
use carapana_protocol::{
    DaemonRequest, DaemonResponse, Envelope, MAX_QUEUED_MESSAGE_BYTES, MessageOrigin, Selection,
};
#[cfg(unix)]
use carapana_storage::user_database_path;

#[derive(Parser)]
#[command(name = "carapana", about = "Local Carapanã session client")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the local daemon until SIGINT or SIGTERM.
    Daemon,
    /// Create a paused session for an explicitly selected workspace.
    #[command(name = "create")]
    CreateSession {
        #[arg(long, value_name = "PATH")]
        workspace: PathBuf,
    },
    /// Queue one message from JSON on stdin; this does not start processing.
    Queue { session_id: String },
    /// Mark an idle, paused session as hibernated without changing its queue.
    Hibernate { session_id: String },
    /// Wake a hibernated session into the paused state without starting work.
    Wake { session_id: String },
    /// List persisted sessions from the running daemon.
    Sessions {
        #[arg(long)]
        json: bool,
    },
    /// List sessions that need operator attention.
    Attention {
        #[arg(long)]
        json: bool,
    },
    /// Review explicitly observed workspace files without changing the session.
    ReviewWorkspace {
        session_id: String,
        #[arg(long)]
        json: bool,
    },
    /// Explicitly acknowledge one changed, missing, or too-large recovery path.
    AcknowledgeRecoveryPath {
        session_id: String,
        path: String,
        #[arg(long)]
        confirm: bool,
    },
    /// Resume recovered active work after fresh review and explicit confirmation.
    ResumeRecovered {
        session_id: String,
        #[arg(long)]
        confirm: bool,
    },
    /// Inspect a session snapshot and optionally read events after a cursor.
    Show {
        session_id: String,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        events_after: Option<i64>,
    },
    /// Open the terminal client for session management, inspection, and gated recovery actions.
    Tui,
    /// Sign in to ChatGPT and use its eligible plan without configuring an API key.
    #[command(subcommand, name = "chatgpt")]
    ChatGpt(ChatGptCommands),
}

#[derive(Subcommand)]
enum ChatGptCommands {
    /// Register a new ChatGPT account or reauthorize an existing registration.
    SignIn {
        #[arg(long, value_name = "CLIENT_ID")]
        client_id: Option<String>,
        /// Explicitly request consent again for ChatGPT-plan usage on an existing registration.
        #[arg(long)]
        enable_plan_usage: bool,
    },
    /// List locally saved ChatGPT registrations without displaying credentials.
    Accounts {
        #[arg(long)]
        json: bool,
    },
    /// List models visible to one saved ChatGPT account.
    Models {
        #[arg(long, value_name = "CLIENT_ID")]
        client_id: String,
    },
    /// Send one fixed synthetic prompt to the selected model; no session or workspace context is sent.
    SmokeTest {
        #[arg(long, value_name = "CLIENT_ID")]
        client_id: String,
        #[arg(long, value_name = "MODEL")]
        model: String,
        /// Confirm one real remote inference request using the selected account's ChatGPT plan.
        #[arg(long)]
        confirm_remote: bool,
    },
}

#[cfg(unix)]
fn socket_path() -> Result<std::path::PathBuf, Box<dyn Error>> {
    Ok(user_database_path()?.with_file_name("daemon.sock"))
}

#[cfg(unix)]
fn run_daemon() -> Result<(), Box<dyn Error>> {
    use std::sync::{Arc, atomic::AtomicBool};

    let now_ms = i64::try_from(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis())?;
    let mut service = DaemonService::open_user(now_ms)?;
    let shutdown = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&shutdown))?;
    signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&shutdown))?;
    eprintln!(
        "Carapanã daemon listening on {}",
        service.socket_path().display()
    );
    service.serve_until(&shutdown)?;
    Ok(())
}

#[cfg(unix)]
fn run_client(
    command: Commands,
    path: &Path,
    output: &mut impl Write,
) -> Result<(), Box<dyn Error>> {
    match command {
        Commands::Daemon => unreachable!("daemon command is handled before client dispatch"),
        Commands::Tui => return tui::run(path),
        Commands::ChatGpt(_) => unreachable!("ChatGPT commands are handled before IPC dispatch"),
        Commands::CreateSession { workspace } => {
            let workspace = if workspace.is_absolute() {
                workspace
            } else {
                std::env::current_dir()?.join(workspace)
            };
            let workspace_path = workspace.to_str().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "workspace path must be valid UTF-8",
                )
            })?;
            let response = ipc_request(
                path,
                Envelope::new(DaemonRequest::CreateSession {
                    workspace_path: workspace_path.to_owned(),
                }),
            )?;
            match response.payload {
                DaemonResponse::SessionCreated { session_id } => {
                    writeln!(
                        output,
                        "Created paused session {session_id}. No provider or tools were started."
                    )?;
                }
                DaemonResponse::Error { code } => {
                    return Err(format!("daemon rejected session creation: {code:?}").into());
                }
                _ => return Err("daemon returned an incompatible session-creation response".into()),
            }
        }
        Commands::Queue { session_id } => {
            let (text, selection) = read_queue_input()?;
            let response = ipc_request(
                path,
                Envelope::new(DaemonRequest::EnqueueMessage {
                    session_id: session_id.clone(),
                    text,
                    selection,
                    origin: MessageOrigin::Cli,
                }),
            )?;
            match response.payload {
                DaemonResponse::MessageQueued {
                    session_id,
                    message_id,
                } => {
                    writeln!(
                        output,
                        "Queued message {message_id} for session {session_id}. The session remains paused; no provider or tools were started."
                    )?;
                }
                DaemonResponse::Error { code } => {
                    return Err(format!("daemon rejected message enqueue: {code:?}").into());
                }
                _ => return Err("daemon returned an incompatible enqueue response".into()),
            }
        }
        Commands::Hibernate { session_id } => {
            let response = ipc_request(
                path,
                Envelope::new(DaemonRequest::HibernateSession {
                    session_id: session_id.clone(),
                }),
            )?;
            match response.payload {
                DaemonResponse::SessionHibernated { session_id } => {
                    writeln!(
                        output,
                        "Session {session_id} hibernated. Queue and history are preserved; no resources or processes were changed."
                    )?;
                }
                DaemonResponse::Error { code } => {
                    return Err(format!("daemon refused session hibernation: {code:?}").into());
                }
                _ => return Err("daemon returned an incompatible hibernation response".into()),
            }
        }
        Commands::Wake { session_id } => {
            let response = ipc_request(
                path,
                Envelope::new(DaemonRequest::WakeSession {
                    session_id: session_id.clone(),
                }),
            )?;
            match response.payload {
                DaemonResponse::SessionWoken { session_id } => {
                    writeln!(
                        output,
                        "Session {session_id} is paused again. Queue is preserved; no work was started."
                    )?;
                }
                DaemonResponse::Error { code } => {
                    return Err(format!("daemon refused to wake session: {code:?}").into());
                }
                _ => return Err("daemon returned an incompatible wake response".into()),
            }
        }
        Commands::Sessions { json } => {
            let response = ipc_request(path, Envelope::new(DaemonRequest::ListSessions {}))?;
            let DaemonResponse::Sessions { sessions } = response.payload else {
                return Err(format!(
                    "daemon returned unexpected response: {:?}",
                    response.payload
                )
                .into());
            };
            if json {
                serde_json::to_writer_pretty(&mut *output, &sessions)?;
                writeln!(output)?;
            } else if sessions.is_empty() {
                writeln!(output, "No sessions.")?;
            } else {
                for session in sessions {
                    writeln!(
                        output,
                        "{}\t{:?}\tqueued={}\tactive={}\tattention={}",
                        session.session_id,
                        session.status,
                        session.queued_count,
                        session.has_active_message,
                        session.recovery_needs_revalidation || session.active_work_uncertain,
                    )?;
                }
            }
        }
        Commands::Attention { json } => {
            let response = ipc_request(path, Envelope::new(DaemonRequest::ListAttention {}))?;
            let DaemonResponse::Attention { items } = response.payload else {
                return Err(format!(
                    "daemon returned unexpected response: {:?}",
                    response.payload
                )
                .into());
            };
            if json {
                serde_json::to_writer_pretty(&mut *output, &items)?;
                writeln!(output)?;
            } else if items.is_empty() {
                writeln!(output, "Nothing needs attention.")?;
            } else {
                for item in items {
                    writeln!(
                        output,
                        "{}\t{:?}\tactive_work_uncertain={}\tevent_sequence={}",
                        item.session_id,
                        item.reason,
                        item.active_work_uncertain,
                        item.event_sequence,
                    )?;
                }
            }
        }
        Commands::ReviewWorkspace { session_id, json } => {
            let response = ipc_request(
                path,
                Envelope::new(DaemonRequest::ReviewWorkspace { session_id }),
            )?;
            let DaemonResponse::WorkspaceReview { review } = response.payload else {
                return Err(format!(
                    "daemon returned unexpected response: {:?}",
                    response.payload
                )
                .into());
            };
            if json {
                serde_json::to_writer_pretty(&mut *output, &review.files)?;
                writeln!(output)?;
            } else {
                writeln!(
                    output,
                    "Read-only report; this does not authorize resuming."
                )?;
                if review.files.is_empty() {
                    writeln!(output, "No explicitly observed workspace files.")?;
                } else {
                    for file in review.files {
                        writeln!(
                            output,
                            "{:?}\t{}",
                            file.path,
                            review_status_label(file.status)
                        )?;
                    }
                }
            }
        }
        Commands::AcknowledgeRecoveryPath {
            session_id,
            path: relative_path,
            confirm,
        } => {
            if !confirm {
                return Err("acknowledging a recovery path requires --confirm".into());
            }
            let response = ipc_request(
                path,
                Envelope::new(DaemonRequest::AcknowledgeRecoveryPath {
                    session_id,
                    path: relative_path,
                }),
            )?;
            let DaemonResponse::RecoveryPathAcknowledged { path, status } = response.payload else {
                return Err(format!(
                    "daemon did not acknowledge the recovery path: {:?}",
                    response.payload
                )
                .into());
            };
            writeln!(
                output,
                "Acknowledged {:?} as {}. This does not resume the session.",
                path,
                review_status_label(status)
            )?;
        }
        Commands::ResumeRecovered {
            session_id,
            confirm,
        } => {
            let report_response = ipc_request(
                path,
                Envelope::new(DaemonRequest::ReviewWorkspace {
                    session_id: session_id.clone(),
                }),
            )?;
            let DaemonResponse::WorkspaceReview { review } = report_response.payload else {
                return Err(format!(
                    "daemon could not review the workspace before resume: {:?}",
                    report_response.payload
                )
                .into());
            };
            writeln!(output, "Fresh recovery review:")?;
            if review.files.is_empty() {
                writeln!(output, "No explicitly observed workspace files.")?;
            } else {
                for file in review.files {
                    writeln!(
                        output,
                        "{:?}\t{}",
                        file.path,
                        review_status_label(file.status)
                    )?;
                }
            }
            output.flush()?;
            if !confirm {
                writeln!(
                    output,
                    "Session remains paused. Review the paths, then run `resume-recovered {session_id} --confirm` to explicitly resume."
                )?;
                return Ok(());
            }
            let response = ipc_request(
                path,
                Envelope::new(DaemonRequest::ResumeRecovered { session_id }),
            )?;
            let DaemonResponse::RecoveryResumed { session_id } = response.payload else {
                return Err(
                    format!("daemon refused recovery resume: {:?}", response.payload).into(),
                );
            };
            writeln!(
                output,
                "Recovery resumed for {session_id}. No provider or tools were started."
            )?;
        }
        Commands::Show {
            session_id,
            json,
            events_after,
        } => {
            let mut attachment = IpcAttachment::attach(path, session_id)?;
            let snapshot = attachment.snapshot().clone();
            let events = match events_after {
                Some(cursor) => Some(attachment.events_after(cursor)?),
                None => None,
            };
            attachment.detach()?;

            if json {
                serde_json::to_writer_pretty(
                    &mut *output,
                    &serde_json::json!({ "snapshot": snapshot, "events": events }),
                )?;
                writeln!(output)?;
            } else {
                writeln!(
                    output,
                    "Session {}\nStatus: {:?}\nQueued: {}\nActive work uncertain: {}\nEvent sequence: {}",
                    snapshot.session_id,
                    snapshot.status,
                    snapshot.queued_messages.len(),
                    snapshot.active_work_uncertain,
                    snapshot.event_sequence,
                )?;
                if let Some(events) = events {
                    for event in events.events {
                        writeln!(output, "{}\t{:?}", event.sequence, event.event)?;
                    }
                    if events.has_more {
                        writeln!(
                            output,
                            "More events after sequence {}.",
                            events.next_sequence
                        )?;
                    }
                }
            }
        }
    }
    output.flush()?;
    Ok(())
}

fn run_chatgpt(command: ChatGptCommands, output: &mut impl Write) -> Result<(), Box<dyn Error>> {
    match command {
        ChatGptCommands::SignIn {
            client_id,
            enable_plan_usage: request_consent,
        } => {
            let account = if request_consent {
                let client_id = client_id.as_deref().ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "--enable-plan-usage requires an existing --client-id",
                    )
                })?;
                enable_plan_usage(client_id)?
            } else {
                authenticate(client_id.as_deref())?
            };
            let summary = account.summary();
            writeln!(output, "ChatGPT account authorized.")?;
            writeln!(output, "client_id: {}", summary.client_id)?;
            if let Some(email) = summary
                .email
                .as_deref()
                .filter(|email| is_safe_terminal_text(email, 320))
            {
                writeln!(output, "account: {email}")?;
            }
            writeln!(output, "ChatGPT plan usage: {}", summary.plan_usage_enabled)?;
            writeln!(
                output,
                "Refresh token available: {}",
                summary.has_refresh_token
            )?;
            if !summary.plan_usage_enabled {
                writeln!(
                    output,
                    "No inference will run until plan usage is authorized. To request consent again, use `carapana chatgpt sign-in --client-id {} --enable-plan-usage`.",
                    summary.client_id
                )?;
            }
            writeln!(output, "No session, tools, or tasks were started.")?;
        }
        ChatGptCommands::Accounts { json } => {
            let accounts = CredentialStore::for_user()?.accounts()?;
            let summaries = accounts
                .iter()
                .map(|account| account.summary())
                .collect::<Vec<_>>();
            if json {
                let accounts = summaries
                    .iter()
                    .map(|summary| {
                        serde_json::json!({
                            "client_id": summary.client_id,
                            "email": summary.email,
                            "plan_usage_enabled": summary.plan_usage_enabled,
                            "has_refresh_token": summary.has_refresh_token,
                            "expires_at_unix_seconds": summary.expires_at_unix_seconds,
                        })
                    })
                    .collect::<Vec<_>>();
                serde_json::to_writer_pretty(&mut *output, &accounts)?;
                writeln!(output)?;
            } else if summaries.is_empty() {
                writeln!(
                    output,
                    "No saved ChatGPT accounts. Run `carapana chatgpt sign-in`."
                )?;
            } else {
                for summary in summaries {
                    let email = summary
                        .email
                        .as_deref()
                        .filter(|email| is_safe_terminal_text(email, 320))
                        .unwrap_or("email unavailable");
                    writeln!(
                        output,
                        "{}\t{}\tplan_usage={}\trefresh_token={}",
                        summary.client_id,
                        email,
                        summary.plan_usage_enabled,
                        summary.has_refresh_token
                    )?;
                }
            }
        }
        ChatGptCommands::Models { client_id } => {
            let account = load_fresh_account(&client_id)?;
            let provider = ChatGptProvider::new(account)
                .map_err(|error| provider_error("could not initialize ChatGPT", error))?;
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            let models = runtime
                .block_on(provider.available_models(CancellationToken::new()))
                .map_err(|error| provider_error("model listing failed", error))?;
            if models.is_empty() {
                writeln!(
                    output,
                    "No models are currently available for this account."
                )?;
            } else {
                for model in models {
                    writeln!(output, "{}\t{}", model.slug, model.display_name)?;
                }
            }
        }
        ChatGptCommands::SmokeTest {
            client_id,
            model,
            confirm_remote,
        } => {
            if !confirm_remote {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "a real inference request requires --confirm-remote",
                )
                .into());
            }
            let account = load_fresh_account(&client_id)?;
            let provider = ChatGptProvider::new(account)
                .map_err(|error| provider_error("could not initialize ChatGPT", error))?;
            let request = ProviderRequest {
                selection: ResolvedModelSelection {
                    provider: "chatgpt".to_owned(),
                    model,
                    variant: "default".to_owned(),
                    capabilities: ModelCapabilities {
                        streaming: true,
                        cancellation: true,
                        tool_calling: false,
                    },
                },
                messages: vec![ProviderMessage {
                    role: ProviderMessageRole::User,
                    content: "Reply with exactly: Carapana SIWC smoke test OK.".to_owned(),
                }],
            };
            writeln!(
                output,
                "Sending one fixed synthetic prompt to ChatGPT. No session or workspace context and no tools will be sent."
            )?;
            output.flush()?;
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            runtime
                .block_on(async {
                    let mut events = provider.stream(request, CancellationToken::new());
                    while let Some(event) = events.next().await {
                        match event {
                            Ok(ProviderEvent::TextDelta(text)) => {
                                write_terminal_safe_text(output, &text)?
                            }
                            Ok(ProviderEvent::ToolCallRequested { .. }) => {
                                writeln!(
                                    output,
                                    "\nA tool call was returned but was not executed."
                                )?;
                            }
                            Err(error) => return Err(provider_error("inference failed", error)),
                        }
                    }
                    writeln!(output)?;
                    writeln!(
                        output,
                        "Smoke test completed; no conversation was persisted."
                    )?;
                    Ok::<(), io::Error>(())
                })
                .map_err(|error| -> Box<dyn Error> { Box::new(error) })?;
        }
    }
    output.flush()?;
    Ok(())
}

fn provider_error(context: &str, error: carapana_core::provider::ProviderError) -> io::Error {
    io::Error::other(format!("{context}: {error:?}"))
}

fn is_safe_terminal_text(value: &str, max_bytes: usize) -> bool {
    !value.is_empty() && value.len() <= max_bytes && !value.chars().any(char::is_control)
}

fn write_terminal_safe_text(output: &mut impl Write, text: &str) -> io::Result<()> {
    let mut safe_text = String::with_capacity(text.len());
    for character in text.chars() {
        if character == '\n' || character == '\t' {
            safe_text.push(character);
        } else if character.is_control() {
            safe_text.push_str(&character.escape_default().to_string());
        } else {
            safe_text.push(character);
        }
    }
    output.write_all(safe_text.as_bytes())
}

#[cfg(unix)]
fn read_queue_input() -> Result<(String, Selection), Box<dyn Error>> {
    // JSON may expand one input control byte to a six-byte escape before IPC framing.
    const MAX_QUEUE_INPUT_BYTES: usize = MAX_QUEUED_MESSAGE_BYTES * 6 + 8 * 1024;

    let mut input = Vec::new();
    let mut stdin = io::stdin()
        .take(u64::try_from(MAX_QUEUE_INPUT_BYTES + 1).expect("queue input bound fits u64"));
    stdin.read_to_end(&mut input)?;
    if input.len() > MAX_QUEUE_INPUT_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "queue input exceeds the maximum size",
        )
        .into());
    }

    let value: serde_json::Value = serde_json::from_slice(&input).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "stdin must contain a JSON object with text and selection fields",
        )
    })?;
    let Some(object) = value.as_object() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "stdin must contain a JSON object with text and selection fields",
        )
        .into());
    };
    if object.len() != 2 || !object.contains_key("text") || !object.contains_key("selection") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "queue input must contain only text and selection fields",
        )
        .into());
    }

    let text = object
        .get("text")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "queue text must be a string"))?
        .to_owned();
    let selection_value = object.get("selection").ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "queue input must contain only text and selection fields",
        )
    })?;
    let selection = serde_json::from_value(selection_value.clone()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "selection must include profile, work, autonomy, provider, model, and variant",
        )
    })?;
    Ok((text, selection))
}

#[cfg(unix)]
fn review_status_label(status: carapana_protocol::WorkspaceFileReviewStatus) -> &'static str {
    use carapana_protocol::WorkspaceFileReviewStatus;

    match status {
        WorkspaceFileReviewStatus::Unchanged => "unchanged",
        WorkspaceFileReviewStatus::Changed => "changed",
        WorkspaceFileReviewStatus::Missing => "missing",
        WorkspaceFileReviewStatus::Unreadable => "unreadable",
        WorkspaceFileReviewStatus::Unsafe => "unsafe",
        WorkspaceFileReviewStatus::TooLarge => "too_large",
        WorkspaceFileReviewStatus::Unavailable => "unavailable",
    }
}

#[cfg(unix)]
fn run(cli: Cli) -> Result<(), Box<dyn Error>> {
    match cli.command {
        Commands::Daemon => run_daemon(),
        Commands::Tui => tui::run(&socket_path()?),
        Commands::ChatGpt(command) => run_chatgpt(command, &mut io::stdout().lock()),
        command => run_client(command, &socket_path()?, &mut io::stdout().lock()),
    }
}

#[cfg(not(unix))]
fn run(_cli: Cli) -> Result<(), Box<dyn Error>> {
    Err("the Carapanã local daemon client currently requires Unix IPC".into())
}

fn main() -> ExitCode {
    #[cfg(unix)]
    {
        let mut arguments = std::env::args_os();
        let _ = arguments.next();
        if arguments
            .next()
            .is_some_and(|argument| argument == "__carapana_internal_process_host_v1")
        {
            if arguments.next().is_some() {
                return ExitCode::FAILURE;
            }
            return match carapana_daemon::run_process_host() {
                Ok(()) => ExitCode::SUCCESS,
                Err(_) => {
                    eprintln!("carapana: private process host failed");
                    ExitCode::FAILURE
                }
            };
        }
    }
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("carapana: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::{
        ChatGptCommands, Commands, is_safe_terminal_text, review_status_label, run_chatgpt,
        run_client, write_terminal_safe_text,
    };
    use carapana_daemon::{DaemonRuntime, IpcServer};
    use carapana_protocol::{Autonomy, QueuedMessage, Selection, WorkMode};
    use carapana_storage::{SessionRegistry, WorkspaceMetadata};
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        path::PathBuf,
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicU64, Ordering},
        },
        thread,
        time::SystemTime,
    };

    struct PrivateDir(PathBuf);

    impl PrivateDir {
        fn new() -> Self {
            static NEXT_ID: AtomicU64 = AtomicU64::new(0);
            let nonce = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            // `/tmp` is short on macOS, where Unix socket paths have a small `SUN_LEN` limit.
            let path = fs::canonicalize("/tmp")
                .unwrap()
                .join(format!("carapana-cli-{}-{nonce}-{id}", std::process::id()));
            fs::create_dir(&path).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
            Self(path)
        }
    }

    impl Drop for PrivateDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn message() -> QueuedMessage {
        QueuedMessage {
            id: "message-1".into(),
            text: "preserved user message".into(),
            origin: "cli-test".into(),
            selection: Selection {
                profile: "ask".into(),
                work: WorkMode::Plan,
                autonomy: Autonomy::Ask,
                provider: "mock".into(),
                model: "mock-model".into(),
                variant: "default".into(),
            },
        }
    }

    #[test]
    fn should_keep_workspace_review_json_status_names_stable() {
        use carapana_protocol::WorkspaceFileReviewStatus as Status;

        for (status, expected) in [
            (Status::Unchanged, "unchanged"),
            (Status::Changed, "changed"),
            (Status::Missing, "missing"),
            (Status::Unreadable, "unreadable"),
            (Status::Unsafe, "unsafe"),
            (Status::TooLarge, "too_large"),
            (Status::Unavailable, "unavailable"),
        ] {
            assert_eq!(review_status_label(status), expected);
        }
    }

    #[test]
    fn should_create_short_canonical_socket_paths_for_tests() {
        let directory = PrivateDir::new();
        let socket_path = directory.0.join("daemon.sock");

        assert_eq!(fs::canonicalize(&directory.0).unwrap(), directory.0);
        assert!(socket_path.to_str().unwrap().len() < 104);
    }

    #[test]
    fn chatgpt_smoke_test_requires_explicit_remote_confirmation_before_loading_account() {
        let mut output = Vec::new();

        let result = run_chatgpt(
            ChatGptCommands::SmokeTest {
                client_id: "oaiapp-not-a-real-account".to_owned(),
                model: "explicit-test-model".to_owned(),
                confirm_remote: false,
            },
            &mut output,
        );

        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("requires --confirm-remote")
        );
        assert!(output.is_empty());
    }

    #[test]
    fn chatgpt_output_escapes_terminal_controls_and_omits_unsafe_email() {
        let mut output = Vec::new();

        write_terminal_safe_text(&mut output, "plain\n\u{1b}[31mred\u{1b}[0m").unwrap();

        assert_eq!(
            String::from_utf8(output).unwrap(),
            "plain\n\\u{1b}[31mred\\u{1b}[0m"
        );
        assert!(is_safe_terminal_text("account@example.invalid", 320));
        assert!(!is_safe_terminal_text("account\nname@example.invalid", 320));
    }

    #[test]
    fn should_inspect_sessions_attention_and_recovery_events_without_mutations() {
        let directory = PrivateDir::new();
        let database_path = directory.0.join("sessions.sqlite3");
        let mut registry = SessionRegistry::open(&database_path).unwrap();
        registry.create("session-1", 10).unwrap();
        registry.enqueue("session-1", message(), 11).unwrap();
        registry.start_next("session-1", 12).unwrap();
        drop(registry);

        let mut runtime = DaemonRuntime::open(&database_path, 20).unwrap();
        let socket_path = directory.0.join("daemon.sock");
        let server = IpcServer::bind(&socket_path).unwrap();
        let shutdown = Arc::new(AtomicBool::new(false));
        let server_shutdown = Arc::clone(&shutdown);
        let server_thread = thread::spawn(move || {
            server
                .serve_until(&mut runtime, server_shutdown.as_ref())
                .unwrap();
        });

        let mut sessions_json = Vec::new();
        run_client(
            Commands::Sessions { json: true },
            &socket_path,
            &mut sessions_json,
        )
        .unwrap();
        let sessions: serde_json::Value = serde_json::from_slice(&sessions_json).unwrap();
        assert_eq!(sessions[0]["session_id"], "session-1");
        assert_eq!(sessions[0]["status"], "paused");
        assert_eq!(sessions[0]["active_work_uncertain"], true);

        let mut attention_json = Vec::new();
        run_client(
            Commands::Attention { json: true },
            &socket_path,
            &mut attention_json,
        )
        .unwrap();
        let attention: serde_json::Value = serde_json::from_slice(&attention_json).unwrap();
        assert_eq!(attention[0]["session_id"], "session-1");
        assert_eq!(attention[0]["reason"], "recovery_review");

        let mut show_json = Vec::new();
        run_client(
            Commands::Show {
                session_id: "session-1".into(),
                json: true,
                events_after: Some(0),
            },
            &socket_path,
            &mut show_json,
        )
        .unwrap();
        let inspected: serde_json::Value = serde_json::from_slice(&show_json).unwrap();
        assert_eq!(inspected["snapshot"]["status"], "paused");
        assert_eq!(inspected["snapshot"]["active_work_uncertain"], true);
        assert_eq!(inspected["events"]["events"].as_array().unwrap().len(), 4);

        shutdown.store(true, Ordering::Release);
        server_thread.join().unwrap();
        assert!(!socket_path.exists());
    }

    #[test]
    fn should_display_workspace_review_paths_and_statuses_without_mutating_recovery() {
        let directory = PrivateDir::new();
        let database_path = directory.0.join("sessions.sqlite3");
        let workspace_path = directory.0.join("workspace");
        fs::create_dir(&workspace_path).unwrap();
        fs::write(workspace_path.join("review.txt"), b"private file contents").unwrap();
        fs::write(
            workspace_path.join("line\nbreak.txt"),
            b"another private file",
        )
        .unwrap();
        fs::write(
            workspace_path.join("tab\tbreak.txt"),
            b"tabbed private file",
        )
        .unwrap();
        fs::write(
            workspace_path.join("escape\u{1b}[31m.txt"),
            b"control private file",
        )
        .unwrap();
        {
            let mut registry = SessionRegistry::open(&database_path).unwrap();
            registry
                .create_with_workspace(
                    "session-1",
                    WorkspaceMetadata::capture(&workspace_path).unwrap(),
                    10,
                )
                .unwrap();
            registry
                .observe_workspace_file_with_hash("session-1", "review.txt", 11)
                .unwrap();
            registry
                .observe_workspace_file("session-1", "line\nbreak.txt", 12)
                .unwrap();
            registry
                .observe_workspace_file("session-1", "tab\tbreak.txt", 13)
                .unwrap();
            registry
                .observe_workspace_file("session-1", "escape\u{1b}[31m.txt", 14)
                .unwrap();
        }
        fs::write(
            workspace_path.join("review.txt"),
            b"changed private file contents",
        )
        .unwrap();

        let mut runtime = DaemonRuntime::open(&database_path, 20).unwrap();
        let socket_path = directory.0.join("daemon.sock");
        let server = IpcServer::bind(&socket_path).unwrap();
        let shutdown = Arc::new(AtomicBool::new(false));
        let server_shutdown = Arc::clone(&shutdown);
        let server_thread = thread::spawn(move || {
            server
                .serve_until(&mut runtime, server_shutdown.as_ref())
                .unwrap();
        });

        let mut json_output = Vec::new();
        run_client(
            Commands::ReviewWorkspace {
                session_id: "session-1".into(),
                json: true,
            },
            &socket_path,
            &mut json_output,
        )
        .unwrap();
        let report: serde_json::Value = serde_json::from_slice(&json_output).unwrap();
        assert_eq!(report.as_array().unwrap().len(), 4);
        assert_eq!(report[0]["path"], "review.txt");
        assert_eq!(report[0]["status"], "changed");
        assert_eq!(report[1]["path"], "line\nbreak.txt");
        assert_eq!(report[1]["status"], "unchanged");
        assert_eq!(report[2]["path"], "tab\tbreak.txt");
        assert_eq!(report[2]["status"], "unchanged");
        assert_eq!(report[3]["path"], "escape\u{1b}[31m.txt");
        assert_eq!(report[3]["status"], "unchanged");
        assert!(
            !json_output
                .windows(b"private file contents".len())
                .any(|window| window == b"private file contents")
        );
        assert!(!String::from_utf8_lossy(&json_output).contains("content_sha256"));
        assert!(
            !String::from_utf8_lossy(&json_output).contains(&*workspace_path.to_string_lossy())
        );

        let mut text_output = Vec::new();
        run_client(
            Commands::ReviewWorkspace {
                session_id: "session-1".into(),
                json: false,
            },
            &socket_path,
            &mut text_output,
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(text_output).unwrap(),
            "Read-only report; this does not authorize resuming.\n\"review.txt\"\tchanged\n\"line\\nbreak.txt\"\tunchanged\n\"tab\\tbreak.txt\"\tunchanged\n\"escape\\u{1b}[31m.txt\"\tunchanged\n"
        );
        assert!(!String::from_utf8_lossy(&json_output).contains('\u{1b}'));
        assert!(!String::from_utf8_lossy(&json_output).contains("private file contents"));

        shutdown.store(true, Ordering::Release);
        server_thread.join().unwrap();
        let registry = SessionRegistry::open(&database_path).unwrap();
        let session = registry.get("session-1").unwrap();
        assert_eq!(session.event_sequence, 5);
        assert!(!session.recovery_needs_revalidation);
        assert_eq!(
            session.status,
            carapana_storage::StoredSessionStatus::Paused
        );
    }
}
