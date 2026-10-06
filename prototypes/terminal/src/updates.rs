//! Deterministic Delivery 0 preview. No downloads, timers, signature verification,
//! installation, process control, persistence or access to conversation state.
use crate::{App, interaction::Menu, palette::Palette};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::Modifier,
    text::Line,
    widgets::{Block, Borders, Clear, Paragraph},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Package {
    Interface,
    Runtime,
    Invalid,
    Migration,
}
impl Package {
    pub const ALL: [Self; 4] = [
        Self::Interface,
        Self::Runtime,
        Self::Invalid,
        Self::Migration,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Interface => "Interface · sem reinício",
            Self::Runtime => "Runtime · reinício seguro",
            Self::Invalid => "Assinatura inválida · bloquear",
            Self::Migration => "Migração · sem rollback automático",
        }
    }
    fn rollback(self) -> bool {
        matches!(self, Self::Interface | Self::Runtime)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Available,
    Scheduled,
    Verifying,
    Ready,
    Installed,
    Failed,
    RolledBack,
}
impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Self::Available => "Nova versão disponível",
            Self::Scheduled => "Atualização agendada",
            Self::Verifying => "Verificação pendente",
            Self::Ready => "Reinício pendente",
            Self::Installed => "Atualização simulada concluída",
            Self::Failed => "Atualização bloqueada",
            Self::RolledBack => "Rollback simulado concluído",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Now,
    Schedule,
    Later,
    Idle,
    Verify,
    Activate,
    Rollback,
    Package,
    Channel,
}
impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Self::Now => "Atualizar agora · mock",
            Self::Schedule => "Ao terminar a tarefa",
            Self::Later => "Lembrar depois",
            Self::Idle => "Simular término da tarefa e fila",
            Self::Verify => "Simular verificação",
            Self::Activate => "Simular reinício e retomada",
            Self::Rollback => "Simular rollback",
            Self::Package => "Pacote de demonstração",
            Self::Channel => "Canal de atualização",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Stable,
    Beta,
    Nightly,
}
impl Channel {
    pub const ALL: [Self; 3] = [Self::Stable, Self::Beta, Self::Nightly];
    pub fn label(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Beta => "beta",
            Self::Nightly => "nightly",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Update {
    pub package: Package,
    pub channel: Channel,
    pub status: Status,
    /// Independent illustrative fixture, never the actual session's idle state.
    pub busy: bool,
    pub hidden: bool,
    pub feedback: String,
}
impl Default for Update {
    fn default() -> Self {
        Self {
            package: Package::Interface,
            channel: Channel::Stable,
            status: Status::Available,
            busy: true,
            hidden: false,
            feedback: String::new(),
        }
    }
}
impl Update {
    pub fn select_package(&mut self, package: Package) {
        *self = Self {
            package,
            channel: self.channel,
            ..Self::default()
        };
    }
    pub fn select_channel(&mut self, channel: Channel) {
        *self = Self {
            channel,
            package: self.package,
            ..Self::default()
        };
    }
    pub fn version(&self) -> String {
        format!("0.0.1-{}-demo", self.channel.label())
    }
    pub fn actions(&self) -> Vec<Action> {
        let mut actions = match self.status {
            Status::Available => vec![Action::Now, Action::Schedule],
            Status::Scheduled => vec![Action::Idle, Action::Now],
            Status::Verifying => vec![Action::Verify],
            Status::Ready if self.busy => vec![Action::Idle, Action::Activate],
            Status::Ready => vec![Action::Activate],
            Status::Installed if self.package.rollback() => vec![Action::Rollback],
            _ => vec![],
        };
        actions.extend([Action::Later, Action::Package, Action::Channel]);
        actions
    }
    pub fn apply(&mut self, action: Action) {
        self.feedback.clear();
        match action {
            Action::Later => self.hidden = true,
            Action::Now if matches!(self.status, Status::Available | Status::Scheduled) => {
                self.status = Status::Verifying;
                self.hidden = false;
            }
            Action::Schedule if self.status == Status::Available => {
                self.status = Status::Scheduled;
                self.hidden = false;
            }
            Action::Idle => {
                self.busy = false;
                self.hidden = false;
                if self.status == Status::Scheduled {
                    self.status = Status::Verifying;
                }
            }
            Action::Verify if self.status == Status::Verifying => {
                self.status = match self.package {
                    Package::Invalid => Status::Failed,
                    Package::Interface => Status::Installed,
                    _ => Status::Ready,
                };
                self.hidden = false;
            }
            Action::Activate if self.status == Status::Ready && !self.busy => {
                self.status = Status::Installed;
                self.hidden = false;
            }
            Action::Rollback if self.status == Status::Installed && self.package.rollback() => {
                self.status = Status::RolledBack;
                self.hidden = false;
            }
            _ => self.feedback = "Ação bloqueada; versão atual preservada.".into(),
        }
    }
    fn description(&self) -> &'static str {
        match self.status {
            Status::Available if self.package == Package::Interface => {
                "Layout e navegação: interface compatível, sem reinício."
            }
            Status::Available => "Runtime fictício: preparar agora; reinício só em ponto seguro.",
            Status::Scheduled => "Aguardar tarefa e fila; simular término não altera a conversa.",
            Status::Verifying => "Simular manifest, assinatura e hash; nenhuma verificação real.",
            Status::Ready if self.busy => {
                "Sessão ocupada na fixture: reinício permanece bloqueado."
            }
            Status::Ready => "Ponto seguro na fixture: confirmar reinício explicitamente.",
            Status::Installed if self.package == Package::Interface => {
                "Interface aplicada no mock, sem reiniciar o agente."
            }
            Status::Installed => "Reinício e retomada simulados; nenhum processo reiniciado.",
            Status::Failed => "Assinatura inválida: instalação bloqueada; versão preservada.",
            Status::RolledBack => "Versão anterior restaurada no mock; nenhum arquivo alterado.",
        }
    }
}
impl App {
    pub fn update_action(&mut self, action: Action) {
        if self.scenario().id == "offline" && action != Action::Later {
            self.update.feedback = "Desconectado: ações de atualização bloqueadas.".into();
            return;
        }
        match action {
            Action::Package => self.open_menu(Menu::UpdatePackage),
            Action::Channel => self.open_menu(Menu::UpdateChannel),
            Action::Later => {
                self.update.apply(action);
                self.dialog = None;
            }
            _ => {
                self.update.apply(action);
                self.open_menu(Menu::Updates);
            }
        }
    }
}

pub fn render(frame: &mut Frame, app: &App) {
    let palette = Palette::new(app.plain, app.ansi256);
    let width = frame.area().width.saturating_sub(4).min(86);
    let height = frame.area().height.saturating_sub(2).min(22);
    let area = Rect::new(
        (frame.area().width - width) / 2,
        (frame.area().height - height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, area);
    let block = Block::default()
        .title(" Atualizações · mock ")
        .borders(Borders::ALL)
        .border_style(palette.interaction);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let rows = Layout::vertical([
        Constraint::Length(7),
        Constraint::Min(5),
        Constraint::Length(3),
    ])
    .split(inner);
    let update = &app.update;
    let status_style = match update.status {
        Status::Installed | Status::RolledBack => palette.success,
        Status::Scheduled | Status::Ready | Status::Failed => palette.attention,
        _ => palette.interaction,
    };
    let current = if update.status == Status::Installed {
        update.version()
    } else {
        "0.0.0-demo".into()
    };
    let integrity = match update.status {
        Status::Failed => "reprovados no mock",
        Status::Ready | Status::Installed | Status::RolledBack => "aprovados no mock",
        _ => "não verificados · mock",
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled(
                " Nada é baixado, instalado ou reiniciado de verdade.",
                palette.muted,
            ),
            Line::from(format!(" Atual: {current} → Nova: {}", update.version())),
            Line::styled(
                format!(" {}", update.status.label()),
                status_style.add_modifier(Modifier::BOLD),
            ),
            Line::from(format!(" {}", update.package.label())),
            Line::from(format!(" {}", update.description())),
            Line::styled(
                format!(" Manifest / assinatura / hash: {integrity}"),
                palette.muted,
            ),
            Line::styled(
                if update.package.rollback() {
                    " Rollback compatível nesta fixture."
                } else {
                    " Sem rollback automático: recuperação exige planejamento."
                },
                palette.muted,
            ),
        ]),
        rows[0],
    );
    let cursor = app.dialog.as_ref().map_or(0, |d| d.cursor);
    for (index, action) in update.actions().iter().enumerate() {
        let blocked = (app.scenario().id == "offline" && *action != Action::Later)
            || (*action == Action::Activate && update.busy);
        let label = format!(
            " {} {}{}",
            if index == cursor { ">" } else { " " },
            action.label(),
            if blocked { " [bloqueado]" } else { "" }
        );
        frame.render_widget(
            Paragraph::new(label).style(if index == cursor {
                palette.selected
            } else if blocked {
                palette.muted
            } else {
                palette.interaction
            }),
            Rect::new(rows[1].x, rows[1].y + index as u16, rows[1].width, 1),
        );
    }
    let feedback = if app.scenario().id == "offline" {
        "Desconectado: ações de atualização bloqueadas."
    } else if !update.feedback.is_empty() {
        &update.feedback
    } else if update.hidden {
        "Aviso oculto; /update continua acessível pela paleta."
    } else {
        "Fixture independente; fila, rascunho e aprovações intactos."
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled(format!(" {feedback}"), palette.attention),
            Line::styled(
                " ↑/↓ ou Ctrl+P/N escolhe · Enter confirma · Esc fecha",
                palette.muted,
            ),
            Line::styled(
                " Reinício real exigiria revalidar aprovações. Somente mock.",
                palette.muted,
            ),
        ]),
        rows[2],
    );
}
