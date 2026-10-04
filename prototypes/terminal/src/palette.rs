//! Semantic terminal colors. Backgrounds remain owned by the terminal theme.
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

pub struct Palette {
    pub interaction: Style,
    pub assistant: Style,
    pub success: Style,
    pub attention: Style,
    pub muted: Style,
    pub selected: Style,
}

impl Palette {
    pub fn new(plain: bool, ansi256: bool) -> Self {
        let color = |index, fallback| {
            if plain {
                Style::default()
            } else {
                Style::default().fg(if ansi256 {
                    Color::Indexed(index)
                } else {
                    fallback
                })
            }
        };
        Self {
            interaction: color(75, Color::Blue),
            assistant: color(80, Color::Cyan),
            success: color(114, Color::Green),
            attention: color(179, Color::Yellow).add_modifier(Modifier::BOLD),
            muted: color(245, Color::DarkGray),
            selected: if plain {
                Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
            } else {
                Style::default()
                    .fg(Color::White)
                    .bg(if ansi256 {
                        Color::Indexed(24)
                    } else {
                        Color::Blue
                    })
                    .add_modifier(Modifier::BOLD)
            },
        }
    }

    pub fn transcript_line<'a>(&self, text: &'a str) -> Line<'a> {
        if let Some(message) = text.strip_prefix("> ") {
            return Line::from(vec![
                Span::styled("> ", self.interaction.add_modifier(Modifier::BOLD)),
                Span::styled(message, Style::default().add_modifier(Modifier::BOLD)),
            ]);
        }
        if let Some(reply) = text.strip_prefix("● ") {
            let marker = if reply.starts_with("Rotação demonstrada")
                || reply.starts_with("Plano demonstrado")
            {
                self.success
            } else {
                self.assistant
            };
            return Line::from(vec![
                Span::styled("● ", marker.add_modifier(Modifier::BOLD)),
                Span::raw(reply),
            ]);
        }
        if let Some(activity) = text.trim_start().strip_prefix("✓ ") {
            let (name, detail) = activity.split_once(' ').unwrap_or((activity, ""));
            return Line::from(vec![
                Span::styled("  ✓ ", self.success),
                Span::styled(name, self.interaction),
                Span::styled(format!(" {detail}"), self.muted),
            ]);
        }
        let style = if text.contains("Aguardando sua decisão")
            || text.contains("Em pausa")
            || text.starts_with("Intervenção pendente")
        {
            self.attention
        } else if text.contains("Em andamento") {
            self.assistant
        } else if text.starts_with("    ") {
            self.muted
        } else {
            Style::default()
        };
        Line::styled(text, style)
    }
}
