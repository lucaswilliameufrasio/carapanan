use carapana_ui_prototype::{App, interaction::Menu, render};
use ratatui::{
    Terminal,
    backend::TestBackend,
    buffer::Buffer,
    style::{Color, Modifier},
};
use std::time::Duration;

fn draw(app: &App) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(100, 32)).unwrap();
    terminal.draw(|frame| render(frame, app)).unwrap();
    terminal.backend().buffer().clone()
}

fn cell_at_text<'a>(buffer: &'a Buffer, text: &str) -> &'a ratatui::buffer::Cell {
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            let tail: String = (x..buffer.area.width)
                .map(|column| buffer[(column, y)].symbol())
                .collect();
            if tail.starts_with(text) {
                return &buffer[(x, y)];
            }
        }
    }
    panic!("missing text {text}");
}

#[test]
fn should_distinguish_user_assistant_tools_and_success_with_semantic_colors() {
    let mut app = App {
        input: "mensagem".into(),
        ..App::default()
    };
    app.send(false);
    app.advance(Duration::from_secs(1));
    let buffer = draw(&app);
    assert_eq!(cell_at_text(&buffer, "> mensagem").fg, Color::Indexed(75));
    assert_eq!(cell_at_text(&buffer, "● Mensagem").fg, Color::Indexed(80));
    app.selected = 2;
    app.enqueue_demo("demonstração");
    app.advance(Duration::from_secs(1));
    let buffer = draw(&app);
    assert_eq!(cell_at_text(&buffer, "✓ Read").fg, Color::Indexed(114));
    assert_eq!(cell_at_text(&buffer, "Read auth").fg, Color::Indexed(75));
    for _ in 0..5 {
        app.advance(Duration::from_secs(1));
    }
    assert_eq!(
        cell_at_text(&draw(&app), "● Rotação").fg,
        Color::Indexed(114)
    );
}

#[test]
fn should_use_blue_for_focus_and_amber_for_pending_approval() {
    let mut app = App::default();
    app.open_menu(Menu::Commands);
    let buffer = draw(&app);
    let selected = cell_at_text(&buffer, "> /demo");
    assert_eq!(selected.bg, Color::Indexed(24));
    assert_eq!(selected.fg, Color::White);
    assert!(selected.modifier.contains(Modifier::BOLD));
    app.dialog = None;
    app.enqueue_demo("demonstração");
    for _ in 0..3 {
        app.advance(Duration::from_secs(1));
    }
    assert_eq!(
        cell_at_text(&draw(&app), "Permitir esta ação?").fg,
        Color::Indexed(179)
    );
}

#[test]
fn should_preserve_text_and_selection_in_plain_mode_without_foreground_or_background_colors() {
    let mut app = App {
        plain: true,
        ..App::default()
    };
    app.open_menu(Menu::Commands);
    let buffer = draw(&app);
    assert!(
        buffer
            .content
            .iter()
            .all(|cell| cell.fg == Color::Reset && cell.bg == Color::Reset)
    );
    assert!(
        cell_at_text(&buffer, "> /demo")
            .modifier
            .contains(Modifier::REVERSED)
    );
    app.dialog = None;
    app.enqueue_demo("demonstração");
    for _ in 0..3 {
        app.advance(Duration::from_secs(1));
    }
    let buffer = draw(&app);
    assert!(
        buffer
            .content
            .iter()
            .all(|cell| cell.fg == Color::Reset && cell.bg == Color::Reset)
    );
    assert!(
        cell_at_text(&buffer, "Negar e pausar")
            .modifier
            .contains(Modifier::BOLD)
    );
}

#[test]
fn should_fall_back_to_ansi_colors_without_requiring_256_colors_or_truecolor() {
    let mut app = App {
        ansi256: false,
        ..App::default()
    };
    app.open_menu(Menu::Commands);
    let buffer = draw(&app);
    assert_eq!(cell_at_text(&buffer, "> /demo").bg, Color::Blue);
    assert!(!buffer.content.iter().any(|cell| matches!(
        cell.fg,
        Color::Indexed(_) | Color::Rgb(..)
    ) || matches!(
        cell.bg,
        Color::Indexed(_) | Color::Rgb(..)
    )));
}
