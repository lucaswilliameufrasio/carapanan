use carapana_ui_prototype::{App, Message, interaction::Menu, render};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};

fn press(app: &mut App, code: KeyCode) {
    app.key(KeyEvent::new(code, KeyModifiers::NONE));
}
fn type_text(app: &mut App, text: &str) {
    for c in text.chars() {
        press(app, KeyCode::Char(c));
    }
}

#[test]
fn should_open_palette_with_ctrl_p_or_forwarded_command_p_without_losing_draft() {
    for modifier in [KeyModifiers::CONTROL, KeyModifiers::SUPER] {
        let mut app = App::default();
        type_text(&mut app, "rascunho preservado");
        let selection = app.next_selection().clone();
        app.key(KeyEvent::new(KeyCode::Char('p'), modifier));
        assert_eq!(app.dialog.as_ref().map(|d| d.menu), Some(Menu::Commands));
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.input, "rascunho preservado");
        assert_eq!(*app.next_selection(), selection);
    }
}

#[test]
fn should_keep_ctrl_p_as_previous_item_inside_a_dialog() {
    let mut app = App::default();
    app.open_menu(Menu::Model);
    press(&mut app, KeyCode::Down);
    app.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
    let dialog = app.dialog.as_ref().unwrap();
    assert_eq!(dialog.menu, Menu::Model);
    assert_eq!(dialog.cursor, 0);
}

#[test]
fn should_interrupt_running_work_with_escape_or_ctrl_c_without_discarding_anything() {
    for key in [
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
    ] {
        let mut app = App::review();
        app.set_scene("running");
        type_text(&mut app, "rascunho");
        let queued = app.queue[0].text.clone();
        assert!(!app.key(key));
        assert_eq!(app.scenario().id, "recovery");
        assert_eq!(app.input, "rascunho");
        assert_eq!(app.queue[0].text, queued);
        app.finish();
        assert_eq!(app.queue.len(), 1);
    }
}

#[test]
fn should_cancel_only_the_popup_before_interrupting_and_require_two_idle_ctrl_c_to_exit() {
    let mut app = App::default();
    app.set_scene("running");
    app.open_menu(Menu::Model);
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.scenario().id, "running");
    app.set_scene("completed");
    type_text(&mut app, "rascunho");
    let cancel = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert!(!app.key(cancel));
    assert!(app.input.is_empty());
    type_text(&mut app, "novo rascunho");
    assert!(!app.key(cancel));
    assert!(app.key(cancel));
}

#[test]
fn should_insert_newlines_with_shift_enter_or_backslash_enter_without_sending() {
    let mut app = App::default();
    type_text(&mut app, "linha 1");
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT));
    type_text(&mut app, "linha 2\\");
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.input, "linha 1\nlinha 2\n");
    assert_eq!(app.queue.len(), 0);
}

#[test]
fn should_paste_multiline_commands_as_draft_data_and_not_execute_them() {
    let mut app = App::default();
    app.paste("/finish\r\n/model\rtexto\u{1b}[31m");
    assert_eq!(app.input, "/finish\n/model\ntexto[31m");
    assert!(app.dialog.is_none());
    assert_eq!(app.queue.len(), 0);
    assert_eq!(app.scenario().id, "empty");
}

#[test]
fn should_recall_submitted_messages_and_restore_unsent_draft_without_touching_the_queue() {
    let mut app = App::default();
    app.paste("primeira\nmultilinha");
    press(&mut app, KeyCode::Enter);
    type_text(&mut app, "segunda");
    press(&mut app, KeyCode::Enter);
    type_text(&mut app, "rascunho");
    press(&mut app, KeyCode::Up);
    assert_eq!(app.input, "segunda");
    press(&mut app, KeyCode::Up);
    assert_eq!(app.input, "primeira\nmultilinha");
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Down);
    assert_eq!(app.input, "rascunho");
    assert_eq!(app.queue.len(), 1);
    assert_eq!(app.turns.len(), 1);
}

#[test]
fn should_toggle_tool_details_and_complete_commands_without_applying_them() {
    let mut app = App::review();
    app.approve(true);
    app.step_demo();
    app.step_demo();
    let details = KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL);
    app.key(details);
    assert!(app.pane_text().contains("exit: 0"));
    app.key(details);
    assert!(!app.pane_text().contains("exit: 0"));
    type_text(&mut app, "/mod");
    press(&mut app, KeyCode::Tab);
    assert_eq!(app.dialog.as_ref().unwrap().query, "model");
    assert_eq!(app.next_selection().model, "GPT mock");
}

#[test]
fn should_preserve_pending_intervention_on_interrupt_and_never_treat_command_c_as_interrupt() {
    let mut app = App::review();
    type_text(&mut app, "intervenção");
    app.send(true);
    type_text(&mut app, "rascunho");
    app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::SUPER));
    assert_eq!(app.scenario().id, "running");
    assert_eq!(app.input, "rascunho");
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.scenario().id, "recovery");
    assert_eq!(app.pending.get().unwrap().text, "intervenção");
    app.approve(true);
    assert_eq!(app.scenario().id, "recovery");
}

#[test]
fn should_toggle_plan_without_affecting_execution_and_not_exit_on_expired_ctrl_c_confirmation() {
    let mut app = App::default();
    let executing = app.executing.clone();
    let tasks = KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL);
    app.key(tasks);
    assert_eq!(app.pane, 1);
    app.key(tasks);
    assert_eq!(app.pane, 0);
    assert_eq!(app.executing, executing);
    app.set_scene("completed");
    app.exit_armed = Some(std::time::Instant::now() - std::time::Duration::from_secs(3));
    assert!(!app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)));
}
fn screen(app: &App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| render(frame, app)).unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect()
}

#[test]
fn should_type_immediately_and_filter_slash_commands_without_sending_them() {
    let mut app = App::default();
    type_text(&mut app, "quero corrigir");
    assert_eq!(app.input, "quero corrigir");
    app.input.clear();
    app.input_cursor = None;
    type_text(&mut app, "/model");
    assert_eq!(app.menu_items().len(), 1);
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.dialog.as_ref().unwrap().menu, Menu::Model);
    assert_eq!(app.queue.len(), 0);
}

#[test]
fn should_cancel_model_browsing_without_mutating_draft_selection_or_execution() {
    let mut app = App::default();
    type_text(&mut app, "rascunho 中文");
    let before = app.next_selection().clone();
    let executing = app.executing.clone();
    app.open_menu(Menu::Model);
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Right);
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.input, "rascunho 中文");
    assert_eq!(*app.next_selection(), before);
    assert_eq!(app.executing, executing);
    assert!(app.dialog.is_none());
}

#[test]
fn should_require_explicit_compatible_effort_and_only_apply_to_next_message() {
    let mut app = App {
        selected: 2,
        ..App::review()
    };
    let executing = app.executing.clone();
    let queued = app.queue[0].selection.clone();
    app.open_menu(Menu::Model);
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Enter);
    assert!(app.dialog.is_some());
    assert_eq!(app.next_selection().model, "GPT mock");
    press(&mut app, KeyCode::Right);
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.next_selection().model, "Claude mock");
    assert_eq!(app.next_selection().variant, "default");
    assert_eq!(app.executing, executing);
    assert_eq!(app.queue[0].selection, queued);
}

#[test]
fn should_edit_any_queued_message_and_restore_composer_draft_after_cancel_or_save() {
    for save in [false, true] {
        let mut app = App::review();
        app.queue.push(Message {
            text: "segunda".into(),
            selection: app.profiles[2].clone(),
            demo: true,
        });
        type_text(&mut app, "rascunho original");
        app.open_menu(Menu::Queue);
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Enter);
        app.input = "segunda editada".into();
        app.input_cursor = None;
        app.open_menu(Menu::Model);
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Right);
        press(&mut app, KeyCode::Enter);
        press(&mut app, if save { KeyCode::Enter } else { KeyCode::Esc });
        assert_eq!(app.input, "rascunho original");
        assert_eq!(
            app.queue[1].text,
            if save { "segunda editada" } else { "segunda" }
        );
        assert_eq!(app.next_selection().name, "Perguntar");
        assert_eq!(
            app.queue[1].selection.model,
            if save { "Claude mock" } else { "GPT mock" }
        );
    }
}

#[test]
fn should_reorder_and_remove_highlighted_messages_but_never_mutate_offline_queue() {
    let mut app = App::review();
    app.queue.push(Message {
        text: "segunda".into(),
        selection: app.executing.clone(),
        demo: true,
    });
    app.open_menu(Menu::Queue);
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Char('-'));
    assert_eq!(app.queue[0].text, "segunda");
    press(&mut app, KeyCode::Char('d'));
    assert_eq!(app.queue.len(), 1);
    app.set_scene("offline");
    press(&mut app, KeyCode::Char('d'));
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.queue.len(), 1);
    assert!(app.queue_edit.is_none());
}

#[test]
fn should_preserve_edit_identity_by_blocking_progression_and_nested_queue_edits() {
    let mut app = App::review();
    app.set_scene("running");
    app.open_menu(Menu::Queue);
    press(&mut app, KeyCode::Enter);
    app.finish();
    assert_eq!(app.queue.len(), 1);
    app.command("/queue");
    assert!(app.dialog.is_none());
    assert!(app.queue_edit.is_some());
}

#[test]
fn should_intervene_from_command_menu_without_special_terminal_keys() {
    let mut app = App::review();
    type_text(&mut app, "pare e esclareça");
    app.key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
    type_text(&mut app, "intervene");
    press(&mut app, KeyCode::Enter);
    assert!(app.pending.has_pending());
    assert_eq!(app.executing.name, "Auto");
    app.command("/approve");
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.scenario().id, "running");
    app.command("/safe");
    assert_eq!(app.executing.name, "Perguntar");
}

#[test]
fn should_keep_unicode_editor_cursor_at_valid_boundaries() {
    let mut app = App::default();
    type_text(&mut app, "a中文ã");
    press(&mut app, KeyCode::Left);
    press(&mut app, KeyCode::Backspace);
    assert_eq!(app.input, "a中ã");
    type_text(&mut app, "x");
    assert_eq!(app.input, "a中xã");
    press(&mut app, KeyCode::Delete);
    assert_eq!(app.input, "a中x");
    // TestBackend retains the continuation cell of the double-width character.
    assert!(screen(&app, 80, 24).contains("a中 x"));
}

#[test]
fn should_render_all_popups_at_minimum_and_wide_sizes_with_visible_selection_and_footer() {
    for (width, height) in [(80, 24), (160, 48)] {
        let mut app = App {
            plain: true,
            ..App::default()
        };
        for menu in [
            Menu::Commands,
            Menu::Model,
            Menu::Profile,
            Menu::Effort,
            Menu::Queue,
            Menu::Scenario,
            Menu::Approval,
            Menu::Config,
            Menu::Help,
        ] {
            app.open_menu(menu);
            for _ in 0..25 {
                press(&mut app, KeyCode::Down);
            }
            let text = screen(&app, width, height);
            assert!(text.contains("Esc"));
            assert!(text.contains("Próxima"));
            if menu != Menu::Help {
                assert!(text.contains(">"));
            }
        }
    }
}

#[test]
fn should_render_empty_filtered_menus_and_empty_queue_without_panicking() {
    let mut app = App::default();
    app.open_menu(Menu::Model);
    type_text(&mut app, "inexistente");
    press(&mut app, KeyCode::Enter);
    assert!(screen(&app, 80, 24).contains("Nenhum resultado"));
    app.queue.clear();
    app.open_menu(Menu::Queue);
    press(&mut app, KeyCode::Enter);
    assert!(screen(&app, 80, 24).contains("Nada na fila"));
}

#[test]
fn should_scroll_long_unicode_drafts_without_hiding_the_caret_or_footer() {
    let mut app = App::default();
    type_text(&mut app, &format!("{} FIM", "中文 palavra ".repeat(100)));
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let text = screen(&app, 80, 24);
    assert!(text.contains("FIM"));
    assert!(text.contains("Próxima"));
    let cursor = terminal.get_cursor_position().unwrap();
    assert!(cursor.x < 80 && cursor.y < 23);
}

#[test]
fn should_cycle_only_the_staged_queue_profile_and_scroll_in_standard_directions() {
    let mut app = App::review();
    app.open_menu(Menu::Queue);
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::BackTab);
    assert_eq!(app.next_selection().name, "Yolo");
    assert_eq!(app.profiles[app.selected].name, "Perguntar");
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::PageUp);
    assert_eq!(app.scroll, 5);
    press(&mut app, KeyCode::PageDown);
    assert_eq!(app.scroll, 0);
}
