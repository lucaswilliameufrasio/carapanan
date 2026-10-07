use clap::{Parser, Subcommand};
use std::{
    error::Error,
    io::{self, Write},
    path::Path,
    process::ExitCode,
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
mod tui;

#[cfg(unix)]
use carapana_daemon::{DaemonService, IpcAttachment, ipc_request};
#[cfg(unix)]
use carapana_protocol::{DaemonRequest, DaemonResponse, Envelope};
#[cfg(unix)]
use carapana_storage::user_database_path;

#[derive(Parser)]
#[command(name = "carapana", about = "Read-only local Carapanã session client")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the local daemon until SIGINT or SIGTERM.
    Daemon,
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
    /// Inspect a session snapshot and optionally read events after a cursor.
    Show {
        session_id: String,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        events_after: Option<i64>,
    },
    /// Open a read-only terminal view of daemon sessions.
    Tui,
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

#[cfg(unix)]
fn run(cli: Cli) -> Result<(), Box<dyn Error>> {
    match cli.command {
        Commands::Daemon => run_daemon(),
        Commands::Tui => tui::run(&socket_path()?),
        command => run_client(command, &socket_path()?, &mut io::stdout().lock()),
    }
}

#[cfg(not(unix))]
fn run(_cli: Cli) -> Result<(), Box<dyn Error>> {
    Err("the Carapanã local daemon client currently requires Unix IPC".into())
}

fn main() -> ExitCode {
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
    use super::{Commands, run_client};
    use carapana_daemon::{DaemonRuntime, IpcServer};
    use carapana_protocol::{Autonomy, QueuedMessage, Selection, WorkMode};
    use carapana_storage::SessionRegistry;
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
            let path = std::env::temp_dir()
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
}
