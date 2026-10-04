use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use serde::{Deserialize, Serialize};

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
    pub editing: bool,
    pub editing_queue: bool,
    pub pane: usize,
    pub scroll: u16,
    pub notice: String,
    pub plain: bool,
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
            editing: false,
            editing_queue: false,
            pane: 0,
            scroll: 0,
            notice: "Protótipo. Nenhum arquivo/comando/provider real.".into(),
            plain: false,
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
        self.selected = (self.selected + 1) % self.profiles.len();
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
        !(self.profiles[self.selected].model == "Claude mock"
            && self.profiles[self.selected].variant == "high")
    }
    pub fn send(&mut self, intervene: bool) {
        if self.scenario().id == "offline" {
            self.notice = "Desconectado: rascunho preservado.".into();
            return;
        }
        if !self.compatible() {
            self.notice = "Variante incompatível. Escolha explicitamente com Alt+V.".into();
            return;
        }
        if self.input.trim().is_empty() {
            return;
        }
        let message = Message {
            text: self.input.trim().into(),
            selection: self.profiles[self.selected].clone(),
        };
        if self.editing_queue && !self.queue.is_empty() {
            self.queue[0] = message;
            self.notice = "Texto e seleção da primeira mensagem atualizados.".into();
        } else if intervene {
            self.pending = Some(message);
            self.notice =
                "Intervenção pendente. Approval anterior invalidado. F2: etapa segura.".into();
        } else {
            self.queue.push(message);
            self.notice = "Enfileirado com a seleção do envio.".into();
        }
        self.input.clear();
        self.editing = false;
        self.editing_queue = false;
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
            0 => format!("Você: Corrige a rotação dos tokens. Preserva minhas alterações.\n\n✓ Inspecionar autenticação\n✓ Atualizar auth/service.rs\n✓ cargo test: 183 passaram · 4,8s (mock)\n\n{}\n\n{}", self.scenario().summary, if self.scenario().id == "approval" { "git push origin feature/refresh-token\nRemote: github.com/acme/quintal-api\nRisco: escrita remota. Motivo: publicar a PR.\nA: permitir uma vez · N: negar" } else { "Evidências e estados são simulados." }),
            1 => "✓ Inspecionar implementação\n✓ Reproduzir bug\n✓ Corrigir rotação\n✓ Adicionar testes\n○ Revisar publicação\n\nPlanejar não implementa sozinho.".into(),
            2 => "Alterações +124 −38 · mock\nauth/service.rs +48 −12\nauth/repository.rs +21 −4\nauth/service_test.rs +55 −22\nMiddleware: alterações anteriores preservadas\n\n- self.tokens.insert(token).await?;\n+ let mut tx = self.db.begin().await?;\n+ self.tokens.invalidate_previous(&mut tx).await?;\n+ tx.commit().await?;\n\nCheckpoint: prévia + confirmação; nunca descartar trabalho alheio.".into(),
            3 => "Validação · mock\n✓ cargo fmt --check\n✓ cargo clippy\n✓ cargo test --workspace: 183 passed\n✓ API smoke test\nNão validado: Google OAuth callback\n\nValidação incompleta pausa fila. Nenhum teste real do projeto foi executado.".into(),
            4 => "Precisa de você\n! quintal-api · Corrigir login · aprovação\n\nEm andamento\n• tixnow-web · Testes\n\nEm pausa\n• carapana · Atalhos TUI\n\nSessão ativa: Anexar / Nova / Voltar. Sem duplicação silenciosa.\nWorktrees são escolha do operador.".into(),
            5 => "MCP · mock\nai-memory   pinned/usuário     saudável\nplaywright  sob demanda       desativado\npostgres    projeto           saudável\n\nPermissões ai-memory: ler/permitir; gravar/projeto; excluir/perguntar\nNova tool/acesso ampliado exige approval. Sem processo MCP real.\nWeb: revisar import diff, ativar/desativar e reiniciar.".into(),
            6 => "Skills · mock\nsecurity          obrigatória\nmaintainability   ativa\ntesting           ativa\nlocal-development ativa\nfrontend-design   opcional\ngit               opcional\nrust              opcional\n\nHooks não contornam permissões. Web: ativar skills opcionais.".into(),
            7 => "Providers · mock\nOpenAI/ChatGPT: conectado (fixture, não entitlement real)\nAnthropic: desconectado\nLocal: fixture\n\nAlt+M muda modelo da próxima mensagem.\nAlt+V muda variante. Nunca fallback silencioso.\nTroca de provider real exigirá confirmação; no mock não há transmissão.".into(),
            8 => "Devices / Remote attach · mock\nMacBook TUI: local\nCelular PWA: view/prompts/approvals comuns\nAdmins: desativados por padrão\n\nCódigo mock: CARA-2048 · não é credencial\nhttps://host.example.invalid · TLS/VPN/app auth\n\nNenhuma conexão é estabelecida. Web: fluxo de pareamento.".into(),
            9 => "Recursos · mock\nHarness: 742 MiB / 2 GiB\nSessões: 3 ativas / 7 hibernadas\nArtifacts 3,2 GB / Cache 640 MiB\n\nPressão: continuar sozinho; não aumentar budgets.\nParar encerra temporários, não serviços anteriores.\nWeb: limites, Manter rodando e prévia de limpeza.".into(),
            10 => format!("Perfis · memória da sessão\n{}\n\nShift+Tab / Alt+P: alternar\nAlt+M: modelo / Alt+V: variante\nSem enviar, execução atual não muda.\nTema: --plain para sem cor.\nWeb: criar/reordenar perfis e salvar padrões explícitos.", self.profiles.iter().map(|p| format!("{}: {} / {}", p.name, p.model, p.variant)).collect::<Vec<_>>().join("\n")),
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
    let rows = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(2),
        Constraint::Min(6),
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Length(2),
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
    frame.render_widget(Paragraph::new("carapanã · quintal-api · feature/refresh-token\nDelivery 0: mocks somente · sem comandos/providers/filesystem").block(Block::default().borders(Borders::ALL)).style(accent), rows[0]);
    frame.render_widget(
        Paragraph::new(format!(
            "{} {} [{}]\n{}",
            if app.scenario().attention { "!" } else { "•" },
            app.scenario().label,
            app.scenario().status,
            app.notice
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
                    .title(format!(" {} · Tab para navegar ", PANES[app.pane]))
                    .borders(Borders::ALL),
            ),
        rows[2],
    );
    let selected = &app.profiles[app.selected];
    frame.render_widget(
        Paragraph::new(format!(
            "Executando: {} / {} / {}\nPróxima: {} / {} / {}{}",
            app.executing.name,
            app.executing.model,
            app.executing.variant,
            selected.name,
            selected.model,
            selected.variant,
            if app.compatible() {
                ""
            } else {
                " [incompatível]"
            }
        ))
        .block(Block::default().borders(Borders::TOP))
        .style(accent),
        rows[3],
    );
    let queued = app
        .queue
        .first()
        .map(|m| {
            format!(
                "{} [{} / {} / {}]",
                m.text, m.selection.name, m.selection.model, m.selection.variant
            )
        })
        .unwrap_or_else(|| "Nada na fila".into());
    frame.render_widget(
        Paragraph::new(format!(
            "Fila: {} · F5 editar primeira / F6 remover / F7 reordenar\n{}",
            app.queue.len(),
            queued
        ))
        .wrap(Wrap { trim: false }),
        rows[4],
    );
    frame.render_widget(
        Paragraph::new(app.input.as_str()).block(
            Block::default()
                .title(if app.editing {
                    " Escrevendo: Enter envia / Alt+I intervém "
                } else {
                    " Enter para escrever · seleção não altera execução "
                })
                .borders(Borders::ALL),
        ),
        rows[5],
    );
    frame.render_widget(Paragraph::new("c cenário | f etapa segura | g concluir | a/n approval | t painel | j/k rolar\nb perfil | m modelo | v variante | e/d/o fila | w intervir | q sair"), rows[6]);
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
