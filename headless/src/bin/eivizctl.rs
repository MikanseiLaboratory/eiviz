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
    Shutdown,
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
        Cmd::Watch => loop {
            let snap = client.snapshot_json().await.map_err(|e| e.to_string())?;
            println!("{snap}");
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        },
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
        Cmd::Shutdown => {
            client.shutdown().await.map_err(|e| e.to_string())?;
            if !json {
                println!("ok");
            }
        }
    }
    Ok(())
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
