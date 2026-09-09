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
    /// Write a portable `.eivzx` that embeds Still/Video files.
    Export {
        #[arg(long)]
        session: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    /// List in-file session history (newest first).
    History {
        #[arg(long)]
        session: PathBuf,
    },
    /// Write a history entry out as a standalone `.eivz`.
    Restore {
        #[arg(long)]
        session: PathBuf,
        #[arg(long)]
        index: u32,
        #[arg(long)]
        output: PathBuf,
    },
    /// Apply a session, host the control API, and wait until shutdown.
    Run {
        #[arg(long)]
        session: Option<PathBuf>,
        #[arg(long)]
        bind: Option<String>,
        #[arg(long, help = "GPU renderer: auto, dx12, vulkan, or metal")]
        renderer: Option<String>,
        #[arg(
            long,
            env = "EIVIZ_MEDIA_DIRECTORY",
            help = "Uploaded media directory (default: OS local app data/eiviz/media)"
        )]
        media_directory: Option<PathBuf>,
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
        Cmd::Export { session, output } => match export(&session, &output) {
            Ok(()) => ExitCode::SUCCESS,
            Err(code) => ExitCode::from(code),
        },
        Cmd::History { session } => match history(&session) {
            Ok(()) => ExitCode::SUCCESS,
            Err(code) => ExitCode::from(code),
        },
        Cmd::Restore {
            session,
            index,
            output,
        } => match restore(&session, index, &output) {
            Ok(()) => ExitCode::SUCCESS,
            Err(code) => ExitCode::from(code),
        },
        Cmd::Run {
            session,
            bind,
            renderer,
            media_directory,
        } => match run_daemon(session, bind, renderer, media_directory) {
            Ok(()) => ExitCode::SUCCESS,
            Err(code) => ExitCode::from(code),
        },
    }
}

fn load_valid(path: &PathBuf) -> Result<eiviz_control::Document, u8> {
    let doc = session::read_document(path).map_err(|error| {
        let missing = std::fs::metadata(path).is_err();
        eprintln!(
            "eiviz-headless error={} {error}",
            if missing { "read" } else { "session" }
        );
        if missing { EXIT_ARGS } else { EXIT_SESSION }
    })?;
    validate_for_apply(&doc).map_err(|error| {
        eprintln!("eiviz-headless error=session {}", error.message);
        EXIT_SESSION
    })?;
    Ok(doc)
}

#[cfg(feature = "runtime")]
fn resolve_run_session(session: Option<PathBuf>) -> Result<(PathBuf, eiviz_control::Document), u8> {
    if let Some(path) = session {
        return Ok((path.clone(), load_valid(&path)?));
    }
    let dir = eiviz_api::default_sessions_directory();
    std::fs::create_dir_all(&dir).map_err(|error| {
        eprintln!("eiviz-headless error=session {error}");
        EXIT_SESSION
    })?;
    let path = dir.join(session::dated_session_filename_now());
    let document = session::default_document();
    session::write_document(&path, &document).map_err(|error| {
        eprintln!("eiviz-headless error=session {error}");
        EXIT_SESSION
    })?;
    Ok((path, document))
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

fn export(input: &PathBuf, output: &PathBuf) -> Result<(), u8> {
    let doc = load_valid(input)?;
    session::export_document(output, &doc).map_err(|error| {
        eprintln!("eiviz-headless error=session {error}");
        EXIT_SESSION
    })?;
    Ok(())
}

fn history(path: &PathBuf) -> Result<(), u8> {
    let entries = session::read_history(path).map_err(|error| {
        eprintln!("eiviz-headless error=session {error}");
        EXIT_SESSION
    })?;
    if entries.is_empty() {
        println!("0");
        return Ok(());
    }
    for entry in entries {
        println!("{}\t{}\t{}", entry.index, entry.unix_ms, entry.revision);
    }
    Ok(())
}

fn restore(input: &PathBuf, index: u32, output: &PathBuf) -> Result<(), u8> {
    let doc = session::extract_history(input, index).map_err(|error| {
        eprintln!("eiviz-headless error=session {error}");
        EXIT_SESSION
    })?;
    let bytes = session::encode_file(&doc).map_err(|error| {
        eprintln!("eiviz-headless error=session {error}");
        EXIT_SESSION
    })?;
    std::fs::write(output, bytes).map_err(|error| {
        eprintln!("eiviz-headless error=session {error}");
        EXIT_SESSION
    })?;
    Ok(())
}

fn run_daemon(
    session: Option<PathBuf>,
    bind: Option<String>,
    renderer: Option<String>,
    media_directory: Option<PathBuf>,
) -> Result<(), u8> {
    #[cfg(not(feature = "runtime"))]
    {
        let _ = (session, bind, renderer, media_directory);
        eprintln!("eiviz-headless error=runtime binary built without mixer runtime");
        Err(EXIT_OTHER)
    }
    #[cfg(feature = "runtime")]
    {
        run_daemon_runtime(session, bind, renderer, media_directory)
    }
}

#[cfg(feature = "runtime")]
fn run_daemon_runtime(
    session: Option<PathBuf>,
    bind: Option<String>,
    renderer: Option<String>,
    media_directory: Option<PathBuf>,
) -> Result<(), u8> {
    let prefs = eiviz_headless::HeadlessPrefs::load().map_err(|error| {
        eprintln!("eiviz-headless error=prefs {error}");
        EXIT_ARGS
    })?;
    let renderer =
        eiviz_headless::resolve_renderer(renderer.as_deref(), &prefs).map_err(|error| {
            eprintln!("eiviz-headless error=renderer {error}");
            EXIT_ARGS
        })?;
    let bind = bind
        .or(prefs.bind.clone())
        .unwrap_or_else(|| "127.0.0.1:9400".into());
    let media_directory =
        media_directory.or_else(|| prefs.media_directory.as_ref().map(PathBuf::from));
    let (session, mut document) = resolve_run_session(session)?;
    document.settings.renderer = renderer;
    let ws_addr: SocketAddr = bind.parse().map_err(|error| {
        eprintln!("eiviz-headless error=bind {error}");
        EXIT_BIND
    })?;
    let auth = auth_from_prefs(&prefs).map_err(|error| {
        eprintln!("eiviz-headless error=auth {error}");
        EXIT_ARGS
    })?;
    let stdin_token = auth.token.clone();
    if !ws_addr.ip().is_loopback() && !auth.require_auth {
        eprintln!(
            "eiviz-headless error=bind remote bind requires a token (eivizctl prefs set token)"
        );
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
            let path = session.canonicalize().unwrap_or(session);
            svc.set_session_path(Some(path.clone()));
            eprintln!("eiviz-headless session={}", path.display());
        }
        let media_path = media_directory.unwrap_or_else(|| eiviz_api::resolve_media_directory(""));
        let media =
            eiviz_api::FileMediaStorage::new(eiviz_api::MediaStorageConfig::new(media_path))
                .map_err(|error| {
                    eprintln!("eiviz-headless error=media {error}");
                    EXIT_OTHER
                })
                .map(|store| Arc::new(store) as Arc<dyn eiviz_api::MediaStorage>)?;
        let config = ServerConfig {
            bind: ws_addr,
            auth,
            idle_timeout: std::time::Duration::from_secs(60),
            max_clients: 32,
            media: Some(media),
        };
        let control: Arc<dyn eiviz_control::ControlFacade> = Arc::new(eiviz_mixer::MixerFacade);
        let mut handle = listen(config, control).await.map_err(|error| {
            eprintln!("eiviz-headless error=bind {error}");
            EXIT_BIND
        })?;
        eprintln!("eiviz-headless ready ws={}", handle.bind.ws_addr);
        let loopback = eiviz_headless::stdin_control::loopback_ws_url(handle.bind.ws_addr);
        let stdin_session = match eiviz_api::ControlClient::websocket(&loopback, stdin_token)
            .connect()
            .await
        {
            Ok(session) => Some(session),
            Err(error) => {
                eprintln!("eiviz-headless error=stdin {error}");
                None
            }
        };
        let mut stdin_rx = eiviz_headless::stdin_control::spawn_lines();
        let mut stdin_open = stdin_session.is_some();
        loop {
            tokio::select! {
                _ = shutdown_signal() => break,
                _ = handle.wait_shutdown_request() => break,
                line = stdin_rx.recv(), if stdin_open => {
                    match (line, stdin_session.as_ref()) {
                        (Some(line), Some(session)) => {
                            eiviz_headless::stdin_control::handle_line(session, &line).await;
                        }
                        _ => stdin_open = false,
                    }
                }
            }
        }
        spawn_exit_watchdog();
        eprintln!("eiviz-headless stopping");
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
        if let Err(error) = handle.shutdown().await {
            eprintln!("eiviz-headless error=api {error}");
        }
        eprintln!("eiviz-headless exit complete");
        Ok(())
    })
}

#[cfg(feature = "runtime")]
fn auth_from_prefs(prefs: &eiviz_headless::HeadlessPrefs) -> Result<AuthConfig, String> {
    let token = prefs
        .token
        .clone()
        .filter(|value| !value.is_empty())
        .unwrap_or_default();
    let require_auth = !token.is_empty();
    let max_role = match prefs.max_role.as_deref() {
        Some(name) => eiviz_api::Role::try_from_name(name)?,
        None => {
            if require_auth {
                eiviz_api::Role::Admin
            } else {
                eiviz_api::Role::Read
            }
        }
    };
    Ok(AuthConfig {
        token,
        require_auth,
        max_role,
    })
}

#[cfg(feature = "runtime")]
fn spawn_exit_watchdog() {
    std::thread::Builder::new()
        .name("eiviz-exit-watchdog".into())
        .spawn(|| {
            std::thread::sleep(std::time::Duration::from_secs(8));
            eprintln!("eiviz-headless error=shutdown watchdog");
            std::process::exit(i32::from(EXIT_OTHER));
        })
        .ok();
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
