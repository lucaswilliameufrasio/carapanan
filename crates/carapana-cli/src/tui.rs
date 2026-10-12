use std::{
    collections::HashSet,
    error::Error,
    io,
    path::{Path, PathBuf},
};

use carapana_daemon::{IpcAttachment, ipc_request};
use carapana_protocol::{
    AttentionItem, Autonomy, DaemonRequest, DaemonResponse, Envelope, MAX_QUEUED_MESSAGE_BYTES,
    MAX_SELECTION_FIELD_BYTES, MAX_WORKSPACE_PATH_BYTES, MessageOrigin, Selection,
    SessionEventRecord, SessionSnapshot, SessionSummary, WorkMode, WorkspaceFileReview,
    WorkspaceFileReviewStatus,
};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Text},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

struct App {
    socket_path: PathBuf,
    sessions: Vec<SessionSummary>,
    attention: Vec<AttentionItem>,
    selected: usize,
    detail: Option<SessionSnapshot>,
    workspace_review: Option<WorkspaceFileReview>,
    selected_review_file: usize,
    dialog: DialogState,
    notice: Option<String>,
    detail_scroll: usize,
    event_mode: bool,
    events: Vec<SessionEventRecord>,
    event_cursor: i64,
    event_has_more: bool,
    error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum DialogState {
    None,
    AcknowledgePath {
        session_id: String,
        path: String,
        status: WorkspaceFileReviewStatus,
    },
    ConfirmResume {
        session_id: String,
    },
    CreateWorkspace {
        input: String,
    },
    QueueMessage {
        session_id: String,
        stage: QueueStage,
        input: String,
        draft: QueueDraft,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum QueueStage {
    Text,
    Profile,
    Work,
    Autonomy,
    Provider,
    Model,
    Variant,
    Confirm,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct QueueDraft {
    text: String,
    profile: String,
    work: Option<WorkMode>,
    autonomy: Option<Autonomy>,
    provider: String,
    model: String,
    variant: String,
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
            selected_review_file: 0,
            dialog: DialogState::None,
            notice: None,
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
                self.selected_review_file = self
                    .selected_review_file
                    .min(review.files.len().saturating_sub(1));
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

    fn refresh_detail(&mut self) {
        let Some(session_id) = self
            .detail
            .as_ref()
            .map(|snapshot| snapshot.session_id.clone())
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

    fn hibernate_selected(&mut self) {
        let Some(snapshot) = self.detail.as_ref() else {
            return;
        };
        if snapshot.status != carapana_protocol::SessionStatus::Paused
            || snapshot.active_message.is_some()
            || snapshot.recovery_needs_revalidation
            || snapshot.active_work_uncertain
        {
            self.error = Some("only an idle paused session can be hibernated".into());
            return;
        }
        let session_id = snapshot.session_id.clone();
        match ipc_request(
            &self.socket_path,
            Envelope::new(DaemonRequest::HibernateSession {
                session_id: session_id.clone(),
            }),
        )
        .map(|response| response.payload)
        {
            Ok(DaemonResponse::SessionHibernated {
                session_id: returned_id,
            }) if returned_id == session_id => {
                self.refresh();
                self.refresh_detail();
                self.notice = Some(
                    "Session hibernated; queue is preserved and no resources or processes were changed."
                        .into(),
                );
            }
            Ok(DaemonResponse::Error { code }) => {
                self.error = Some(format!("session hibernation failed: {code:?}"));
            }
            Ok(_) => self.error = Some("daemon returned an invalid hibernation response".into()),
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    fn wake_selected(&mut self) {
        let Some(snapshot) = self.detail.as_ref() else {
            return;
        };
        if snapshot.status != carapana_protocol::SessionStatus::Hibernated {
            self.error = Some("only a hibernated session can be woken".into());
            return;
        }
        let session_id = snapshot.session_id.clone();
        match ipc_request(
            &self.socket_path,
            Envelope::new(DaemonRequest::WakeSession {
                session_id: session_id.clone(),
            }),
        )
        .map(|response| response.payload)
        {
            Ok(DaemonResponse::SessionWoken {
                session_id: returned_id,
            }) if returned_id == session_id => {
                self.refresh();
                self.refresh_detail();
                self.notice = Some(
                    "Session is paused again; queue is preserved and no work was started.".into(),
                );
            }
            Ok(DaemonResponse::Error { code }) => {
                self.error = Some(format!("session wake failed: {code:?}"));
            }
            Ok(_) => self.error = Some("daemon returned an invalid wake response".into()),
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    fn acknowledge_selected_path(&mut self) {
        let Some(snapshot) = self.detail.as_ref() else {
            return;
        };
        if !snapshot.recovery_needs_revalidation {
            self.error = Some("session has no pending recovery review".into());
            return;
        }
        let Some(file) = self
            .workspace_review
            .as_ref()
            .and_then(|review| review.files.get(self.selected_review_file))
        else {
            self.error = Some("review a workspace path before acknowledging it".into());
            return;
        };
        if !matches!(
            file.status,
            WorkspaceFileReviewStatus::Changed
                | WorkspaceFileReviewStatus::Missing
                | WorkspaceFileReviewStatus::TooLarge
        ) {
            self.error =
                Some("only changed, missing, or too-large paths need acknowledgment".into());
            return;
        }
        self.dialog = DialogState::AcknowledgePath {
            session_id: snapshot.session_id.clone(),
            path: file.path.clone(),
            status: file.status,
        };
        self.error = None;
    }

    fn begin_recovery_resume(&mut self) {
        let Some(snapshot) = self.detail.as_ref() else {
            return;
        };
        if !snapshot.recovery_needs_revalidation {
            self.error = Some("session has no pending recovery review".into());
            return;
        }
        let session_id = snapshot.session_id.clone();
        self.review_workspace();
        if self.workspace_review.is_some() && self.error.is_none() {
            self.dialog = DialogState::ConfirmResume { session_id };
        }
    }

    fn confirm_recovery_dialog(&mut self) {
        let dialog = std::mem::replace(&mut self.dialog, DialogState::None);
        match dialog {
            DialogState::None => {}
            DialogState::AcknowledgePath {
                session_id,
                path,
                status: _,
            } => {
                match ipc_request(
                    &self.socket_path,
                    Envelope::new(DaemonRequest::AcknowledgeRecoveryPath { session_id, path }),
                )
                .map(|response| response.payload)
                {
                    Ok(DaemonResponse::RecoveryPathAcknowledged { .. }) => {
                        self.refresh_detail();
                        self.review_workspace();
                    }
                    Ok(other) => {
                        self.error = Some(format!("recovery path acknowledgment failed: {other:?}"))
                    }
                    Err(error) => self.error = Some(error.to_string()),
                }
            }
            DialogState::ConfirmResume { session_id } => {
                match ipc_request(
                    &self.socket_path,
                    Envelope::new(DaemonRequest::ResumeRecovered { session_id }),
                )
                .map(|response| response.payload)
                {
                    Ok(DaemonResponse::RecoveryResumed { .. }) => {
                        self.refresh();
                        self.refresh_detail();
                        self.workspace_review = None;
                    }
                    Ok(other) => self.error = Some(format!("recovery resume failed: {other:?}")),
                    Err(error) => self.error = Some(error.to_string()),
                }
            }
            other => self.dialog = other,
        }
    }

    fn begin_create_workspace(&mut self) {
        self.dialog = DialogState::CreateWorkspace {
            input: String::new(),
        };
        self.error = None;
        self.notice = None;
    }

    fn begin_queue_message(&mut self) {
        let Some(session_id) = self
            .detail
            .as_ref()
            .map(|snapshot| snapshot.session_id.clone())
        else {
            return;
        };
        self.dialog = DialogState::QueueMessage {
            session_id,
            stage: QueueStage::Text,
            input: String::new(),
            draft: QueueDraft::default(),
        };
        self.error = None;
        self.notice = None;
    }

    fn submit_create_workspace(&mut self, input: String) {
        if input.trim().is_empty() {
            self.error = Some("enter a workspace root".into());
            self.dialog = DialogState::CreateWorkspace { input };
            return;
        }
        let retry_input = input.clone();
        let selected_path = PathBuf::from(input);
        let selected_path = if selected_path.is_absolute() {
            selected_path
        } else {
            match std::env::current_dir() {
                Ok(current_dir) => current_dir.join(selected_path),
                Err(_) => {
                    self.error = Some("could not resolve the selected workspace root".into());
                    self.dialog = DialogState::CreateWorkspace { input: retry_input };
                    return;
                }
            }
        };
        let Some(workspace_path) = selected_path.to_str() else {
            self.error = Some("workspace root must be valid UTF-8".into());
            self.dialog = DialogState::CreateWorkspace { input: retry_input };
            return;
        };

        match ipc_request(
            &self.socket_path,
            Envelope::new(DaemonRequest::CreateSession {
                workspace_path: workspace_path.to_owned(),
            }),
        )
        .map(|response| response.payload)
        {
            Ok(DaemonResponse::SessionCreated { session_id }) => {
                self.refresh();
                if let Some(index) = self
                    .sessions
                    .iter()
                    .position(|session| session.session_id == session_id)
                {
                    self.selected = index;
                    self.open_selected();
                } else {
                    self.error =
                        Some("created session was not present in the refreshed list".into());
                }
                self.notice = Some(format!(
                    "Created paused session {session_id}; no provider or tools were started."
                ));
            }
            Ok(DaemonResponse::Error { code }) => {
                self.error = Some(format!("session creation failed: {code:?}"));
                self.dialog = DialogState::CreateWorkspace { input: retry_input };
            }
            Ok(_) => {
                self.error = Some("daemon returned an invalid session-creation response".into());
                self.dialog = DialogState::CreateWorkspace { input: retry_input };
            }
            Err(error) => {
                self.error = Some(error.to_string());
                self.dialog = DialogState::CreateWorkspace { input: retry_input };
            }
        }
    }

    fn handle_dialog_key(&mut self, key: KeyEvent) {
        if matches!(
            &self.dialog,
            DialogState::AcknowledgePath { .. } | DialogState::ConfirmResume { .. }
        ) {
            match key.code {
                KeyCode::Char('y') => self.confirm_recovery_dialog(),
                KeyCode::Char('n') | KeyCode::Esc => {
                    self.dialog = DialogState::None;
                    self.error = None;
                }
                _ => {}
            }
            return;
        }

        let dialog = std::mem::replace(&mut self.dialog, DialogState::None);
        match dialog {
            DialogState::CreateWorkspace { mut input } => match key.code {
                KeyCode::Esc => {
                    self.error = None;
                }
                KeyCode::Backspace => {
                    input.pop();
                    self.dialog = DialogState::CreateWorkspace { input };
                    self.error = None;
                }
                KeyCode::Enter => self.submit_create_workspace(input),
                KeyCode::Char(character)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    if character.is_control() {
                        self.error =
                            Some("control characters are not accepted in a workspace path".into());
                    } else if input.len().saturating_add(character.len_utf8())
                        > MAX_WORKSPACE_PATH_BYTES
                    {
                        self.error = Some("workspace path reached its maximum size".into());
                    } else {
                        input.push(character);
                        self.error = None;
                    }
                    self.dialog = DialogState::CreateWorkspace { input };
                }
                _ => self.dialog = DialogState::CreateWorkspace { input },
            },
            DialogState::QueueMessage {
                session_id,
                stage,
                mut input,
                draft,
            } => self.handle_queue_dialog_key(session_id, stage, &mut input, draft, key),
            other => self.dialog = other,
        }
    }

    fn handle_queue_dialog_key(
        &mut self,
        session_id: String,
        stage: QueueStage,
        input: &mut String,
        draft: QueueDraft,
        key: KeyEvent,
    ) {
        if stage == QueueStage::Confirm {
            match key.code {
                KeyCode::Char('y') => self.submit_queue_message(session_id, draft),
                KeyCode::Char('n') | KeyCode::Esc => {
                    self.error = None;
                }
                _ => {
                    self.dialog = DialogState::QueueMessage {
                        session_id,
                        stage,
                        input: input.clone(),
                        draft,
                    };
                }
            }
            return;
        }

        match key.code {
            KeyCode::Esc => self.error = None,
            KeyCode::Backspace => {
                input.pop();
                self.error = None;
                self.dialog = DialogState::QueueMessage {
                    session_id,
                    stage,
                    input: input.clone(),
                    draft,
                };
            }
            KeyCode::Enter => self.advance_queue_stage(session_id, stage, input.clone(), draft),
            KeyCode::Char('j')
                if stage == QueueStage::Text && key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                self.append_queue_input(session_id, stage, input, draft, '\n');
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.append_queue_input(session_id, stage, input, draft, character);
            }
            _ => {
                self.dialog = DialogState::QueueMessage {
                    session_id,
                    stage,
                    input: input.clone(),
                    draft,
                };
            }
        }
    }

    fn append_queue_input(
        &mut self,
        session_id: String,
        stage: QueueStage,
        input: &mut String,
        draft: QueueDraft,
        character: char,
    ) {
        let maximum = if stage == QueueStage::Text {
            MAX_QUEUED_MESSAGE_BYTES
        } else {
            MAX_SELECTION_FIELD_BYTES
        };
        if input.len().saturating_add(character.len_utf8()) > maximum {
            self.error = Some("input reached its maximum size".into());
        } else if character.is_control() && character != '\n' {
            self.error = Some("control characters are not accepted in this field".into());
        } else {
            input.push(character);
            self.error = None;
        }
        self.dialog = DialogState::QueueMessage {
            session_id,
            stage,
            input: input.clone(),
            draft,
        };
    }

    fn advance_queue_stage(
        &mut self,
        session_id: String,
        stage: QueueStage,
        input: String,
        mut draft: QueueDraft,
    ) {
        let next_stage = match stage {
            QueueStage::Text => {
                if input.trim().is_empty() {
                    self.error = Some("message text cannot be empty".into());
                    self.dialog = DialogState::QueueMessage {
                        session_id,
                        stage,
                        input,
                        draft,
                    };
                    return;
                }
                draft.text = input;
                QueueStage::Profile
            }
            QueueStage::Profile => {
                if !valid_selection_value(&input) {
                    self.error = Some("profile id must not be blank and must fit 256 bytes".into());
                    self.dialog = DialogState::QueueMessage {
                        session_id,
                        stage,
                        input,
                        draft,
                    };
                    return;
                }
                draft.profile = input;
                QueueStage::Work
            }
            QueueStage::Work => {
                draft.work = match input.trim() {
                    "plan" => Some(WorkMode::Plan),
                    "execute" => Some(WorkMode::Execute),
                    _ => None,
                };
                if draft.work.is_none() {
                    self.error = Some("work must be `plan` or `execute`".into());
                    self.dialog = DialogState::QueueMessage {
                        session_id,
                        stage,
                        input,
                        draft,
                    };
                    return;
                }
                QueueStage::Autonomy
            }
            QueueStage::Autonomy => {
                draft.autonomy = match input.trim() {
                    "ask" => Some(Autonomy::Ask),
                    "auto" => Some(Autonomy::Auto),
                    "yolo" => Some(Autonomy::Yolo),
                    _ => None,
                };
                if draft.autonomy.is_none() {
                    self.error = Some("autonomy must be `ask`, `auto`, or `yolo`".into());
                    self.dialog = DialogState::QueueMessage {
                        session_id,
                        stage,
                        input,
                        draft,
                    };
                    return;
                }
                QueueStage::Provider
            }
            QueueStage::Provider => {
                if !valid_selection_value(&input) {
                    self.error =
                        Some("provider id must not be blank and must fit 256 bytes".into());
                    self.dialog = DialogState::QueueMessage {
                        session_id,
                        stage,
                        input,
                        draft,
                    };
                    return;
                }
                draft.provider = input;
                QueueStage::Model
            }
            QueueStage::Model => {
                if !valid_selection_value(&input) {
                    self.error = Some("model id must not be blank and must fit 256 bytes".into());
                    self.dialog = DialogState::QueueMessage {
                        session_id,
                        stage,
                        input,
                        draft,
                    };
                    return;
                }
                draft.model = input;
                QueueStage::Variant
            }
            QueueStage::Variant => {
                if !valid_selection_value(&input) {
                    self.error = Some("variant id must not be blank and must fit 256 bytes".into());
                    self.dialog = DialogState::QueueMessage {
                        session_id,
                        stage,
                        input,
                        draft,
                    };
                    return;
                }
                draft.variant = input;
                QueueStage::Confirm
            }
            QueueStage::Confirm => return,
        };
        self.dialog = DialogState::QueueMessage {
            session_id,
            stage: next_stage,
            input: String::new(),
            draft,
        };
        self.error = None;
    }

    fn submit_queue_message(&mut self, session_id: String, draft: QueueDraft) {
        let (Some(work), Some(autonomy)) = (draft.work, draft.autonomy) else {
            self.error = Some("message selection is incomplete".into());
            return;
        };
        let retry_draft = draft.clone();
        let text_length = draft.text.len();
        let selection = Selection {
            profile: draft.profile.clone(),
            work,
            autonomy,
            provider: draft.provider.clone(),
            model: draft.model.clone(),
            variant: draft.variant.clone(),
        };
        match ipc_request(
            &self.socket_path,
            Envelope::new(DaemonRequest::EnqueueMessage {
                session_id: session_id.clone(),
                text: draft.text.clone(),
                selection,
                origin: MessageOrigin::Tui,
            }),
        )
        .map(|response| response.payload)
        {
            Ok(DaemonResponse::MessageQueued {
                session_id: returned_id,
                ..
            }) if returned_id == session_id => {
                self.refresh();
                self.refresh_detail();
                self.notice = Some(format!(
                    "Queued {text_length} bytes; session remains paused and no provider or tools were started."
                ));
            }
            Ok(DaemonResponse::Error { code }) => {
                self.error = Some(format!("message enqueue failed: {code:?}"));
                self.dialog = DialogState::QueueMessage {
                    session_id,
                    stage: QueueStage::Confirm,
                    input: String::new(),
                    draft: retry_draft,
                };
            }
            Ok(_) => {
                self.error = Some("daemon returned an invalid enqueue response".into());
                self.dialog = DialogState::QueueMessage {
                    session_id,
                    stage: QueueStage::Confirm,
                    input: String::new(),
                    draft: retry_draft,
                };
            }
            Err(error) => {
                self.error = Some(error.to_string());
                self.dialog = DialogState::QueueMessage {
                    session_id,
                    stage: QueueStage::Confirm,
                    input: String::new(),
                    draft: retry_draft,
                };
            }
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> bool {
        if self.dialog != DialogState::None {
            self.handle_dialog_key(key);
            return false;
        }
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
            KeyCode::Char('c') if self.detail.is_none() => {
                self.begin_create_workspace();
                false
            }
            KeyCode::Char('m') if self.detail.is_some() && !self.event_mode => {
                self.begin_queue_message();
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
            KeyCode::Char('h') if self.detail.is_some() && !self.event_mode => {
                self.hibernate_selected();
                false
            }
            KeyCode::Char('w') if self.detail.is_some() && !self.event_mode => {
                self.wake_selected();
                false
            }
            KeyCode::Char('[') if self.detail.is_some() && !self.event_mode => {
                self.selected_review_file = self.selected_review_file.saturating_sub(1);
                false
            }
            KeyCode::Char(']') if self.detail.is_some() && !self.event_mode => {
                if let Some(review) = &self.workspace_review {
                    self.selected_review_file =
                        (self.selected_review_file + 1).min(review.files.len().saturating_sub(1));
                }
                false
            }
            KeyCode::Char('a') if self.detail.is_some() && !self.event_mode => {
                self.acknowledge_selected_path();
                false
            }
            KeyCode::Char('R') if self.detail.is_some() && !self.event_mode => {
                self.begin_recovery_resume();
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

fn valid_selection_value(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= MAX_SELECTION_FIELD_BYTES
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
        Paragraph::new("Local control · no provider or tool runner")
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
                app.selected_review_file,
                &app.dialog,
                app.notice.as_deref(),
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

    let footer_text = if matches!(
        &app.dialog,
        DialogState::AcknowledgePath { .. } | DialogState::ConfirmResume { .. }
    ) {
        "y confirm · n/Esc cancel"
    } else if matches!(&app.dialog, DialogState::CreateWorkspace { .. }) {
        "Type workspace root · Enter create paused session · Esc cancel"
    } else if let DialogState::QueueMessage { stage, .. } = &app.dialog {
        if *stage == QueueStage::Confirm {
            "y queue · n/Esc cancel"
        } else {
            "Enter next field · Ctrl+J newline in text · Backspace · Esc cancel"
        }
    } else if app.event_mode {
        "↑/↓ scroll · n next page · r reload · Esc details · q quit"
    } else if app.detail.is_some() {
        "↑/↓ · m queue · h sleep · w wake · v review · a ack · R resume · e events · Esc back · q quit"
    } else {
        "↑/↓ select · Enter inspect · c create session · r refresh · q quit"
    };
    frame.render_widget(
        Paragraph::new(footer_text).block(Block::default().borders(Borders::ALL)),
        footer,
    );
    draw_dialog(frame, &app.dialog, app.error.as_deref());
}

fn draw_dialog(frame: &mut Frame<'_>, dialog: &DialogState, error: Option<&str>) {
    let (title, mut lines) = match dialog {
        DialogState::CreateWorkspace { input } => (
            "Create paused session",
            vec![
                Line::from("Workspace root (input hidden; relative paths use current directory):"),
                Line::from(if input.is_empty() {
                    "[type path]".to_owned()
                } else {
                    "•".repeat(input.chars().count().min(64))
                }),
                Line::from("The path is sent only as input; no provider or tools will start."),
            ],
        ),
        DialogState::QueueMessage {
            stage,
            input,
            draft,
            ..
        } => {
            let prompt = match stage {
                QueueStage::Text => "Message text (Ctrl+J inserts a newline):",
                QueueStage::Profile => "Profile id:",
                QueueStage::Work => "Work mode (`plan` or `execute`; stored as data only):",
                QueueStage::Autonomy => "Autonomy (`ask`, `auto`, or `yolo`; stored as data only):",
                QueueStage::Provider => "Provider id (metadata only; no provider will start):",
                QueueStage::Model => "Model id (metadata only; no provider will start):",
                QueueStage::Variant => "Variant id:",
                QueueStage::Confirm => "Queue this message without starting work?",
            };
            let mut lines = vec![Line::from(prompt)];
            if *stage == QueueStage::Confirm {
                lines.push(Line::from(format!("Text size: {} bytes", draft.text.len())));
                lines.push(Line::from(format!(
                    "Selection: {} · {:?} · {:?} · {} · {} · {}",
                    draft.profile,
                    draft.work,
                    draft.autonomy,
                    draft.provider,
                    draft.model,
                    draft.variant
                )));
                lines.push(Line::from(
                    "The selection is not permission or provider availability.",
                ));
            } else {
                lines.push(Line::from(input.clone()));
            }
            ("Queue explicit intent", lines)
        }
        _ => return,
    };
    if let Some(error) = error {
        lines.push(Line::from(""));
        lines.push(Line::styled(
            error.to_owned(),
            Style::default().fg(Color::Red),
        ));
    }

    let area = frame.area();
    let width = area.width.saturating_sub(4).min(80);
    let height = area.height.saturating_sub(2).min(14);
    if width == 0 || height == 0 {
        return;
    }
    let popup = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(Text::from(std::mem::take(&mut lines)))
            .wrap(Wrap { trim: false })
            .block(Block::default().borders(Borders::ALL).title(title)),
        popup,
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
    if let Some(notice) = &app.notice {
        lines.push(Line::from(""));
        lines.push(Line::styled(
            notice.clone(),
            Style::default().fg(Color::Green),
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
    selected_review_file: usize,
    dialog: &DialogState,
    notice: Option<&str>,
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
            for (index, file) in review.files.iter().enumerate() {
                let line = format!(
                    "{} {:?} · {}",
                    if index == selected_review_file {
                        ">"
                    } else {
                        " "
                    },
                    file.path,
                    workspace_review_status_label(file.status)
                );
                lines.push(if index == selected_review_file {
                    Line::styled(line, Style::default().add_modifier(Modifier::REVERSED))
                } else {
                    Line::from(line)
                });
            }
        }
    }
    match dialog {
        DialogState::None
        | DialogState::CreateWorkspace { .. }
        | DialogState::QueueMessage { .. } => {}
        DialogState::AcknowledgePath { path, status, .. } => {
            lines.push(Line::from(""));
            lines.push(Line::styled(
                format!(
                    "Confirm acknowledgment: {:?} · {}?",
                    path,
                    workspace_review_status_label(*status)
                ),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
            lines.push(Line::from(
                "This records your review of this path; it does not resume the session.",
            ));
        }
        DialogState::ConfirmResume { .. } => {
            lines.push(Line::from(""));
            lines.push(Line::styled(
                "Confirm recovery resume? The daemon will revalidate paths again.",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
            lines.push(Line::from(
                "This clears recovery pause only; no provider or tools are started.",
            ));
        }
    }
    if let Some(notice) = notice {
        lines.push(Line::from(""));
        lines.push(Line::styled(
            notice.to_owned(),
            Style::default().fg(Color::Green),
        ));
    }
    if let Some(error) = error {
        lines.push(Line::from(""));
        lines.push(Line::styled(
            format!("Action/error: {error} · press r to refresh"),
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
        AttentionItem, AttentionReason, Autonomy, DaemonSessionEvent, QueuedMessage, Selection,
        SessionStatus, SessionSummary, WorkMode, WorkspaceFileReviewStatus,
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

    fn type_dialog_input(app: &mut App, value: &str) {
        for character in value.chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
        }
    }

    fn press_enter(app: &mut App) {
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
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
    fn should_hibernate_and_wake_from_the_tui_without_starting_queued_work() {
        let directory = PrivateDir::new();
        let database_path = directory.0.join("sessions.sqlite3");
        let mut registry = SessionRegistry::open(&database_path).unwrap();
        registry.create("session-1", 10).unwrap();
        registry
            .enqueue("session-1", message_with_id("queued-1"), 11)
            .unwrap();
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
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.detail.as_ref().unwrap().status, SessionStatus::Paused);

        app.handle_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE));
        assert!(app.error.is_none());
        let hibernated = app.detail.as_ref().unwrap();
        assert_eq!(hibernated.status, SessionStatus::Hibernated);
        assert_eq!(
            hibernated.queued_messages,
            vec![message_with_id("queued-1")]
        );
        assert!(hibernated.active_message.is_none());

        app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE));
        assert!(app.error.is_none());
        let woken = app.detail.as_ref().unwrap();
        assert_eq!(woken.status, SessionStatus::Paused);
        assert_eq!(woken.queued_messages, vec![message_with_id("queued-1")]);
        assert!(woken.active_message.is_none());

        app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE));
        assert_eq!(app.events.len(), 4);
        assert!(matches!(
            &app.events[2].event,
            DaemonSessionEvent::Hibernated
        ));
        assert!(matches!(&app.events[3].event, DaemonSessionEvent::Woken));
        assert!(
            app.events.iter().all(|event| {
                !matches!(&event.event, DaemonSessionEvent::MessageStarted { .. })
            })
        );

        shutdown.store(true, Ordering::Release);
        server_thread.join().unwrap();
        assert!(!socket_path.exists());
    }

    #[test]
    fn should_create_and_queue_explicit_work_from_the_tui_without_starting_it() {
        let directory = PrivateDir::new();
        let workspace = directory.0.join("workspace");
        fs::create_dir(&workspace).unwrap();
        let database_path = directory.0.join("sessions.sqlite3");
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
        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
        type_dialog_input(&mut app, workspace.to_str().unwrap());
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|frame| super::draw(frame, &app)).unwrap();
        let rendered_workspace_dialog = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<Vec<_>>()
            .join("");
        assert!(!rendered_workspace_dialog.contains(workspace.to_str().unwrap()));
        assert!(rendered_workspace_dialog.contains("input hidden"));
        press_enter(&mut app);

        let snapshot = app.detail.as_ref().expect("new session should open");
        let session_id = snapshot.session_id.clone();
        assert_eq!(snapshot.status, SessionStatus::Paused);
        assert!(snapshot.queued_messages.is_empty());
        assert!(
            !app.notice
                .as_deref()
                .unwrap()
                .contains(workspace.to_str().unwrap())
        );

        app.handle_key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE));
        type_dialog_input(&mut app, "first line");
        app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL));
        type_dialog_input(&mut app, "second line");
        press_enter(&mut app);

        type_dialog_input(&mut app, "profile-operator");
        press_enter(&mut app);
        type_dialog_input(&mut app, "plan");
        press_enter(&mut app);
        type_dialog_input(&mut app, "ask");
        press_enter(&mut app);
        type_dialog_input(&mut app, "provider-not-configured");
        press_enter(&mut app);
        type_dialog_input(&mut app, "model-not-configured");
        press_enter(&mut app);
        type_dialog_input(&mut app, "default");
        press_enter(&mut app);
        assert!(matches!(
            &app.dialog,
            super::DialogState::QueueMessage {
                stage: super::QueueStage::Confirm,
                ..
            }
        ));
        app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));

        let snapshot = app.detail.as_ref().unwrap();
        assert_eq!(snapshot.session_id, session_id);
        assert_eq!(snapshot.status, SessionStatus::Paused);
        assert!(snapshot.active_message.is_none());
        assert_eq!(snapshot.queued_messages.len(), 1);
        assert_eq!(snapshot.queued_messages[0].text, "first line\nsecond line");
        assert_eq!(snapshot.queued_messages[0].origin, "tui");
        assert_eq!(
            snapshot.queued_messages[0].selection.profile,
            "profile-operator"
        );
        assert_eq!(snapshot.queued_messages[0].selection.work, WorkMode::Plan);
        assert_eq!(
            snapshot.queued_messages[0].selection.autonomy,
            Autonomy::Ask
        );
        assert_eq!(
            snapshot.queued_messages[0].selection.provider,
            "provider-not-configured"
        );
        assert_eq!(
            snapshot.queued_messages[0].selection.model,
            "model-not-configured"
        );
        assert_eq!(snapshot.queued_messages[0].selection.variant, "default");
        assert!(
            app.notice
                .as_deref()
                .unwrap()
                .contains("no provider or tools were started")
        );

        app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE));
        assert_eq!(app.events.len(), 2);
        assert!(
            app.events.iter().all(|event| {
                !matches!(&event.event, DaemonSessionEvent::MessageStarted { .. })
            })
        );
        assert!(matches!(
            &app.events[1].event,
            DaemonSessionEvent::MessageQueued { message }
                if message.origin == "tui"
        ));

        shutdown.store(true, Ordering::Release);
        server_thread.join().unwrap();
        assert!(!socket_path.exists());
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
            app.selected_review_file,
            &app.dialog,
            app.notice.as_deref(),
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
    fn should_require_tui_acknowledgment_and_final_confirmation_to_resume_recovery() {
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
        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        assert!(matches!(
            app.dialog,
            super::DialogState::AcknowledgePath { .. }
        ));
        app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        let snapshot = app.detail.as_ref().unwrap();
        assert_eq!(snapshot.status, SessionStatus::Paused);
        assert!(snapshot.recovery_needs_revalidation);
        assert!(snapshot.active_work_uncertain);
        assert_eq!(snapshot.active_message.as_ref().unwrap().id, "active-work");
        assert_eq!(snapshot.queued_messages[0].id, "queued-work");
        assert_eq!(snapshot.event_sequence, 7);
        app.handle_key(KeyEvent::new(KeyCode::Char('R'), KeyModifiers::NONE));
        assert!(matches!(
            app.dialog,
            super::DialogState::ConfirmResume { .. }
        ));
        app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
        assert_eq!(app.detail.as_ref().unwrap().status, SessionStatus::Paused);
        app.handle_key(KeyEvent::new(KeyCode::Char('R'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        let snapshot = app.detail.as_ref().unwrap();
        assert_eq!(snapshot.status, SessionStatus::Active);
        assert!(!snapshot.recovery_needs_revalidation);
        assert!(!snapshot.active_work_uncertain);
        assert_eq!(snapshot.active_message.as_ref().unwrap().id, "active-work");
        assert_eq!(snapshot.queued_messages[0].id, "queued-work");
        assert!(app.attention.is_empty());

        let detail = super::detail_lines(
            snapshot,
            None,
            app.selected_review_file,
            &app.dialog,
            app.notice.as_deref(),
            None,
        );
        assert!(detail.iter().any(|line| {
            line.to_string()
                .contains("Recovery needs revalidation: false")
        }));
        assert!(
            detail
                .iter()
                .any(|line| line.to_string().contains("Active work uncertain: false"))
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
        assert!(rendered.contains("v review"));
        assert!(rendered.contains("R resume"));
        assert!(rendered.contains("no provider or tool runner"));

        shutdown.store(true, Ordering::Release);
        server_thread.join().unwrap();
        let registry = SessionRegistry::open(&database_path).unwrap();
        let session = registry.get("session-1").unwrap();
        assert_eq!(
            session.status,
            carapana_storage::StoredSessionStatus::Active
        );
        assert!(!session.recovery_needs_revalidation);
        assert!(!session.active_work_uncertain);
        assert_eq!(session.active_message, Some(message_with_id("active-work")));
        assert_eq!(
            session.queued_messages,
            vec![message_with_id("queued-work")]
        );
        assert_eq!(session.event_sequence, 8);
    }
}
