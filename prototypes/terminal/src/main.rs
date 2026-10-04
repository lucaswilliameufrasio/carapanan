use carapana_ui_prototype::{App, headless, render, scenarios};
use clap::{Parser, Subcommand};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use std::{
    io::{self, IsTerminal},
    process::ExitCode,
};

#[derive(Parser)]
#[command(
    name = "carapana-prototype",
    about = "Delivery 0: CLI/TUI interativa com mocks. Não executa tarefas reais."
)]
struct Cli {
    #[arg(long, global = true)]
    plain: bool,
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Retorna um resultado simulado, sem espera por approval.
    Run {
        #[arg(long, default_value = "approval")]
        scenario: String,
        #[arg(long)]
        json: bool,
    },
    /// Lista cenários compartilhados entre web e terminal.
    Scenarios,
    /// Diagnóstico de exemplo, sem inspecionar a máquina.
    Doctor,
    /// Informação de exemplo, sem ler o workspace.
    Info,
    /// Configuração efetiva ilustrativa, não persistida.
    Config,
    /// Sessões ilustrativas, sem daemon.
    Sessions,
}

fn tui(plain: bool) -> io::Result<()> {
    let mut terminal = ratatui::init();
    let result = (|| {
        let mut app = App {
            plain,
            ..App::default()
        };
        loop {
            terminal.draw(|frame| render(frame, &app))?;
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('q') {
                    break;
                }
                if key.modifiers.contains(KeyModifiers::ALT) {
                    match key.code {
                        KeyCode::Char('p') => app.cycle_profile(),
                        KeyCode::Char('m') => app.cycle_model(),
                        KeyCode::Char('v') => app.cycle_variant(),
                        KeyCode::Char('i') => app.send(true),
                        _ => {}
                    }
                    continue;
                }
                match key.code {
                    KeyCode::Char('q') if !app.editing => break,
                    KeyCode::Char('b') if !app.editing => app.cycle_profile(),
                    KeyCode::Char('m') if !app.editing => app.cycle_model(),
                    KeyCode::Char('v') if !app.editing => app.cycle_variant(),
                    KeyCode::Char('i') if !app.editing => {
                        app.editing = true;
                        app.notice = "Digite a intervenção; Alt+I envia. Ou Enter enfileira, depois w intervém.".into();
                    }
                    KeyCode::Char('w') if !app.editing && app.scenario().id != "offline" => {
                        if let Some(message) = app.queue.pop() {
                            app.pending = Some(message);
                            app.notice = "Intervenção pendente; approval invalidado. f aplica na etapa segura.".into();
                        }
                    }
                    KeyCode::Char('j') if !app.editing => app.scroll = app.scroll.saturating_add(1),
                    KeyCode::Char('k') if !app.editing => app.scroll = app.scroll.saturating_sub(1),
                    KeyCode::BackTab => app.cycle_profile(),
                    KeyCode::F(2) | KeyCode::Char('f') if !app.editing => app.safe_step(),
                    KeyCode::F(3) | KeyCode::Char('g') if !app.editing => app.finish(),
                    KeyCode::F(4) | KeyCode::Char('c') if !app.editing => {
                        app.scene = (app.scene + 1) % app.scenes.len();
                        app.pending = None;
                        app.scroll = 0;
                    }
                    KeyCode::F(5) | KeyCode::Char('e') if !app.editing => {
                        if let Some(m) = app.queue.first() {
                            app.input = m.text.clone();
                            app.editing = true;
                            app.editing_queue = true;
                            if let Some(i) = app.profiles.iter().position(|p| p == &m.selection) {
                                app.selected = i;
                            }
                        }
                    }
                    KeyCode::F(6) | KeyCode::Char('d')
                        if !app.editing && app.scenario().id != "offline" =>
                    {
                        if !app.queue.is_empty() {
                            app.queue.remove(0);
                        }
                    }
                    KeyCode::F(7) | KeyCode::Char('o')
                        if !app.editing && app.scenario().id != "offline" =>
                    {
                        if app.queue.len() > 1 {
                            app.queue.swap(0, 1);
                        }
                    }
                    KeyCode::Enter => {
                        if app.editing {
                            app.send(false);
                        } else {
                            app.editing = true;
                        }
                    }
                    KeyCode::Esc => {
                        app.editing = false;
                        app.editing_queue = false;
                    }
                    KeyCode::Backspace if app.editing => {
                        app.input.pop();
                    }
                    KeyCode::Char(c) if app.editing => app.input.push(c),
                    KeyCode::Char('a') => app.approve(true),
                    KeyCode::Char('n') => app.approve(false),
                    KeyCode::Char('p') => {
                        app.set_scene("recovery");
                        app.notice = "Pausa simulada. Fila e arquivos preservados.".into();
                    }
                    KeyCode::Char('r') if app.scenario().id == "recovery" => {
                        app.set_scene("running")
                    }
                    KeyCode::Char('s') => {
                        app.set_scene("recovery");
                        app.notice =
                            "Parado (mock): temporários encerrados, arquivos preservados.".into();
                    }
                    KeyCode::Tab | KeyCode::Char('t') if !app.editing => {
                        app.pane = (app.pane + 1) % carapana_ui_prototype::PANES.len();
                        app.scroll = 0;
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    })();
    ratatui::restore();
    result
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Some(Commands::Run { scenario, json }) => {
            let scenes = scenarios();
            let Some(scene) = scenes.iter().find(|s| s.id == scenario) else {
                eprintln!("Cenário desconhecido. Use scenarios. Nenhuma execução real.");
                return ExitCode::from(2);
            };
            let result = headless(scene);
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&result).expect("serializable fixture")
                );
            } else {
                println!(
                    "Protótipo · nenhuma execução real\n{}\n{}\nStatus: {}\nCódigo: {}",
                    scene.label, result.summary, result.status, result.exit_code
                );
            }
            ExitCode::from(result.exit_code)
        }
        Some(Commands::Scenarios) => {
            for scene in scenarios() {
                println!("{:14} {}", scene.id, scene.label);
            }
            ExitCode::SUCCESS
        }
        Some(Commands::Doctor) => {
            println!(
                "Doctor · mock\nConfig/Git/Storage/Sandbox/Providers/MCP/Recursos: fixtures OK\nNenhuma inspeção/correção real. Sem código/prompts/secrets no report."
            );
            ExitCode::SUCCESS
        }
        Some(Commands::Info) => {
            println!(
                "Workspace: quintal-api (mock)\nPerfil: Perguntar / GPT mock / default\nNenhum arquivo, credencial ou provider real."
            );
            ExitCode::SUCCESS
        }
        Some(Commands::Config) => {
            println!(
                "model.primary = gpt-mock\nagent.mode = ask\nsource = user (mock)\nSem persistência. Projeto não amplia permissões."
            );
            ExitCode::SUCCESS
        }
        Some(Commands::Sessions) => {
            println!(
                "! quintal-api approval\n• tixnow-web running\n○ carapana paused\nTodas as sessões são simuladas; sem daemon."
            );
            ExitCode::SUCCESS
        }
        None => {
            if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
                eprintln!(
                    "TUI precisa de terminal. Use run --scenario approval --json para headless simulado."
                );
                return ExitCode::from(2);
            }
            match tui(cli.plain) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("Erro do protótipo: {error}");
                    ExitCode::from(1)
                }
            }
        }
    }
}
