use std::{
    collections::HashSet,
    error::Error,
    io,
    path::{Path, PathBuf},
};

use carapana_daemon::{IpcAttachment, ipc_request};
use carapana_protocol::{
    AttentionItem, DaemonRequest, DaemonResponse, Envelope, SessionEventRecord, SessionSnapshot,
    SessionSummary, WorkspaceFileReview, WorkspaceFileReviewStatus,
};
use crossterm::event::{self, Event, KeyCode, KeyEvent};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Text},
    widgets::{Block, Borders, Paragraph, Wrap},
};

struct App {
    socket_path: PathBuf,
    sessions: Vec<SessionSummary>,
    attention: Vec<AttentionItem>,
    selected: usize,
    detail: Option<SessionSnapshot>,
    workspace_review: Option<WorkspaceFileReview>,
    detail_scroll: usize,
    event_mode: bool,
    events: Vec<SessionEventRecord>,
    event_cursor: i64,
    event_has_more: bool,
    error: Option<String>,
}

impl App {
    fn new(socket_path: &Path) -> Self {
        Self {
            socket_path: socket_path.to_owned(),
            sessions: Vec::new(),
            attention: Vec::new(),
            selected: 0,
            detail: None,
            workspace_review: None,
            detail_scroll: 0,
            event_mode: false,
            events: Vec::new(),
            event_cursor: 0,
            event_has_more: false,
            error: None,
        }
    }

    fn refresh(&mut self) {
        let result = (|| {
            let sessions = match ipc_request(
                &self.socket_path,
                Envelope::new(DaemonRequest::ListSessions {}),
            )?
            .payload
            {
                DaemonResponse::Sessions { sessions } => sessions,
                other => return Err(format!("unexpected sessions response: {other:?}").into()),
            };
            let attention = match ipc_request(
                &self.socket_path,
                Envelope::new(DaemonRequest::ListAttention {}),
            )?
            .payload
            {
                DaemonResponse::Attention { items } => items,
                other => return Err(format!("unexpected attention response: {other:?}").into()),
            };
            Ok::<_, Box<dyn Error>>((sessions, attention))
        })();

        match result {
            Ok((sessions, attention)) => {
                self.sessions = sessions;
                self.attention = attention;
                self.selected = self.selected.min(self.sessions.len().saturating_sub(1));
                self.error = None;
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    fn open_selected(&mut self) {
        let Some(session_id) = self
            .sessions
            .get(self.selected)
            .map(|session| session.session_id.clone())
        else {
            return;
        };
        match IpcAttachment::attach(&self.socket_path, session_id) {
            Ok(attachment) => {
                let snapshot = attachment.snapshot().clone();
                match attachment.detach() {
                    Ok(_) => {
                        self.detail = Some(snapshot);
                        self.workspace_review = None;
                        self.detail_scroll = 0;
                        self.event_mode = false;
                        self.events.clear();
                        self.event_cursor = 0;
                        self.event_has_more = false;
                        self.error = None;
                    }
                    Err(error) => self.error = Some(error.to_string()),
                }
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    fn load_event_page(&mut self) {
        let Some(session_id) = self
            .detail
            .as_ref()
            .map(|snapshot| snapshot.session_id.clone())
        else {
            return;
        };
        let result = (|| {
            let mut attachment = IpcAttachment::attach(&self.socket_path, session_id)?;
            let batch = attachment.events_after(self.event_cursor);
            let detached = attachment.detach();
            let batch = batch?;
            detached?;
            Ok::<_, Box<dyn Error>>(batch)
        })();
        match result {
            Ok(batch) => {
                self.events.extend(batch.events);
                self.event_cursor = batch.next_sequence;
                self.event_has_more = batch.has_more;
                self.detail_scroll = 0;
                self.error = None;
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    fn review_workspace(&mut self) {
        let Some(session_id) = self
            .detail
            .as_ref()
            .map(|snapshot| snapshot.session_id.clone())
        else {
            return;
        };
        self.workspace_review = None;
        match ipc_request(
            &self.socket_path,
            Envelope::new(DaemonRequest::ReviewWorkspace { session_id }),
        )
        .map(|response| response.payload)
        {
            Ok(DaemonResponse::WorkspaceReview { review }) => {
                self.workspace_review = Some(review);
                self.detail_scroll = 0;
                self.error = None;
            }
            Ok(other) => {
                self.error = Some(format!("unexpected workspace review response: {other:?}"))
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('q') => true,
            KeyCode::Char('r') => {
                if self.event_mode {
                    self.events.clear();
                    self.event_cursor = 0;
                    self.event_has_more = false;
                    self.detail_scroll = 0;
                    self.load_event_page();
                } else if self.detail.is_some() {
                    self.workspace_review = None;
                    let session_id = self.detail.as_ref().unwrap().session_id.clone();
                    match IpcAttachment::attach(&self.socket_path, session_id) {
                        Ok(attachment) => {
                            let snapshot = attachment.snapshot().clone();
                            match attachment.detach() {
                                Ok(_) => {
                                    self.detail = Some(snapshot);
                                    self.workspace_review = None;
                                    self.detail_scroll = 0;
                                    self.error = None;
                                }
                                Err(error) => self.error = Some(error.to_string()),
                            }
                        }
                        Err(error) => self.error = Some(error.to_string()),
                    }
                } else {
                    self.refresh();
                }
                false
            }
            KeyCode::Esc if self.detail.is_some() => {
                if self.event_mode {
                    self.event_mode = false;
                    self.detail_scroll = 0;
                    self.error = None;
                    return false;
                }
                self.detail = None;
                self.workspace_review = None;
                self.error = None;
                false
            }
            KeyCode::Up if self.detail.is_some() => {
                self.detail_scroll = self.detail_scroll.saturating_sub(1);
                false
            }
            KeyCode::Down if self.detail.is_some() => {
                self.detail_scroll = self.detail_scroll.saturating_add(1);
                false
            }
            KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
                false
            }
            KeyCode::Down => {
                if !self.sessions.is_empty() {
                    self.selected = (self.selected + 1).min(self.sessions.len() - 1);
                }
                false
            }
            KeyCode::Enter if self.detail.is_none() => {
                self.open_selected();
                false
            }
            KeyCode::Char('e') if self.detail.is_some() => {
                self.event_mode = true;
                self.detail_scroll = 0;
                if self.events.is_empty() {
                    self.load_event_page();
                } else {
                    self.error = None;
                }
                false
            }
            KeyCode::Char('v') if self.detail.is_some() && !self.event_mode => {
                self.review_workspace();
                false
            }
            KeyCode::Char('n') if self.event_mode && self.event_has_more => {
                self.load_event_page();
                false
            }
            KeyCode::Esc => {
                self.error = None;
                false
            }
            _ => false,
        }
    }
}

fn draw(frame: &mut Frame<'_>, app: &App) {
    let [header, content, footer] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(3),
        ])
        .areas(frame.area());

    let title = if let Some(snapshot) = &app.detail {
        if app.event_mode {
            format!("Carapanã · {} · Events", snapshot.session_id)
        } else {
            format!("Carapanã · {}", snapshot.session_id)
        }
    } else {
        "Carapanã · Sessions".to_owned()
    };
    frame.render_widget(
        Paragraph::new("Read-only · daemon is the source of truth")
            .block(Block::default().borders(Borders::ALL).title(title)),
        header,
    );

    let content_block = Block::default().borders(Borders::ALL);
    let inner = content_block.inner(content);
    frame.render_widget(content_block, content);
    let (lines, scroll) = if let Some(snapshot) = &app.detail {
        let lines = if app.event_mode {
            event_lines(
                &app.events,
                app.event_cursor,
                app.event_has_more,
                app.error.as_deref(),
            )
        } else {
            detail_lines(
                snapshot,
                app.workspace_review.as_ref(),
                app.error.as_deref(),
            )
        };
        let width = usize::from(inner.width.max(1));
        let visual_height = lines
            .iter()
            .map(|line| line.width().max(1).div_ceil(width))
            .sum::<usize>();
        let max_scroll = visual_height.saturating_sub(inner.height as usize);
        (lines, app.detail_scroll.min(max_scroll) as u16)
    } else {
        let (lines, selected_line) = overview_lines(app);
        let scroll = selected_line
            .map(|line| overview_scroll(line, inner.height))
            .unwrap_or_default();
        (lines, scroll)
    };
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0)),
        inner,
    );

    let footer_text = if app.event_mode {
        "↑/↓ scroll · n next page · r reload · Esc details · q quit"
    } else if app.detail.is_some() {
        "↑/↓ scroll · v review files · e events · r refresh · Esc back · q quit"
    } else {
        "↑/↓ select · Enter inspect · r refresh · q quit"
    };
    frame.render_widget(
        Paragraph::new(footer_text).block(Block::default().borders(Borders::ALL)),
        footer,
    );
}

fn overview_lines(app: &App) -> (Vec<Line<'static>>, Option<usize>) {
    const MAX_VISIBLE_ATTENTION: usize = 3;
    let attention_ids = app
        .attention
        .iter()
        .map(|item| item.session_id.as_str())
        .collect::<HashSet<_>>();
    let mut lines = vec![Line::styled(
        format!("Needs attention ({})", app.attention.len()),
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )];
    if app.attention.is_empty() {
        lines.push(Line::from("  Nothing needs attention."));
    } else {
        for item in app.attention.iter().take(MAX_VISIBLE_ATTENTION) {
            lines.push(Line::from(format!(
                "  ! {} · {:?}{}",
                item.session_id,
                item.reason,
                if item.active_work_uncertain {
                    " · work uncertain"
                } else {
                    ""
                }
            )));
        }
        if app.attention.len() > MAX_VISIBLE_ATTENTION {
            lines.push(Line::from(format!(
                "  … and {} more; inspect sessions below",
                app.attention.len() - MAX_VISIBLE_ATTENTION
            )));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::styled(
        format!("Sessions ({})", app.sessions.len()),
        Style::default().add_modifier(Modifier::BOLD),
    ));
    let mut selected_line = None;
    if app.sessions.is_empty() {
        lines.push(Line::from("  No sessions."));
    } else {
        for (index, session) in app.sessions.iter().enumerate() {
            let selected = index == app.selected;
            let attention = attention_ids.contains(session.session_id.as_str());
            let marker = if selected { ">" } else { " " };
            let attention_marker = if attention { "!" } else { " " };
            let line = format!(
                "{marker}{attention_marker} {} · {:?} · queued {} · active {}",
                session.session_id,
                session.status,
                session.queued_count,
                session.has_active_message,
            );
            if selected {
                selected_line = Some(lines.len());
            }
            lines.push(if selected {
                Line::styled(line, Style::default().add_modifier(Modifier::REVERSED))
            } else {
                Line::from(line)
            });
        }
    }
    if let Some(error) = &app.error {
        lines.push(Line::from(""));
        lines.push(Line::styled(
            format!("Connection/error: {error} · press r to retry"),
            Style::default().fg(Color::Red),
        ));
    }
    (lines, selected_line)
}

fn overview_scroll(selected_line: usize, viewport_height: u16) -> u16 {
    let height = viewport_height as usize;
    selected_line
        .saturating_add(1)
        .saturating_sub(height)
        .min(u16::MAX as usize) as u16
}

fn detail_lines(
    snapshot: &SessionSnapshot,
    workspace_review: Option<&WorkspaceFileReview>,
    error: Option<&str>,
) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(format!("Status: {:?}", snapshot.status)),
        Line::from(format!("Event sequence: {}", snapshot.event_sequence)),
        Line::from(format!(
            "Active work uncertain: {}",
            snapshot.active_work_uncertain
        )),
        Line::from(format!(
            "Recovery needs revalidation: {}",
            snapshot.recovery_needs_revalidation
        )),
        Line::from(format!("Attached clients: {}", snapshot.attached_clients)),
        Line::from(""),
    ];
    if let Some(active) = &snapshot.active_message {
        lines.push(Line::styled(
            format!("Active: {} · {}", active.id, active.text),
            Style::default().fg(Color::Cyan),
        ));
    }
    lines.push(Line::styled(
        format!("Queued messages ({})", snapshot.queued_messages.len()),
        Style::default().add_modifier(Modifier::BOLD),
    ));
    if snapshot.queued_messages.is_empty() {
        lines.push(Line::from("  No queued messages."));
    } else {
        for message in &snapshot.queued_messages {
            lines.push(Line::from(format!("  {} · {}", message.id, message.text)));
        }
    }
    if let Some(review) = workspace_review {
        lines.push(Line::from(""));
        lines.push(Line::styled(
            format!("Workspace review ({})", review.files.len()),
            Style::default().add_modifier(Modifier::BOLD),
        ));
        lines.push(Line::from(
            "  Read-only report; this does not authorize resuming.",
        ));
        if review.files.is_empty() {
            lines.push(Line::from("  No explicitly observed files."));
        } else {
            for file in &review.files {
                lines.push(Line::from(format!(
                    "  {:?} · {}",
                    file.path,
                    workspace_review_status_label(file.status)
                )));
            }
        }
    }
    if let Some(error) = error {
        lines.push(Line::from(""));
        lines.push(Line::styled(
            format!("Refresh error: {error} · press r to retry"),
            Style::default().fg(Color::Red),
        ));
    }
    lines
}

fn workspace_review_status_label(status: WorkspaceFileReviewStatus) -> &'static str {
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

fn event_lines(
    events: &[SessionEventRecord],
    cursor: i64,
    has_more: bool,
    error: Option<&str>,
) -> Vec<Line<'static>> {
    let mut lines = vec![Line::styled(
        format!("Event history · {} loaded · cursor {cursor}", events.len()),
        Style::default().add_modifier(Modifier::BOLD),
    )];
    if events.is_empty() {
        lines.push(Line::from("  No events available."));
    } else {
        for event in events {
            lines.push(Line::from(format!(
                "{} · {} · {:?}",
                event.sequence, event.occurred_at_ms, event.event
            )));
        }
    }
    if has_more {
        lines.push(Line::styled(
            format!("More events available after cursor {cursor}; press n."),
            Style::default().fg(Color::Yellow),
        ));
    }
    if let Some(error) = error {
        lines.push(Line::styled(
            format!("Event read failed: {error} · press r to reload"),
            Style::default().fg(Color::Red),
        ));
    }
    lines
}

pub(super) fn run(socket_path: &Path) -> Result<(), Box<dyn Error>> {
    let mut app = App::new(socket_path);
    app.refresh();
    let mut terminal: Terminal<CrosstermBackend<io::Stdout>> = ratatui::init();
    let result = (|| {
        loop {
            terminal.draw(|frame| draw(frame, &app))?;
            if let Event::Key(key) = event::read()?
                && app.handle_key(key)
            {
                break;
            }
        }
        Ok::<_, Box<dyn Error>>(())
    })();
    ratatui::restore();
    result
}

#[cfg(test)]
mod tests {
    use super::{App, workspace_review_status_label};
    use carapana_daemon::{DaemonRuntime, IpcServer};
    use carapana_protocol::{
        AttentionItem, AttentionReason, Autonomy, QueuedMessage, Selection, SessionStatus,
        SessionSummary, WorkMode, WorkspaceFileReviewStatus,
    };
    use carapana_storage::{SessionRegistry, WorkspaceMetadata};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend};
    use std::{
        fs,
        os::unix::{ffi::OsStringExt, fs::PermissionsExt},
        path::PathBuf,
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicU64, Ordering},
        },
        thread,
        time::SystemTime,
    };

    struct PrivateDir(PathBuf);

    #[test]
    fn should_keep_workspace_review_status_labels_stable() {
        use WorkspaceFileReviewStatus as Status;

        for (status, expected) in [
            (Status::Unchanged, "unchanged"),
            (Status::Changed, "changed"),
            (Status::Missing, "missing"),
            (Status::Unreadable, "unreadable"),
            (Status::Unsafe, "unsafe"),
            (Status::TooLarge, "too_large"),
            (Status::Unavailable, "unavailable"),
        ] {
            assert_eq!(workspace_review_status_label(status), expected);
        }
    }

    impl PrivateDir {
        fn new() -> Self {
            static NEXT_ID: AtomicU64 = AtomicU64::new(0);
            let nonce = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "carapana-cli-tui-{}-{nonce}-{id}",
                std::process::id()
            ));
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

    fn message_with_id(id: impl Into<String>) -> QueuedMessage {
        QueuedMessage {
            id: id.into(),
            text: "preserve this queued item".into(),
            origin: "tui-test".into(),
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
    fn should_render_a_narrow_read_only_overview_and_empty_connection_state() {
        let directory = PrivateDir::new();
        let mut app = App::new(&directory.0.join("missing.sock"));
        app.error = Some("daemon unavailable".into());
        let backend = TestBackend::new(50, 16);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| super::draw(frame, &app)).unwrap();

        app.error = None;
        app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE));
        assert!(app.error.is_some());
        assert!(app.sessions.is_empty());
        assert!(app.attention.is_empty());
    }

    #[test]
    fn should_keep_selected_sessions_visible_in_an_80_by_24_viewport() {
        let directory = PrivateDir::new();
        let mut app = App::new(&directory.0.join("unused.sock"));
        app.sessions = (0..40)
            .map(|index| SessionSummary {
                session_id: format!("session-{index:02}"),
                status: SessionStatus::Paused,
                queued_count: 0,
                has_active_message: false,
                recovery_needs_revalidation: false,
                active_work_uncertain: false,
                attached_clients: 0,
                updated_at_ms: index,
            })
            .collect();
        app.attention = (0..10)
            .map(|index| AttentionItem {
                session_id: format!("session-{index:02}"),
                reason: AttentionReason::RecoveryReview,
                active_work_uncertain: false,
                updated_at_ms: index,
                event_sequence: index,
            })
            .collect();
        app.selected = 39;

        let (lines, selected_line) = super::overview_lines(&app);
        let selected_line = selected_line.unwrap();
        let scroll = super::overview_scroll(selected_line, 16) as usize;
        assert!(selected_line >= scroll);
        assert!(selected_line - scroll < 16);
        assert_eq!(
            lines
                .iter()
                .filter(|line| line.to_string().contains("; inspect sessions below"))
                .count(),
            1
        );

        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|frame| super::draw(frame, &app)).unwrap();
    }

    #[test]
    fn should_refresh_navigate_and_inspect_without_mutating_the_session() {
        let directory = PrivateDir::new();
        let database_path = directory.0.join("sessions.sqlite3");
        let mut registry = SessionRegistry::open(&database_path).unwrap();
        registry.create("session-1", 10).unwrap();
        let queued_messages = (0..20)
            .map(|index| message_with_id(format!("queued-{index:02}")))
            .collect::<Vec<_>>();
        for (index, message) in queued_messages.iter().cloned().enumerate() {
            registry
                .enqueue("session-1", message, 11 + index as i64)
                .unwrap();
        }
        let workspace_path = directory.0.join("workspace");
        fs::create_dir(&workspace_path).unwrap();
        fs::write(workspace_path.join("review.txt"), b"original content").unwrap();
        fs::write(workspace_path.join("line\nbreak.txt"), b"stat-only file").unwrap();
        fs::write(workspace_path.join("tab\tbreak.txt"), b"stat-only tab file").unwrap();
        fs::write(
            workspace_path.join("escape\u{1b}[31m.txt"),
            b"stat-only control file",
        )
        .unwrap();
        let non_utf8_name = std::ffi::OsString::from_vec(vec![
            b'n', 0xff, b'a', b'm', b'e', b'.', b't', b'x', b't',
        ]);
        let non_utf8_path = PathBuf::from(&non_utf8_name);
        fs::write(
            workspace_path.join(&non_utf8_path),
            b"synthetic TUI filename contents",
        )
        .unwrap();
        registry
            .create_with_workspace(
                "session-2",
                WorkspaceMetadata::capture(&workspace_path).unwrap(),
                8,
            )
            .unwrap();
        registry
            .observe_workspace_file_with_hash("session-2", "review.txt", 9)
            .unwrap();
        registry
            .observe_workspace_file("session-2", "line\nbreak.txt", 10)
            .unwrap();
        registry
            .observe_workspace_file("session-2", "tab\tbreak.txt", 11)
            .unwrap();
        registry
            .observe_workspace_file("session-2", "escape\u{1b}[31m.txt", 12)
            .unwrap();
        registry
            .observe_workspace_file("session-2", &non_utf8_path, 13)
            .unwrap();
        drop(registry);
        fs::write(workspace_path.join("review.txt"), b"changed content").unwrap();

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

        let mut app = App::new(&socket_path);
        app.refresh();
        assert_eq!(app.sessions.len(), 2);
        assert!(app.attention.is_empty());
        assert_eq!(app.sessions[0].status, SessionStatus::Paused);

        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.selected, 1);
        let selected_session_id = app.sessions[app.selected].session_id.clone();
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.detail.as_ref().unwrap().session_id, selected_session_id);
        app.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE));
        let review = app
            .workspace_review
            .as_ref()
            .unwrap_or_else(|| panic!("workspace review missing: {:?}", app.error));
        assert_eq!(review.files.len(), 5);
        assert_eq!(review.files[0].path, "review.txt");
        assert_eq!(review.files[0].status, WorkspaceFileReviewStatus::Changed);
        assert_eq!(review.files[1].path, "line\nbreak.txt");
        assert_eq!(review.files[1].status, WorkspaceFileReviewStatus::Unchanged);
        assert_eq!(review.files[2].path, "tab\tbreak.txt");
        assert_eq!(review.files[2].status, WorkspaceFileReviewStatus::Unchanged);
        assert_eq!(review.files[3].path, "escape\u{1b}[31m.txt");
        assert_eq!(review.files[3].status, WorkspaceFileReviewStatus::Unchanged);
        let safe_non_utf8_name = non_utf8_name.to_string_lossy();
        assert_eq!(review.files[4].path, safe_non_utf8_name);
        assert_eq!(review.files[4].status, WorkspaceFileReviewStatus::Unchanged);
        let detail = super::detail_lines(
            app.detail.as_ref().unwrap(),
            app.workspace_review.as_ref(),
            None,
        );
        assert!(
            detail
                .iter()
                .any(|line| line.to_string().contains("review.txt"))
        );
        assert!(
            detail
                .iter()
                .any(|line| line.to_string().contains("changed"))
        );
        assert!(
            detail
                .iter()
                .any(|line| line.to_string().contains("line\\nbreak.txt"))
        );
        assert!(
            detail
                .iter()
                .any(|line| line.to_string().contains("tab\\tbreak.txt"))
        );
        assert!(
            detail
                .iter()
                .any(|line| line.to_string().contains("escape\\u{1b}[31m.txt"))
        );
        assert!(
            detail
                .iter()
                .any(|line| line.to_string().contains(safe_non_utf8_name.as_ref()))
        );
        assert!(
            detail
                .iter()
                .all(|line| !line.to_string().contains("synthetic TUI filename contents"))
        );
        assert!(detail.iter().all(|line| {
            !line
                .to_string()
                .contains(workspace_path.to_string_lossy().as_ref())
        }));
        assert!(
            detail
                .iter()
                .all(|line| !line.to_string().contains('\u{1b}'))
        );
        assert!(
            detail
                .iter()
                .any(|line| line.to_string().contains("does not authorize resuming"))
        );
        let mut review_terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        review_terminal
            .draw(|frame| super::draw(frame, &app))
            .unwrap();
        let rendered = review_terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<Vec<_>>()
            .join("");
        assert!(rendered.contains("line\\nbreak.txt"));
        assert!(rendered.contains("tab\\tbreak.txt"));
        assert!(rendered.contains("escape\\u{1b}[31m.txt"));
        assert!(rendered.contains(safe_non_utf8_name.as_ref()));
        assert!(rendered.contains("changed"));
        assert!(rendered.contains("unchanged"));
        assert!(!rendered.contains('\n'));
        assert!(!rendered.contains('\t'));
        assert!(!rendered.contains('\u{1b}'));
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        let session_one_index = app
            .sessions
            .iter()
            .position(|session| session.session_id == "session-1")
            .unwrap();
        if app.selected < session_one_index {
            app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        } else if app.selected > session_one_index {
            app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        }
        assert_eq!(app.sessions[app.selected].session_id, "session-1");

        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(
            app.detail.as_ref().unwrap().queued_messages,
            queued_messages
        );
        assert!(app.error.is_none());
        app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE));
        assert!(app.event_mode);
        assert_eq!(app.events.len(), 16);
        assert!(app.event_has_more);
        let first_page_last_sequence = app.events.last().unwrap().sequence;
        app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
        assert_eq!(app.events.len(), 21);
        assert!(!app.event_has_more);
        assert_eq!(app.events[16].sequence, first_page_last_sequence + 1);
        assert!(
            app.events
                .windows(2)
                .all(|pair| pair[1].sequence == pair[0].sequence + 1)
        );
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.detail_scroll, 1);
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|frame| super::draw(frame, &app)).unwrap();
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(!app.event_mode);
        assert!(app.detail.is_some());
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.detail.is_none());

        app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE));
        assert_eq!(app.sessions[0].attached_clients, 0);
        let registry = SessionRegistry::open(&database_path).unwrap();
        let session = registry.get("session-1").unwrap();
        assert_eq!(
            session.status,
            carapana_storage::StoredSessionStatus::Paused
        );
        assert_eq!(session.queued_messages, queued_messages);
        let reviewed_session = registry.get("session-2").unwrap();
        assert_eq!(reviewed_session.event_sequence, 6);
        assert!(!reviewed_session.recovery_needs_revalidation);

        shutdown.store(true, Ordering::Release);
        server_thread.join().unwrap();
        assert!(!socket_path.exists());
    }

    #[test]
    fn should_keep_recovered_session_paused_while_tui_displays_workspace_review() {
        let directory = PrivateDir::new();
        let database_path = directory.0.join("sessions.sqlite3");
        let workspace_path = directory.0.join("workspace");
        fs::create_dir(&workspace_path).unwrap();
        fs::write(workspace_path.join("source.txt"), b"original contents").unwrap();
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
                .observe_workspace_file_with_hash("session-1", "source.txt", 11)
                .unwrap();
            registry
                .enqueue("session-1", message_with_id("active-work"), 12)
                .unwrap();
            registry
                .enqueue("session-1", message_with_id("queued-work"), 13)
                .unwrap();
            registry.start_next("session-1", 14).unwrap();
        }
        fs::write(workspace_path.join("source.txt"), b"operator edit").unwrap();

        let mut runtime = DaemonRuntime::open(&database_path, 20).unwrap();
        assert_eq!(runtime.startup_recovered().len(), 1);
        let socket_path = directory.0.join("daemon.sock");
        let server = IpcServer::bind(&socket_path).unwrap();
        let shutdown = Arc::new(AtomicBool::new(false));
        let server_shutdown = Arc::clone(&shutdown);
        let server_thread = thread::spawn(move || {
            server
                .serve_until(&mut runtime, server_shutdown.as_ref())
                .unwrap();
        });

        let mut app = App::new(&socket_path);
        app.refresh();
        assert_eq!(app.attention.len(), 1);
        assert!(app.sessions[0].recovery_needs_revalidation);
        assert!(app.sessions[0].active_work_uncertain);
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let snapshot = app.detail.as_ref().unwrap();
        assert_eq!(snapshot.status, SessionStatus::Paused);
        assert!(snapshot.recovery_needs_revalidation);
        assert!(snapshot.active_work_uncertain);
        assert_eq!(snapshot.active_message.as_ref().unwrap().id, "active-work");
        assert_eq!(snapshot.queued_messages[0].id, "queued-work");

        app.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE));
        assert_eq!(
            app.workspace_review.as_ref().unwrap().files[0].status,
            WorkspaceFileReviewStatus::Changed
        );
        app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE));
        let snapshot = app.detail.as_ref().unwrap();
        assert_eq!(snapshot.status, SessionStatus::Paused);
        assert!(snapshot.recovery_needs_revalidation);
        assert!(snapshot.active_work_uncertain);
        assert_eq!(snapshot.active_message.as_ref().unwrap().id, "active-work");
        assert_eq!(snapshot.queued_messages[0].id, "queued-work");

        let detail = super::detail_lines(snapshot, None, None);
        assert!(detail.iter().any(|line| {
            line.to_string()
                .contains("Recovery needs revalidation: true")
        }));
        assert!(
            detail
                .iter()
                .any(|line| line.to_string().contains("Active work uncertain: true"))
        );
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal.draw(|frame| super::draw(frame, &app)).unwrap();
        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<Vec<_>>()
            .join("");
        assert!(rendered.contains("v review files"));
        assert!(!rendered.contains("resume"));

        shutdown.store(true, Ordering::Release);
        server_thread.join().unwrap();
        let registry = SessionRegistry::open(&database_path).unwrap();
        let session = registry.get("session-1").unwrap();
        assert_eq!(
            session.status,
            carapana_storage::StoredSessionStatus::Paused
        );
        assert!(session.recovery_needs_revalidation);
        assert!(session.active_work_uncertain);
        assert_eq!(session.active_message, Some(message_with_id("active-work")));
        assert_eq!(
            session.queued_messages,
            vec![message_with_id("queued-work")]
        );
        assert_eq!(session.event_sequence, 6);
    }
}
