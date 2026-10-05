use carapana_ui_prototype::{
    App,
    interaction::Menu,
    render,
    updates::{Action, Package, Status, Update},
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};

fn press(app: &mut App, code: KeyCode) {
    app.key(KeyEvent::new(code, KeyModifiers::NONE));
}
fn choose(app: &mut App, label: &str) {
    let cursor = app
        .menu_items()
        .iter()
        .position(|(name, _)| name == label)
        .unwrap();
    app.dialog.as_mut().unwrap().cursor = cursor;
    press(app, KeyCode::Enter);
}
fn screen(app: &App) -> String {
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|f| render(f, app)).unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect()
}

#[test]
fn should_update_interface_and_rollback_without_touching_session_state() {
    let mut app = App::review();
    app.input = "rascunho preservado".into();
    let executing = app.executing.clone();
    let queued = app.queue[0].selection.clone();
    let approval = app.approval_action.clone();
    app.command("/update");
    assert_eq!(app.dialog.as_ref().unwrap().menu, Menu::Updates);
    choose(&mut app, "Atualizar agora · mock");
    assert_eq!(app.update.status, Status::Verifying);
    choose(&mut app, "Simular verificação");
    assert_eq!(app.update.status, Status::Installed);
    assert!(app.update.busy);
    assert!(screen(&app).contains("sem reiniciar"));
    choose(&mut app, "Simular rollback");
    assert_eq!(app.update.status, Status::RolledBack);
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.input, "rascunho preservado");
    assert_eq!(app.executing, executing);
    assert_eq!(app.queue[0].selection, queued);
    assert_eq!(app.queue.len(), 1);
    assert_eq!(app.scenario().id, "approval");
    assert_eq!(app.approval_action, approval);
}

#[test]
fn should_schedule_runtime_and_keep_restart_explicit_and_blocked_until_idle() {
    let mut update = Update::default();
    update.select_package(Package::Runtime);
    update.apply(Action::Schedule);
    update.apply(Action::Later);
    assert!(update.hidden);
    assert_eq!(update.status, Status::Scheduled);
    update.apply(Action::Idle);
    assert!(!update.hidden);
    assert_eq!(update.status, Status::Verifying);
    update.apply(Action::Verify);
    assert_eq!(update.status, Status::Ready);
    update.apply(Action::Activate);
    assert_eq!(update.status, Status::Installed);
    update.select_package(Package::Runtime);
    update.apply(Action::Now);
    update.apply(Action::Verify);
    update.apply(Action::Activate);
    assert_eq!(update.status, Status::Ready);
    assert!(update.busy);
}

#[test]
fn should_reject_invalid_packages_unsafe_rollback_and_out_of_order_actions() {
    for package in [Package::Invalid, Package::Migration] {
        let mut update = Update::default();
        update.select_package(package);
        update.apply(Action::Verify);
        assert_eq!(update.status, Status::Available);
        update.apply(Action::Now);
        update.apply(Action::Verify);
        update.apply(Action::Idle);
        update.apply(Action::Activate);
        let expected = if package == Package::Invalid {
            Status::Failed
        } else {
            Status::Installed
        };
        assert_eq!(update.status, expected);
        update.apply(Action::Rollback);
        assert_eq!(update.status, expected);
    }
}

#[test]
fn should_keep_hidden_updates_discoverable_and_cancel_nested_pickers_without_changes() {
    let mut app = App::default();
    app.command("/update");
    choose(&mut app, "Lembrar depois");
    assert!(app.update.hidden);
    assert!(app.dialog.is_none());
    app.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
    for c in "update".chars() {
        press(&mut app, KeyCode::Char(c));
    }
    press(&mut app, KeyCode::Enter);
    choose(&mut app, "Pacote de demonstração");
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.update.package, Package::Interface);
    assert_eq!(app.dialog.as_ref().unwrap().menu, Menu::Updates);
    choose(&mut app, "Canal de atualização");
    choose(&mut app, "beta");
    assert!(screen(&app).contains("0.0.1-beta-demo"));
    assert!(!app.update.hidden);
}

#[test]
fn should_render_every_update_state_in_plain_minimum_terminal_and_block_offline_actions() {
    let mut app = App {
        plain: true,
        ..App::review()
    };
    for package in Package::ALL {
        app.update.select_package(package);
        app.command("/update");
        for action in [
            Action::Schedule,
            Action::Now,
            Action::Verify,
            Action::Idle,
            Action::Activate,
            Action::Rollback,
        ] {
            app.update.apply(action);
            let text = screen(&app);
            assert!(text.contains("Atualizações · mock"));
            assert!(text.contains("Esc fecha"));
            assert!(text.contains("Nada é baixado"));
        }
    }
    app.update = Update::default();
    app.set_scene("offline");
    app.command("/update");
    choose(&mut app, "Atualizar agora · mock");
    assert_eq!(app.update.status, Status::Available);
    assert!(screen(&app).contains("Desconectado"));
    choose(&mut app, "Pacote de demonstração");
    assert_eq!(app.dialog.as_ref().unwrap().menu, Menu::Updates);
    assert_eq!(app.update.package, Package::Interface);
    choose(&mut app, "Lembrar depois");
    assert!(app.update.hidden);
}
