//! Small, keyboard-navigable help pages that fit the minimum terminal size.
pub struct HelpPage {
    pub title: &'static str,
    pub entries: &'static [(&'static str, &'static str)],
    pub note: &'static str,
}

pub const PAGES: [HelpPage; 4] = [
    HelpPage {
        title: "Conversa",
        entries: &[
            ("Enter", "Enviar mensagem"),
            ("Ctrl+J", "Inserir nova linha"),
            ("Shift+Enter / \\+Enter", "Alternativas para nova linha"),
            ("↑ / ↓", "Recuperar mensagens do histórico"),
            ("Tab", "Completar comando iniciado com /"),
            ("Ctrl+O", "Expandir ou recolher detalhes"),
        ],
        note: "Colagem multilinha nunca envia automaticamente.",
    },
    HelpPage {
        title: "Seleção",
        entries: &[
            ("Ctrl+P", "Abrir opções e buscar comandos"),
            ("/", "Abrir opções ao iniciar a mensagem"),
            ("Shift+Tab", "Trocar perfil da próxima mensagem"),
            ("/model", "Escolher modelo; ←/→ muda raciocínio"),
            ("/effort", "Escolher raciocínio compatível"),
            ("↑/↓ · Enter · Esc", "Navegar, confirmar ou cancelar menu"),
        ],
        note: "Alternativas: Ctrl+K paleta; Alt+P/M/V modelo/perfil/raciocínio.",
    },
    HelpPage {
        title: "Fila",
        entries: &[
            ("/queue", "Abrir mensagens e seleções capturadas"),
            ("Enter", "Editar mensagem; no editor, salvar"),
            ("d", "Remover mensagem selecionada"),
            ("- / +", "Mover mensagem para cima ou baixo"),
            ("/intervene", "Aplicar rascunho na próxima etapa segura"),
            ("/safe · /finish", "Simular etapa segura ou conclusão"),
        ],
        note: "Esc cancela edição. Aprovação: 1/2 seleciona; Enter confirma.",
    },
    HelpPage {
        title: "Controle",
        entries: &[
            ("Esc", "Fechar menu; na conversa, interromper"),
            ("Ctrl+C", "Cancelar/interromper; ocioso, limpar"),
            ("Ctrl+Q", "Sair da aplicação"),
            ("Ctrl+T", "Alternar plano e conversa"),
            ("PgUp / PgDown", "Rolar conversa"),
            ("/resume", "Retomar trabalho pausado"),
        ],
        note: "Ctrl+C ocioso 2× sai. Cmd+C copia. Interromper preserva fila e rascunho.",
    },
];
