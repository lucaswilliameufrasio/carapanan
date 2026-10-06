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

#[test]
fn should_use_a_distinct_mode_color_and_three_effort_intensities_without_changing_execution() {
    use carapana_ui_prototype::palette::Palette;
    let colors = ["Planejar", "Perguntar", "Auto", "Yolo"]
        .map(|name| Palette::mode(false, true, name, "default").fg);
    for (i, color) in colors.iter().enumerate() {
        assert!(!colors[..i].contains(color));
    }
    for mode in ["Planejar", "Perguntar", "Auto", "Yolo"] {
        let levels =
            ["low", "default", "high"].map(|level| Palette::mode(false, true, mode, level).fg);
        assert_ne!(levels[0], levels[1]);
        assert_ne!(levels[1], levels[2]);
        assert_eq!(Palette::mode(true, true, mode, "low").fg, None);
    }
    let mut app = App::default();
    let executing = app.executing.name.clone();
    for _ in 0..4 {
        app.cycle_profile();
        let _ = draw(&app);
        assert_eq!(app.executing.name, executing);
    }
}

#[test]
fn should_animate_only_the_ascii_wings_and_keep_plain_and_disabled_animation_static() {
    let mut app = App::default();
    let first = draw(&app);
    app.visual_elapsed = Duration::from_millis(250);
    let next = draw(&app);
    assert_ne!(first, next);
    assert_eq!(
        cell_at_text(&next, "carapanã").fg,
        cell_at_text(&first, "carapanã").fg
    );
    app.animated = false;
    let static_frame = draw(&app);
    app.visual_elapsed = Duration::ZERO;
    assert_eq!(static_frame, draw(&app));
    app.plain = true;
    app.animated = true;
    let plain_frame = draw(&app);
    app.visual_elapsed = Duration::from_millis(250);
    assert_eq!(plain_frame, draw(&app));
}

#[test]
fn should_render_approval_as_distinct_vertical_options_with_focus_only_on_the_selected_row() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut app = App::default();
    app.enqueue_demo("demonstração");
    for _ in 0..3 {
        app.advance(Duration::from_secs(1));
    }
    for (width, height) in [(80, 24), (160, 48)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| render(frame, &app)).unwrap();
        let buffer = terminal.backend().buffer();
        let lines: Vec<String> = (0..height)
            .map(|y| (0..width).map(|x| buffer[(x, y)].symbol()).collect())
            .collect();
        let allow_row = lines
            .iter()
            .position(|line| line.contains("1. Permitir uma vez"))
            .unwrap();
        let deny_row = lines
            .iter()
            .position(|line| line.contains("> 2. Negar e pausar"))
            .unwrap();
        assert_eq!(deny_row, allow_row + 1);
        assert_eq!(
            cell_at_text(buffer, "Negar e pausar").bg,
            Color::Indexed(24)
        );
        assert_eq!(cell_at_text(buffer, "Permitir uma vez").bg, Color::Reset);
        assert_eq!(
            cell_at_text(buffer, "Somente esta ação").fg,
            Color::Indexed(245)
        );
        // The selected band spans the option row, not just its label.
        assert_eq!(buffer[(4, deny_row as u16)].bg, Color::Indexed(24));
    }
    app.key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(app.scenario().id, "approval"); // Navigation never authorizes.
    let buffer = draw(&app);
    assert_eq!(
        cell_at_text(&buffer, "Permitir uma vez").bg,
        Color::Indexed(24)
    );
    assert_eq!(cell_at_text(&buffer, "Negar e pausar").bg, Color::Reset);
    app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(
        cell_at_text(&draw(&app), "Permitir uma vez").bg,
        Color::Reset
    );
    app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(
        cell_at_text(&draw(&app), "Permitir uma vez").bg,
        Color::Indexed(24)
    );
}
