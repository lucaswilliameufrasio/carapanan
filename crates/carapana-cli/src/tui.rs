use std::{
    collections::HashSet,
    error::Error,
    io,
    path::{Path, PathBuf},
};

use carapana_daemon::{IpcAttachment, ipc_request};
use carapana_protocol::{
    AttentionItem, DaemonRequest, DaemonResponse, Envelope, SessionSnapshot, SessionSummary,
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
                        self.error = None;
                    }
                    Err(error) => self.error = Some(error.to_string()),
                }
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('q') => true,
            KeyCode::Char('r') => {
                if self.detail.is_some() {
                    let session_id = self.detail.as_ref().unwrap().session_id.clone();
                    match IpcAttachment::attach(&self.socket_path, session_id) {
                        Ok(attachment) => {
                            let snapshot = attachment.snapshot().clone();
                            match attachment.detach() {
                                Ok(_) => {
                                    self.detail = Some(snapshot);
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
                self.detail = None;
                self.error = None;
                false
            }
            KeyCode::Up if self.detail.is_none() => {
                self.selected = self.selected.saturating_sub(1);
                false
            }
            KeyCode::Down if self.detail.is_none() => {
                if !self.sessions.is_empty() {
                    self.selected = (self.selected + 1).min(self.sessions.len() - 1);
                }
                false
            }
            KeyCode::Enter if self.detail.is_none() => {
                self.open_selected();
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
        format!("Carapanã · {}", snapshot.session_id)
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
    let lines = if let Some(snapshot) = &app.detail {
        detail_lines(snapshot, app.error.as_deref())
    } else {
        overview_lines(app)
    };
    frame.render_widget(
        Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }),
        inner,
    );

    let footer_text = if app.detail.is_some() {
        "Esc back · r refresh · q quit"
    } else {
        "↑/↓ select · Enter inspect · r refresh · q quit"
    };
    frame.render_widget(
        Paragraph::new(footer_text).block(Block::default().borders(Borders::ALL)),
        footer,
    );
}

fn overview_lines(app: &App) -> Vec<Line<'static>> {
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
        for item in &app.attention {
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
    }

    lines.push(Line::from(""));
    lines.push(Line::styled(
        format!("Sessions ({})", app.sessions.len()),
        Style::default().add_modifier(Modifier::BOLD),
    ));
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
    lines
}

fn detail_lines(snapshot: &SessionSnapshot, error: Option<&str>) -> Vec<Line<'static>> {
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
    if let Some(error) = error {
        lines.push(Line::from(""));
        lines.push(Line::styled(
            format!("Refresh error: {error} · press r to retry"),
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
    use super::App;
    use carapana_daemon::{DaemonRuntime, IpcServer};
    use carapana_protocol::{Autonomy, QueuedMessage, Selection, SessionStatus, WorkMode};
    use carapana_storage::SessionRegistry;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend};
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

    fn message() -> QueuedMessage {
        QueuedMessage {
            id: "queued-1".into(),
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
    fn should_refresh_navigate_and_inspect_without_mutating_the_session() {
        let directory = PrivateDir::new();
        let database_path = directory.0.join("sessions.sqlite3");
        let mut registry = SessionRegistry::open(&database_path).unwrap();
        registry.create("session-1", 10).unwrap();
        registry.enqueue("session-1", message(), 11).unwrap();
        registry.create("session-2", 12).unwrap();
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
            vec![message()]
        );
        assert!(app.error.is_none());
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
        assert_eq!(session.queued_messages, vec![message()]);

        shutdown.store(true, Ordering::Release);
        server_thread.join().unwrap();
        assert!(!socket_path.exists());
    }
}
