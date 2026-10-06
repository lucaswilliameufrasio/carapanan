use carapana_ui_prototype::{App, headless, render, scenarios};
use clap::{Parser, Subcommand};
use crossterm::event::{self, Event};
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
    /// Mantém o carapanã ASCII imóvel.
    #[arg(long, global = true)]
    no_animation: bool,
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

fn tui(plain: bool, no_animation: bool) -> io::Result<()> {
    let mut terminal = ratatui::init();
    let result = (|| {
        crossterm::execute!(io::stdout(), crossterm::event::EnableBracketedPaste)?;
        let mut app = App {
            plain,
            animated: !no_animation,
            ansi256: std::env::var("TERM").is_ok_and(|term| term.contains("256color"))
                || std::env::var("COLORTERM")
                    .is_ok_and(|term| matches!(term.as_str(), "truecolor" | "24bit")),
            ..App::default()
        };
        let mut last_tick = std::time::Instant::now();
        loop {
            terminal.draw(|frame| render(frame, &app))?;
            if event::poll(std::time::Duration::from_millis(50))? {
                match event::read()? {
                    Event::Key(key) if app.key(key) => break,
                    Event::Paste(text) => app.paste(&text),
                    _ => {}
                }
            }
            let now = std::time::Instant::now();
            let delta = now.duration_since(last_tick);
            app.visual_elapsed += delta;
            app.advance(delta);
            last_tick = now;
        }
        Ok(())
    })();
    let _ = crossterm::execute!(io::stdout(), crossterm::event::DisableBracketedPaste);
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
            match tui(cli.plain, cli.no_animation) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("Erro do protótipo: {error}");
                    ExitCode::from(1)
                }
            }
        }
    }
}
