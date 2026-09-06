use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use eiviz_control::session::{self, validate_for_apply};

#[cfg(feature = "runtime")]
use eiviz_api::auth::AuthConfig;
#[cfg(feature = "runtime")]
use eiviz_api::{ServerConfig, listen};
#[cfg(feature = "runtime")]
use eiviz_control::{Command, RequestKey};
#[cfg(feature = "runtime")]
use std::net::SocketAddr;
#[cfg(feature = "runtime")]
use std::sync::Arc;

const EXIT_ARGS: u8 = 2;
const EXIT_SESSION: u8 = 3;
#[cfg(feature = "runtime")]
const EXIT_GPU: u8 = 4;
#[cfg(feature = "runtime")]
const EXIT_BIND: u8 = 5;
const EXIT_OTHER: u8 = 6;

#[derive(Parser)]
#[command(name = "eiviz-headless", about = "Headless eiviz daemon")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Parse and validate a session without initializing the GPU.
    Validate {
        #[arg(long)]
        session: PathBuf,
    },
    /// Write canonical session JSON to stdout without initializing the GPU.
    Canonicalize {
        #[arg(long)]
        session: PathBuf,
    },
    /// Apply a session, host the control API, and wait until shutdown.
    Run {
        #[arg(long)]
        session: PathBuf,
        #[arg(long, default_value = "127.0.0.1:9400")]
        bind: String,
    },
}

fn main() -> ExitCode {
    match Cli::parse().cmd {
        Cmd::Validate { session } => match load_valid(&session) {
            Ok(_) => ExitCode::SUCCESS,
            Err(code) => ExitCode::from(code),
        },
        Cmd::Canonicalize { session } => match canonicalize(&session) {
            Ok(()) => ExitCode::SUCCESS,
            Err(code) => ExitCode::from(code),
        },
        Cmd::Run { session, bind } => match run_daemon(session, bind) {
            Ok(()) => ExitCode::SUCCESS,
            Err(code) => ExitCode::from(code),
        },
    }
}

fn load_valid(path: &PathBuf) -> Result<eiviz_control::Document, u8> {
    let bytes = std::fs::read(path).map_err(|error| {
        eprintln!("eiviz-headless error=read {error}");
        EXIT_ARGS
    })?;
    let doc = session::parse(&bytes).map_err(|error| {
        eprintln!("eiviz-headless error=session {error}");
        EXIT_SESSION
    })?;
    validate_for_apply(&doc).map_err(|error| {
        eprintln!("eiviz-headless error=session {}", error.message);
        EXIT_SESSION
    })?;
    Ok(doc)
}

fn canonicalize(path: &PathBuf) -> Result<(), u8> {
    let doc = load_valid(path)?;
    let bytes = session::to_vec(&doc).map_err(|error| {
        eprintln!("eiviz-headless error=session {error}");
        EXIT_SESSION
    })?;
    std::io::Write::write_all(&mut std::io::stdout(), &bytes).map_err(|_| EXIT_OTHER)?;
    Ok(())
}

fn run_daemon(session: PathBuf, bind: String) -> Result<(), u8> {
    #[cfg(not(feature = "runtime"))]
    {
        let _ = (session, bind);
        eprintln!("eiviz-headless error=runtime binary built without mixer runtime");
        Err(EXIT_OTHER)
    }
    #[cfg(feature = "runtime")]
    {
        run_daemon_runtime(session, bind)
    }
}

#[cfg(feature = "runtime")]
fn run_daemon_runtime(session: PathBuf, bind: String) -> Result<(), u8> {
    let document = load_valid(&session)?;
    let ws_addr: SocketAddr = bind.parse().map_err(|error| {
        eprintln!("eiviz-headless error=bind {error}");
        EXIT_BIND
    })?;
    let auth = AuthConfig::from_env();
    if !ws_addr.ip().is_loopback() && !auth.require_auth {
        eprintln!("eiviz-headless error=bind remote bind requires authentication");
        return Err(EXIT_BIND);
    }

    let rt = tokio::runtime::Runtime::new().map_err(|error| {
        eprintln!("eiviz-headless error=runtime {error}");
        EXIT_OTHER
    })?;
    rt.block_on(async move {
        {
            let mut svc = eiviz_mixer::control_service().lock().map_err(|_| {
                eprintln!("eiviz-headless error=runtime control lock");
                EXIT_OTHER
            })?;
            svc.execute(
                RequestKey {
                    client_instance_id: "headless".into(),
                    request_id: "boot".into(),
                },
                Command::ReplaceSession {
                    document: Box::new(document),
                    expected_revision: None,
                },
            )
            .map_err(|error| {
                let code = if error.code() == "IO" || error.message().contains("device") {
                    EXIT_GPU
                } else {
                    EXIT_GPU
                };
                eprintln!("eiviz-headless error=gpu {error}");
                code
            })?;
        }
        let config = ServerConfig {
            bind: ws_addr,
            auth,
            idle_timeout: std::time::Duration::from_secs(60),
            max_clients: 32,
        };
        let control: Arc<dyn eiviz_control::ControlFacade> = Arc::new(eiviz_mixer::MixerFacade);
        let (bound, task) = listen(config, control).await.map_err(|error| {
            eprintln!("eiviz-headless error=bind {error}");
            EXIT_BIND
        })?;
        eprintln!("eiviz-headless ready ws={}", bound.ws_addr);
        shutdown_signal().await;
        {
            if let Ok(mut svc) = eiviz_mixer::control_service().lock() {
                let _ = svc.execute(
                    RequestKey {
                        client_instance_id: "headless".into(),
                        request_id: "shutdown".into(),
                    },
                    Command::Shutdown,
                );
            }
        }
        task.abort();
        Ok(())
    })
}

#[cfg(feature = "runtime")]
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("sigterm");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = sigterm.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
