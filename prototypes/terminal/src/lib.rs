use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use serde::{Deserialize, Serialize};
use unicode_width::UnicodeWidthChar;

pub mod interaction;
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
        let executing = profiles[2].clone();
        Self {
            scenes: scenarios(),
            scene: 0,
            selected: 1,
            executing: executing.clone(),
            queue: vec![Message {
                text: "Verifica a reutilização de tokens.".into(),
                selection: executing,
            }],
            profiles,
            pending: None,
            input: String::new(),
            pane: 0,
            scroll: 0,
            notice: "Protótipo. Nenhum arquivo/comando/provider real.".into(),
            plain: false,
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
            self.notice =
                "Intervenção pendente. Approval invalidado. /safe aplica na etapa segura.".into();
        } else {
            self.queue.push(message);
            self.notice = "Enfileirado com a seleção do envio.".into();
        }
        self.input = self.saved_draft.take().unwrap_or_default();
        self.input_cursor = None;
        self.queue_edit = None;
        self.edit_selection = None;
    }
    pub fn safe_step(&mut self) {
        if self.scenario().id == "offline" {
            return;
        }
        if let Some(message) = self.pending.take() {
            self.executing = message.selection;
            self.set_scene("running");
            self.notice = format!("Etapa segura simulada: {}", message.text);
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
        self.set_scene(if allow { "running" } else { "recovery" });
        self.notice = "Decisão simulada; nenhum comando executado.".into();
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
        if self.queue.is_empty() {
            self.set_scene("completed");
        } else {
            self.executing = self.queue.remove(0).selection;
            self.set_scene("running");
        }
        self.notice = "Conclusão/avanço simulado. Nenhum trabalho real.".into();
    }
    pub fn pane_text(&self) -> String {
        match self.pane {
            0 => format!("Você\nCorrige a rotação dos tokens. Preserva minhas alterações.\n\nCarapanã\n✓ Inspecionar autenticação\n✓ Atualizar auth/service.rs\n✓ cargo test: 183 passaram · 4,8s (mock)\n{}\n{}\n\n{}", if self.verbose { "\nDetalhes simulados · cargo test\nexit: 0 · duração: 4,8s · 183 passed; 0 failed\nSaída fixture; nenhum processo executado.\nCtrl+O recolhe detalhes.\n" } else { "" }, self.scenario().summary, if self.pending.is_some() { "Intervenção pendente; aprovação anterior invalidada.\n/safe simula a próxima etapa segura." } else if self.scenario().id == "approval" { "git push origin feature/refresh-token\nRemote: github.com/acme/quintal-api\nRisco: escrita remota. Motivo: publicar a PR.\n/approve abre as opções de aprovação." } else { "Evidências e estados são simulados." }),
            1 => "✓ Inspecionar implementação\n✓ Reproduzir bug\n✓ Corrigir rotação\n✓ Adicionar testes\n○ Revisar publicação\n\nPlanejar não implementa sozinho.".into(),
            2 => "Alterações +124 −38 · mock\nauth/service.rs +48 −12\nauth/repository.rs +21 −4\nauth/service_test.rs +55 −22\nMiddleware: alterações anteriores preservadas\n\n- self.tokens.insert(token).await?;\n+ let mut tx = self.db.begin().await?;\n+ self.tokens.invalidate_previous(&mut tx).await?;\n+ tx.commit().await?;\n\nCheckpoint: prévia + confirmação; nunca descartar trabalho alheio.".into(),
            3 => "Validação · mock\n✓ cargo fmt --check\n✓ cargo clippy\n✓ cargo test --workspace: 183 passed\n✓ API smoke test\nNão validado: Google OAuth callback\n\nValidação incompleta pausa fila. Nenhum teste real do projeto foi executado.".into(),
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
    let (input, cursor_row, cursor_col) = project_input(app, area.width as usize);
    let input_height = (input.lines().count() as u16 + 2).clamp(3, 6);
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(2),
        Constraint::Min(6),
        Constraint::Length(1),
        Constraint::Length(input_height),
        Constraint::Length(1),
    ])
    .split(area);
    let accent = if app.plain {
        Style::default()
    } else {
        Style::default().fg(Color::LightBlue)
    };
    let attention = if app.plain {
        Style::default().add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Yellow)
    };
    frame.render_widget(
        Paragraph::new("carapanã   quintal-api / feature/refresh-token   Delivery 0 · mock")
            .style(accent),
        rows[0],
    );
    frame.render_widget(
        Paragraph::new(format!(
            "{} {}\nExecutando: {} / {} / {}",
            if app.scenario().attention { "!" } else { "•" },
            app.scenario().label,
            app.executing.name,
            app.executing.model,
            app.executing.variant,
        ))
        .style(attention),
        rows[1],
    );
    frame.render_widget(
        Paragraph::new(app.pane_text())
            .wrap(Wrap { trim: false })
            .scroll((app.scroll, 0))
            .block(
                Block::default()
                    .title(format!(" {} ", PANES[app.pane]))
                    .borders(Borders::TOP),
            ),
        rows[2],
    );
    frame.render_widget(
        Paragraph::new(app.notice.as_str()).style(attention),
        rows[3],
    );
    let input_scroll = cursor_row.saturating_sub(input_height as usize - 3);
    frame.render_widget(
        Paragraph::new(input)
            .scroll((input_scroll as u16, 0))
            .block(
                Block::default()
                    .title(app.queue_edit.map_or(" Mensagem ".into(), |i| {
                        format!(" Editando mensagem {} da fila · Esc cancela ", i + 1)
                    }))
                    .borders(Borders::TOP | Borders::BOTTOM),
            ),
        rows[4],
    );
    let selected = app.next_selection();
    frame.render_widget(
        Paragraph::new(format!(
            "Próxima: {} / {} / {}{}   Fila: {}   Ctrl+P opções · ? ajuda",
            selected.name,
            selected.model,
            selected.variant,
            if app.compatible() { "" } else { " !" },
            app.queue.len()
        ))
        .style(accent),
        rows[5],
    );
    if app.dialog.is_some() {
        render_dialog(frame, app, rows[4].y);
    } else {
        frame.set_cursor_position((
            rows[4].x + cursor_col as u16,
            rows[4].y + 1 + (cursor_row - input_scroll) as u16,
        ));
    }
}

// Hard-wrap the editor and calculate the cursor using the same visual cells,
// including double-width characters. Word wrapping would misplace the caret.
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

fn render_dialog(frame: &mut Frame, app: &App, input_y: u16) {
    use interaction::Menu;
    let dialog = app.dialog.as_ref().unwrap();
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
    let width = frame.area().width.min(86);
    let desired_height = if dialog.menu == Menu::Help {
        14
    } else {
        (items.len().clamp(1, 6) * 2 + 6) as u16
    };
    let height = input_y.saturating_sub(1).min(17).min(desired_height);
    let area = Rect::new(0, input_y - height, width, height);
    // Clear the whole horizontal band so fragments of the transcript do not leak
    // around a compact dialog on wide terminals.
    frame.render_widget(Clear, Rect::new(0, area.y, frame.area().width, height));
    let block = Block::default()
        .title(format!(" {title} "))
        .borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if dialog.menu == Menu::Help {
        frame.render_widget(Paragraph::new("Enter envia; Shift+Enter / Ctrl+J / \\+Enter: nova linha.\nCtrl+P / Ctrl+K / /: paleta; Tab completa comando.\nAlt+P: modelo; Alt+M: perfil; Alt+V: raciocínio.\nMenus: ↑/↓ ou Ctrl+P/N; Enter confirma; Esc cancela.\nModelo: ←/→ raciocínio; Shift+Tab: próximo perfil.\nEsc na conversa: interrompe; preserva fila e rascunho.\nCtrl+C: cancela menu/interrompe; ocioso limpa, 2x sai.\n↑/↓: histórico; colagem multilinha nunca envia sozinha.\nCtrl+O: detalhes/conversa; Ctrl+T: plano; PgUp/Down: rolar.\nFila: Enter edita; d remove; -/+ reordena; Esc cancela.\nIntervenção: Alt+I; F2 etapa segura; F3 concluir (mocks).\nCtrl+Q sai. Cmd+C continua sendo copiar no terminal.").wrap(Wrap { trim: false }),inner);
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
    } else {
        format!(
            "Buscar: {}\nEsc cancela sem alterar rascunho ou seleção.",
            dialog.query
        )
    };
    frame.render_widget(Paragraph::new(heading), rows[0]);
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
        let text = format!(
            "{mark} {name}{}{binding_label}\n  {detail}",
            if is_current { " [atual]" } else { "" }
        );
        let style = if selected {
            Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
        } else {
            Style::default()
        };
        frame.render_widget(
            Paragraph::new(text).style(style),
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
    frame.render_widget(Paragraph::new(footer), rows[2]);
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
        let mut app = App::default();
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
            ..App::default()
        };
        let selection = app.profiles[app.selected].clone();
        app.send(false);
        app.cycle_profile();
        assert_eq!(app.queue.last().unwrap().selection, selection);
    }
    #[test]
    fn should_wait_for_safe_step_and_invalidate_approval() {
        let mut app = App::default();
        let original = app.executing.clone();
        app.input = "Intervenção".into();
        app.send(true);
        app.approve(true);
        assert_eq!(app.executing, original);
        assert_eq!(app.scenario().id, "approval");
        app.safe_step();
        assert_eq!(app.executing.name, "Perguntar");
        assert_eq!(app.scenario().id, "running");
    }
    #[test]
    fn should_block_offline_send_without_losing_draft() {
        let mut app = App::default();
        app.set_scene("offline");
        app.input = "Preservar".into();
        app.send(false);
        assert_eq!(app.input, "Preservar");
        assert_eq!(app.queue.len(), 1);
    }
    #[test]
    fn should_block_advancement_on_validation_and_recovery() {
        for scene in ["incomplete", "quota", "recovery", "loop"] {
            let mut app = App::default();
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
            ..App::default()
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
            let mut app = App::default();
            app.queue = (0..30)
                .map(|_| Message {
                    text: "Carapanã 中文 token".into(),
                    selection: app.executing.clone(),
                })
                .collect();
            terminal.draw(|frame| render(frame, &app)).unwrap();
        }
    }
}
