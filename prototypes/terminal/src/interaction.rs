//! Mock-only input and dialog state. Browsing never changes a selected configuration.
use crate::{App, Selection};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Menu {
    Commands,
    Model,
    Profile,
    Effort,
    Queue,
    Scenario,
    Approval,
    Config,
    Help,
    Updates,
    UpdatePackage,
    UpdateChannel,
}

#[derive(Clone)]
pub struct Dialog {
    pub menu: Menu,
    pub query: String,
    pub cursor: usize,
    pub variant: String,
}

pub const COMMANDS: [(&str, &str); 26] = [
    ("/demo", "Iniciar exemplo de execução com aprovação"),
    ("/model", "Escolher modelo e raciocínio"),
    ("/profile", "Escolher perfil da próxima mensagem"),
    ("/effort", "Escolher variante de raciocínio"),
    ("/queue", "Revisar e editar mensagens da fila"),
    ("/config", "Configurações da sessão"),
    ("/sessions", "Sessões e atenção"),
    ("/plan", "Plano de trabalho"),
    ("/diff", "Alterações e checkpoint"),
    ("/validation", "Evidências de validação"),
    ("/mcp", "Servidores e permissões MCP"),
    ("/skills", "Skills disponíveis"),
    ("/providers", "Providers simulados"),
    ("/devices", "Pareamento e acesso remoto"),
    ("/resources", "Recursos e limites"),
    ("/doctor", "Diagnóstico simulado"),
    ("/scenario", "Selecionar cenário de revisão"),
    ("/approve", "Responder à aprovação"),
    ("/intervene", "Intervir com o rascunho na etapa segura"),
    ("/safe", "Simular etapa segura"),
    ("/finish", "Simular conclusão"),
    ("/pause", "Pausar sem perder a fila"),
    ("/resume", "Retomar após confirmação"),
    ("/stop", "Parar preservando alterações"),
    (
        "/update",
        "Atualizações: versão, reinício e rollback simulados",
    ),
    ("/help", "Teclado e comandos"),
];

pub fn variants(model: &str) -> &'static [&'static str] {
    if model == "Claude mock" {
        &["default", "low"]
    } else {
        &["default", "low", "high"]
    }
}

pub fn shortcut(command: &str) -> &'static str {
    match command {
        "/model" => "Alt+P",
        "/profile" => "Alt+M",
        "/effort" => "Alt+V",
        "/queue" => "F5",
        "/scenario" => "F4",
        "/intervene" => "Alt+I",
        "/safe" => "F2",
        "/finish" => "F3",
        "/plan" => "Ctrl+T",
        "/help" => "?",
        _ => "",
    }
}

impl App {
    pub fn next_selection(&self) -> &Selection {
        self.edit_selection
            .as_ref()
            .unwrap_or(&self.profiles[self.selected])
    }

    fn set_next(&mut self, selection: Selection) {
        if self.queue_edit.is_some() {
            self.edit_selection = Some(selection);
        } else {
            self.profiles[self.selected] = selection;
        }
    }

    pub fn open_menu(&mut self, menu: Menu) {
        let cursor = match menu {
            Menu::Profile => self.selected,
            Menu::Model => usize::from(self.next_selection().model == "Claude mock"),
            Menu::Effort => variants(&self.next_selection().model)
                .iter()
                .position(|v| *v == self.next_selection().variant)
                .unwrap_or(0),
            Menu::Scenario => self.scene,
            Menu::Approval => 1,
            _ => 0,
        };
        self.dialog = Some(Dialog {
            menu,
            query: String::new(),
            cursor,
            variant: self.next_selection().variant.clone(),
        });
    }

    pub fn menu_items(&self) -> Vec<(String, String)> {
        let Some(dialog) = &self.dialog else {
            return vec![];
        };
        let items: Vec<(String, String)> = match dialog.menu {
            Menu::Commands => COMMANDS
                .iter()
                .map(|(cmd, desc)| (cmd.to_string(), desc.to_string()))
                .collect(),
            Menu::Model => vec![
                ("GPT mock".into(), "OpenAI / ChatGPT · fixture".into()),
                ("Claude mock".into(), "Anthropic · fixture".into()),
            ],
            Menu::Profile => self
                .profiles
                .iter()
                .map(|p| (p.name.clone(), format!("{} / {}", p.model, p.variant)))
                .collect(),
            Menu::Effort => variants(&self.next_selection().model)
                .iter()
                .map(|v| {
                    (
                        v.to_string(),
                        if *v == self.next_selection().variant {
                            "Selecionado".into()
                        } else {
                            "Próxima mensagem apenas".into()
                        },
                    )
                })
                .collect(),
            Menu::Scenario => self
                .scenes
                .iter()
                .map(|s| (s.id.clone(), s.label.clone()))
                .collect(),
            Menu::Queue => self
                .queue
                .iter()
                .enumerate()
                .map(|(i, m)| {
                    (
                        format!("{}: {}", i + 1, m.text),
                        format!(
                            "{} / {} / {}",
                            m.selection.name, m.selection.model, m.selection.variant
                        ),
                    )
                })
                .collect(),
            Menu::Approval => vec![
                (
                    "Permitir uma vez".into(),
                    "Somente esta ação simulada".into(),
                ),
                ("Negar".into(), "Pausar e preservar fila".into()),
            ],
            Menu::Config => vec![
                ("/profile".into(), "Perfil da próxima mensagem".into()),
                ("/model".into(), "Modelo e raciocínio".into()),
                ("/effort".into(), "Variante compatível".into()),
                ("/resources".into(), "Limites e consumo".into()),
                ("/providers".into(), "Providers".into()),
                ("/devices".into(), "Dispositivos".into()),
                ("/update".into(), "Atualizações simuladas".into()),
                ("/help".into(), "Ajuda".into()),
            ],
            Menu::Help => vec![],
            Menu::Updates => self
                .update
                .actions()
                .iter()
                .map(|a| (a.label().into(), String::new()))
                .collect(),
            Menu::UpdatePackage => crate::updates::Package::ALL
                .iter()
                .map(|p| {
                    (
                        p.label().into(),
                        "Trocar reinicia apenas a prévia de atualização".into(),
                    )
                })
                .collect(),
            Menu::UpdateChannel => crate::updates::Channel::ALL
                .iter()
                .map(|c| {
                    (
                        c.label().into(),
                        "Canal fictício; nenhuma consulta remota".into(),
                    )
                })
                .collect(),
        };
        if matches!(dialog.menu, Menu::Queue | Menu::Updates) {
            return items;
        }
        let query = dialog.query.trim_start_matches('/').to_lowercase();
        items
            .into_iter()
            .filter(|(name, desc)| format!("{name} {desc}").to_lowercase().contains(&query))
            .collect()
    }

    pub fn command(&mut self, command: &str) {
        self.dialog = None;
        match command {
            "/demo" => self.enqueue_demo("Demonstração: corrigir a rotação dos tokens."),
            "/model" => self.open_menu(Menu::Model),
            "/profile" => self.open_menu(Menu::Profile),
            "/effort" => self.open_menu(Menu::Effort),
            "/queue" if self.queue_edit.is_some() => {
                self.notice = "Salve ou cancele a edição atual antes de reabrir a fila.".into()
            }
            "/queue" => self.open_menu(Menu::Queue),
            "/scenario" => self.open_menu(Menu::Scenario),
            "/config" => self.open_menu(Menu::Config),
            "/update" => self.open_menu(Menu::Updates),
            "/approve"
                if self.scenario().status == "waiting-for-approval" && self.pending.is_none() =>
            {
                self.open_menu(Menu::Approval)
            }
            "/approve" => self.notice = "Nenhuma aprovação válida pendente.".into(),
            "/help" => self.open_menu(Menu::Help),
            "/intervene" => self.send(true),
            "/safe" => self.safe_step(),
            "/finish" => self.finish(),
            "/pause" | "/stop" if self.scenario().id != "offline" => {
                self.pause_demo();
                self.notice = "Pausado (mock): fila e alterações preservadas.".into();
            }
            "/resume" => self.resume(),
            _ => {
                if let Some(pane) = [
                    "/conversation",
                    "/plan",
                    "/diff",
                    "/validation",
                    "/sessions",
                    "/mcp",
                    "/skills",
                    "/providers",
                    "/devices",
                    "/resources",
                    "/settings",
                    "/doctor",
                    "/effective",
                ]
                .iter()
                .position(|cmd| *cmd == command)
                {
                    self.pane = pane;
                    self.scroll = 0;
                }
            }
        }
    }

    fn confirm_menu(&mut self) {
        let Some(dialog) = self.dialog.clone() else {
            return;
        };
        let items = self.menu_items();
        let Some((name, _)) = items.get(dialog.cursor) else {
            return;
        };
        match dialog.menu {
            Menu::Commands | Menu::Config => self.command(name),
            Menu::Model => {
                if !variants(name).contains(&dialog.variant.as_str()) {
                    self.notice =
                        "Variante incompatível: use esquerda/direita para escolher.".into();
                    return;
                }
                let mut selection = self.next_selection().clone();
                selection.model = name.clone();
                selection.variant = dialog.variant;
                self.set_next(selection);
                self.dialog = None;
                self.notice = "Seleção confirmada; execução atual não mudou.".into();
            }
            Menu::Profile => {
                let index = self.profiles.iter().position(|p| p.name == *name).unwrap();
                if self.queue_edit.is_some() {
                    self.edit_selection = Some(self.profiles[index].clone());
                } else {
                    self.selected = index;
                }
                self.dialog = None;
            }
            Menu::Effort => {
                let mut selection = self.next_selection().clone();
                selection.variant = name.clone();
                self.set_next(selection);
                self.dialog = None;
            }
            Menu::Scenario => {
                self.load_scenario(name);
                self.dialog = None;
                self.pane = 0;
                self.scroll = 0;
                self.notice =
                    "Cenário carregado explicitamente; rascunho e fila preservados.".into();
            }
            Menu::Queue => {
                if self.scenario().id == "offline" {
                    self.notice = "Desconectado: fila não alterada.".into();
                    return;
                }
                let index = dialog.cursor;
                self.saved_draft = Some(self.input.clone());
                self.input = self.queue[index].text.clone();
                self.input_cursor = None;
                self.edit_selection = Some(self.queue[index].selection.clone());
                self.queue_edit = Some(index);
                self.dialog = None;
                self.notice = "Editando fila: Enter salva; Esc cancela; Alt+P abre modelo.".into();
            }
            Menu::Approval => {
                self.approve(dialog.cursor == 0);
                self.dialog = None;
            }
            Menu::Help => {}
            Menu::Updates => {
                if let Some(action) = self.update.actions().get(dialog.cursor) {
                    self.update_action(*action);
                }
            }
            Menu::UpdatePackage => {
                if self.scenario().id == "offline" {
                    self.update.feedback = "Desconectado: ações de atualização bloqueadas.".into();
                    self.open_menu(Menu::Updates);
                    return;
                }
                if let Some(package) = crate::updates::Package::ALL
                    .iter()
                    .find(|p| p.label() == name)
                {
                    self.update.select_package(*package);
                    self.open_menu(Menu::Updates);
                }
            }
            Menu::UpdateChannel => {
                if self.scenario().id == "offline" {
                    self.update.feedback = "Desconectado: ações de atualização bloqueadas.".into();
                    self.open_menu(Menu::Updates);
                    return;
                }
                if let Some(channel) = crate::updates::Channel::ALL
                    .iter()
                    .find(|c| c.label() == name)
                {
                    self.update.select_channel(*channel);
                    self.open_menu(Menu::Updates);
                }
            }
        }
    }

    /// Returns true only when exiting the application. The editor is always ready.
    pub fn key(&mut self, key: KeyEvent) -> bool {
        if key.kind != KeyEventKind::Press {
            return false;
        }
        let ctrl_c =
            key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c');
        if !ctrl_c {
            self.exit_armed = None;
        }
        if ctrl_c {
            if self.dialog.take().is_some() {
                self.notice = "Menu cancelado; rascunho e execução preservados.".into();
            } else if self.queue_edit.is_some() {
                self.cancel_queue_edit();
            } else if !self.interrupt() {
                if self
                    .exit_armed
                    .is_some_and(|at| at.elapsed() < std::time::Duration::from_secs(2))
                {
                    return true;
                }
                self.input.clear();
                self.input_cursor = None;
                self.history_index = None;
                self.history_draft = None;
                self.exit_armed = Some(std::time::Instant::now());
                self.notice =
                    "Input limpo. Ctrl+C novamente em até 2s sai; Ctrl+Q sai diretamente.".into();
            }
            return false;
        }
        if key.code == KeyCode::Char('p')
            && (key.modifiers.contains(KeyModifiers::SUPER)
                || (key.modifiers.contains(KeyModifiers::CONTROL) && self.dialog.is_none()))
        {
            self.open_menu(Menu::Commands);
            return false;
        }
        // Command is handled by the terminal (for example Cmd+C copies), not
        // inserted as a literal character or treated as an interrupt alias.
        if key.modifiers.contains(KeyModifiers::SUPER) {
            return false;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('q') => return true,
                KeyCode::Char('k') => {
                    self.open_menu(Menu::Commands);
                    return false;
                }
                KeyCode::Char('o') => {
                    if self.pane == 0 {
                        self.verbose = !self.verbose;
                    } else {
                        self.pane = 0;
                    }
                    self.scroll = 0;
                    return false;
                }
                KeyCode::Char('t') if self.dialog.is_none() => {
                    self.pane = if self.pane == 1 { 0 } else { 1 };
                    self.scroll = 0;
                    return false;
                }
                KeyCode::Char('j') if self.dialog.is_none() => {
                    self.insert_char('\n');
                    return false;
                }
                KeyCode::Char('u') if self.dialog.is_none() => {
                    self.input.clear();
                    self.input_cursor = None;
                    return false;
                }
                KeyCode::Char('a') if self.dialog.is_none() => {
                    self.input_cursor = Some(0);
                    return false;
                }
                KeyCode::Char('e') if self.dialog.is_none() => {
                    self.input_cursor = None;
                    return false;
                }
                _ => {}
            }
        }
        if self.dialog.is_some() {
            self.dialog_key(key);
            return false;
        }
        if key.modifiers.contains(KeyModifiers::ALT) {
            match key.code {
                KeyCode::Char('p') => self.open_menu(Menu::Model),
                KeyCode::Char('m') => self.open_menu(Menu::Profile),
                KeyCode::Char('v') => self.open_menu(Menu::Effort),
                KeyCode::Char('i') => self.send(true),
                _ => {}
            }
            return false;
        }
        if self.scenario().status == "waiting-for-approval" && self.pending.is_none() {
            if key.code == KeyCode::Tab {
                self.approval_focus = !self.approval_focus;
                return false;
            }
            if self.approval_focus {
                match key.code {
                    KeyCode::Left | KeyCode::Up | KeyCode::Char('1') => self.approval_cursor = 0,
                    KeyCode::Right | KeyCode::Down | KeyCode::Char('2') => self.approval_cursor = 1,
                    KeyCode::Enter => self.approve(self.approval_cursor == 0),
                    KeyCode::Esc => {
                        self.interrupt();
                    }
                    KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                        self.approval_focus = false;
                        self.insert_char(c);
                    }
                    _ => {}
                }
                return false;
            }
        }
        match key.code {
            KeyCode::BackTab => self.cycle_profile(),
            KeyCode::PageUp => self.scroll = self.scroll.saturating_add(5),
            KeyCode::PageDown => {
                self.scroll = self.scroll.saturating_sub(5);
            }
            KeyCode::F(2) => self.safe_step(),
            KeyCode::F(3) => self.finish(),
            KeyCode::F(4) => self.open_menu(Menu::Scenario),
            KeyCode::F(5) => self.command("/queue"),
            KeyCode::Esc => {
                if self.queue_edit.is_some() {
                    self.cancel_queue_edit();
                } else {
                    self.interrupt();
                }
            }
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => self.insert_char('\n'),
            KeyCode::Enter
                if self.input[..self.input_cursor.unwrap_or(self.input.len())].ends_with('\\') =>
            {
                let cursor = self.input_cursor.unwrap_or(self.input.len());
                self.input.remove(cursor - 1);
                self.input_cursor = Some(cursor - 1);
                self.insert_char('\n');
            }
            KeyCode::Enter => self.send(false),
            KeyCode::Up
                if (!self.input.contains('\n') || self.history_index.is_some())
                    && self.queue_edit.is_none() =>
            {
                self.browse_history(true)
            }
            KeyCode::Down
                if (!self.input.contains('\n') || self.history_index.is_some())
                    && self.queue_edit.is_none() =>
            {
                self.browse_history(false)
            }
            KeyCode::Left => {
                let cursor = self.input_cursor.unwrap_or(self.input.len());
                self.input_cursor = Some(
                    self.input[..cursor]
                        .char_indices()
                        .last()
                        .map_or(0, |(i, _)| i),
                );
            }
            KeyCode::Right => {
                let cursor = self.input_cursor.unwrap_or(self.input.len());
                self.input_cursor = Some(
                    cursor
                        + self.input[cursor..]
                            .chars()
                            .next()
                            .map_or(0, char::len_utf8),
                );
            }
            KeyCode::Home => self.input_cursor = Some(0),
            KeyCode::End => self.input_cursor = None,
            KeyCode::Delete => {
                let cursor = self.input_cursor.unwrap_or(self.input.len());
                if cursor < self.input.len() {
                    self.input.remove(cursor);
                }
            }
            KeyCode::Backspace => {
                let cursor = self.input_cursor.unwrap_or(self.input.len());
                if let Some((previous, _)) = self.input[..cursor].char_indices().last() {
                    self.input.remove(previous);
                    self.input_cursor = Some(previous);
                }
            }
            KeyCode::Char('/') if self.input.is_empty() => self.open_menu(Menu::Commands),
            KeyCode::Char('?') if self.input.is_empty() => self.open_menu(Menu::Help),
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.insert_char(c)
            }
            _ => {}
        }
        false
    }

    fn insert_char(&mut self, c: char) {
        let cursor = self.input_cursor.unwrap_or(self.input.len());
        self.input.insert(cursor, c);
        self.input_cursor = Some(cursor + c.len_utf8());
    }

    pub fn paste(&mut self, text: &str) {
        if self.dialog.is_some() {
            return;
        }
        self.exit_armed = None;
        // Paste is data, never command/key events. Normalize terminal line endings.
        for c in text.replace("\r\n", "\n").replace('\r', "\n").chars() {
            if c == '\n' || c == '\t' || !c.is_control() {
                self.insert_char(c);
            }
        }
    }

    fn interrupt(&mut self) -> bool {
        if matches!(self.scenario().id.as_str(), "offline" | "recovery") {
            return false;
        }
        if self.pending.is_some()
            || matches!(
                self.scenario().id.as_str(),
                "running" | "approval" | "sandbox" | "trust" | "shared" | "secret"
            )
        {
            self.pause_demo();
            self.notice = "Interrompido (mock); fila, rascunho e alterações preservados. /resume retoma explicitamente.".into();
            // A pending intervention remains visible, but a second interrupt is idle.
            return true;
        }
        false
    }

    fn cancel_queue_edit(&mut self) {
        self.queue_edit = None;
        self.input = self.saved_draft.take().unwrap_or_default();
        self.edit_selection = None;
        self.input_cursor = None;
        self.notice = "Edição cancelada; fila e rascunho preservados.".into();
    }

    fn browse_history(&mut self, previous: bool) {
        if self.history.is_empty() {
            return;
        }
        let index = if previous {
            if self.history_index.is_none() {
                self.history_draft = Some(self.input.clone());
            }
            Some(
                self.history_index
                    .unwrap_or(self.history.len())
                    .saturating_sub(1),
            )
        } else {
            self.history_index.and_then(|i| {
                if i + 1 < self.history.len() {
                    Some(i + 1)
                } else {
                    None
                }
            })
        };
        if !previous && self.history_index.is_none() {
            return;
        }
        self.history_index = index;
        self.input = index.map_or_else(
            || self.history_draft.take().unwrap_or_default(),
            |i| self.history[i].clone(),
        );
        self.input_cursor = None;
    }

    fn dialog_key(&mut self, key: KeyEvent) {
        let menu = self.dialog.as_ref().unwrap().menu;
        let count = self.menu_items().len();
        if key.code == KeyCode::Esc {
            if matches!(menu, Menu::UpdatePackage | Menu::UpdateChannel) {
                self.open_menu(Menu::Updates);
            } else {
                self.dialog = None;
            }
            return;
        }
        if menu == Menu::Help {
            let dialog = self.dialog.as_mut().unwrap();
            let pages = crate::help::PAGES.len();
            match key.code {
                KeyCode::Left | KeyCode::Up | KeyCode::BackTab => {
                    dialog.cursor = (dialog.cursor + pages - 1) % pages
                }
                KeyCode::Right | KeyCode::Down | KeyCode::Tab => {
                    dialog.cursor = (dialog.cursor + 1) % pages
                }
                KeyCode::Char(number @ '1'..='4') if key.modifiers.is_empty() => {
                    dialog.cursor = number as usize - '1' as usize
                }
                _ => {}
            }
            return;
        }
        if key.code == KeyCode::Enter {
            self.confirm_menu();
            return;
        }
        if key.code == KeyCode::Tab && menu == Menu::Commands {
            if let Some((name, _)) = self.menu_items().get(self.dialog.as_ref().unwrap().cursor) {
                let name = name.trim_start_matches('/').to_string();
                let dialog = self.dialog.as_mut().unwrap();
                dialog.query = name;
                dialog.cursor = 0;
            }
            return;
        }
        if menu == Menu::Queue && matches!(key.code, KeyCode::Char('d' | '-' | '+')) {
            if self.scenario().id == "offline" {
                self.notice = "Desconectado: fila não alterada.".into();
                return;
            }
            let dialog = self.dialog.as_mut().unwrap();
            let index = dialog.cursor;
            if index >= self.queue.len() {
                return;
            }
            match key.code {
                KeyCode::Char('d') => {
                    self.queue.remove(index);
                    dialog.cursor = index.min(self.queue.len().saturating_sub(1));
                }
                KeyCode::Char('-') if index > 0 => {
                    if let Some(target) = self.queue.move_by(index, -1) {
                        dialog.cursor = target;
                    }
                }
                KeyCode::Char('+') if index + 1 < self.queue.len() => {
                    if let Some(target) = self.queue.move_by(index, 1) {
                        dialog.cursor = target;
                    }
                }
                _ => {}
            }
            return;
        }
        if menu == Menu::Model && matches!(key.code, KeyCode::Left | KeyCode::Right) {
            let items = self.menu_items();
            let dialog = self.dialog.as_mut().unwrap();
            if let Some((model, _)) = items.get(dialog.cursor) {
                let options = variants(model);
                let current = options.iter().position(|v| *v == dialog.variant);
                let index = match (current, key.code) {
                    (Some(i), KeyCode::Left) => (i + options.len() - 1) % options.len(),
                    (Some(i), _) => (i + 1) % options.len(),
                    _ => 0,
                };
                dialog.variant = options[index].into();
            }
            return;
        }
        let dialog = self.dialog.as_mut().unwrap();
        match key.code {
            KeyCode::Up | KeyCode::Char('p')
                if key.code == KeyCode::Up || key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                dialog.cursor = dialog.cursor.saturating_sub(1)
            }
            KeyCode::Down | KeyCode::Char('n')
                if key.code == KeyCode::Down || key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                dialog.cursor = (dialog.cursor + 1).min(count.saturating_sub(1))
            }
            KeyCode::Backspace => {
                dialog.query.pop();
                dialog.cursor = 0;
            }
            KeyCode::Char(c)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !matches!(
                        menu,
                        Menu::Help | Menu::Queue | Menu::Approval | Menu::Updates
                    ) =>
            {
                dialog.query.push(c);
                dialog.cursor = 0;
            }
            _ => {}
        }
    }
}
