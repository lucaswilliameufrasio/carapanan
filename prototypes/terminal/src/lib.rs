use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use serde::{Deserialize, Serialize};
use unicode_width::UnicodeWidthChar;

pub mod help;
pub mod interaction;
pub mod palette;
pub mod session;
use interaction::Dialog;

#[derive(Clone, Deserialize, Serialize)]
pub struct Scenario {
    pub id: String,
    pub label: String,
    pub status: String,
    pub summary: String,
    pub attention: bool,
    pub blocking: bool,
}

pub fn scenarios() -> Vec<Scenario> {
    serde_json::from_str(include_str!("../../../fixtures/scenarios.json"))
        .expect("checked-in scenario fixtures must be valid")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    pub name: String,
    pub model: String,
    pub variant: String,
}

#[derive(Clone)]
pub struct Message {
    pub text: String,
    pub selection: Selection,
    pub demo: bool,
}

pub struct App {
    pub scenes: Vec<Scenario>,
    pub scene: usize,
    pub profiles: Vec<Selection>,
    pub selected: usize,
    pub executing: Selection,
    pub queue: Vec<Message>,
    pub pending: Option<Message>,
    pub input: String,
    pub pane: usize,
    pub scroll: u16,
    pub notice: String,
    pub plain: bool,
    pub ansi256: bool,
    pub animated: bool,
    pub visual_elapsed: std::time::Duration,
    pub dialog: Option<Dialog>,
    pub queue_edit: Option<usize>,
    pub edit_selection: Option<Selection>,
    pub saved_draft: Option<String>,
    pub input_cursor: Option<usize>,
    pub exit_armed: Option<std::time::Instant>,
    pub verbose: bool,
    pub history: Vec<String>,
    pub history_index: Option<usize>,
    pub history_draft: Option<String>,
    pub turns: Vec<session::Turn>,
    pub active: Option<usize>,
    pub elapsed: std::time::Duration,
    pub approval_focus: bool,
    pub approval_cursor: usize,
    pub approval_action: String,
}

pub const PANES: [&str; 15] = [
    "Conversa",
    "Plano",
    "Diff",
    "Validação",
    "Sessões",
    "MCP",
    "Skills",
    "Providers",
    "Dispositivos",
    "Recursos",
    "Configurações",
    "Doctor",
    "Config efetiva",
    "Cenários",
    "Fila",
];

impl Default for App {
    fn default() -> Self {
        let profiles = vec![
            Selection {
                name: "Planejar".into(),
                model: "Claude mock".into(),
                variant: "low".into(),
            },
            Selection {
                name: "Perguntar".into(),
                model: "GPT mock".into(),
                variant: "default".into(),
            },
            Selection {
                name: "Auto".into(),
                model: "GPT mock".into(),
                variant: "high".into(),
            },
            Selection {
                name: "Yolo".into(),
                model: "GPT mock".into(),
                variant: "default".into(),
            },
        ];
        let executing = profiles[1].clone();
        let scenes = scenarios();
        let scene = scenes
            .iter()
            .position(|scenario| scenario.id == "empty")
            .expect("empty startup fixture");
        Self {
            scenes,
            scene,
            selected: 1,
            executing: executing.clone(),
            queue: vec![],
            profiles,
            pending: None,
            input: String::new(),
            pane: 0,
            scroll: 0,
            notice: String::new(),
            plain: false,
            ansi256: true,
            animated: true,
            visual_elapsed: std::time::Duration::ZERO,
            dialog: None,
            queue_edit: None,
            edit_selection: None,
            saved_draft: None,
            input_cursor: None,
            exit_armed: None,
            verbose: false,
            history: Vec::new(),
            history_index: None,
            history_draft: None,
            turns: Vec::new(),
            active: None,
            elapsed: std::time::Duration::ZERO,
            approval_focus: false,
            approval_cursor: 1,
            approval_action: "Edit auth/service.rs (simulado) · +12 −3".into(),
        }
    }
}

impl App {
    pub fn scenario(&self) -> &Scenario {
        &self.scenes[self.scene]
    }
    pub fn set_scene(&mut self, id: &str) {
        if let Some(index) = self.scenes.iter().position(|s| s.id == id) {
            self.scene = index;
        }
    }
    pub fn cycle_profile(&mut self) {
        if let Some(selection) = &self.edit_selection {
            let current = self
                .profiles
                .iter()
                .position(|p| p.name == selection.name)
                .unwrap_or(0);
            self.edit_selection = Some(self.profiles[(current + 1) % self.profiles.len()].clone());
        } else {
            self.selected = (self.selected + 1) % self.profiles.len();
        }
    }
    pub fn cycle_model(&mut self) {
        let profile = &mut self.profiles[self.selected];
        profile.model = if profile.model == "GPT mock" {
            "Claude mock"
        } else {
            "GPT mock"
        }
        .into();
        // No implicit variant fallback: high remains visibly incompatible on Claude.
    }
    pub fn cycle_variant(&mut self) {
        let profile = &mut self.profiles[self.selected];
        profile.variant = match profile.variant.as_str() {
            "default" => "low",
            "low" => "high",
            _ => "default",
        }
        .into();
    }
    pub fn compatible(&self) -> bool {
        let selection = self.next_selection();
        !(selection.model == "Claude mock" && selection.variant == "high")
    }
    pub fn send(&mut self, intervene: bool) {
        if self.scenario().id == "offline" {
            self.notice = "Desconectado: rascunho preservado.".into();
            return;
        }
        if !self.compatible() {
            self.notice = "Variante incompatível. Abra /effort para escolher.".into();
            return;
        }
        if self.input.trim().is_empty() {
            return;
        }
        let message = Message {
            text: self.input.trim().into(),
            selection: self.next_selection().clone(),
            demo: self
                .queue_edit
                .is_some_and(|index| self.queue.get(index).is_some_and(|m| m.demo)),
        };
        if self.queue_edit.is_none() {
            self.history.push(self.input.clone());
            self.history_index = None;
            self.history_draft = None;
        }
        if let Some(index) = self.queue_edit {
            if index >= self.queue.len() {
                self.notice = "Mensagem já processada; edição preservada como rascunho.".into();
                self.queue_edit = None;
                self.edit_selection = None;
                return;
            }
            self.queue[index] = message;
            self.notice = "Mensagem da fila atualizada com sua seleção explícita.".into();
        } else if intervene {
            self.pending = Some(message);
            self.approval_focus = false;
            if self.scenario().status == "waiting-for-approval" {
                self.set_scene("running");
                if let Some(index) = self.active {
                    self.turns[index].status = session::TurnStatus::Processing;
                }
            }
            if self.active.is_none() && matches!(self.scenario().id.as_str(), "empty" | "completed")
            {
                self.set_scene("running");
            }
            self.notice =
                "Intervenção pendente. Approval invalidado. /safe aplica na etapa segura.".into();
        } else {
            self.queue.push(message);
            self.notice.clear();
        }
        self.input = self.saved_draft.take().unwrap_or_default();
        self.input_cursor = None;
        self.queue_edit = None;
        self.edit_selection = None;
        self.start_next();
    }
    pub fn safe_step(&mut self) {
        if self.scenario().id == "offline" {
            return;
        }
        if let Some(message) = self.pending.take() {
            if let Some(index) = self.active.take() {
                self.turns[index].status = session::TurnStatus::Superseded;
                self.turns[index].result = Some("Trabalho redirecionado por intervenção na etapa segura; alterações preservadas.".into());
            }
            self.start_message(message);
        }
    }
    pub fn approve(&mut self, allow: bool) {
        if self.pending.is_some() {
            self.notice = "already_resolved: approval invalidado pela intervenção.".into();
            return;
        }
        if self.scenario().id == "offline" {
            self.notice = "Desconectado: approval não enviado.".into();
            return;
        }
        if !matches!(
            self.scenario().id.as_str(),
            "approval" | "sandbox" | "trust" | "shared" | "secret"
        ) {
            return;
        }
        self.approval_focus = false;
        if let Some(index) = self.active {
            self.turns[index].events.push(
                if allow {
                    "Autorização: permitir uma vez (simulado)"
                } else {
                    "Autorização negada; fila preservada"
                }
                .into(),
            );
            if allow {
                self.turns[index].step = 3;
                self.turns[index].status = session::TurnStatus::Processing;
                self.turns[index].needs_approval = false;
            }
        }
        if allow {
            self.set_scene("running");
            self.elapsed = std::time::Duration::ZERO;
        } else {
            self.pause_demo();
        }
        self.notice.clear();
    }
    pub fn finish(&mut self) {
        if self.queue_edit.is_some() {
            self.notice = "Salve ou cancele a edição antes de avançar a fila.".into();
            return;
        }
        if self.scenario().blocking {
            self.notice = "Resolva o bloqueio; a fila permanece intacta.".into();
            return;
        }
        if self.pending.is_some() {
            self.safe_step();
            return;
        }
        if let Some(index) = self.active {
            if self.turns[index].step < 5 {
                self.step_demo();
                return;
            }
            self.turns[index].result =
                Some("Rotação demonstrada. Validação simulada: 183 testes passaram.".into());
        }
        self.complete_turn();
    }
    pub fn pane_text(&self) -> String {
        match self.pane {
            0 => self.transcript(),
            1 => self.plan_text(),
            2 => self.diff_text(),
            3 => self.validation_text(),
            4 => "Precisa de você\n! quintal-api · Corrigir login · aprovação\n\nEm andamento\n• tixnow-web · Testes\n\nEm pausa\n• carapana · Atalhos TUI\n\nSessão ativa: Anexar / Nova / Voltar. Sem duplicação silenciosa.\nWorktrees são escolha do operador.".into(),
            5 => "MCP · mock\nai-memory   pinned/usuário     saudável\nplaywright  sob demanda       desativado\npostgres    projeto           saudável\n\nPermissões ai-memory: ler/permitir; gravar/projeto; excluir/perguntar\nNova tool/acesso ampliado exige approval. Sem processo MCP real.\nWeb: revisar import diff, ativar/desativar e reiniciar.".into(),
            6 => "Skills · mock\nsecurity          obrigatória\nmaintainability   ativa\ntesting           ativa\nlocal-development ativa\nfrontend-design   opcional\ngit               opcional\nrust              opcional\n\nHooks não contornam permissões. Web: ativar skills opcionais.".into(),
            7 => "Providers · mock\nOpenAI/ChatGPT: conectado (fixture, não entitlement real)\nAnthropic: desconectado\nLocal: fixture\n\nAlt+P muda modelo da próxima mensagem.\nAlt+V muda variante. Nunca fallback silencioso.\nTroca de provider real exigirá confirmação; no mock não há transmissão.".into(),
            8 => "Devices / Remote attach · mock\nMacBook TUI: local\nCelular PWA: view/prompts/approvals comuns\nAdmins: desativados por padrão\n\nCódigo mock: CARA-2048 · não é credencial\nhttps://host.example.invalid · TLS/VPN/app auth\n\nNenhuma conexão é estabelecida. Web: fluxo de pareamento.".into(),
            9 => "Recursos · mock\nHarness: 742 MiB / 2 GiB\nSessões: 3 ativas / 7 hibernadas\nArtifacts 3,2 GB / Cache 640 MiB\n\nPressão: continuar sozinho; não aumentar budgets.\nParar encerra temporários, não serviços anteriores.\nWeb: limites, Manter rodando e prévia de limpeza.".into(),
            10 => format!("Perfis · memória da sessão\n{}\n\nShift+Tab: alternar / Alt+M: picker de perfil\nAlt+P: modelo / Alt+V: variante\nSem enviar, execução atual não muda.\nTema: --plain para sem cor.\nWeb: criar/reordenar perfis e salvar padrões explícitos.", self.profiles.iter().map(|p| format!("{}: {} / {}", p.name, p.model, p.variant)).collect::<Vec<_>>().join("\n")),
            11 => "Doctor · relatório simulado\n✓ Config / Storage / Git / Sandbox\n✓ Provider / Skills / MCP / Recursos\n\nSem tokens, prompts, código ou env no report.\nNenhuma inspeção do host ou correção real.".into(),
            12 => "Config efetiva · mock\nagent.mode: ask\nmodel.primary: gpt-mock\nsource: user\ndefaults → system → user → project → env → CLI → session\n\nProjeto não amplia privilégios sozinho.\nSegredos não ficam em TOML.\nCli: config / info / doctor são fixtures.".into(),
            13 => self.scenes.iter().enumerate().map(|(i,s)| format!("{} {}: {}", if i == self.scene { ">" } else { " " }, s.id, s.label)).collect::<Vec<_>>().join("\n"),
            _ => self.queue.iter().enumerate().map(|(i,m)| format!("{}. {}\n   {} / {} / {}", i+1, m.text, m.selection.name, m.selection.model, m.selection.variant)).collect::<Vec<_>>().join("\n\n"),
        }
    }
}

pub fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();
    if area.width < 80 || area.height < 24 {
        frame.render_widget(Paragraph::new("Carapanã · Delivery 0\nAmplie o terminal para pelo menos 80×24.\nCtrl+Q para sair. Nenhuma execução real.").wrap(Wrap { trim: false }), area);
        return;
    }
    let content = Rect::new(2, 1, area.width.saturating_sub(4), area.height - 2);
    let (input, cursor_row, cursor_col) = project_input(app, content.width as usize);
    let input_height = (input.lines().count() as u16 + 2).clamp(3, 6);
    let palette = palette::Palette::new(app.plain, app.ansi256);
    let next_mode = palette::Palette::mode(
        app.plain,
        app.ansi256,
        &app.next_selection().name,
        &app.next_selection().variant,
    );
    let executing_mode = palette::Palette::mode(
        app.plain,
        app.ansi256,
        &app.executing.name,
        &app.executing.variant,
    );
    let accent = palette.muted;
    let attention = palette.attention;
    let approval_height = if app.dialog.is_none()
        && app.scenario().status == "waiting-for-approval"
        && app.pending.is_none()
    {
        8
    } else {
        0
    };
    let queue_height = if app.queue.is_empty() || app.dialog.is_some() {
        0
    } else {
        2
    };
    let notice_height = u16::from(!app.notice.is_empty() && app.dialog.is_none());
    let transcript = wrap_terminal(&app.pane_text(), content.width as usize);
    let line_count = transcript.lines().count() as u16;
    let budget = content
        .height
        .saturating_sub(3 + approval_height + queue_height + notice_height + input_height + 2)
        .max(1);
    // Stable composer anchored at the bottom, independently of transcript length.
    let transcript_height = budget;
    let rows = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(transcript_height),
        Constraint::Length(approval_height),
        Constraint::Length(queue_height),
        Constraint::Length(notice_height),
        Constraint::Length(input_height),
        Constraint::Length(2),
    ])
    .split(content);
    let flying = !app.plain
        && app.animated
        && (app.visual_elapsed.as_secs() < 4
            || app
                .active
                .is_some_and(|i| app.turns[i].status == session::TurnStatus::Processing));
    let wings = if flying && (app.visual_elapsed.as_millis() / 220) % 2 == 1 {
        "   (/ \\)     "
    } else {
        "   (\\ /)     "
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled(wings, next_mode),
            Line::styled(" <==o-o---->  ", next_mode),
            Line::styled("  /|\\ /|\\    ", next_mode),
        ]),
        Rect::new(rows[0].x, rows[0].y, 13, 3),
    );
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled("carapanã", Style::default().add_modifier(Modifier::BOLD)),
            Line::styled("workspace de exemplo · Delivery 0 / simulado", accent),
        ]),
        Rect::new(
            rows[0].x + 14,
            rows[0].y,
            rows[0].width.saturating_sub(14),
            rows[0].height,
        ),
    );
    let max_scroll = line_count.saturating_sub(transcript_height);
    let raw_offset = max_scroll.saturating_sub(app.scroll);
    // Follow complete turns rather than starting in the tail of an old reply.
    let offset = if app.scroll == 0 && raw_offset > 0 {
        transcript
            .lines()
            .enumerate()
            .skip(raw_offset as usize)
            .find(|(_, line)| line.starts_with("> "))
            .map_or(raw_offset, |(index, _)| index as u16)
    } else {
        raw_offset
    };
    let styled_transcript: Vec<Line<'_>> = transcript
        .lines()
        .map(|line| palette.transcript_line(line))
        .collect();
    let latest_turn = app.active.or_else(|| app.turns.len().checked_sub(1));
    let current_heading = latest_turn.map(|index| {
        wrap_terminal(
            &format!("> {}", app.turns[index].message.text),
            content.width as usize,
        )
        .lines()
        .next()
        .unwrap_or_default()
        .to_string()
    });
    let request_is_visible = current_heading.as_ref().is_some_and(|heading| {
        transcript
            .lines()
            .skip(offset as usize)
            .any(|line| line == heading)
    });
    if offset > 0
        && app.scroll == 0
        && !request_is_visible
        && let Some(index) = latest_turn
    {
        // Keep the current request recognizable even when an approval + queue
        // leaves only a few transcript rows at 80×24. Older context is scrollable.
        let request = format!("> {}", app.turns[index].message.text.replace('\n', " ↵ "));
        frame.render_widget(
            Paragraph::new(palette.transcript_line(&request)),
            Rect::new(rows[1].x, rows[1].y, rows[1].width, 1),
        );
        if rows[1].height > 1 {
            frame.render_widget(
                Paragraph::new(styled_transcript).scroll((offset + 1, 0)),
                Rect::new(rows[1].x, rows[1].y + 1, rows[1].width, rows[1].height - 1),
            );
        }
    } else {
        frame.render_widget(
            Paragraph::new(styled_transcript).scroll((offset, 0)),
            rows[1],
        );
    }
    if approval_height > 0 {
        render_approval(frame, app, rows[2], &palette);
    }
    if queue_height > 0 {
        let queue = format!(
            "Na fila ({})  {}\n/queue revisa texto e seleção",
            app.queue.len(),
            app.queue
                .first()
                .map_or(String::new(), |message| message.text.replace('\n', " ↵ "))
        );
        let queue_mode = app.queue.first().map_or(palette.muted, |message| {
            palette::Palette::mode(
                app.plain,
                app.ansi256,
                &message.selection.name,
                &message.selection.variant,
            )
        });
        frame.render_widget(Paragraph::new(queue).style(queue_mode), rows[3]);
    }
    frame.render_widget(
        Paragraph::new(app.notice.as_str()).style(attention),
        rows[4],
    );
    let input_scroll = cursor_row.saturating_sub(input_height as usize - 3);
    frame.render_widget(
        Paragraph::new(input)
            .style(if app.input.is_empty() {
                palette.muted
            } else {
                Style::default()
            })
            .scroll((input_scroll as u16, 0))
            .block(
                Block::default()
                    .title(app.queue_edit.map_or(String::new(), |i| {
                        format!(" Editando mensagem {} da fila · Esc cancela ", i + 1)
                    }))
                    .border_style(if app.approval_focus || app.dialog.is_some() {
                        palette.muted
                    } else {
                        next_mode
                    })
                    .borders(Borders::TOP | Borders::BOTTOM),
            ),
        rows[5],
    );
    let selected = app.next_selection();
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled("Próxima: ", accent),
                Span::styled(
                    selected.name.as_str(),
                    next_mode.add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(
                        " / {} / {}{}   Ctrl+P opções · ? ajuda{}",
                        selected.model,
                        selected.variant,
                        if app.compatible() { "" } else { " !" },
                        if app.scroll > 0 {
                            "   PgDown volta ao fim"
                        } else {
                            ""
                        }
                    ),
                    accent,
                ),
            ]),
            Line::styled(
                if let Some(index) = app.active {
                    format!(
                        "Executando: {} / {} / {} · {}",
                        app.executing.name,
                        app.executing.model,
                        app.executing.variant,
                        match app.turns[index].status {
                            session::TurnStatus::Processing => "Em andamento · Esc interrompe",
                            session::TurnStatus::Waiting => "aguardando decisão",
                            _ => "em pausa · /resume",
                        }
                    )
                } else {
                    "Pronto · Shift+Tab muda o perfil · /demo inicia uma demonstração".into()
                },
                if let Some(index) = app.active {
                    match app.turns[index].status {
                        session::TurnStatus::Processing => executing_mode,
                        session::TurnStatus::Waiting | session::TurnStatus::Paused => {
                            palette.attention
                        }
                        _ => palette.muted,
                    }
                } else {
                    palette.success
                },
            ),
        ]),
        rows[6],
    );
    if app.dialog.is_some() {
        render_dialog(frame, app, rows[5].y);
    } else if !app.approval_focus {
        frame.set_cursor_position((
            rows[5].x + cursor_col as u16,
            rows[5].y + 1 + (cursor_row - input_scroll) as u16,
        ));
    }
}

fn wrap_terminal(text: &str, width: usize) -> String {
    let mut result = String::new();
    let mut col = 0;
    for c in text.chars() {
        let cells = c.width().unwrap_or(0);
        if c != '\n' && col + cells > width {
            result.push('\n');
            col = 0;
        }
        result.push(c);
        if c == '\n' {
            col = 0;
        } else {
            col += cells;
        }
    }
    result
}

// Hard-wrap the editor and calculate the cursor using the same visual cells,
// including double-width characters. Word wrapping would misplace the caret.
fn render_approval(frame: &mut Frame, app: &App, area: Rect, palette: &palette::Palette) {
    // A bounded decision control, not another full-width log paragraph.
    let area = Rect::new(area.x, area.y, area.width.min(86), area.height);
    let block = Block::default()
        .title(Line::styled(" Permitir esta ação? ", palette.attention))
        .borders(Borders::ALL)
        .border_style(if app.approval_focus {
            palette.attention
        } else {
            palette.muted
        });
    let inner = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(
        Paragraph::new(app.approval_action.as_str()),
        Rect::new(inner.x + 1, inner.y, inner.width.saturating_sub(2), 1),
    );
    frame.render_widget(
        Paragraph::new("Somente esta ação · demonstração de alteração protegida")
            .style(palette.muted),
        Rect::new(inner.x + 1, inner.y + 1, inner.width.saturating_sub(2), 1),
    );
    for (index, label) in ["Permitir uma vez", "Negar e pausar"].iter().enumerate() {
        let selected = app.approval_cursor == index;
        let style = if selected && app.approval_focus {
            palette.selected
        } else if selected {
            palette.muted.add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        let marker = if selected { ">" } else { " " };
        frame.render_widget(
            Paragraph::new(format!(" {marker} {}. {label}", index + 1)).style(style),
            Rect::new(inner.x, inner.y + 2 + index as u16, inner.width, 1),
        );
    }
    let hints = if app.approval_focus {
        " ↑/↓ escolhe · 1/2 seleciona · Enter confirma\n Tab escreve na fila · Esc interrompe"
    } else {
        " Escrevendo na fila · Enter só envia a mensagem\n Tab volta à decisão · Esc interrompe"
    };
    frame.render_widget(
        Paragraph::new(hints).style(palette.muted),
        Rect::new(inner.x, inner.y + 4, inner.width, 2),
    );
}

fn project_input(app: &App, width: usize) -> (String, usize, usize) {
    let cursor = app.input_cursor.unwrap_or(app.input.len());
    let mut output = String::from("> ");
    let (mut row, mut col) = (0, 2);
    let (mut cursor_row, mut cursor_col) = (0, 2);
    for (index, character) in app.input.char_indices() {
        let cells = character.width().unwrap_or(0);
        if character != '\n' && col + cells >= width {
            // Reserve the final cell for a visible cursor at the end of a line.
            output.push('\n');
            row += 1;
            col = 0;
        }
        if index == cursor {
            cursor_row = row;
            cursor_col = col;
        }
        output.push(character);
        if character == '\n' {
            row += 1;
            col = 0;
        } else {
            col += cells;
        }
        if index + character.len_utf8() == cursor {
            cursor_row = row;
            cursor_col = col;
        }
    }
    if app.input.is_empty() {
        output.push_str("Escreva sua mensagem, ou / para opções");
    }
    (output, cursor_row, cursor_col)
}

fn render_help(frame: &mut Frame, page: usize, area: Rect, palette: &palette::Palette) {
    let page = page.min(help::PAGES.len() - 1);
    let rows = Layout::vertical([
        Constraint::Length(if area.height >= 10 { 2 } else { 1 }),
        Constraint::Min(6),
        Constraint::Length(2),
    ])
    .split(area);
    let tabs = help::PAGES
        .iter()
        .enumerate()
        .map(|(index, section)| {
            Span::styled(
                format!(" {} {} ", index + 1, section.title),
                if index == page {
                    palette.selected
                } else {
                    palette.muted
                },
            )
        })
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(Line::from(tabs)), rows[0]);
    let entries = help::PAGES[page]
        .entries
        .iter()
        .map(|(key, description)| {
            Line::from(vec![
                Span::styled(
                    format!(" {key:<23}"),
                    palette.interaction.add_modifier(Modifier::BOLD),
                ),
                Span::raw(*description),
            ])
        })
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(entries), rows[1]);
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled(format!(" {}", help::PAGES[page].note), palette.muted),
            Line::from(vec![
                Span::styled(" ←/→ ou Tab", palette.interaction),
                Span::styled(" assunto   ", palette.muted),
                Span::styled("1–4", palette.interaction),
                Span::styled(" acesso direto   ", palette.muted),
                Span::styled("Esc", palette.interaction),
                Span::styled(" fecha", palette.muted),
            ]),
        ]),
        rows[2],
    );
}

fn render_dialog(frame: &mut Frame, app: &App, input_y: u16) {
    use interaction::Menu;
    let dialog = app.dialog.as_ref().unwrap();
    let palette = palette::Palette::new(app.plain, app.ansi256);
    let title = match dialog.menu {
        Menu::Commands => "Opções",
        Menu::Model => "Modelo da próxima mensagem",
        Menu::Profile => "Perfil da próxima mensagem",
        Menu::Effort => "Raciocínio",
        Menu::Queue => "Fila",
        Menu::Scenario => "Cenário de revisão",
        Menu::Approval => "Aprovação simulada",
        Menu::Config => "Configurações",
        Menu::Help => "Ajuda",
    };
    let items = app.menu_items();
    let width = frame.area().width.saturating_sub(4).min(86);
    let desired_height = if dialog.menu == Menu::Help {
        14
    } else {
        (items.len().clamp(1, 6) * 2 + 6) as u16
    };
    let height = input_y.saturating_sub(4).min(17).min(desired_height);
    let area = Rect::new(2, input_y.saturating_sub(height).max(4), width, height);
    // Clear the whole horizontal band so fragments of the transcript do not leak
    // around a compact dialog on wide terminals.
    frame.render_widget(Clear, Rect::new(0, area.y, frame.area().width, height));
    let block = Block::default()
        .title(Line::styled(
            format!(" {title} "),
            palette.interaction.add_modifier(Modifier::BOLD),
        ))
        .border_style(palette.interaction)
        .borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if dialog.menu == Menu::Help {
        render_help(frame, dialog.cursor, inner, &palette);
        return;
    }
    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(2),
        Constraint::Length(2),
    ])
    .split(inner);
    let heading = if dialog.menu == Menu::Model {
        format!(
            "Buscar: {}\nAtual: {} / {}",
            dialog.query,
            app.next_selection().model,
            app.next_selection().variant
        )
    } else if dialog.menu == Menu::Queue {
        "Selecione a mensagem; a seleção acompanha cada envio.\nAlterações são simuladas; Enter abre edição.".into()
    } else if dialog.menu == Menu::Approval {
        app.approval_action.clone()
    } else {
        format!(
            "Buscar: {}\nEsc cancela sem alterar rascunho ou seleção.",
            dialog.query
        )
    };
    frame.render_widget(Paragraph::new(heading).style(palette.muted), rows[0]);
    let capacity = (rows[1].height / 2).max(1) as usize;
    let start = dialog.cursor.saturating_sub(capacity - 1);
    if items.is_empty() {
        frame.render_widget(
            Paragraph::new(if dialog.menu == Menu::Queue {
                "Nada na fila."
            } else {
                "Nenhum resultado. Backspace altera a busca."
            }),
            rows[1],
        );
    }
    for (index, (name, detail)) in items.iter().enumerate().skip(start).take(capacity) {
        let selected = index == dialog.cursor;
        let mark = if selected { ">" } else { " " };
        let is_current = match dialog.menu {
            Menu::Model => *name == app.next_selection().model,
            Menu::Profile => *name == app.next_selection().name,
            Menu::Effort => *name == app.next_selection().variant,
            Menu::Scenario => *name == app.scenario().id,
            _ => false,
        };
        let binding = if matches!(dialog.menu, Menu::Commands | Menu::Config) {
            interaction::shortcut(name)
        } else {
            ""
        };
        let binding_label = if binding.is_empty() {
            String::new()
        } else {
            format!("  [{binding}]")
        };
        let label_style = if selected {
            palette.selected
        } else {
            palette.interaction
        };
        let detail_style = if selected {
            palette.selected
        } else {
            palette.muted
        };
        let text = vec![
            Line::from(vec![
                Span::styled(format!("{mark} {name}"), label_style),
                Span::styled(
                    if is_current { " [atual]" } else { "" },
                    if selected {
                        palette.selected
                    } else {
                        palette.success
                    },
                ),
                Span::styled(
                    binding_label,
                    if selected {
                        palette.selected
                    } else {
                        palette.assistant
                    },
                ),
            ]),
            Line::styled(format!("  {detail}"), detail_style),
        ];
        frame.render_widget(
            Paragraph::new(text).style(if selected {
                palette.selected
            } else {
                Style::default()
            }),
            Rect::new(
                rows[1].x,
                rows[1].y + ((index - start) * 2) as u16,
                rows[1].width,
                2,
            ),
        );
    }
    let footer = match dialog.menu {
        Menu::Model => {
            let compatible = items.get(dialog.cursor).is_some_and(|(name, _)| {
                interaction::variants(name).contains(&dialog.variant.as_str())
            });
            format!(
                "Variante: {}{} · ←/→ escolhe\n↑/↓ navegar · Enter confirmar · Esc cancelar",
                dialog.variant,
                if compatible { "" } else { " [incompatível]" }
            )
        }
        Menu::Queue => {
            "↑/↓ selecionar · Enter editar · d remover\n-/+ reordenar · Esc fechar".into()
        }
        _ => format!(
            "↑/↓ ou Ctrl+P/N · Enter confirmar · Esc cancelar\n{} opções · seleção {}/{}",
            items.len(),
            if items.is_empty() {
                0
            } else {
                dialog.cursor + 1
            },
            items.len()
        ),
    };
    frame.render_widget(Paragraph::new(footer).style(palette.muted), rows[2]);
}

#[derive(Serialize)]
pub struct HeadlessResult {
    pub prototype: bool,
    pub executed: bool,
    pub scenario: String,
    pub status: String,
    pub summary: String,
    pub exit_code: u8,
}

pub fn headless(scenario: &Scenario) -> HeadlessResult {
    let code = match scenario.id.as_str() {
        "completed" => 0,
        "incomplete" => 3,
        "approval" | "sandbox" | "trust" | "shared" | "secret" => 4,
        _ => 2,
    };
    HeadlessResult {
        prototype: true,
        executed: false,
        scenario: scenario.id.clone(),
        status: scenario.status.clone(),
        summary: scenario.summary.clone(),
        exit_code: code,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn should_share_all_scenario_fixtures() {
        assert_eq!(scenarios().len(), 21);
    }
    #[test]
    fn should_keep_execution_and_queue_unchanged_when_switching_profiles() {
        let mut app = App::review();
        let original = app.executing.clone();
        app.cycle_profile();
        app.cycle_model();
        assert_eq!(app.executing, original);
        assert_eq!(app.queue[0].selection, original);
    }
    #[test]
    fn should_snapshot_selection_when_sending() {
        let mut app = App {
            input: "Teste".into(),
            ..App::review()
        };
        let selection = app.profiles[app.selected].clone();
        app.send(false);
        app.cycle_profile();
        assert_eq!(app.queue.last().unwrap().selection, selection);
    }
    #[test]
    fn should_wait_for_safe_step_and_invalidate_approval() {
        let mut app = App::review();
        let original = app.executing.clone();
        app.input = "Intervenção".into();
        app.send(true);
        app.approve(true);
        assert_eq!(app.executing, original);
        assert_eq!(app.scenario().id, "running");
        app.safe_step();
        assert_eq!(app.executing.name, "Perguntar");
        assert_eq!(app.scenario().id, "running");
    }
    #[test]
    fn should_block_offline_send_without_losing_draft() {
        let mut app = App::review();
        app.set_scene("offline");
        app.input = "Preservar".into();
        app.send(false);
        assert_eq!(app.input, "Preservar");
        assert_eq!(app.queue.len(), 1);
    }
    #[test]
    fn should_block_advancement_on_validation_and_recovery() {
        for scene in ["incomplete", "quota", "recovery", "loop"] {
            let mut app = App::review();
            app.set_scene(scene);
            app.finish();
            assert_eq!(app.queue.len(), 1);
        }
    }
    #[test]
    fn should_report_noninteractive_approval_without_execution() {
        let result = headless(&scenarios()[0]);
        assert_eq!(result.exit_code, 4);
        assert!(!result.executed);
    }
    #[test]
    fn should_render_all_scenarios_and_panes_at_minimum_size_without_truecolor() {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let mut app = App {
            plain: true,
            ..App::review()
        };
        for scene in 0..app.scenes.len() {
            app.scene = scene;
            for pane in 0..PANES.len() {
                app.pane = pane;
                terminal.draw(|frame| render(frame, &app)).unwrap();
            }
        }
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(text.contains("Executando"));
        assert!(text.contains("Próxima"));
    }
    #[test]
    fn should_render_narrow_wide_unicode_and_large_queues() {
        for (width, height) in [(60, 20), (80, 24), (160, 48)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            let mut app = App::review();
            app.queue = (0..30)
                .map(|_| Message {
                    text: "Carapanã 中文 token".into(),
                    selection: app.executing.clone(),
                    demo: true,
                })
                .collect();
            terminal.draw(|frame| render(frame, &app)).unwrap();
        }
    }
}
