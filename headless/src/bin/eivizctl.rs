use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{CommandFactory, Parser};
use eiviz_api::client::{ControlClient, ControlSession};
use eiviz_headless::ctl::{self, CtlCommand, Line, PrefsCmd, parse_line};

#[derive(Parser)]
#[command(
    name = "eivizctl",
    about = "eiviz control client",
    arg_required_else_help = true
)]
struct Cli {
    /// Control WebSocket URL
    #[arg(long, default_value = "ws://127.0.0.1:9400")]
    url: String,
    /// Auth token for the control WebSocket
    #[arg(long, conflicts_with = "token_file")]
    token: Option<String>,
    /// Read the token from a file (trailing whitespace is trimmed)
    #[arg(long)]
    token_file: Option<PathBuf>,
    /// Emit machine-readable JSON for one-shot commands
    #[arg(long)]
    json: bool,
    /// Open an interactive prompt instead of running one command
    #[arg(long)]
    repl: bool,
    #[command(subcommand)]
    cmd: Option<CtlCommand>,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let token = match load_token(cli.token, cli.token_file) {
        Ok(token) => token,
        Err(error) => {
            eprintln!("eivizctl error={error}");
            return ExitCode::from(6);
        }
    };
    let client = ControlClient::websocket(cli.url, token);
    if cli.repl && cli.cmd.is_some() {
        eprintln!("eivizctl error=--repl cannot be combined with a command");
        return ExitCode::from(2);
    }
    let result = if cli.repl {
        repl(&client, cli.json).await
    } else {
        match cli.cmd {
            Some(CtlCommand::Prefs { action }) => prefs_cmd(action),
            Some(cmd) if cmd.is_watch() => run_watch(&client).await,
            Some(cmd) => run_once(&client, cmd, cli.json).await,
            None => {
                let mut cmd = Cli::command();
                if let Err(error) = cmd.print_help() {
                    eprintln!("eivizctl error={error}");
                } else {
                    println!();
                }
                return ExitCode::from(2);
            }
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("eivizctl error={error}");
            ExitCode::from(6)
        }
    }
}

fn load_token(token: Option<String>, token_file: Option<PathBuf>) -> Result<String, String> {
    if let Some(path) = token_file {
        return std::fs::read_to_string(path)
            .map(|text| text.trim().to_string())
            .map_err(|error| error.to_string());
    }
    Ok(token.unwrap_or_default())
}

async fn run_once(client: &ControlClient, cmd: CtlCommand, json: bool) -> Result<(), String> {
    let session = client.connect().await.map_err(|e| e.to_string())?;
    ctl::run_cmd(&session, cmd, json).await
}

async fn run_watch(client: &ControlClient) -> Result<(), String> {
    let session = client.connect().await.map_err(|e| e.to_string())?;
    ctl::run_watch(&session).await
}

fn prefs_cmd(action: Option<PrefsCmd>) -> Result<(), String> {
    match action {
        None => {
            let prefs = eiviz_headless::HeadlessPrefs::load()?;
            println!("path={}", eiviz_headless::HeadlessPrefs::path().display());
            println!("{}", prefs.display());
            Ok(())
        }
        Some(PrefsCmd::Get { key }) => {
            let prefs = eiviz_headless::HeadlessPrefs::load()?;
            match prefs.get(&key) {
                Ok(Some(value)) if normalize_prefs_key(&key) == "token" => {
                    println!("{}", if value.is_empty() { "" } else { "(set)" });
                    Ok(())
                }
                Ok(Some(value)) => {
                    println!("{value}");
                    Ok(())
                }
                Ok(None) => {
                    println!();
                    Ok(())
                }
                Err(error) => Err(error),
            }
        }
        Some(PrefsCmd::Set { key, value }) => {
            let joined = value.join(" ");
            let mut prefs = eiviz_headless::HeadlessPrefs::load()?;
            prefs.set(&key, &joined)?;
            let path = prefs.save()?;
            println!("ok path={}", path.display());
            Ok(())
        }
    }
}

fn normalize_prefs_key(key: &str) -> String {
    key.trim().replace(['-', '_'], "").to_ascii_lowercase()
}

async fn repl(client: &ControlClient, json: bool) -> Result<(), String> {
    let mut stdout = io::stdout();
    let mut session: Option<ControlSession> = None;
    writeln!(
        stdout,
        "eivizctl {}  (exit/quit to leave, help for commands)",
        client.endpoint
    )
    .map_err(|e| e.to_string())?;
    loop {
        write!(stdout, "eiviz> ").map_err(|e| e.to_string())?;
        stdout.flush().map_err(|e| e.to_string())?;
        let Some(line) = read_line().await? else {
            break;
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match parse_repl_line(line) {
            ReplAction::Exit => break,
            ReplAction::Help => {
                let mut cmd = Line::command();
                if let Err(error) = cmd.print_help() {
                    eprintln!("{error}");
                }
                println!();
            }
            ReplAction::Cmd(CtlCommand::Prefs { action }) => {
                if let Err(error) = prefs_cmd(action) {
                    eprintln!("{error}");
                }
            }
            ReplAction::Cmd(cmd) if cmd.is_watch() => {
                eprintln!("watch is not supported in the REPL");
            }
            ReplAction::Cmd(cmd) => match ensure_session(client, &mut session).await {
                Ok(session) => {
                    if let Err(error) = ctl::run_cmd(session, cmd, json).await {
                        eprintln!("{error}");
                    }
                }
                Err(error) => eprintln!("{error}"),
            },
            ReplAction::Error(error) => eprintln!("{error}"),
        }
    }
    Ok(())
}

async fn ensure_session<'a>(
    client: &ControlClient,
    session: &'a mut Option<ControlSession>,
) -> Result<&'a ControlSession, String> {
    if session.is_none() {
        let opened = client.connect().await.map_err(|e| e.to_string())?;
        if let Err(error) = opened.subscribe(0).await {
            eprintln!("subscribe warning={error}");
        }
        eprintln!("connected {}", client.endpoint);
        *session = Some(opened);
    }
    Ok(session.as_ref().expect("session just inserted"))
}

async fn read_line() -> Result<Option<String>, String> {
    tokio::task::spawn_blocking(|| {
        let mut line = String::new();
        match io::stdin().read_line(&mut line) {
            Ok(0) => Ok(None),
            Ok(_) => Ok(Some(line)),
            Err(error) => Err(error.to_string()),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

enum ReplAction {
    Exit,
    Help,
    Cmd(CtlCommand),
    Error(String),
}

fn parse_repl_line(line: &str) -> ReplAction {
    match line {
        "exit" | "quit" => return ReplAction::Exit,
        "help" | "?" => return ReplAction::Help,
        _ => {}
    }
    match parse_line(line) {
        Ok(cmd) => ReplAction::Cmd(cmd),
        Err(error) => ReplAction::Error(error),
    }
}
