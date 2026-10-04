use carapana_ui_prototype::{App, interaction::Menu, render, session::TurnStatus};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};
use std::time::Duration;

fn send(app: &mut App, text: &str) {
    app.input = text.into();
    app.input_cursor = None;
    app.send(false);
}

fn tick(app: &mut App, count: usize) {
    for _ in 0..count {
        app.advance(Duration::from_millis(950));
    }
}

fn screen(app: &App, width: u16, height: u16) -> (String, (u16, u16)) {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| render(frame, app)).unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    let caret = terminal.get_cursor_position().unwrap();
    (text, (caret.x, caret.y))
}

#[test]
fn should_start_empty_without_fabricated_work_messages_or_approvals() {
    let app = App::default();
    assert!(app.turns.is_empty());
    assert!(app.queue.is_empty());
    assert!(app.active.is_none());
    assert_eq!(app.next_selection().name, "Perguntar");
    let (text, caret) = screen(&app, 160, 48);
    assert!(!text.contains("183"));
    assert!(!text.contains("Permitir esta ação"));
    assert!(
        caret.1 < 20,
        "editor should follow the welcome, not sit 30 blank rows away"
    );
}

#[test]
fn should_show_the_actual_message_and_progress_to_inline_approval_without_more_keys() {
    let mut app = App::default();
    send(&mut app, "Meu pedido visível");
    assert_eq!(app.turns[0].message.text, "Meu pedido visível");
    assert_eq!(app.executing.name, "Perguntar");
    assert!(app.queue.is_empty());
    tick(&mut app, 3);
    assert_eq!(app.scenario().id, "approval");
    assert!(app.approval_focus);
    assert!(!app.turns[0].events.iter().any(|e| e.starts_with("Edit")));
    let (text, _) = screen(&app, 100, 32);
    assert!(text.contains("Meu pedido visível"));
    assert!(text.contains("Permitir esta ação?"));
    assert!(text.contains("Negar e pausar"));
    assert!(!text.contains("/approve abre"));
}

#[test]
fn should_complete_approved_work_then_automatically_start_the_captured_next_selection() {
    let mut app = App::default();
    send(&mut app, "primeira");
    app.selected = 0;
    send(&mut app, "segunda em Planejar");
    app.selected = 3;
    assert_eq!(app.executing.name, "Perguntar");
    assert_eq!(app.queue[0].selection.name, "Planejar");
    tick(&mut app, 3);
    tick(&mut app, 20);
    assert_eq!(app.turns.len(), 1, "approval must block advancement");
    app.key(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    tick(&mut app, 3);
    assert_eq!(app.turns[0].status, TurnStatus::Completed);
    assert!(app.turns[0].result.as_ref().unwrap().contains("primeira"));
    assert_eq!(app.turns[1].message.text, "segunda em Planejar");
    assert_eq!(app.executing.name, "Planejar");
    assert_eq!(app.next_selection().name, "Yolo");
    tick(&mut app, 3);
    assert_eq!(app.turns[1].status, TurnStatus::Completed);
    assert!(
        !app.turns[1]
            .events
            .iter()
            .any(|e| e.starts_with("Edit") || e.starts_with("Test"))
    );
}

#[test]
fn should_keep_seven_submitted_messages_visible_and_process_each_once_in_order() {
    let mut app = App {
        selected: 2,
        ..App::default()
    };
    for i in 1..=7 {
        send(&mut app, &format!("pedido {i}"));
    }
    assert_eq!(app.turns.len(), 1);
    assert_eq!(app.queue.len(), 6);
    let (text, _) = screen(&app, 160, 48);
    assert!(text.contains("pedido 1"));
    assert!(text.contains("pedido 2"));
    assert!(text.contains("Na fila (6)"));
    tick(&mut app, 42);
    assert_eq!(app.turns.len(), 7);
    assert!(app.queue.is_empty());
    assert!(app.active.is_none());
    for (i, turn) in app.turns.iter().enumerate() {
        assert_eq!(turn.message.text, format!("pedido {}", i + 1));
        assert_eq!(turn.status, TurnStatus::Completed);
    }
    let (text, _) = screen(&app, 80, 24);
    assert!(text.contains("pedido 7"));
    assert!(text.contains("Roteiro concluído"));
}

#[test]
fn should_deny_by_default_pause_and_reask_after_explicit_resume() {
    let mut app = App::default();
    send(&mut app, "primeira");
    send(&mut app, "segunda");
    tick(&mut app, 3);
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.scenario().id, "recovery");
    tick(&mut app, 30);
    assert_eq!(app.queue.len(), 1);
    assert_eq!(app.turns[0].status, TurnStatus::Paused);
    app.command("/resume");
    assert_eq!(app.scenario().id, "approval");
    assert!(app.approval_focus);
}

#[test]
fn should_send_to_the_queue_from_the_approval_composer_without_authorizing_the_action() {
    let mut app = App::default();
    send(&mut app, "primeira");
    tick(&mut app, 3);
    app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    app.paste("pedido enquanto aguardo");
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.scenario().id, "approval");
    assert_eq!(app.queue[0].text, "pedido enquanto aguardo");
    assert!(app.turns[0].needs_approval);
}

#[test]
fn should_not_steal_composer_focus_when_approval_arrives_during_a_draft() {
    let mut app = App::default();
    send(&mut app, "primeira");
    app.paste("estou escrevendo a segunda");
    tick(&mut app, 3);
    assert!(!app.approval_focus);
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.scenario().id, "approval");
    assert_eq!(app.queue[0].text, "estou escrevendo a segunda");
    assert!(app.turns[0].needs_approval);
}

#[test]
fn should_pause_on_interrupt_and_not_continue_in_the_background() {
    let mut app = App {
        selected: 2,
        ..App::default()
    };
    send(&mut app, "primeira");
    send(&mut app, "segunda");
    tick(&mut app, 1);
    app.input = "meu rascunho".into();
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    let events = app.turns[0].events.clone();
    tick(&mut app, 30);
    assert_eq!(app.turns[0].events, events);
    assert_eq!(app.input, "meu rascunho");
    assert_eq!(app.queue.len(), 1);
    app.command("/resume");
    tick(&mut app, 5);
    assert_eq!(app.turns.len(), 2);
}

#[test]
fn should_apply_intervention_at_the_next_safe_tick_and_preserve_the_original_turn() {
    let mut app = App {
        selected: 2,
        ..App::default()
    };
    send(&mut app, "primeira");
    app.selected = 0;
    app.input = "priorize o plano".into();
    app.send(true);
    assert_eq!(app.executing.name, "Auto");
    assert!(app.pending.is_some());
    tick(&mut app, 1);
    assert_eq!(app.executing.name, "Planejar");
    assert_eq!(app.turns[0].status, TurnStatus::Superseded);
    assert_eq!(app.turns[1].message.text, "priorize o plano");
    assert!(app.pending.is_none());
}

#[test]
fn should_freeze_the_mock_clock_while_editing_the_queue_or_browsing_a_picker() {
    let mut app = App {
        selected: 2,
        ..App::default()
    };
    send(&mut app, "primeira");
    send(&mut app, "segunda");
    app.open_menu(Menu::Model);
    tick(&mut app, 10);
    assert_eq!(app.turns[0].step, 0);
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    app.open_menu(Menu::Queue);
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    tick(&mut app, 10);
    assert_eq!(app.turns[0].step, 0);
    app.input = "segunda editada".into();
    app.send(false);
    assert_eq!(app.queue[0].text, "segunda editada");
    tick(&mut app, 1);
    assert_eq!(app.turns[0].step, 1);
}

#[test]
fn should_keep_approval_queue_editor_and_selection_visible_at_minimum_size() {
    let mut app = App::default();
    send(&mut app, &"中文 pedido longo ".repeat(40));
    for i in 0..7 {
        send(&mut app, &format!("fila {i}"));
    }
    tick(&mut app, 3);
    let (text, _) = screen(&app, 80, 24);
    for label in [
        "Permitir esta ação?",
        "Na fila (7)",
        "fila 0",
        "Próxima:",
        "Ctrl+P opções",
    ] {
        assert!(text.contains(label), "missing {label}");
    }
}

#[test]
fn should_not_report_plan_diff_or_validation_before_the_corresponding_mock_activity() {
    let mut app = App::default();
    assert!(app.diff_text().contains("Nenhuma alteração"));
    assert!(app.validation_text().contains("Nenhuma validação"));
    send(&mut app, "pedido");
    assert!(!app.plan_text().contains("✓"));
    tick(&mut app, 3);
    assert!(app.diff_text().contains("Prévia antes da aprovação"));
    assert!(app.validation_text().contains("Nenhuma validação"));
    app.approve(true);
    tick(&mut app, 1);
    assert!(app.diff_text().contains("Alteração demonstrada"));
    assert!(app.validation_text().contains("Nenhuma validação"));
    tick(&mut app, 1);
    assert!(app.validation_text().contains("183 passaram"));
}

#[test]
fn should_not_skip_approval_with_manual_finish_or_advance_blocked_review_scenarios() {
    let mut app = App::default();
    send(&mut app, "pedido");
    for _ in 0..10 {
        app.finish();
    }
    assert_eq!(app.scenario().id, "approval");
    assert!(app.turns[0].result.is_none());
    for id in [
        "quota",
        "incomplete",
        "offline",
        "conflict",
        "recovery",
        "loop",
    ] {
        app.load_scenario(id);
        let index = app.active.unwrap();
        let events = app.turns[index].events.clone();
        tick(&mut app, 20);
        app.finish();
        assert_eq!(app.turns[index].events, events);
        assert!(app.turns[index].result.is_none());
    }
}
