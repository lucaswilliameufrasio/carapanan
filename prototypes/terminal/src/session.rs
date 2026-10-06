//! Scripted, in-memory Delivery 0 session. No tool, model or filesystem execution.
use crate::{App, Message, interaction::Menu};
use carapana_core::{DemoProvider, DemoRequest, DemoResponse, DeterministicDemoProvider};
use carapana_protocol::{Autonomy, WorkMode};
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TurnStatus {
    Processing,
    Waiting,
    Paused,
    Completed,
    Superseded,
}

pub struct Turn {
    pub message: Message,
    pub events: Vec<String>,
    pub result: Option<String>,
    pub status: TurnStatus,
    pub step: u8,
    pub needs_approval: bool,
}

impl App {
    /// An explicit review fixture, never the startup experience.
    pub fn review() -> Self {
        let mut app = Self::default();
        app.load_scenario("approval");
        app.approval_focus = false;
        app.queue.push(Message {
            text: "Verifica a reutilização de tokens.".into(),
            selection: app.profiles[2].clone(),
            demo: true,
        });
        app
    }

    pub fn load_scenario(&mut self, id: &str) {
        if !self.scenes.iter().any(|scene| scene.id == id) {
            return;
        }
        if self.queue_edit.is_some() {
            self.notice = "Salve ou cancele a edição antes de carregar um cenário.".into();
            return;
        }
        if let Some(index) = self.active.take() {
            self.turns[index].status = TurnStatus::Superseded;
            self.turns[index].result =
                Some("Demonstração anterior encerrada ao trocar de cenário.".into());
        }
        self.pending = None;
        self.approval_focus = false;
        self.set_scene(id);
        self.scroll = 0;
        self.pane = 0;
        if id == "empty" {
            self.notice = "Sessão vazia; fila e histórico anteriores preservados.".into();
            return;
        }
        let message = Message {
            text: "Demonstração: corrigir a rotação de tokens, preservando minhas alterações."
                .into(),
            selection: self.profiles[2].clone(),
            demo: true,
        };
        self.executing = message.selection.clone();
        let waiting = self.scenario().status == "waiting-for-approval";
        self.turns.push(Turn {
            message,
            events: vec![
                "Read auth/service.rs (simulado) · 82 linhas".into(),
                "Plano: inspecionar → alterar → validar (fixture)".into(),
            ],
            result: if id == "completed" {
                Some(
                    "Exemplo concluído · 183 testes passaram (fixture). Nenhum arquivo alterado."
                        .into(),
                )
            } else {
                None
            },
            status: if id == "completed" {
                TurnStatus::Completed
            } else if waiting {
                TurnStatus::Waiting
            } else if self.scenario().blocking {
                TurnStatus::Paused
            } else {
                TurnStatus::Processing
            },
            step: if waiting { 2 } else { 0 },
            needs_approval: waiting,
        });
        if id == "completed" {
            let turn = self.turns.last_mut().unwrap();
            turn.events
                .push("Edit auth/service.rs (simulado) · +12 −3".into());
            turn.events
                .push("Test cargo test (simulado) · 183 passaram · 4,8s".into());
        }
        if id != "completed" {
            self.active = Some(self.turns.len() - 1);
        }
        if waiting {
            self.request_approval();
            self.approval_action = match id {
                "approval" => "git push origin feature/refresh-token (simulado) · escrita remota",
                "sandbox" => "Trabalhar sem sandbox (simulado)",
                "trust" => "Confiar no workspace de exemplo (simulado)",
                "shared" => "Editar diretório compartilhado (simulado)",
                "secret" => "Ler arquivo sensível .env.example (simulado)",
                _ => "Ação protegida (simulado)",
            }
            .into();
        }
        if self.scenario().blocking && !waiting {
            let summary = self.scenario().summary.clone();
            self.turns.last_mut().unwrap().events.push(summary);
        }
        self.elapsed = Duration::ZERO;
        self.notice = format!("Cenário de revisão: {} (simulado)", self.scenario().label);
    }

    pub fn transcript(&self) -> String {
        if self.turns.is_empty() {
            return "Prévia interativa · sem modelo conectado\n\nConverse para testar o editor e a fila.\n/demo inicia o exemplo de execução com aprovação.".into();
        }
        let mut text = String::new();
        for turn in &self.turns {
            text.push_str(&format!("> {}\n\n", turn.message.text));
            if self.verbose {
                text.push_str(&format!(
                    "  {} / {} / {}\n",
                    turn.message.selection.name,
                    turn.message.selection.model,
                    turn.message.selection.variant
                ));
            }
            if !self.verbose && turn.status == TurnStatus::Completed {
                let tools: Vec<&str> = turn
                    .events
                    .iter()
                    .filter_map(|event| {
                        if event.starts_with("Read ") {
                            Some("Read")
                        } else if event.starts_with("Edit ") {
                            Some("Edit")
                        } else if event.starts_with("Test ") {
                            Some("Test")
                        } else {
                            None
                        }
                    })
                    .collect();
                if !tools.is_empty() {
                    text.push_str(&format!(
                        "  ✓ {} · {} ações\n",
                        tools.join(" · "),
                        tools.len()
                    ));
                }
            }
            for event in &turn.events {
                if !self.verbose && turn.status == TurnStatus::Completed {
                    continue;
                }
                if !self.verbose
                    && !(event.starts_with("Read ")
                        || event.starts_with("Edit ")
                        || event.starts_with("Test "))
                {
                    continue;
                }
                let compact = event.replace(" (simulado)", "").replace(" (fixture)", "");
                text.push_str(&format!("  ✓ {compact}\n"));
                if self.verbose && event.starts_with("Test") {
                    text.push_str("    fixture: exit: 0 · 183 passed · 0 failed · 4,8s\n    Nenhum processo foi iniciado.\n");
                }
            }
            if let Some(result) = &turn.result {
                text.push_str(&format!("\n● {result}\n"));
            } else {
                let status = match turn.status {
                    TurnStatus::Processing => "Em andamento…",
                    TurnStatus::Waiting => "Aguardando sua decisão; fila não avança.",
                    TurnStatus::Paused => "Em pausa; /resume retoma explicitamente.",
                    _ => "Demonstração encerrada.",
                };
                text.push_str(&format!("\n  {status}\n"));
            }
            text.push('\n');
        }
        if let Some(message) = &self.pending {
            text.push_str(&format!(
                "Intervenção pendente: {}\nAplicação na próxima etapa segura.\n",
                message.text
            ));
        }
        text
    }

    pub fn enqueue_demo(&mut self, text: &str) {
        if self.queue_edit.is_some() {
            self.notice = "Salve ou cancele a edição antes de iniciar uma demonstração.".into();
            return;
        }
        if self.scenario().id == "offline" || !self.compatible() {
            self.notice = "Demonstração bloqueada; revise conexão e seleção.".into();
            return;
        }
        self.queue.push(Message {
            text: text.into(),
            selection: self.next_selection().clone(),
            demo: true,
        });
        self.start_next();
    }

    pub fn plan_text(&self) -> String {
        let Some(turn) = self.turns.last() else {
            return "Plano\nNenhum trabalho iniciado. Envie uma mensagem para começar.".into();
        };
        if !turn.message.demo {
            return "Plano\nNenhuma execução solicitada. /demo inicia o roteiro de revisão.".into();
        }
        let mut text = format!("Plano da demonstração\n{}\n\n", turn.message.text);
        for (prefix, label) in [
            ("Read", "Inspecionar autenticação"),
            ("Plano:", "Propor passos"),
            ("Edit", "Demonstrar alteração"),
            ("Test", "Demonstrar validação"),
        ] {
            if turn.message.selection.name == "Planejar" && matches!(prefix, "Edit" | "Test") {
                continue;
            }
            text.push_str(&format!(
                "{} {label}\n",
                if turn.events.iter().any(|e| e.starts_with(prefix)) {
                    "✓"
                } else {
                    "○"
                }
            ));
        }
        text.push_str("\nPlano/atividade são fixtures; nada é executado de verdade.");
        text
    }

    pub fn diff_text(&self) -> String {
        let Some(turn) = self.turns.last() else {
            return "Diff\nNenhuma alteração demonstrada.".into();
        };
        let edited = turn.events.iter().any(|e| e.starts_with("Edit"));
        if !edited && !turn.needs_approval {
            return "Diff\nNenhuma alteração demonstrada neste trabalho.".into();
        }
        format!(
            "{} · auth/service.rs +12 −3 (fixture)\n\n- self.tokens.insert(token).await?;\n+ let mut tx = self.db.begin().await?;\n+ self.tokens.invalidate_previous(&mut tx).await?;\n+ tx.commit().await?;\n\nNenhum arquivo foi alterado. Checkpoints reais ainda não existem.",
            if edited {
                "Alteração demonstrada"
            } else {
                "Prévia antes da aprovação"
            }
        )
    }

    pub fn validation_text(&self) -> String {
        if self.scenario().id == "incomplete" {
            return "Validação incompleta (fixture)\nCallback OAuth externo não validado. Fila pausada.\nNenhum teste real executado.".into();
        }
        let validated = self
            .turns
            .last()
            .is_some_and(|turn| turn.events.iter().any(|e| e.starts_with("Test")));
        if validated {
            "Validação demonstrada\nTest cargo test · 183 passaram · 0 falharam · 4,8s (fixture)\nNenhum processo foi iniciado; não é evidência de testes reais.".into()
        } else {
            "Validação\nNenhuma validação demonstrada neste trabalho.".into()
        }
    }

    pub fn start_next(&mut self) {
        if self.active.is_some()
            || self.pending.is_some()
            || self.queue.is_empty()
            || self.queue_edit.is_some()
            || (self.scenario().blocking && self.scenario().id != "empty")
        {
            return;
        }
        let Some(message) = self.queue.take_next() else {
            return;
        };
        self.start_message(message);
    }

    pub(crate) fn start_message(&mut self, message: Message) {
        self.executing = message.selection.clone();
        self.turns.push(Turn {
            message,
            events: Vec::new(),
            result: None,
            status: TurnStatus::Processing,
            step: 0,
            needs_approval: false,
        });
        self.active = Some(self.turns.len() - 1);
        self.set_scene("running");
        self.elapsed = Duration::ZERO;
        self.scroll = 0;
        self.notice.clear();
        self.approval_action = "Edit auth/service.rs (simulado) · +12 −3".into();
    }

    pub fn advance(&mut self, elapsed: Duration) {
        // Reading a menu or editing the queue freezes the mock clock, not a real job.
        if self.dialog.is_some() || self.queue_edit.is_some() || self.scenario().blocking {
            return;
        }
        self.elapsed += elapsed;
        if self.elapsed < Duration::from_millis(900) {
            return;
        }
        self.elapsed = Duration::ZERO;
        if self.pending.is_some() {
            self.safe_step();
            return;
        }
        self.step_demo();
    }

    pub fn step_demo(&mut self) {
        if self.scenario().blocking || self.queue_edit.is_some() {
            return;
        }
        let Some(index) = self.active else {
            self.start_next();
            return;
        };
        let turn = &mut self.turns[index];
        let response = DeterministicDemoProvider.next(DemoRequest {
            step: turn.step,
            work: if turn.message.selection.name == "Planejar" {
                WorkMode::Plan
            } else {
                WorkMode::Execute
            },
            autonomy: match turn.message.selection.name.as_str() {
                "Auto" => Autonomy::Auto,
                "Yolo" => Autonomy::Yolo,
                _ => Autonomy::Ask,
            },
            explicit_demo: turn.message.demo,
        });
        if response == DemoResponse::Acknowledge {
            turn.result = Some(
                "Mensagem recebida nesta prévia. /demo inicia uma execução demonstrativa.".into(),
            );
            self.complete_turn();
            return;
        }
        match response {
            DemoResponse::Read => turn
                .events
                .push("Read auth/service.rs (simulado) · 82 linhas".into()),
            DemoResponse::Plan => turn
                .events
                .push("Plano: inspecionar → alterar → validar (fixture)".into()),
            DemoResponse::PlanComplete => {
                turn.result = Some("Plano demonstrado: inspecionar a invalidação, propor transação e verificar reutilização.\nPlanejar não altera arquivos nem executa testes.".into());
                self.complete_turn();
                return;
            }
            DemoResponse::ApprovalRequired => {
                self.request_approval();
                return;
            }
            DemoResponse::MockPolicyAllows => turn
                .events
                .push("Policy da demonstração: alteração local permitida".into()),
            DemoResponse::ApprovedActivity => turn.events.push(self.approval_action.clone()),
            DemoResponse::Validation => turn
                .events
                .push("Test cargo test (simulado) · 183 passaram · 4,8s".into()),
            DemoResponse::Complete => {
                self.finish();
                return;
            }
            DemoResponse::Acknowledge => unreachable!("acknowledgment handled before activity"),
        }
        self.turns[index].step += 1;
    }

    fn request_approval(&mut self) {
        if let Some(index) = self.active {
            self.turns[index].status = TurnStatus::Waiting;
            self.turns[index].needs_approval = true;
        }
        if self.scenario().status != "waiting-for-approval" {
            self.set_scene("approval");
        }
        self.approval_focus = self.input.is_empty();
        self.approval_action = "Edit auth/service.rs (simulado) · +12 −3".into();
        self.approval_cursor = 1; // No accidental Enter-to-authorize.
        self.elapsed = Duration::ZERO;
    }

    pub fn approval_text(&self) -> String {
        let action = &self.approval_action;
        format!(
            "Permitir esta ação?\n{action}\nEscopo: apenas esta ação · motivo: demonstrar alteração protegida\n{} 1. Permitir uma vez\n{} 2. Negar e pausar\n{}",
            if self.approval_cursor == 0 { ">" } else { " " },
            if self.approval_cursor == 1 { ">" } else { " " },
            if self.approval_focus {
                "↑/↓ escolhe · 1/2 seleciona · Enter confirma · Tab escreve na fila · Esc interrompe"
            } else {
                "Escrevendo na fila · Tab volta à decisão; Enter só envia a mensagem"
            }
        )
    }

    pub fn complete_turn(&mut self) {
        if let Some(index) = self.active.take() {
            self.turns[index].status = TurnStatus::Completed;
        }
        self.approval_focus = false;
        self.set_scene("completed");
        self.elapsed = Duration::ZERO;
        self.start_next();
    }

    pub fn resume(&mut self) {
        if self.scenario().id != "recovery" {
            return;
        }
        self.notice.clear();
        if let Some(index) = self.active {
            if self.turns[index].needs_approval && self.pending.is_none() {
                let action = self.approval_action.clone();
                self.request_approval();
                self.approval_action = action;
            } else {
                self.turns[index].status = TurnStatus::Processing;
                self.set_scene("running");
            }
        } else {
            self.set_scene("completed");
            self.start_next();
        }
    }

    pub fn pause_demo(&mut self) {
        if let Some(index) = self.active {
            self.turns[index].status = TurnStatus::Paused;
        }
        self.approval_focus = false;
        self.set_scene("recovery");
        if self
            .dialog
            .as_ref()
            .is_some_and(|d| d.menu == Menu::Approval)
        {
            self.dialog = None;
        }
    }
}
