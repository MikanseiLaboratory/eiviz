use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{CommandFactory, Parser, Subcommand};
use eiviz_api::client::{ControlClient, ControlSession};

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
    /// Suppress the "ok" line on live ops
    #[arg(long)]
    json: bool,
    /// Open an interactive prompt instead of running one command
    #[arg(long)]
    repl: bool,
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Parser)]
#[command(
    name = "eivizctl",
    no_binary_name = true,
    disable_version_flag = true,
    subcommand_required = true
)]
struct ReplLine {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    Status,
    Snapshot,
    Watch,
    Preview {
        #[arg(long, default_value_t = 1)]
        unit: u64,
        #[arg(long)]
        scene: u64,
    },
    Cut {
        #[arg(long, default_value_t = 1)]
        unit: u64,
        #[arg(long, default_value_t = true)]
        swap: bool,
    },
    Auto {
        #[arg(long, default_value_t = 1)]
        unit: u64,
        #[arg(long, default_value_t = 1000)]
        duration_ms: u32,
    },
    Replace {
        #[arg(long)]
        session: PathBuf,
        #[arg(long, default_value_t = 0)]
        expected_revision: u64,
    },
    Mutate {
        json: String,
        #[arg(long, default_value_t = 0)]
        expected_revision: u64,
    },
    Prefs {
        #[command(subcommand)]
        action: Option<PrefsCmd>,
    },
    Shutdown,
}

#[derive(Subcommand)]
enum PrefsCmd {
    Get { key: String },
    Set { key: String, value: Vec<String> },
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
            Some(Cmd::Prefs { action }) => prefs_cmd(action),
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

async fn run_once(client: &ControlClient, cmd: Cmd, json: bool) -> Result<(), String> {
    let session = client.connect().await.map_err(|e| e.to_string())?;
    run_cmd(&session, cmd, json).await
}

async fn run_cmd(session: &ControlSession, cmd: Cmd, json: bool) -> Result<(), String> {
    match cmd {
        Cmd::Status | Cmd::Snapshot => {
            let snap = session.snapshot_json().await.map_err(|e| e.to_string())?;
            println!("{snap}");
        }
        Cmd::Watch => {
            session.subscribe(0).await.map_err(|e| e.to_string())?;
            loop {
                for kind in session.take_events() {
                    println!("{kind}");
                }
                let view = session.view();
                if !view.document_json.is_empty() {
                    println!("{}", String::from_utf8_lossy(&view.document_json));
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        }
        Cmd::Preview { unit, scene } => {
            session
                .preview(unit, scene)
                .await
                .map_err(|e| e.to_string())?;
            if !json {
                println!("ok");
            }
        }
        Cmd::Cut { unit, swap } => {
            session.cut(unit, swap).await.map_err(|e| e.to_string())?;
            if !json {
                println!("ok");
            }
        }
        Cmd::Auto { unit, duration_ms } => {
            session
                .auto(unit, duration_ms, true)
                .await
                .map_err(|e| e.to_string())?;
            if !json {
                println!("ok");
            }
        }
        Cmd::Replace {
            session: path,
            expected_revision,
        } => {
            let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
            session
                .replace_session(bytes, expected_revision)
                .await
                .map_err(|e| e.to_string())?;
            if !json {
                println!("ok");
            }
        }
        Cmd::Mutate {
            json: body,
            expected_revision,
        } => {
            session
                .mutate_session(body.into_bytes(), expected_revision)
                .await
                .map_err(|e| e.to_string())?;
            if !json {
                println!("ok");
            }
        }
        Cmd::Prefs { action } => prefs_cmd(action)?,
        Cmd::Shutdown => {
            session.shutdown().await.map_err(|e| e.to_string())?;
            if !json {
                println!("ok");
            }
        }
    }
    Ok(())
}

fn prefs_cmd(action: Option<PrefsCmd>) -> Result<(), String> {
    match action {
        None => {
            let prefs = eiviz_headless::HeadlessPrefs::load();
            println!("path={}", eiviz_headless::HeadlessPrefs::path().display());
            println!("{}", prefs.display());
            Ok(())
        }
        Some(PrefsCmd::Get { key }) => {
            let prefs = eiviz_headless::HeadlessPrefs::load();
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
            let mut prefs = eiviz_headless::HeadlessPrefs::load();
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
                let mut cmd = ReplLine::command();
                if let Err(error) = cmd.print_help() {
                    eprintln!("{error}");
                }
                println!();
            }
            ReplAction::Cmd(Cmd::Prefs { action }) => {
                if let Err(error) = prefs_cmd(action) {
                    eprintln!("{error}");
                }
            }
            ReplAction::Cmd(cmd) => match ensure_session(client, &mut session).await {
                Ok(session) => {
                    if let Err(error) = run_cmd(session, cmd, json).await {
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
    Cmd(Cmd),
    Error(String),
}

fn parse_repl_line(line: &str) -> ReplAction {
    match line {
        "exit" | "quit" => return ReplAction::Exit,
        "help" | "?" => return ReplAction::Help,
        _ => {}
    }
    if let Some(body) = line.strip_prefix("mutate ") {
        let body = body.trim();
        if body.starts_with('{') {
            return ReplAction::Cmd(Cmd::Mutate {
                json: body.to_string(),
                expected_revision: 0,
            });
        }
    }
    let args = match split_repl_args(line) {
        Ok(args) => args,
        Err(error) => return ReplAction::Error(error),
    };
    match ReplLine::try_parse_from(args) {
        Ok(parsed) => ReplAction::Cmd(parsed.cmd),
        Err(error) => ReplAction::Error(error.to_string()),
    }
}

fn split_repl_args(line: &str) -> Result<Vec<String>, String> {
    let mut args = Vec::new();
    let mut cur = String::new();
    let mut quote = None;
    for c in line.chars() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, c) if c.is_whitespace() => {
                if !cur.is_empty() {
                    args.push(std::mem::take(&mut cur));
                }
            }
            (_, c) => cur.push(c),
        }
    }
    if quote.is_some() {
        return Err("unclosed quote".into());
    }
    if !cur.is_empty() {
        args.push(cur);
    }
    Ok(args)
}
