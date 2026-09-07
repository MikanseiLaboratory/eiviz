use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use eiviz_api::client::ControlClient;

#[derive(Parser)]
#[command(name = "eivizctl", about = "eiviz control client")]
struct Cli {
    #[arg(long, env = "EIVIZ_API_URL", default_value = "ws://127.0.0.1:9400")]
    url: String,
    #[arg(long)]
    json: bool,
    #[command(subcommand)]
    cmd: Option<Cmd>,
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
    let token = std::env::var("EIVIZ_API_TOKEN")
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| {
            std::env::var("EIVIZ_API_TOKEN_FILE").ok().and_then(|path| {
                std::fs::read_to_string(path)
                    .ok()
                    .map(|text| text.trim().to_string())
            })
        })
        .unwrap_or_default();
    let client = ControlClient::websocket(cli.url.clone(), token);
    let result = match cli.cmd {
        Some(cmd) => run_cmd(&client, cmd, cli.json).await,
        None => repl(&client, cli.json).await,
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("eivizctl error={error}");
            ExitCode::from(6)
        }
    }
}

async fn run_cmd(client: &ControlClient, cmd: Cmd, json: bool) -> Result<(), String> {
    match cmd {
        Cmd::Status | Cmd::Snapshot => {
            let snap = client.snapshot_json().await.map_err(|e| e.to_string())?;
            println!("{snap}");
        }
        Cmd::Watch => {
            let session = client.connect().await.map_err(|e| e.to_string())?;
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
            client
                .preview(unit, scene)
                .await
                .map_err(|e| e.to_string())?;
            if !json {
                println!("ok");
            }
        }
        Cmd::Cut { unit, swap } => {
            client.cut(unit, swap).await.map_err(|e| e.to_string())?;
            if !json {
                println!("ok");
            }
        }
        Cmd::Auto { unit, duration_ms } => {
            client
                .auto(unit, duration_ms, true)
                .await
                .map_err(|e| e.to_string())?;
            if !json {
                println!("ok");
            }
        }
        Cmd::Replace {
            session,
            expected_revision,
        } => {
            let bytes = std::fs::read(session).map_err(|e| e.to_string())?;
            client
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
            client
                .mutate_session(body.into_bytes(), expected_revision)
                .await
                .map_err(|e| e.to_string())?;
            if !json {
                println!("ok");
            }
        }
        Cmd::Prefs { action } => prefs_cmd(action)?,
        Cmd::Shutdown => {
            client.shutdown().await.map_err(|e| e.to_string())?;
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
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    loop {
        write!(stdout, "eiviz> ").map_err(|e| e.to_string())?;
        stdout.flush().map_err(|e| e.to_string())?;
        let mut line = String::new();
        if stdin
            .lock()
            .read_line(&mut line)
            .map_err(|e| e.to_string())?
            == 0
        {
            break;
        }
        let line = line.trim();
        if line.is_empty() || line == "exit" || line == "quit" {
            if line == "exit" || line == "quit" {
                break;
            }
            continue;
        }
        if line == "prefs" {
            if let Err(error) = prefs_cmd(None) {
                eprintln!("{error}");
            }
            continue;
        }
        if let Some(key) = line.strip_prefix("prefs get ") {
            if let Err(error) = prefs_cmd(Some(PrefsCmd::Get {
                key: key.trim().to_string(),
            })) {
                eprintln!("{error}");
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("prefs set ") {
            let Some((key, value)) = rest.split_once(char::is_whitespace) else {
                eprintln!("usage: prefs set <key> <value>");
                continue;
            };
            if let Err(error) = prefs_cmd(Some(PrefsCmd::Set {
                key: key.to_string(),
                value: vec![value.to_string()],
            })) {
                eprintln!("{error}");
            }
            continue;
        }
        if let Some(body) = line.strip_prefix("mutate ") {
            if let Err(error) = run_cmd(
                client,
                Cmd::Mutate {
                    json: body.to_string(),
                    expected_revision: 0,
                },
                json,
            )
            .await
            {
                eprintln!("{error}");
            }
            continue;
        }
        let mut args = vec!["eivizctl"];
        args.extend(line.split_whitespace());
        match Cli::try_parse_from(args) {
            Ok(cli) => {
                if let Some(cmd) = cli.cmd
                    && let Err(error) = run_cmd(client, cmd, json || cli.json).await
                {
                    eprintln!("{error}");
                }
            }
            Err(error) => eprintln!("{error}"),
        }
    }
    Ok(())
}
