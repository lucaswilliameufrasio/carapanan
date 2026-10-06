use carapana_ui_prototype::{App, help::PAGES, interaction::Menu, render};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend, style::Color};

#[test]
fn should_show_every_help_entry_and_navigation_hint_without_clipping_at_minimum_size() {
    for plain in [false, true] {
        let mut app = App {
            plain,
            input: "rascunho\nsegunda\nterceira\nquarta".into(),
            ..App::default()
        };
        app.open_menu(Menu::Help);
        for (index, page) in PAGES.iter().enumerate() {
            app.key(KeyEvent::new(
                KeyCode::Char(char::from(b'1' + index as u8)),
                KeyModifiers::NONE,
            ));
            let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
            terminal.draw(|frame| render(frame, &app)).unwrap();
            let buffer = terminal.backend().buffer();
            let lines: Vec<String> = (0..24)
                .map(|y| (0..80).map(|x| buffer[(x, y)].symbol()).collect())
                .collect();
            for (key, description) in page.entries {
                let row = lines
                    .iter()
                    .position(|line| line.contains(key) && line.contains(description))
                    .unwrap_or_else(|| panic!("clipped {key}: {description}"));
                let column = lines[row]
                    .chars()
                    .position(|c| c == key.chars().next().unwrap())
                    .unwrap();
                assert_eq!(
                    buffer[(column as u16, row as u16)].fg,
                    if plain {
                        Color::Reset
                    } else {
                        Color::Indexed(75)
                    }
                );
            }
            let text = lines.join("\n");
            assert!(text.contains(page.note));
            assert!(text.contains("1–4 acesso direto"), "{text}");
            assert!(text.contains("Esc fecha"));
        }
    }
}

#[test]
fn should_navigate_help_topics_and_close_without_mutating_the_draft_or_selection() {
    let mut app = App {
        input: "rascunho".into(),
        ..App::default()
    };
    let selection = app.next_selection().clone();
    app.open_menu(Menu::Help);
    for (key, page) in [
        (KeyCode::Left, 3),
        (KeyCode::Tab, 0),
        (KeyCode::Right, 1),
        (KeyCode::Down, 2),
        (KeyCode::BackTab, 1),
        (KeyCode::Char('4'), 3),
    ] {
        app.key(KeyEvent::new(key, KeyModifiers::NONE));
        assert_eq!(app.dialog.as_ref().unwrap().cursor, page);
    }
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(app.turns.is_empty());
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.dialog.is_none());
    assert_eq!(app.input, "rascunho");
    assert_eq!(app.next_selection().name, selection.name);
    assert_eq!(app.next_selection().model, selection.model);
    assert_eq!(app.next_selection().variant, selection.variant);
}
