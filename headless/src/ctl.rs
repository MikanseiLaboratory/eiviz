//! Shared eivizctl / headless-stdin command surface.

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use eiviz_api::client::ControlSession;
use eiviz_control::SessionMutation;
use eiviz_control::session::{
    BusDto, Document, InputDto, MultiviewDto, OutputDto, OverlaySlot, SceneDto, SceneLayer,
    SceneLayoutPreset, SessionSettings, UnitDto,
};
use serde_json::{Value, json};

#[derive(Parser)]
#[command(
    name = "eivizctl",
    no_binary_name = true,
    disable_version_flag = true,
    subcommand_required = true
)]
pub struct Line {
    #[command(subcommand)]
    pub cmd: CtlCommand,
}

#[derive(Debug, Subcommand)]
pub enum CtlCommand {
    Status,
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
        #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
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
        #[arg(long)]
        expected_revision: Option<u64>,
        #[arg(long)]
        force: bool,
    },
    Save,
    Shutdown,
    Prefs {
        #[command(subcommand)]
        action: Option<PrefsCmd>,
    },
    #[command(subcommand)]
    Session(SessionCmd),
    #[command(subcommand)]
    Input(InputCmd),
    #[command(subcommand)]
    Scene(SceneCmd),
    #[command(subcommand)]
    Unit(UnitCmd),
    #[command(subcommand)]
    Overlay(OverlayCmd),
    #[command(subcommand)]
    Multiview(MultiviewCmd),
    #[command(subcommand)]
    Output(OutputCmd),
    #[command(subcommand)]
    Bus(BusCmd),
    #[command(subcommand)]
    Settings(SettingsCmd),
    #[command(subcommand)]
    Tag(TagCmd),
    #[command(subcommand)]
    Preset(PresetCmd),
    #[command(subcommand)]
    Mix(MixCmd),
    #[command(subcommand)]
    Video(VideoCmd),
    #[command(subcommand)]
    Audio(AudioCmd),
    #[command(subcommand)]
    Discover(DiscoverCmd),
    #[command(subcommand)]
    Capture(CaptureCmd),
}

#[derive(Debug, Subcommand)]
pub enum PrefsCmd {
    Get { key: String },
    Set { key: String, value: Vec<String> },
}

#[derive(Debug, Subcommand)]
pub enum SessionCmd {
    Show,
    Replace {
        #[arg(long)]
        session: PathBuf,
        #[arg(long)]
        expected_revision: Option<u64>,
        #[arg(long)]
        force: bool,
    },
    Save,
    Watch,
}

#[derive(Debug, Subcommand)]
pub enum InputCmd {
    List,
    Get {
        #[arg(long)]
        id: u64,
    },
    Add {
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        kind: Option<String>,
        #[arg(long)]
        address: Option<String>,
        #[arg(long)]
        path: Option<String>,
        #[arg(long)]
        from_json: Option<PathBuf>,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        expected_revision: Option<u64>,
    },
    Edit {
        #[arg(long)]
        id: u64,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        address: Option<String>,
        #[arg(long)]
        path: Option<String>,
        #[arg(long)]
        from_json: Option<PathBuf>,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        expected_revision: Option<u64>,
    },
    Delete {
        #[arg(long)]
        id: u64,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        expected_revision: Option<u64>,
    },
    Upload {
        #[arg(long)]
        file: PathBuf,
        #[arg(long, default_value = "still")]
        kind: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long, default_value_t = true)]
        video_loop: bool,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        expected_revision: Option<u64>,
    },
    Relink {
        #[arg(long)]
        directory: Vec<String>,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        expected_revision: Option<u64>,
    },
}

#[derive(Debug, Subcommand)]
pub enum SceneCmd {
    List,
    Get {
        #[arg(long)]
        id: u64,
    },
    Add {
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        from_json: Option<PathBuf>,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        expected_revision: Option<u64>,
    },
    Edit {
        #[arg(long)]
        id: u64,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        from_json: Option<PathBuf>,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        expected_revision: Option<u64>,
    },
    Delete {
        #[arg(long)]
        id: u64,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        expected_revision: Option<u64>,
    },
    #[command(subcommand)]
    Layer(LayerCmd),
}

#[derive(Debug, Subcommand)]
pub enum LayerCmd {
    List {
        #[arg(long)]
        scene: u64,
    },
    Add {
        #[arg(long)]
        scene: u64,
        #[arg(long)]
        input: u64,
        #[arg(long, default_value_t = 0.0)]
        x: f32,
        #[arg(long, default_value_t = 0.0)]
        y: f32,
        #[arg(long, default_value_t = 1.0)]
        width: f32,
        #[arg(long, default_value_t = 1.0)]
        height: f32,
        #[arg(long)]
        from_json: Option<PathBuf>,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        expected_revision: Option<u64>,
    },
    Edit {
        #[arg(long)]
        scene: u64,
        #[arg(long)]
        index: usize,
        #[arg(long)]
        input: Option<u64>,
        #[arg(long)]
        x: Option<f32>,
        #[arg(long)]
        y: Option<f32>,
        #[arg(long)]
        width: Option<f32>,
        #[arg(long)]
        height: Option<f32>,
        #[arg(long)]
        from_json: Option<PathBuf>,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        expected_revision: Option<u64>,
    },
    Move {
        #[arg(long)]
        scene: u64,
        #[arg(long)]
        from: usize,
        #[arg(long)]
        to: usize,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        expected_revision: Option<u64>,
    },
    Delete {
        #[arg(long)]
        scene: u64,
        #[arg(long)]
        index: usize,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        expected_revision: Option<u64>,
    },
    Replace {
        #[arg(long)]
        scene: u64,
        #[arg(long)]
        from_json: PathBuf,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        expected_revision: Option<u64>,
    },
}

#[derive(Debug, Subcommand)]
pub enum UnitCmd {
    List,
    Get {
        #[arg(long)]
        id: u64,
    },
    Add {
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        from_json: Option<PathBuf>,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        expected_revision: Option<u64>,
    },
    Edit {
        #[arg(long)]
        id: u64,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        from_json: Option<PathBuf>,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        expected_revision: Option<u64>,
    },
    Delete {
        #[arg(long)]
        id: u64,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        expected_revision: Option<u64>,
    },
}

#[derive(Debug, Subcommand)]
pub enum OverlayCmd {
    Set {
        #[arg(long, default_value_t = 1)]
        unit: u64,
        #[arg(long)]
        index: u32,
        #[arg(long)]
        from_json: PathBuf,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        expected_revision: Option<u64>,
    },
    Auto {
        #[arg(long, default_value_t = 1)]
        unit: u64,
        #[arg(long)]
        index: u32,
        #[arg(long, default_value_t = 250)]
        duration_ms: u32,
        #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
        on: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum MultiviewCmd {
    List,
    Get {
        #[arg(long)]
        id: u64,
    },
    Add {
        #[arg(long)]
        from_json: PathBuf,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        expected_revision: Option<u64>,
    },
    Edit {
        #[arg(long)]
        id: u64,
        #[arg(long)]
        from_json: PathBuf,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        expected_revision: Option<u64>,
    },
    Delete {
        #[arg(long)]
        id: u64,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        expected_revision: Option<u64>,
    },
}

#[derive(Debug, Subcommand)]
pub enum OutputCmd {
    List,
    Add {
        #[arg(long)]
        from_json: PathBuf,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        expected_revision: Option<u64>,
    },
    Edit {
        #[arg(long)]
        id: u64,
        #[arg(long)]
        from_json: PathBuf,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        expected_revision: Option<u64>,
    },
    Delete {
        #[arg(long)]
        id: u64,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        expected_revision: Option<u64>,
    },
}

#[derive(Debug, Subcommand)]
pub enum BusCmd {
    List,
    Add {
        #[arg(long)]
        from_json: PathBuf,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        expected_revision: Option<u64>,
    },
    Edit {
        #[arg(long)]
        id: u64,
        #[arg(long)]
        from_json: PathBuf,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        expected_revision: Option<u64>,
    },
    Delete {
        #[arg(long)]
        id: u64,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        expected_revision: Option<u64>,
    },
}

#[derive(Debug, Subcommand)]
pub enum SettingsCmd {
    Get,
    Set {
        #[arg(long)]
        from_json: PathBuf,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        expected_revision: Option<u64>,
    },
}

#[derive(Debug, Subcommand)]
pub enum TagCmd {
    #[command(subcommand)]
    Input(TagAction),
    #[command(subcommand)]
    Scene(TagAction),
}

#[derive(Debug, Subcommand)]
pub enum TagAction {
    List,
    Add {
        tag: String,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        expected_revision: Option<u64>,
    },
    Rename {
        from: String,
        to: String,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        expected_revision: Option<u64>,
    },
    Delete {
        tag: String,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        expected_revision: Option<u64>,
    },
}

#[derive(Debug, Subcommand)]
pub enum PresetCmd {
    #[command(subcommand)]
    Scene(PresetAction),
}

#[derive(Debug, Subcommand)]
pub enum PresetAction {
    List,
    Add {
        #[arg(long)]
        from_json: PathBuf,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        expected_revision: Option<u64>,
    },
    Delete {
        #[arg(long)]
        name: String,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        expected_revision: Option<u64>,
    },
}

#[derive(Debug, Subcommand)]
pub enum MixCmd {
    Preview {
        #[arg(long, default_value_t = 1)]
        unit: u64,
        #[arg(long)]
        scene: u64,
    },
    Cut {
        #[arg(long, default_value_t = 1)]
        unit: u64,
        #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
        swap: bool,
    },
    Auto {
        #[arg(long, default_value_t = 1)]
        unit: u64,
        #[arg(long, default_value_t = 1000)]
        duration_ms: u32,
    },
    Set {
        #[arg(long, default_value_t = 1)]
        unit: u64,
        #[arg(long)]
        value: f32,
    },
}

#[derive(Debug, Subcommand)]
pub enum VideoCmd {
    Play {
        #[arg(long)]
        id: u64,
    },
    Pause {
        #[arg(long)]
        id: u64,
    },
    Loop {
        #[arg(long)]
        id: u64,
        #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
        on: bool,
    },
    Seek {
        #[arg(long)]
        id: u64,
        #[arg(long)]
        position_hns: i64,
    },
}

#[derive(Debug, Subcommand)]
pub enum AudioCmd {
    Input {
        #[arg(long)]
        id: u64,
        #[arg(long)]
        bus_mask: u32,
        #[arg(long)]
        gain: f32,
        #[arg(long)]
        mute: bool,
    },
    Bus {
        #[arg(long)]
        id: u64,
        #[arg(long)]
        gain: f32,
        #[arg(long)]
        mute: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum DiscoverCmd {
    Omt,
    Ndi,
    Uvc,
    #[command(name = "uvc-modes")]
    UvcModes {
        #[arg(long)]
        query: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum CaptureCmd {
    Program {
        #[arg(long, default_value_t = 1)]
        unit: u64,
        #[arg(long)]
        server_path: String,
    },
    Preview {
        #[arg(long, default_value_t = 1)]
        unit: u64,
        #[arg(long)]
        server_path: String,
    },
    Source {
        #[arg(long)]
        id: u64,
        #[arg(long)]
        server_path: String,
    },
}

impl CtlCommand {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Status | Self::Session(SessionCmd::Show) => "session show",
            Self::Watch | Self::Session(SessionCmd::Watch) => "watch",
            Self::Preview { .. } | Self::Mix(MixCmd::Preview { .. }) => "mix preview",
            Self::Cut { .. } | Self::Mix(MixCmd::Cut { .. }) => "mix cut",
            Self::Auto { .. } | Self::Mix(MixCmd::Auto { .. }) => "mix auto",
            Self::Replace { .. } | Self::Session(SessionCmd::Replace { .. }) => "session replace",
            Self::Save | Self::Session(SessionCmd::Save) => "session save",
            Self::Shutdown => "shutdown",
            Self::Prefs { .. } => "prefs",
            Self::Input(_) => "input",
            Self::Scene(_) => "scene",
            Self::Unit(_) => "unit",
            Self::Overlay(_) => "overlay",
            Self::Multiview(_) => "multiview",
            Self::Output(_) => "output",
            Self::Bus(_) => "bus",
            Self::Settings(_) => "settings",
            Self::Tag(_) => "tag",
            Self::Preset(_) => "preset",
            Self::Mix(MixCmd::Set { .. }) => "mix set",
            Self::Video(_) => "video",
            Self::Audio(_) => "audio",
            Self::Discover(_) => "discover",
            Self::Capture(_) => "capture",
        }
    }

    pub fn is_watch(&self) -> bool {
        matches!(self, Self::Watch | Self::Session(SessionCmd::Watch))
    }

    pub fn is_prefs(&self) -> bool {
        matches!(self, Self::Prefs { .. })
    }
}

pub fn parse_line(line: &str) -> Result<CtlCommand, String> {
    let args = split_args(line)?;
    Line::try_parse_from(args)
        .map(|parsed| parsed.cmd)
        .map_err(|error| error.to_string())
}

pub fn split_args(line: &str) -> Result<Vec<String>, String> {
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

pub async fn run_cmd(session: &ControlSession, cmd: CtlCommand, json: bool) -> Result<(), String> {
    let name = cmd.name().to_string();
    match execute(session, cmd).await {
        Ok(result) => {
            print_ok(json, &name, result.revision, result.payload);
            Ok(())
        }
        Err(error) => {
            print_err(json, &name, &error);
            Err(error)
        }
    }
}

struct CmdResult {
    revision: u64,
    payload: Value,
}

async fn execute(session: &ControlSession, cmd: CtlCommand) -> Result<CmdResult, String> {
    match cmd {
        CtlCommand::Status | CtlCommand::Session(SessionCmd::Show) => show_session(session).await,
        CtlCommand::Watch | CtlCommand::Session(SessionCmd::Watch) => {
            Err("watch is not a one-shot command".into())
        }
        CtlCommand::Prefs { .. } => Err("prefs is a local command".into()),
        CtlCommand::Preview { unit, scene } | CtlCommand::Mix(MixCmd::Preview { unit, scene }) => {
            session.preview(unit, scene).await.map_err(map_err)?;
            ok_live(session, json!({ "unit": unit, "scene": scene })).await
        }
        CtlCommand::Cut { unit, swap } | CtlCommand::Mix(MixCmd::Cut { unit, swap }) => {
            session.cut(unit, swap).await.map_err(map_err)?;
            ok_live(session, json!({ "unit": unit, "swap": swap })).await
        }
        CtlCommand::Auto { unit, duration_ms }
        | CtlCommand::Mix(MixCmd::Auto { unit, duration_ms }) => {
            session
                .auto(unit, duration_ms, true)
                .await
                .map_err(map_err)?;
            ok_live(session, json!({ "unit": unit, "durationMs": duration_ms })).await
        }
        CtlCommand::Mix(MixCmd::Set { unit, value }) => {
            session.set_mix(unit, value).await.map_err(map_err)?;
            ok_live(session, json!({ "unit": unit, "value": value })).await
        }
        CtlCommand::Replace {
            session: path,
            expected_revision,
            force,
        }
        | CtlCommand::Session(SessionCmd::Replace {
            session: path,
            expected_revision,
            force,
        }) => {
            let document = eiviz_control::session::read_document(&path)?;
            let document_json = eiviz_control::session::to_vec(&document)?;
            let revision = resolve_revision(session, expected_revision, force).await?;
            session
                .replace_session(document_json, revision)
                .await
                .map_err(map_err)?;
            ok_live(session, json!({ "path": path.display().to_string() })).await
        }
        CtlCommand::Save | CtlCommand::Session(SessionCmd::Save) => {
            let (path, history_count) = session.save_session().await.map_err(map_err)?;
            let revision = session.view().revision;
            Ok(CmdResult {
                revision,
                payload: json!({ "path": path, "historyCount": history_count }),
            })
        }
        CtlCommand::Shutdown => {
            session.shutdown().await.map_err(map_err)?;
            Ok(CmdResult {
                revision: session.view().revision,
                payload: json!({}),
            })
        }
        CtlCommand::Input(cmd) => input_cmd(session, cmd).await,
        CtlCommand::Scene(cmd) => scene_cmd(session, cmd).await,
        CtlCommand::Unit(cmd) => unit_cmd(session, cmd).await,
        CtlCommand::Overlay(cmd) => overlay_cmd(session, cmd).await,
        CtlCommand::Multiview(cmd) => multiview_cmd(session, cmd).await,
        CtlCommand::Output(cmd) => output_cmd(session, cmd).await,
        CtlCommand::Bus(cmd) => bus_cmd(session, cmd).await,
        CtlCommand::Settings(cmd) => settings_cmd(session, cmd).await,
        CtlCommand::Tag(cmd) => tag_cmd(session, cmd).await,
        CtlCommand::Preset(cmd) => preset_cmd(session, cmd).await,
        CtlCommand::Video(cmd) => video_cmd(session, cmd).await,
        CtlCommand::Audio(cmd) => audio_cmd(session, cmd).await,
        CtlCommand::Discover(cmd) => discover_cmd(session, cmd).await,
        CtlCommand::Capture(cmd) => capture_cmd(session, cmd).await,
    }
}

async fn input_cmd(session: &ControlSession, cmd: InputCmd) -> Result<CmdResult, String> {
    match cmd {
        InputCmd::List => {
            let (doc, revision) = load_doc(session).await?;
            Ok(CmdResult {
                revision,
                payload: serde_json::to_value(&doc.inputs).map_err(|e| e.to_string())?,
            })
        }
        InputCmd::Get { id } => {
            let (doc, revision) = load_doc(session).await?;
            let input = doc
                .inputs
                .iter()
                .find(|item| item.id == id)
                .ok_or_else(|| format!("input {id} not found"))?;
            Ok(CmdResult {
                revision,
                payload: serde_json::to_value(input).map_err(|e| e.to_string())?,
            })
        }
        InputCmd::Add {
            name,
            kind,
            address,
            path,
            from_json,
            force,
            expected_revision,
        } => {
            let input = if let Some(file) = from_json {
                read_json::<InputDto>(&file)?
            } else {
                let kind =
                    kind.ok_or_else(|| "kind is required unless --from-json is set".to_string())?;
                let mut value = json!({
                    "id": 0,
                    "name": name.unwrap_or_default(),
                    "kind": kind,
                });
                if let Some(address) = address.or(path) {
                    value["pathOrAddress"] = json!(address);
                }
                serde_json::from_value(value).map_err(|e| e.to_string())?
            };
            mutate(
                session,
                SessionMutation::CreateInput {
                    input: Box::new(input),
                },
                expected_revision,
                force,
            )
            .await
        }
        InputCmd::Edit {
            id,
            name,
            address,
            path,
            from_json,
            force,
            expected_revision,
        } => {
            let (mut doc, revision) = load_doc(session).await?;
            let expected = revision_or(revision, expected_revision, force);
            let input = doc
                .inputs
                .iter_mut()
                .find(|item| item.id == id)
                .ok_or_else(|| format!("input {id} not found"))?;
            if let Some(file) = from_json {
                *input = read_json(&file)?;
                input.id = id;
            }
            if let Some(name) = name {
                input.name = name;
            }
            if let Some(address) = address.or(path) {
                input.path_or_address = Some(address);
            }
            let input = input.clone();
            apply_mutation(
                session,
                SessionMutation::UpsertInput {
                    input: Box::new(input),
                },
                expected,
            )
            .await
        }
        InputCmd::Delete {
            id,
            force,
            expected_revision,
        } => {
            mutate(
                session,
                SessionMutation::DeleteInput { id },
                expected_revision,
                force,
            )
            .await
        }
        InputCmd::Upload {
            file,
            kind,
            name,
            video_loop,
            force,
            expected_revision,
        } => {
            let revision = resolve_revision(session, expected_revision, force).await?;
            let name = name.unwrap_or_else(|| {
                file.file_stem()
                    .and_then(|name| name.to_str())
                    .unwrap_or("Media")
                    .to_string()
            });
            session
                .upload_file(&file, &kind, &name, video_loop, revision)
                .await
                .map_err(map_err)?;
            ok_live(session, json!({ "name": name, "kind": kind })).await
        }
        InputCmd::Relink {
            directory,
            force,
            expected_revision,
        } => {
            mutate(
                session,
                SessionMutation::RelinkMedia {
                    directories: directory,
                },
                expected_revision,
                force,
            )
            .await
        }
    }
}

async fn scene_cmd(session: &ControlSession, cmd: SceneCmd) -> Result<CmdResult, String> {
    match cmd {
        SceneCmd::List => list_field(session, |doc| &doc.scenes).await,
        SceneCmd::Get { id } => {
            let (doc, revision) = load_doc(session).await?;
            let scene = doc
                .scenes
                .iter()
                .find(|item| item.id == id)
                .ok_or_else(|| format!("scene {id} not found"))?;
            Ok(CmdResult {
                revision,
                payload: serde_json::to_value(scene).map_err(|e| e.to_string())?,
            })
        }
        SceneCmd::Add {
            name,
            from_json,
            force,
            expected_revision,
        } => {
            let scene = if let Some(file) = from_json {
                read_json::<SceneDto>(&file)?
            } else {
                serde_json::from_value(json!({
                    "id": 0,
                    "name": name.unwrap_or_default(),
                    "layers": []
                }))
                .map_err(|e| e.to_string())?
            };
            mutate(
                session,
                SessionMutation::CreateScene {
                    scene: Box::new(scene),
                },
                expected_revision,
                force,
            )
            .await
        }
        SceneCmd::Edit {
            id,
            name,
            from_json,
            force,
            expected_revision,
        } => {
            let (mut doc, revision) = load_doc(session).await?;
            let expected = revision_or(revision, expected_revision, force);
            let scene = doc
                .scenes
                .iter_mut()
                .find(|item| item.id == id)
                .ok_or_else(|| format!("scene {id} not found"))?;
            if let Some(file) = from_json {
                *scene = read_json(&file)?;
                scene.id = id;
            }
            if let Some(name) = name {
                scene.name = name;
            }
            let scene = scene.clone();
            apply_mutation(
                session,
                SessionMutation::UpsertScene {
                    scene: Box::new(scene),
                },
                expected,
            )
            .await
        }
        SceneCmd::Delete {
            id,
            force,
            expected_revision,
        } => {
            mutate(
                session,
                SessionMutation::DeleteScene { id },
                expected_revision,
                force,
            )
            .await
        }
        SceneCmd::Layer(cmd) => layer_cmd(session, cmd).await,
    }
}

async fn layer_cmd(session: &ControlSession, cmd: LayerCmd) -> Result<CmdResult, String> {
    match cmd {
        LayerCmd::List { scene } => {
            let (doc, revision) = load_doc(session).await?;
            let found = doc
                .scenes
                .iter()
                .find(|item| item.id == scene)
                .ok_or_else(|| format!("scene {scene} not found"))?;
            Ok(CmdResult {
                revision,
                payload: serde_json::to_value(&found.layers).map_err(|e| e.to_string())?,
            })
        }
        LayerCmd::Add {
            scene,
            input,
            x,
            y,
            width,
            height,
            from_json,
            force,
            expected_revision,
        } => {
            let (mut doc, revision) = load_doc(session).await?;
            let expected = revision_or(revision, expected_revision, force);
            let layer = if let Some(file) = from_json {
                read_json::<SceneLayer>(&file)?
            } else {
                serde_json::from_value(json!({
                    "inputId": input,
                    "x": x,
                    "y": y,
                    "width": width,
                    "height": height
                }))
                .map_err(|e| e.to_string())?
            };
            eiviz_control::session::edit::add_scene_layer(&mut doc, scene, layer)
                .map_err(map_err)?;
            set_layers(session, scene, doc, expected).await
        }
        LayerCmd::Edit {
            scene,
            index,
            input,
            x,
            y,
            width,
            height,
            from_json,
            force,
            expected_revision,
        } => {
            let (mut doc, revision) = load_doc(session).await?;
            let expected = revision_or(revision, expected_revision, force);
            let found = doc
                .scenes
                .iter_mut()
                .find(|item| item.id == scene)
                .ok_or_else(|| format!("scene {scene} not found"))?;
            let layer = found
                .layers
                .get_mut(index)
                .ok_or_else(|| format!("layer {index} not found"))?;
            if let Some(file) = from_json {
                *layer = read_json(&file)?;
            }
            if let Some(input) = input {
                layer.input_id = input;
            }
            if let Some(x) = x {
                layer.x = x;
            }
            if let Some(y) = y {
                layer.y = y;
            }
            if let Some(width) = width {
                layer.width = width;
            }
            if let Some(height) = height {
                layer.height = height;
            }
            let layers = found.layers.clone();
            apply_mutation(
                session,
                SessionMutation::SetSceneLayers {
                    scene_id: scene,
                    layers,
                },
                expected,
            )
            .await
        }
        LayerCmd::Move {
            scene,
            from,
            to,
            force,
            expected_revision,
        } => {
            let (mut doc, revision) = load_doc(session).await?;
            let expected = revision_or(revision, expected_revision, force);
            eiviz_control::session::edit::move_scene_layer(&mut doc, scene, from, to)
                .map_err(map_err)?;
            set_layers(session, scene, doc, expected).await
        }
        LayerCmd::Delete {
            scene,
            index,
            force,
            expected_revision,
        } => {
            let (mut doc, revision) = load_doc(session).await?;
            let expected = revision_or(revision, expected_revision, force);
            eiviz_control::session::edit::delete_scene_layer(&mut doc, scene, index)
                .map_err(map_err)?;
            set_layers(session, scene, doc, expected).await
        }
        LayerCmd::Replace {
            scene,
            from_json,
            force,
            expected_revision,
        } => {
            let layers = read_json::<Vec<SceneLayer>>(&from_json)?;
            mutate(
                session,
                SessionMutation::SetSceneLayers {
                    scene_id: scene,
                    layers,
                },
                expected_revision,
                force,
            )
            .await
        }
    }
}

async fn unit_cmd(session: &ControlSession, cmd: UnitCmd) -> Result<CmdResult, String> {
    match cmd {
        UnitCmd::List => list_field(session, |doc| &doc.units).await,
        UnitCmd::Get { id } => {
            get_named(session, |doc| {
                doc.units.iter().find(|item| item.id == id).cloned()
            })
            .await
        }
        UnitCmd::Add {
            name,
            from_json,
            force,
            expected_revision,
        } => {
            let unit = if let Some(file) = from_json {
                read_json::<UnitDto>(&file)?
            } else {
                serde_json::from_value(json!({
                    "id": 0,
                    "name": name.unwrap_or_default()
                }))
                .map_err(|e| e.to_string())?
            };
            mutate(
                session,
                SessionMutation::CreateUnit {
                    unit: Box::new(unit),
                },
                expected_revision,
                force,
            )
            .await
        }
        UnitCmd::Edit {
            id,
            name,
            from_json,
            force,
            expected_revision,
        } => {
            let (mut doc, revision) = load_doc(session).await?;
            let expected = revision_or(revision, expected_revision, force);
            let unit = doc
                .units
                .iter_mut()
                .find(|item| item.id == id)
                .ok_or_else(|| format!("unit {id} not found"))?;
            if let Some(file) = from_json {
                *unit = read_json(&file)?;
                unit.id = id;
            }
            if let Some(name) = name {
                unit.name = name;
            }
            let unit = unit.clone();
            apply_mutation(
                session,
                SessionMutation::UpsertUnit {
                    unit: Box::new(unit),
                },
                expected,
            )
            .await
        }
        UnitCmd::Delete {
            id,
            force,
            expected_revision,
        } => {
            mutate(
                session,
                SessionMutation::DeleteUnit { id },
                expected_revision,
                force,
            )
            .await
        }
    }
}

async fn overlay_cmd(session: &ControlSession, cmd: OverlayCmd) -> Result<CmdResult, String> {
    match cmd {
        OverlayCmd::Set {
            unit,
            index,
            from_json,
            force,
            expected_revision,
        } => {
            let slot = read_json::<OverlaySlot>(&from_json)?;
            mutate(
                session,
                SessionMutation::SetOverlaySlot {
                    unit_id: unit,
                    index,
                    slot: Box::new(slot),
                },
                expected_revision,
                force,
            )
            .await
        }
        OverlayCmd::Auto {
            unit,
            index,
            duration_ms,
            on,
        } => {
            session
                .overlay_auto(unit, index, duration_ms, on)
                .await
                .map_err(map_err)?;
            ok_live(session, json!({ "unit": unit, "index": index, "toOn": on })).await
        }
    }
}

async fn multiview_cmd(session: &ControlSession, cmd: MultiviewCmd) -> Result<CmdResult, String> {
    match cmd {
        MultiviewCmd::List => list_field(session, |doc| &doc.multiviews).await,
        MultiviewCmd::Get { id } => {
            get_named(session, |doc| {
                doc.multiviews.iter().find(|item| item.id == id).cloned()
            })
            .await
        }
        MultiviewCmd::Add {
            from_json,
            force,
            expected_revision,
        } => {
            let layout = read_json::<MultiviewDto>(&from_json)?;
            mutate(
                session,
                SessionMutation::CreateMultiview {
                    layout: Box::new(layout),
                },
                expected_revision,
                force,
            )
            .await
        }
        MultiviewCmd::Edit {
            id,
            from_json,
            force,
            expected_revision,
        } => {
            let mut layout = read_json::<MultiviewDto>(&from_json)?;
            layout.id = id;
            mutate(
                session,
                SessionMutation::UpsertMultiview {
                    layout: Box::new(layout),
                },
                expected_revision,
                force,
            )
            .await
        }
        MultiviewCmd::Delete {
            id,
            force,
            expected_revision,
        } => {
            mutate(
                session,
                SessionMutation::DeleteMultiview { id },
                expected_revision,
                force,
            )
            .await
        }
    }
}

async fn output_cmd(session: &ControlSession, cmd: OutputCmd) -> Result<CmdResult, String> {
    match cmd {
        OutputCmd::List => list_field(session, |doc| &doc.outputs).await,
        OutputCmd::Add {
            from_json,
            force,
            expected_revision,
        } => {
            patch_outputs(session, expected_revision, force, |doc| {
                doc.outputs.push(read_json::<OutputDto>(&from_json)?);
                Ok(())
            })
            .await
        }
        OutputCmd::Edit {
            id,
            from_json,
            force,
            expected_revision,
        } => {
            patch_outputs(session, expected_revision, force, |doc| {
                let mut output = read_json::<OutputDto>(&from_json)?;
                output.id = id;
                if let Some(existing) = doc.outputs.iter_mut().find(|item| item.id == id) {
                    *existing = output;
                } else {
                    return Err(format!("output {id} not found"));
                }
                Ok(())
            })
            .await
        }
        OutputCmd::Delete {
            id,
            force,
            expected_revision,
        } => {
            patch_outputs(session, expected_revision, force, |doc| {
                let before = doc.outputs.len();
                doc.outputs.retain(|item| item.id != id);
                if doc.outputs.len() == before {
                    return Err(format!("output {id} not found"));
                }
                Ok(())
            })
            .await
        }
    }
}

async fn bus_cmd(session: &ControlSession, cmd: BusCmd) -> Result<CmdResult, String> {
    match cmd {
        BusCmd::List => list_field(session, |doc| &doc.buses).await,
        BusCmd::Add {
            from_json,
            force,
            expected_revision,
        } => {
            patch_outputs(session, expected_revision, force, |doc| {
                doc.buses.push(read_json::<BusDto>(&from_json)?);
                Ok(())
            })
            .await
        }
        BusCmd::Edit {
            id,
            from_json,
            force,
            expected_revision,
        } => {
            patch_outputs(session, expected_revision, force, |doc| {
                let mut bus = read_json::<BusDto>(&from_json)?;
                bus.id = id;
                if let Some(existing) = doc.buses.iter_mut().find(|item| item.id == id) {
                    *existing = bus;
                } else {
                    return Err(format!("bus {id} not found"));
                }
                Ok(())
            })
            .await
        }
        BusCmd::Delete {
            id,
            force,
            expected_revision,
        } => {
            patch_outputs(session, expected_revision, force, |doc| {
                let before = doc.buses.len();
                doc.buses.retain(|item| item.id != id);
                if doc.buses.len() == before {
                    return Err(format!("bus {id} not found"));
                }
                Ok(())
            })
            .await
        }
    }
}

async fn settings_cmd(session: &ControlSession, cmd: SettingsCmd) -> Result<CmdResult, String> {
    match cmd {
        SettingsCmd::Get => {
            let (doc, revision) = load_doc(session).await?;
            Ok(CmdResult {
                revision,
                payload: serde_json::to_value(&doc.settings).map_err(|e| e.to_string())?,
            })
        }
        SettingsCmd::Set {
            from_json,
            force,
            expected_revision,
        } => {
            let settings = read_json::<SessionSettings>(&from_json)?;
            let (doc, revision) = load_doc(session).await?;
            let expected = revision_or(revision, expected_revision, force);
            apply_mutation(
                session,
                SessionMutation::SetSettings {
                    settings: Box::new(settings),
                    outputs: doc.outputs,
                    buses: doc.buses,
                    headphone_copy_master: Some(doc.headphone_copy_master),
                    next_output_id: doc.next_output_id,
                    next_bus_id: doc.next_bus_id,
                },
                expected,
            )
            .await
        }
    }
}

async fn tag_cmd(session: &ControlSession, cmd: TagCmd) -> Result<CmdResult, String> {
    let (catalog, action) = match cmd {
        TagCmd::Input(action) => ("input", action),
        TagCmd::Scene(action) => ("scene", action),
    };
    match action {
        TagAction::List => {
            let (doc, revision) = load_doc(session).await?;
            let tags = if catalog == "input" {
                &doc.input_tags
            } else {
                &doc.scene_tags
            };
            Ok(CmdResult {
                revision,
                payload: json!(tags),
            })
        }
        TagAction::Add {
            tag,
            force,
            expected_revision,
        } => {
            mutate(
                session,
                SessionMutation::AddCatalogTag {
                    catalog: catalog.into(),
                    tag,
                },
                expected_revision,
                force,
            )
            .await
        }
        TagAction::Rename {
            from,
            to,
            force,
            expected_revision,
        } => {
            mutate(
                session,
                SessionMutation::RenameCatalogTag {
                    catalog: catalog.into(),
                    from,
                    to,
                },
                expected_revision,
                force,
            )
            .await
        }
        TagAction::Delete {
            tag,
            force,
            expected_revision,
        } => {
            mutate(
                session,
                SessionMutation::DeleteCatalogTag {
                    catalog: catalog.into(),
                    tag,
                },
                expected_revision,
                force,
            )
            .await
        }
    }
}

async fn preset_cmd(session: &ControlSession, cmd: PresetCmd) -> Result<CmdResult, String> {
    let PresetCmd::Scene(action) = cmd;
    match action {
        PresetAction::List => list_field(session, |doc| &doc.scene_presets).await,
        PresetAction::Add {
            from_json,
            force,
            expected_revision,
        } => {
            let preset = read_json::<SceneLayoutPreset>(&from_json)?;
            mutate(
                session,
                SessionMutation::UpsertScenePreset {
                    preset: Box::new(preset),
                },
                expected_revision,
                force,
            )
            .await
        }
        PresetAction::Delete {
            name,
            force,
            expected_revision,
        } => {
            mutate(
                session,
                SessionMutation::DeleteScenePreset { name },
                expected_revision,
                force,
            )
            .await
        }
    }
}

async fn video_cmd(session: &ControlSession, cmd: VideoCmd) -> Result<CmdResult, String> {
    match cmd {
        VideoCmd::Play { id } => session.video_play(id, true).await.map_err(map_err)?,
        VideoCmd::Pause { id } => session.video_play(id, false).await.map_err(map_err)?,
        VideoCmd::Loop { id, on } => session.video_loop(id, on).await.map_err(map_err)?,
        VideoCmd::Seek { id, position_hns } => session
            .video_seek(id, position_hns)
            .await
            .map_err(map_err)?,
    }
    ok_live(session, json!({})).await
}

async fn audio_cmd(session: &ControlSession, cmd: AudioCmd) -> Result<CmdResult, String> {
    match cmd {
        AudioCmd::Input {
            id,
            bus_mask,
            gain,
            mute,
        } => session
            .audio_set_input(id, bus_mask, gain, mute)
            .await
            .map_err(map_err)?,
        AudioCmd::Bus { id, gain, mute } => session
            .audio_set_bus(id, gain, mute)
            .await
            .map_err(map_err)?,
    }
    ok_live(session, json!({})).await
}

async fn discover_cmd(session: &ControlSession, cmd: DiscoverCmd) -> Result<CmdResult, String> {
    let (kind, query) = match &cmd {
        DiscoverCmd::Omt => ("omt", ""),
        DiscoverCmd::Ndi => ("ndi", ""),
        DiscoverCmd::Uvc => ("uvc", ""),
        DiscoverCmd::UvcModes { query } => ("uvcModes", query.as_str()),
    };
    let payload = session.discover(kind, query).await.map_err(map_err)?;
    Ok(CmdResult {
        revision: session.view().revision,
        payload: json!({ "kind": kind, "payload": payload }),
    })
}

async fn capture_cmd(session: &ControlSession, cmd: CaptureCmd) -> Result<CmdResult, String> {
    let (unit, kind, path) = match cmd {
        CaptureCmd::Program { unit, server_path } => (unit, 0, server_path),
        CaptureCmd::Preview { unit, server_path } => (unit, 1, server_path),
        CaptureCmd::Source { id, server_path } => (id, 3, server_path),
    };
    session
        .snapshot_cmd(unit, kind, &path)
        .await
        .map_err(map_err)?;
    ok_live(session, json!({ "path": path, "kind": kind, "id": unit })).await
}

async fn show_session(session: &ControlSession) -> Result<CmdResult, String> {
    let (json, revision) = session.snapshot_document_json().await.map_err(map_err)?;
    let value: Value = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    Ok(CmdResult {
        revision,
        payload: value,
    })
}

async fn list_field<T: serde::Serialize>(
    session: &ControlSession,
    f: impl Fn(&Document) -> &T,
) -> Result<CmdResult, String> {
    let (doc, revision) = load_doc(session).await?;
    Ok(CmdResult {
        revision,
        payload: serde_json::to_value(f(&doc)).map_err(|e| e.to_string())?,
    })
}

async fn get_named<T: serde::Serialize>(
    session: &ControlSession,
    f: impl Fn(&Document) -> Option<T>,
) -> Result<CmdResult, String> {
    let (doc, revision) = load_doc(session).await?;
    let value = f(&doc).ok_or_else(|| "not found".to_string())?;
    Ok(CmdResult {
        revision,
        payload: serde_json::to_value(value).map_err(|e| e.to_string())?,
    })
}

async fn patch_outputs(
    session: &ControlSession,
    expected_revision: Option<u64>,
    force: bool,
    patch: impl FnOnce(&mut Document) -> Result<(), String>,
) -> Result<CmdResult, String> {
    let (mut doc, revision) = load_doc(session).await?;
    let expected = revision_or(revision, expected_revision, force);
    patch(&mut doc)?;
    apply_mutation(
        session,
        SessionMutation::SetSettings {
            settings: Box::new(doc.settings),
            outputs: doc.outputs,
            buses: doc.buses,
            headphone_copy_master: Some(doc.headphone_copy_master),
            next_output_id: doc.next_output_id,
            next_bus_id: doc.next_bus_id,
        },
        expected,
    )
    .await
}

async fn set_layers(
    session: &ControlSession,
    scene: u64,
    doc: Document,
    expected: u64,
) -> Result<CmdResult, String> {
    let layers = doc
        .scenes
        .iter()
        .find(|item| item.id == scene)
        .map(|item| item.layers.clone())
        .ok_or_else(|| format!("scene {scene} not found"))?;
    apply_mutation(
        session,
        SessionMutation::SetSceneLayers {
            scene_id: scene,
            layers,
        },
        expected,
    )
    .await
}

async fn mutate(
    session: &ControlSession,
    mutation: SessionMutation,
    expected_revision: Option<u64>,
    force: bool,
) -> Result<CmdResult, String> {
    let revision = resolve_revision(session, expected_revision, force).await?;
    apply_mutation(session, mutation, revision).await
}

async fn apply_mutation(
    session: &ControlSession,
    mutation: SessionMutation,
    expected: u64,
) -> Result<CmdResult, String> {
    session
        .mutate_typed(&mutation, expected)
        .await
        .map_err(map_err)?;
    ok_live(session, json!({ "mutation": mutation.kind_name() })).await
}

async fn load_doc(session: &ControlSession) -> Result<(Document, u64), String> {
    session.snapshot_document().await.map_err(map_err)
}

async fn resolve_revision(
    session: &ControlSession,
    expected_revision: Option<u64>,
    force: bool,
) -> Result<u64, String> {
    if force {
        return Ok(0);
    }
    if let Some(revision) = expected_revision {
        return Ok(revision);
    }
    Ok(load_doc(session).await?.1)
}

fn revision_or(current: u64, expected_revision: Option<u64>, force: bool) -> u64 {
    if force {
        0
    } else {
        expected_revision.unwrap_or(current)
    }
}

async fn ok_live(session: &ControlSession, payload: Value) -> Result<CmdResult, String> {
    Ok(CmdResult {
        revision: session.view().revision,
        payload,
    })
}

fn read_json<T: serde::de::DeserializeOwned>(path: &PathBuf) -> Result<T, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

fn map_err(error: impl std::fmt::Display) -> String {
    error.to_string()
}

pub fn print_ok(json: bool, command: &str, revision: u64, resource: Value) {
    if json {
        println!(
            "{}",
            json!({
                "ok": true,
                "command": command,
                "revision": revision,
                "resource": resource,
            })
        );
    } else if command == "session show" {
        println!(
            "{}",
            serde_json::to_string_pretty(&resource).unwrap_or_else(|_| resource.to_string())
        );
    } else if resource.is_array() || resource.is_object() && resource.get("id").is_some() {
        println!(
            "{}",
            serde_json::to_string_pretty(&resource).unwrap_or_else(|_| resource.to_string())
        );
    } else {
        println!("ok");
    }
}

pub fn print_err(json: bool, command: &str, error: &str) {
    if json {
        println!(
            "{}",
            json!({
                "ok": false,
                "command": command,
                "revision": 0,
                "error": error,
            })
        );
    }
}

pub fn stdin_unsupported(cmd: &CtlCommand) -> Option<&'static str> {
    if cmd.is_watch() {
        Some("watch is not supported on headless stdin")
    } else if cmd.is_prefs() {
        Some("prefs is not supported on headless stdin")
    } else {
        None
    }
}

pub async fn run_watch(session: &ControlSession) -> Result<(), String> {
    session.subscribe(0).await.map_err(map_err)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_typed_commands() {
        let cut = parse_line("mix cut --unit 1").unwrap();
        assert!(matches!(
            cut,
            CtlCommand::Mix(MixCmd::Cut {
                unit: 1,
                swap: true
            })
        ));
        let scene = parse_line("scene add --name Opening").unwrap();
        assert!(matches!(scene, CtlCommand::Scene(SceneCmd::Add { .. })));
        let layer = parse_line("scene layer add --scene 1 --input 2").unwrap();
        assert!(matches!(layer, CtlCommand::Scene(SceneCmd::Layer(_))));
        assert!(parse_line("mutate {\"kind\":\"deleteInput\"}").is_err());
    }

    #[test]
    fn aliases_match_mix_group() {
        let a = parse_line("cut --unit 2 --swap false").unwrap();
        let b = parse_line("mix cut --unit 2 --swap false").unwrap();
        match (a, b) {
            (
                CtlCommand::Cut {
                    unit: au,
                    swap: aswap,
                },
                CtlCommand::Mix(MixCmd::Cut {
                    unit: bu,
                    swap: bswap,
                }),
            ) => {
                assert_eq!(au, bu);
                assert_eq!(aswap, bswap);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn stdin_rejects_watch_and_prefs() {
        assert!(stdin_unsupported(&parse_line("watch").unwrap()).is_some());
        assert!(stdin_unsupported(&parse_line("prefs get bind").unwrap()).is_some());
        assert!(stdin_unsupported(&parse_line("status").unwrap()).is_none());
    }
}
