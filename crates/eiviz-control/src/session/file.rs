//! On-disk `.eivz` codec: magic + container version + Protobuf payload.
//! Save keeps the previous document in `history` (newest first, up to HISTORY_LIMIT).
//! Export writes `.eivzx` with embedded Still/Video and empty history.

use std::path::{Path, PathBuf};

use prost::Message;

use super::{
    AudioCaptureMode, AudioDeviceKind, AudioLinkMode, BandwidthSave, BezierHandles, Document,
    HeadphoneDto, InputDto, InputKind, InternalColorFormat, MixSource, MuBusDto, MultiviewDto,
    MultiviewTemplate, MvLabelAnchor, MvLabelUnit, MvSlot, MvSlotKind, NdiBandwidth, OmtQuality,
    OutputDto, OutputSourceKind, OutputTransport, OverlaySlot, RgbColor, SceneDto, SceneLayer,
    SceneLayerGeom, SceneLayoutPreset, SessionSettings, StreamEncoderDto, SwitcherSceneFilter,
    TransitionPreset, UnitDto, VideoPlayWhen, VideoTriggerWhen,
};

mod pb {
    include!(concat!(env!("OUT_DIR"), "/eiviz.session.v1.rs"));
}

pub const MAGIC: &[u8; 4] = b"EIVZ";
pub const CONTAINER_VERSION: u16 = 1;
pub const FORMAT_VERSION: u32 = 1;
pub const HISTORY_LIMIT: usize = 20;

pub fn encode_file(doc: &Document) -> Result<Vec<u8>, String> {
    encode_session(doc, &[], Vec::new())
}

pub fn decode_file(bytes: &[u8]) -> Result<Document, String> {
    decode_session(bytes, None)
}

pub fn read_document(path: impl AsRef<Path>) -> Result<Document, String> {
    let path = path.as_ref();
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    decode_session(&bytes, Some(path))
}

pub fn has_embedded_assets(path: impl AsRef<Path>) -> Result<bool, String> {
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    Ok(!decode_container(&bytes)?.assets.is_empty())
}

/// Extract embedded Still/Video into `media_dir`, rewrite those paths, and write a
/// standalone `.eivz` to `session_dest`. Headless load still extracts next to the
/// export; GUI open of `.eivzx` uses this so the operator picks both destinations.
pub fn import_exported_session(
    export_path: impl AsRef<Path>,
    session_dest: impl AsRef<Path>,
    media_dir: impl AsRef<Path>,
) -> Result<Document, String> {
    let export_path = export_path.as_ref();
    let session_dest = session_dest.as_ref();
    let media_dir = media_dir.as_ref();
    let bytes = std::fs::read(export_path).map_err(|error| error.to_string())?;
    let file = decode_container(&bytes)?;
    let Some(document) = file.document else {
        return Err("eivz file is missing a document".into());
    };
    let mut document = document_from_pb(document)?.canonicalize();
    if !file.assets.is_empty() {
        extract_assets(&mut document, &file.assets, media_dir)?;
    }
    write_document(session_dest, &document)?;
    Ok(document)
}

pub fn write_document(path: impl AsRef<Path>, doc: &Document) -> Result<u32, String> {
    write_document_rev(path, doc, 0)
}

pub fn write_document_rev(
    path: impl AsRef<Path>,
    doc: &Document,
    _revision: u64,
) -> Result<u32, String> {
    let path = path.as_ref();
    let canonical = doc.clone().canonicalize();
    let new_doc_bytes = document_to_pb(&canonical).encode_to_vec();
    let history = if path.exists() {
        let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
        let file = decode_container(&bytes)?;
        let old_doc_bytes = file
            .document
            .as_ref()
            .map(Message::encode_to_vec)
            .unwrap_or_default();
        let mut history = file.history;
        if !old_doc_bytes.is_empty() && old_doc_bytes != new_doc_bytes {
            let generation = history
                .iter()
                .map(|entry| entry.revision)
                .max()
                .unwrap_or(0)
                .saturating_add(1);
            history.insert(
                0,
                pb::HistoryEntry {
                    unix_ms: unix_ms_now(),
                    revision: generation,
                    document: old_doc_bytes,
                },
            );
            history.truncate(HISTORY_LIMIT);
        }
        history
    } else {
        Vec::new()
    };
    let count = history.len() as u32;
    let bytes = encode_session(&canonical, &[], history)?;
    atomic_write(path, &bytes)?;
    Ok(count)
}

pub fn export_document(path: impl AsRef<Path>, doc: &Document) -> Result<(), String> {
    let canonical = doc.clone().canonicalize();
    let assets = collect_assets(&canonical)?;
    let bytes = encode_session(&canonical, &assets, Vec::new())?;
    atomic_write(path.as_ref(), &bytes)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HistoryMeta {
    pub index: u32,
    pub unix_ms: u64,
    pub revision: u64,
}

pub fn read_history(path: impl AsRef<Path>) -> Result<Vec<HistoryMeta>, String> {
    let bytes = std::fs::read(path.as_ref()).map_err(|error| error.to_string())?;
    let file = decode_container(&bytes)?;
    Ok(file
        .history
        .iter()
        .enumerate()
        .map(|(index, entry)| HistoryMeta {
            index: index as u32,
            unix_ms: entry.unix_ms,
            revision: entry.revision,
        })
        .collect())
}

pub fn extract_history(path: impl AsRef<Path>, index: u32) -> Result<Document, String> {
    let bytes = std::fs::read(path.as_ref()).map_err(|error| error.to_string())?;
    let file = decode_container(&bytes)?;
    let entry = file
        .history
        .get(index as usize)
        .ok_or_else(|| format!("history index {index} is missing"))?;
    let document =
        pb::Document::decode(entry.document.as_slice()).map_err(|error| error.to_string())?;
    document_from_pb(document).map(|document| document.canonicalize())
}

fn encode_session(
    doc: &Document,
    assets: &[pb::EmbeddedAsset],
    history: Vec<pb::HistoryEntry>,
) -> Result<Vec<u8>, String> {
    let payload = pb::SessionFile {
        format_version: FORMAT_VERSION,
        document: Some(document_to_pb(doc)),
        assets: assets.to_vec(),
        history,
    }
    .encode_to_vec();
    let mut out = Vec::with_capacity(MAGIC.len() + 2 + payload.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&CONTAINER_VERSION.to_le_bytes());
    out.extend_from_slice(&payload);
    Ok(out)
}

fn decode_session(bytes: &[u8], path: Option<&Path>) -> Result<Document, String> {
    let file = decode_container(bytes)?;
    let Some(document) = file.document else {
        return Err("eivz file is missing a document".into());
    };
    let mut document = document_from_pb(document)?.canonicalize();
    if !file.assets.is_empty() {
        let Some(session_path) = path else {
            return Err("exported session needs a file path to extract media".into());
        };
        extract_assets(&mut document, &file.assets, &media_dir(session_path))?;
    }
    Ok(document)
}

fn decode_container(bytes: &[u8]) -> Result<pb::SessionFile, String> {
    if !bytes.starts_with(MAGIC) {
        return Err("not an eiviz session file".into());
    }
    if bytes.len() < MAGIC.len() + 2 {
        return Err("truncated eivz file".into());
    }
    let version = u16::from_le_bytes([bytes[4], bytes[5]]);
    if version != CONTAINER_VERSION {
        return Err(format!("unsupported eivz container version {version}"));
    }
    let file = pb::SessionFile::decode(&bytes[6..]).map_err(|error| error.to_string())?;
    if file.format_version != FORMAT_VERSION {
        return Err(format!(
            "unsupported eivz format version {}",
            file.format_version
        ));
    }
    Ok(file)
}

fn unix_ms_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn collect_assets(doc: &Document) -> Result<Vec<pb::EmbeddedAsset>, String> {
    let mut assets = Vec::new();
    for input in &doc.inputs {
        if input.kind != InputKind::Still && input.kind != InputKind::Video {
            continue;
        }
        let Some(path) = input.path_or_address.as_deref() else {
            return Err(format!("input {} is missing a file path", input.id));
        };
        let source = Path::new(path);
        if !source.is_file() {
            return Err(format!("input {} file does not exist: {path}", input.id));
        }
        let file_name = source
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .ok_or_else(|| format!("input {} has an invalid file name", input.id))?
            .to_string();
        let data = std::fs::read(source).map_err(|error| error.to_string())?;
        assets.push(pb::EmbeddedAsset {
            input_id: input.id,
            file_name,
            data,
        });
    }
    Ok(assets)
}

fn extract_assets(
    doc: &mut Document,
    assets: &[pb::EmbeddedAsset],
    dir: &Path,
) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    for asset in assets {
        if asset.file_name.is_empty()
            || asset.file_name.contains('/')
            || asset.file_name.contains('\\')
            || asset.file_name.contains("..")
        {
            return Err(format!("asset {} has an invalid file name", asset.input_id));
        }
        let dest = dir.join(format!("{}_{}", asset.input_id, asset.file_name));
        std::fs::write(&dest, &asset.data).map_err(|error| error.to_string())?;
        let Some(input) = doc
            .inputs
            .iter_mut()
            .find(|input| input.id == asset.input_id)
        else {
            return Err(format!("asset references missing input {}", asset.input_id));
        };
        input.path_or_address = Some(dest.to_string_lossy().into_owned());
    }
    Ok(())
}

fn media_dir(session_path: &Path) -> PathBuf {
    let stem = session_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("session");
    match session_path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join(format!("{stem}.media")),
        _ => PathBuf::from(format!("{stem}.media")),
    }
}

/// Writes `bytes` next to `path`, syncs them to disk, then renames over the target so a crash
/// mid-save can never leave a truncated or missing file. `rename` replaces an existing target
/// on both Unix and Windows, so the previous file is never removed first.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;

    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
    }
    let tmp = tmp_path(path);
    let write = || -> std::io::Result<()> {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)
    };
    if let Err(error) = write() {
        let _ = std::fs::remove_file(&tmp);
        return Err(error.to_string());
    }
    sync_parent_dir(path);
    Ok(())
}

#[cfg(unix)]
fn sync_parent_dir(path: &Path) {
    let dir = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    if let Ok(dir) = std::fs::File::open(dir) {
        let _ = dir.sync_all();
    }
}

#[cfg(not(unix))]
fn sync_parent_dir(_path: &Path) {}

fn tmp_path(path: &Path) -> PathBuf {
    let mut tmp = path.as_os_str().to_os_string();
    tmp.push(".tmp");
    PathBuf::from(tmp)
}

fn document_to_pb(doc: &Document) -> pb::Document {
    pb::Document {
        version: doc.version,
        scene_presets: doc.scene_presets.iter().map(preset_to_pb).collect(),
        input_tags: doc.input_tags.clone(),
        scene_tags: doc.scene_tags.clone(),
        replay_tags: doc.replay_tags.clone(),
        settings: Some(settings_to_pb(&doc.settings)),
        inputs: doc.inputs.iter().map(input_to_pb).collect(),
        scenes: doc.scenes.iter().map(scene_to_pb).collect(),
        units: doc.units.iter().map(unit_to_pb).collect(),
        transitions: doc.transitions.iter().map(transition_to_pb).collect(),
        overlays: doc.overlays.iter().map(overlay_to_pb).collect(),
        next_overlay_id: doc.next_overlay_id,
        stream_encoders: doc
            .stream_encoders
            .iter()
            .map(stream_encoder_to_pb)
            .collect(),
        next_stream_encoder_id: doc.next_stream_encoder_id,
        outputs: doc.outputs.iter().map(output_to_pb).collect(),
        multiviews: doc.multiviews.iter().map(multiview_to_pb).collect(),
        next_input_id: doc.next_input_id,
        next_scene_id: doc.next_scene_id,
        next_unit_id: doc.next_unit_id,
        next_output_id: doc.next_output_id,
        next_multiview_id: doc.next_multiview_id,
        selected_unit_id: doc.selected_unit_id,
        headphone_copy_monitor: doc.headphone_copy_monitor,
        headphone: Some(headphone_to_pb(&doc.headphone)),
        headphone_listen_kind: doc.headphone_listen_kind,
        headphone_listen_id: doc.headphone_listen_id,
        replay: doc.replay.as_ref().map(replay_to_pb),
    }
}

fn document_from_pb(doc: pb::Document) -> Result<Document, String> {
    Ok(Document {
        version: doc.version,
        scene_presets: doc.scene_presets.into_iter().map(preset_from_pb).collect(),
        input_tags: doc.input_tags,
        scene_tags: doc.scene_tags,
        replay_tags: doc.replay_tags,
        settings: settings_from_pb(doc.settings.unwrap_or_default())?,
        inputs: doc
            .inputs
            .into_iter()
            .map(input_from_pb)
            .collect::<Result<_, _>>()?,
        scenes: doc.scenes.into_iter().map(scene_from_pb).collect(),
        units: doc
            .units
            .into_iter()
            .map(unit_from_pb)
            .collect::<Result<_, _>>()?,
        transitions: doc
            .transitions
            .into_iter()
            .map(transition_from_pb)
            .collect(),
        overlays: doc.overlays.into_iter().map(overlay_from_pb).collect(),
        next_overlay_id: doc.next_overlay_id,
        stream_encoders: doc
            .stream_encoders
            .into_iter()
            .map(stream_encoder_from_pb)
            .collect::<Result<_, _>>()?,
        next_stream_encoder_id: doc.next_stream_encoder_id,
        outputs: doc
            .outputs
            .into_iter()
            .map(output_from_pb)
            .collect::<Result<_, _>>()?,
        multiviews: doc
            .multiviews
            .into_iter()
            .map(multiview_from_pb)
            .collect::<Result<_, _>>()?,
        next_input_id: doc.next_input_id,
        next_scene_id: doc.next_scene_id,
        next_unit_id: doc.next_unit_id,
        next_output_id: doc.next_output_id,
        next_multiview_id: doc.next_multiview_id,
        selected_unit_id: doc.selected_unit_id,
        headphone_copy_monitor: doc.headphone_copy_monitor,
        headphone_listen_kind: doc.headphone_listen_kind,
        headphone_listen_id: doc.headphone_listen_id,
        headphone: doc
            .headphone
            .map(headphone_from_pb)
            .transpose()?
            .unwrap_or_default(),
        replay: doc.replay.map(replay_from_pb).transpose()?,
    })
}

fn settings_to_pb(settings: &SessionSettings) -> pb::SessionSettings {
    pb::SessionSettings {
        master_fps_num: settings.master_fps_num,
        master_fps_den: settings.master_fps_den,
        default_width: settings.default_width,
        default_height: settings.default_height,
        theme: settings.theme.clone(),
        default_multiview_unit_id: settings.default_multiview_unit_id,
        frame_buffer_frames: settings.frame_buffer_frames,
        default_present_interval: settings.default_present_interval,
        replay_present_interval: settings.replay_present_interval,
        flip_swapchain_limit: settings.flip_swapchain_limit,
        internal_color_format: color_format_to_pb(settings.internal_color_format).into(),
        rebar_optimization: settings.rebar_optimization,
        rebar_direct_sample: settings.rebar_direct_sample,
        ndi_gpu_upload: settings.ndi_gpu_upload,
        preview_color: Some(rgb_to_pb(settings.preview_color)),
        program_color: Some(rgb_to_pb(settings.program_color)),
        inactive_color: Some(rgb_to_pb(settings.inactive_color)),
        multiview_label_size: settings.multiview_label_size,
        multiview_label_unit: label_unit_to_pb(settings.multiview_label_unit).into(),
        multiview_label_anchor: label_anchor_to_pb(settings.multiview_label_anchor).into(),
        vmix_api_enabled: settings.vmix_api_enabled,
        vmix_api_port: settings.vmix_api_port,
        vmix_api_user: settings.vmix_api_user.clone(),
        vmix_api_password: settings.vmix_api_password.clone(),
        vmix_tcp_enabled: settings.vmix_tcp_enabled,
        native_api_enabled: settings.native_api_enabled,
        native_api_port: settings.native_api_port,
    }
}

fn settings_from_pb(settings: pb::SessionSettings) -> Result<SessionSettings, String> {
    let mut out = SessionSettings::default();
    if settings.master_fps_num != 0 {
        out.master_fps_num = settings.master_fps_num;
    }
    if settings.master_fps_den != 0 {
        out.master_fps_den = settings.master_fps_den;
    }
    if settings.default_width != 0 {
        out.default_width = settings.default_width;
    }
    if settings.default_height != 0 {
        out.default_height = settings.default_height;
    }
    if !settings.theme.is_empty() {
        out.theme = settings.theme;
    }
    if settings.default_multiview_unit_id != 0 {
        out.default_multiview_unit_id = settings.default_multiview_unit_id;
    }
    if settings.frame_buffer_frames != 0 {
        out.frame_buffer_frames = settings.frame_buffer_frames;
    }
    if settings.default_present_interval != 0 {
        out.default_present_interval = settings.default_present_interval;
    }
    if settings.replay_present_interval != 0 {
        out.replay_present_interval = settings.replay_present_interval;
    }
    out.flip_swapchain_limit = settings.flip_swapchain_limit;
    out.internal_color_format = color_format_from_pb(settings.internal_color_format)?;
    out.rebar_optimization = settings.rebar_optimization;
    out.rebar_direct_sample = settings.rebar_direct_sample;
    out.ndi_gpu_upload = settings.ndi_gpu_upload;
    if let Some(color) = settings.preview_color {
        out.preview_color = rgb_from_pb(color)?;
    }
    if let Some(color) = settings.program_color {
        out.program_color = rgb_from_pb(color)?;
    }
    if let Some(color) = settings.inactive_color {
        out.inactive_color = rgb_from_pb(color)?;
    }
    if settings.multiview_label_size != 0.0 {
        out.multiview_label_size = settings.multiview_label_size;
    }
    out.multiview_label_unit = label_unit_from_pb(settings.multiview_label_unit)?;
    out.multiview_label_anchor = label_anchor_from_pb(settings.multiview_label_anchor)?;
    out.vmix_api_enabled = settings.vmix_api_enabled;
    if settings.vmix_api_port != 0 {
        out.vmix_api_port = settings.vmix_api_port;
    }
    out.vmix_api_user = settings.vmix_api_user;
    out.vmix_api_password = settings.vmix_api_password;
    out.vmix_tcp_enabled = settings.vmix_tcp_enabled;
    out.native_api_enabled = settings.native_api_enabled;
    if settings.native_api_port != 0 {
        out.native_api_port = settings.native_api_port;
    }
    out.renderer = super::Renderer::Auto;
    out.last_session_path = None;
    Ok(out)
}

fn rgb_to_pb(color: RgbColor) -> pb::RgbColor {
    pb::RgbColor {
        r: u32::from(color.r),
        g: u32::from(color.g),
        b: u32::from(color.b),
    }
}

fn rgb_from_pb(color: pb::RgbColor) -> Result<RgbColor, String> {
    fn channel(value: u32, name: &str) -> Result<u8, String> {
        u8::try_from(value).map_err(|_| format!("{name} color channel {value} is out of range"))
    }
    Ok(RgbColor {
        r: channel(color.r, "r")?,
        g: channel(color.g, "g")?,
        b: channel(color.b, "b")?,
    })
}

fn input_to_pb(input: &InputDto) -> pb::Input {
    pb::Input {
        id: input.id,
        guid: input.guid.clone(),
        name: input.name.clone(),
        kind: input_kind_to_pb(input.kind).into(),
        path_or_address: input.path_or_address.clone(),
        color_r: input.color_r,
        color_g: input.color_g,
        color_b: input.color_b,
        scroll: input.scroll,
        tone_hz: input.tone_hz,
        tone_level_dbfs: input.tone_level_dbfs,
        audio_units: input.audio_units.clone(),
        gain: input.gain,
        mute: input.mute,
        use_gpu: input.use_gpu,
        frame_buffer_frames: input.frame_buffer_frames,
        bandwidth_save: bandwidth_to_pb(input.bandwidth_save).into(),
        keep_full_on_multiview: input.keep_full_on_multiview,
        omt_quality: omt_quality_to_pb(input.omt_quality).into(),
        ndi_bandwidth: ndi_bandwidth_to_pb(input.ndi_bandwidth).into(),
        video_loop: input.video_loop,
        video_play_when: play_when_to_pb(input.video_play_when).into(),
        video_restart_when: trigger_when_to_pb(input.video_restart_when).into(),
        video_pause_when: trigger_when_to_pb(input.video_pause_when).into(),
        capture_width: input.capture_width,
        capture_height: input.capture_height,
        capture_fps_num: input.capture_fps_num,
        capture_fps_den: input.capture_fps_den,
        tags: input.tags.clone(),
        mix_source: mix_source_to_pb(input.mix_source).into(),
        mix_target_id: input.mix_target_id,
        audio_capture_mode: audio_capture_mode_to_pb(input.audio_capture_mode).into(),
        audio_device_kind: device_kind_to_pb(input.audio_device_kind).into(),
        audio_device_id: input.audio_device_id.clone(),
        audio_map_left: input.audio_map_left,
        audio_map_right: input.audio_map_right,
        audio_process_exe: input.audio_process_exe.clone(),
        audio_process_aumid: input.audio_process_aumid.clone(),
        decklink_mode: input.decklink_mode.clone(),
    }
}

fn input_from_pb(input: pb::Input) -> Result<InputDto, String> {
    Ok(InputDto {
        id: input.id,
        guid: input.guid,
        name: input.name,
        kind: input_kind_from_pb(input.kind)?,
        path_or_address: input.path_or_address,
        color_r: input.color_r,
        color_g: input.color_g,
        color_b: input.color_b,
        scroll: input.scroll,
        tone_hz: input.tone_hz,
        tone_level_dbfs: if input.tone_level_dbfs == 0.0 {
            -20.0
        } else {
            input.tone_level_dbfs
        },
        audio_units: input.audio_units,
        gain: input.gain,
        mute: input.mute,
        use_gpu: input.use_gpu,
        frame_buffer_frames: input.frame_buffer_frames,
        bandwidth_save: bandwidth_from_pb(input.bandwidth_save)?,
        keep_full_on_multiview: input.keep_full_on_multiview,
        omt_quality: omt_quality_from_pb(input.omt_quality)?,
        ndi_bandwidth: ndi_bandwidth_from_pb(input.ndi_bandwidth)?,
        video_loop: input.video_loop,
        video_play_when: play_when_from_pb(input.video_play_when)?,
        video_restart_when: trigger_when_from_pb(input.video_restart_when)?,
        video_pause_when: trigger_when_from_pb(input.video_pause_when)?,
        capture_width: input.capture_width,
        capture_height: input.capture_height,
        capture_fps_num: input.capture_fps_num,
        capture_fps_den: input.capture_fps_den,
        tags: input.tags,
        mix_source: mix_source_from_pb(input.mix_source)?,
        mix_target_id: input.mix_target_id,
        audio_capture_mode: audio_capture_mode_from_pb(input.audio_capture_mode)?,
        audio_device_kind: device_kind_from_pb(input.audio_device_kind)?,
        audio_device_id: input.audio_device_id,
        audio_map_left: input.audio_map_left,
        audio_map_right: input.audio_map_right,
        audio_process_exe: input.audio_process_exe,
        audio_process_aumid: input.audio_process_aumid,
        decklink_mode: input.decklink_mode,
    })
}

fn scene_to_pb(scene: &SceneDto) -> pb::Scene {
    pb::Scene {
        id: scene.id,
        guid: scene.guid.clone(),
        name: scene.name.clone(),
        layers: scene.layers.iter().map(layer_to_pb).collect(),
        tags: scene.tags.clone(),
        preview_collapsed: scene.preview_collapsed,
        states: scene.states.iter().map(state_to_pb).collect(),
        sequences: scene.sequences.iter().map(sequence_to_pb).collect(),
        camera: Some(camera_to_pb(&scene.camera)),
    }
}

fn scene_from_pb(scene: pb::Scene) -> SceneDto {
    SceneDto {
        id: scene.id,
        guid: scene.guid,
        name: scene.name,
        layers: scene.layers.into_iter().map(layer_from_pb).collect(),
        tags: scene.tags,
        preview_collapsed: scene.preview_collapsed,
        states: scene.states.into_iter().map(state_from_pb).collect(),
        sequences: scene.sequences.into_iter().map(sequence_from_pb).collect(),
        camera: scene.camera.map(camera_from_pb).unwrap_or_default(),
    }
}

fn layer_to_pb(layer: &SceneLayer) -> pb::SceneLayer {
    pb::SceneLayer {
        input_id: layer.input_id,
        x: layer.x,
        y: layer.y,
        width: layer.width,
        height: layer.height,
        opacity: layer.opacity,
        z: layer.z,
        audio_follow: layer.audio_follow,
        locked: layer.locked,
        size_linked: layer.size_linked,
        crop_x: layer.crop_x,
        crop_y: layer.crop_y,
        crop_width: layer.crop_width,
        crop_height: layer.crop_height,
        hidden: layer.hidden,
        layer_id: layer.layer_id,
    }
}

fn layer_from_pb(layer: pb::SceneLayer) -> SceneLayer {
    SceneLayer {
        input_id: layer.input_id,
        x: layer.x,
        y: layer.y,
        width: if layer.width == 0.0 { 1.0 } else { layer.width },
        height: if layer.height == 0.0 {
            1.0
        } else {
            layer.height
        },
        opacity: if layer.opacity == 0.0 {
            1.0
        } else {
            layer.opacity
        },
        z: layer.z,
        audio_follow: layer.audio_follow,
        locked: layer.locked,
        size_linked: layer.size_linked,
        crop_x: layer.crop_x,
        crop_y: layer.crop_y,
        crop_width: layer.crop_width,
        crop_height: layer.crop_height,
        hidden: layer.hidden,
        layer_id: layer.layer_id,
    }
}

fn motion_to_pb(motion: &super::Motion) -> pb::Motion {
    pb::Motion {
        duration_frames: motion.duration_frames,
        easing: motion.easing,
        bezier: motion.bezier.map(bezier_to_pb),
    }
}

fn motion_from_pb(motion: pb::Motion) -> super::Motion {
    super::Motion {
        duration_frames: if motion.duration_frames == 0 {
            15
        } else {
            motion.duration_frames
        },
        easing: motion.easing,
        bezier: motion.bezier.map(bezier_from_pb),
    }
}

fn state_to_pb(state: &super::SceneState) -> pb::SceneState {
    pb::SceneState {
        id: state.id,
        name: state.name.clone(),
        layers: state
            .layers
            .iter()
            .map(|key| pb::LayerKey {
                layer_id: key.layer_id,
                geom: Some(geom_to_pb(&key.geom)),
            })
            .collect(),
        enter: Some(motion_to_pb(&state.enter)),
        camera: state.camera.as_ref().map(camera_to_pb),
    }
}

fn state_from_pb(state: pb::SceneState) -> super::SceneState {
    super::SceneState {
        id: state.id,
        name: state.name,
        layers: state
            .layers
            .into_iter()
            .map(|key| super::LayerKey {
                layer_id: key.layer_id,
                geom: geom_from_pb(key.geom.unwrap_or_default()),
            })
            .collect(),
        enter: state.enter.map(motion_from_pb).unwrap_or_default(),
        camera: state.camera.map(camera_from_pb),
    }
}

fn camera_to_pb(camera: &super::SceneCamera) -> pb::SceneCamera {
    pb::SceneCamera {
        x: camera.x,
        y: camera.y,
        zoom: camera.zoom,
    }
}

fn camera_from_pb(camera: pb::SceneCamera) -> super::SceneCamera {
    if camera.zoom == 0.0 {
        return super::SceneCamera::default();
    }
    super::SceneCamera {
        x: camera.x,
        y: camera.y,
        zoom: camera.zoom,
    }
}

fn sequence_to_pb(seq: &super::SceneSequence) -> pb::SceneSequence {
    pb::SceneSequence {
        id: seq.id,
        name: seq.name.clone(),
        steps: seq
            .steps
            .iter()
            .map(|step| pb::SequenceStep {
                state_id: step.state_id,
                motion: step.motion.as_ref().map(motion_to_pb),
                hold_frames: step.hold_frames,
                replay: step.replay.as_ref().map(|action| pb::SequenceReplayAction {
                    channel: action.channel,
                    clip_uid: action.clip_uid.clone(),
                    play: action.play,
                }),
            })
            .collect(),
    }
}

fn sequence_from_pb(seq: pb::SceneSequence) -> super::SceneSequence {
    super::SceneSequence {
        id: seq.id,
        name: seq.name,
        steps: seq
            .steps
            .into_iter()
            .map(|step| super::SequenceStep {
                state_id: step.state_id,
                motion: step.motion.map(motion_from_pb),
                hold_frames: step.hold_frames,
                replay: step.replay.map(|action| super::SequenceReplayAction {
                    channel: action.channel,
                    clip_uid: action.clip_uid,
                    play: action.play,
                }),
            })
            .collect(),
    }
}

fn preset_to_pb(preset: &SceneLayoutPreset) -> pb::SceneLayoutPreset {
    pb::SceneLayoutPreset {
        name: preset.name.clone(),
        layers: preset.layers.iter().map(geom_to_pb).collect(),
    }
}

fn preset_from_pb(preset: pb::SceneLayoutPreset) -> SceneLayoutPreset {
    SceneLayoutPreset {
        name: preset.name,
        layers: preset.layers.into_iter().map(geom_from_pb).collect(),
    }
}

fn geom_to_pb(layer: &SceneLayerGeom) -> pb::SceneLayerGeom {
    pb::SceneLayerGeom {
        x: layer.x,
        y: layer.y,
        width: layer.width,
        height: layer.height,
        opacity: layer.opacity,
        z: layer.z,
        crop_x: layer.crop_x,
        crop_y: layer.crop_y,
        crop_width: layer.crop_width,
        crop_height: layer.crop_height,
    }
}

fn geom_from_pb(layer: pb::SceneLayerGeom) -> SceneLayerGeom {
    SceneLayerGeom {
        x: layer.x,
        y: layer.y,
        width: if layer.width == 0.0 { 1.0 } else { layer.width },
        height: if layer.height == 0.0 {
            1.0
        } else {
            layer.height
        },
        opacity: if layer.opacity == 0.0 {
            1.0
        } else {
            layer.opacity
        },
        z: layer.z,
        crop_x: layer.crop_x,
        crop_y: layer.crop_y,
        crop_width: layer.crop_width,
        crop_height: layer.crop_height,
    }
}

fn unit_to_pb(unit: &UnitDto) -> pb::MixingUnit {
    pb::MixingUnit {
        id: unit.id,
        name: unit.name.clone(),
        width: unit.width,
        height: unit.height,
        fps_num: unit.fps_num,
        fps_den: unit.fps_den,
        overlays_on_air: unit.overlays_on_air.clone(),
        audio: Some(mu_bus_to_pb(&unit.audio)),
        audio_link: audio_link_to_pb(unit.audio_link).into(),
        switcher_scene_filter: filter_to_pb(unit.switcher_scene_filter).into(),
        switcher_scene_ids: unit.switcher_scene_ids.clone(),
        always_on_top: unit.always_on_top,
        preview_scene_id: unit.preview_scene_id,
        program_scene_id: unit.program_scene_id,
    }
}

fn unit_from_pb(unit: pb::MixingUnit) -> Result<UnitDto, String> {
    Ok(UnitDto {
        id: unit.id,
        name: unit.name,
        width: unit.width,
        height: unit.height,
        fps_num: unit.fps_num,
        fps_den: unit.fps_den,
        overlays_on_air: unit.overlays_on_air,
        audio: unit
            .audio
            .map(mu_bus_from_pb)
            .transpose()?
            .unwrap_or_default(),
        audio_link: audio_link_from_pb(unit.audio_link)?,
        switcher_scene_filter: filter_from_pb(unit.switcher_scene_filter)?,
        switcher_scene_ids: unit.switcher_scene_ids,
        always_on_top: unit.always_on_top,
        preview_scene_id: unit.preview_scene_id,
        program_scene_id: unit.program_scene_id,
    })
}

fn transition_to_pb(preset: &TransitionPreset) -> pb::TransitionPreset {
    pb::TransitionPreset {
        kind: preset.kind,
        duration_value: preset.duration_value,
        duration_unit: preset.duration_unit,
        swap: preset.swap,
        keep_preview: preset.keep_preview,
        easing: preset.easing,
        direction: preset.direction,
        dip_r: preset.dip_r,
        dip_g: preset.dip_g,
        dip_b: preset.dip_b,
        dip_a: preset.dip_a,
        softness: preset.softness,
        param: preset.param,
        custom_wgsl: preset.custom_wgsl.clone(),
        label: preset.label.clone(),
        bezier: preset.bezier.map(bezier_to_pb),
    }
}

fn transition_from_pb(preset: pb::TransitionPreset) -> TransitionPreset {
    TransitionPreset {
        kind: preset.kind,
        duration_value: preset.duration_value,
        duration_unit: preset.duration_unit,
        swap: preset.swap,
        keep_preview: preset.keep_preview,
        easing: preset.easing,
        direction: preset.direction,
        dip_r: preset.dip_r,
        dip_g: preset.dip_g,
        dip_b: preset.dip_b,
        dip_a: if preset.dip_a == 0.0 {
            1.0
        } else {
            preset.dip_a
        },
        softness: if preset.softness == 0.0 {
            0.02
        } else {
            preset.softness
        },
        param: preset.param,
        custom_wgsl: preset.custom_wgsl,
        label: preset.label,
        bezier: preset.bezier.map(bezier_from_pb),
    }
}

fn bezier_to_pb(handles: BezierHandles) -> pb::BezierHandles {
    pb::BezierHandles {
        x1: handles.x1,
        y1: handles.y1,
        x2: handles.x2,
        y2: handles.y2,
    }
}

fn bezier_from_pb(handles: pb::BezierHandles) -> BezierHandles {
    BezierHandles {
        x1: handles.x1,
        y1: handles.y1,
        x2: handles.x2,
        y2: handles.y2,
    }
}

fn overlay_to_pb(slot: &OverlaySlot) -> pb::OverlaySlot {
    pb::OverlaySlot {
        scene_gpu_id: slot.scene_gpu_id,
        x: slot.x,
        y: slot.y,
        width: slot.width,
        height: slot.height,
        opacity: slot.opacity,
        z: slot.z,
        id: slot.id,
        transition_kind: slot.transition_kind,
        duration_value: slot.duration_value,
        duration_unit: slot.duration_unit,
        easing: slot.easing,
        bezier: slot.bezier.map(bezier_to_pb),
        audio_follow: slot.audio_follow,
        source_kind: slot.source_kind,
        locked: slot.locked,
        hidden: slot.hidden,
        size_linked: slot.size_linked,
        crop_x: slot.crop_x,
        crop_y: slot.crop_y,
        crop_width: slot.crop_width,
        crop_height: slot.crop_height,
    }
}

fn overlay_from_pb(slot: pb::OverlaySlot) -> OverlaySlot {
    OverlaySlot {
        id: slot.id,
        scene_gpu_id: slot.scene_gpu_id,
        x: slot.x,
        y: slot.y,
        width: slot.width,
        height: slot.height,
        opacity: slot.opacity,
        z: slot.z,
        transition_kind: slot.transition_kind,
        duration_value: slot.duration_value,
        duration_unit: slot.duration_unit,
        easing: slot.easing,
        bezier: slot.bezier.map(bezier_from_pb),
        audio_follow: slot.audio_follow,
        source_kind: slot.source_kind,
        locked: slot.locked,
        hidden: slot.hidden,
        size_linked: slot.size_linked,
        crop_x: slot.crop_x,
        crop_y: slot.crop_y,
        crop_width: slot.crop_width,
        crop_height: slot.crop_height,
    }
}

fn output_to_pb(output: &OutputDto) -> pb::Output {
    pb::Output {
        id: output.id,
        name: output.name.clone(),
        transport: transport_to_pb(output.transport).into(),
        source_kind: source_kind_to_pb(output.source_kind).into(),
        source_id: output.source_id,
        unit_id: output.unit_id,
        use_gpu: output.use_gpu,
        enabled: output.enabled,
        audio_unit_id: output.audio_unit_id,
        skip_encode_when_no_receivers: output.skip_encode_when_no_receivers,
        width: output.width,
        height: output.height,
        fps_num: output.fps_num,
        fps_den: output.fps_den,
        decklink_device: output.decklink_device.clone(),
        decklink_mode: output.decklink_mode.clone(),
        decklink_external_key: output.decklink_external_key,
        rtmp_url: output.rtmp_url.clone(),
        rtmp_secret_ref: output.rtmp_secret_ref.clone(),
        stream_encoder_id: output.stream_encoder_id,
    }
}

fn stream_encoder_to_pb(encoder: &StreamEncoderDto) -> pb::StreamEncoder {
    pb::StreamEncoder {
        id: encoder.id,
        name: encoder.name.clone(),
        source_kind: source_kind_to_pb(encoder.source_kind).into(),
        source_id: encoder.source_id,
        unit_id: encoder.unit_id,
        audio_unit_id: encoder.audio_unit_id,
        width: encoder.width,
        height: encoder.height,
        fps_num: encoder.fps_num,
        fps_den: encoder.fps_den,
        video_bitrate: encoder.video_bitrate,
        audio_bitrate: encoder.audio_bitrate,
        keyint: encoder.keyint,
        video_only: encoder.video_only,
        encoder_threads: encoder.encoder_threads,
        buffer_ms: encoder.buffer_ms,
        encoder: encoder.encoder.clone(),
        complexity: encoder.complexity,
        ref_frames: encoder.ref_frames,
        cabac: encoder.cabac,
        deblock: Some(encoder.deblock),
        background_detection: Some(encoder.background_detection),
        adaptive_quant: encoder.adaptive_quant,
        scene_change: encoder.scene_change,
        audio_delay_ms: encoder.audio_delay_ms,
    }
}

fn stream_encoder_from_pb(encoder: pb::StreamEncoder) -> Result<StreamEncoderDto, String> {
    Ok(StreamEncoderDto {
        id: encoder.id,
        name: encoder.name,
        source_kind: source_kind_from_pb(encoder.source_kind)?,
        source_id: encoder.source_id,
        unit_id: encoder.unit_id,
        audio_unit_id: encoder.audio_unit_id,
        width: encoder.width,
        height: encoder.height,
        fps_num: encoder.fps_num,
        fps_den: encoder.fps_den,
        video_bitrate: encoder.video_bitrate,
        audio_bitrate: encoder.audio_bitrate,
        keyint: encoder.keyint,
        video_only: encoder.video_only,
        encoder_threads: encoder.encoder_threads,
        buffer_ms: encoder.buffer_ms,
        encoder: if encoder.encoder.is_empty() {
            super::default_openh264()
        } else {
            encoder.encoder
        },
        complexity: encoder.complexity,
        ref_frames: if encoder.ref_frames == 0 {
            1
        } else {
            encoder.ref_frames
        },
        cabac: encoder.cabac,
        deblock: encoder.deblock.unwrap_or(true),
        background_detection: encoder.background_detection.unwrap_or(true),
        adaptive_quant: encoder.adaptive_quant,
        scene_change: encoder.scene_change,
        audio_delay_ms: encoder.audio_delay_ms,
    })
}

fn output_from_pb(output: pb::Output) -> Result<OutputDto, String> {
    Ok(OutputDto {
        id: output.id,
        name: output.name,
        transport: transport_from_pb(output.transport)?,
        source_kind: source_kind_from_pb(output.source_kind)?,
        source_id: output.source_id,
        unit_id: output.unit_id,
        use_gpu: output.use_gpu,
        enabled: output.enabled,
        audio_unit_id: output.audio_unit_id,
        skip_encode_when_no_receivers: output.skip_encode_when_no_receivers,
        width: output.width,
        height: output.height,
        fps_num: output.fps_num,
        fps_den: output.fps_den,
        decklink_device: output.decklink_device,
        decklink_mode: output.decklink_mode,
        decklink_external_key: output.decklink_external_key,
        rtmp_url: output.rtmp_url,
        rtmp_secret_ref: output.rtmp_secret_ref,
        stream_encoder_id: output.stream_encoder_id,
    })
}

fn multiview_to_pb(layout: &MultiviewDto) -> pb::Multiview {
    pb::Multiview {
        id: layout.id,
        name: layout.name.clone(),
        preview_unit_id: layout.preview_unit_id,
        program_unit_id: layout.program_unit_id,
        present_interval: layout.present_interval,
        tiles: layout.tiles.iter().map(mv_slot_to_pb).collect(),
        template: template_to_pb(layout.template).into(),
        preview_label_follow: layout.preview_label_follow,
        preview_label: layout.preview_label.clone(),
        program_label_follow: layout.program_label_follow,
        program_label: layout.program_label.clone(),
        label_anchor: layout
            .label_anchor
            .map(|value| label_anchor_to_pb(value).into()),
        label_size: layout.label_size,
        label_unit: layout
            .label_unit
            .map(|value| label_unit_to_pb(value).into()),
        always_on_top: layout.always_on_top,
    }
}

fn multiview_from_pb(layout: pb::Multiview) -> Result<MultiviewDto, String> {
    Ok(MultiviewDto {
        id: layout.id,
        name: layout.name,
        preview_unit_id: layout.preview_unit_id,
        program_unit_id: layout.program_unit_id,
        present_interval: layout.present_interval,
        tiles: layout
            .tiles
            .into_iter()
            .map(mv_slot_from_pb)
            .collect::<Result<_, _>>()?,
        template: template_from_pb(layout.template)?,
        preview_label_follow: layout.preview_label_follow,
        preview_label: layout.preview_label,
        program_label_follow: layout.program_label_follow,
        program_label: layout.program_label,
        label_anchor: match layout.label_anchor {
            Some(value) => Some(label_anchor_from_pb(value)?),
            None => None,
        },
        label_size: layout.label_size,
        label_unit: match layout.label_unit {
            Some(value) => Some(label_unit_from_pb(value)?),
            None => None,
        },
        always_on_top: layout.always_on_top,
    })
}

fn mv_slot_to_pb(slot: &MvSlot) -> pb::MvSlot {
    pb::MvSlot {
        kind: mv_kind_to_pb(slot.kind).into(),
        source_id: slot.source_id,
        label_follow: slot.label_follow,
        label: slot.label.clone(),
    }
}

fn mv_slot_from_pb(slot: pb::MvSlot) -> Result<MvSlot, String> {
    Ok(MvSlot {
        kind: mv_kind_from_pb(slot.kind)?,
        source_id: slot.source_id,
        label_follow: slot.label_follow,
        label: slot.label,
    })
}

fn mu_bus_to_pb(bus: &MuBusDto) -> pb::MuBus {
    pb::MuBus {
        device_kind: device_kind_to_pb(bus.device_kind).into(),
        device_id: bus.device_id.clone(),
        map_left: bus.map_left,
        map_right: bus.map_right,
        gain: bus.gain,
        mute: bus.mute,
    }
}

fn mu_bus_from_pb(bus: pb::MuBus) -> Result<MuBusDto, String> {
    Ok(MuBusDto {
        device_kind: device_kind_from_pb(bus.device_kind)?,
        device_id: bus.device_id,
        map_left: bus.map_left,
        map_right: bus.map_right,
        gain: if bus.gain == 0.0 { 1.0 } else { bus.gain },
        mute: bus.mute,
    })
}

fn headphone_to_pb(bus: &HeadphoneDto) -> pb::Headphone {
    pb::Headphone {
        device_kind: device_kind_to_pb(bus.device_kind).into(),
        device_id: bus.device_id.clone(),
        map_left: bus.map_left,
        map_right: bus.map_right,
    }
}

fn headphone_from_pb(bus: pb::Headphone) -> Result<HeadphoneDto, String> {
    Ok(HeadphoneDto {
        device_kind: device_kind_from_pb(bus.device_kind)?,
        device_id: bus.device_id,
        map_left: bus.map_left,
        map_right: bus.map_right,
    })
}

fn unknown(kind: &str, value: i32) -> String {
    format!("unknown {kind} {value}")
}

macro_rules! proto_enum {
    ($to:ident, $from:ident, $domain:ty, $wire:ty, $name:literal, { $($d:ident => $w:ident),+ $(,)? }) => {
        fn $to(value: $domain) -> $wire {
            match value {
                $(<$domain>::$d => <$wire>::$w,)+
            }
        }
        fn $from(value: i32) -> Result<$domain, String> {
            match <$wire>::try_from(value) {
                Ok(<$wire>::Unspecified) => Ok(<$domain>::default()),
                $(Ok(<$wire>::$w) => Ok(<$domain>::$d),)+
                Err(_) => Err(unknown($name, value)),
            }
        }
    };
}

proto_enum!(
    color_format_to_pb,
    color_format_from_pb,
    InternalColorFormat,
    pb::InternalColorFormat,
    "internal color format",
    { Uyvy => Uyvy, Bgra => Bgra }
);
proto_enum!(
    label_unit_to_pb,
    label_unit_from_pb,
    MvLabelUnit,
    pb::MvLabelUnit,
    "multiview label unit",
    { Px => Px, Percent => Percent }
);
proto_enum!(
    label_anchor_to_pb,
    label_anchor_from_pb,
    MvLabelAnchor,
    pb::MvLabelAnchor,
    "multiview label anchor",
    { Top => Top, Bottom => Bottom }
);
proto_enum!(
    mix_source_to_pb,
    mix_source_from_pb,
    MixSource,
    pb::MixSource,
    "mix source",
    { MuPreview => MuPreview, MuProgram => MuProgram, SessionMultiview => SessionMultiview }
);
proto_enum!(
    bandwidth_to_pb,
    bandwidth_from_pb,
    BandwidthSave,
    pb::BandwidthSave,
    "bandwidth save",
    {
        AlwaysLow => AlwaysLow,
        NotOnProgram => NotOnProgram,
        NotOnPreviewOrProgram => NotOnPreviewOrProgram,
        AlwaysFull => AlwaysFull
    }
);
proto_enum!(
    omt_quality_to_pb,
    omt_quality_from_pb,
    OmtQuality,
    pb::OmtQuality,
    "omt quality",
    { Default => Default, Low => Low, Medium => Medium, High => High }
);
proto_enum!(
    ndi_bandwidth_to_pb,
    ndi_bandwidth_from_pb,
    NdiBandwidth,
    pb::NdiBandwidth,
    "ndi bandwidth",
    { Highest => Highest, Lowest => Lowest }
);
proto_enum!(
    play_when_to_pb,
    play_when_from_pb,
    VideoPlayWhen,
    pb::VideoPlayWhen,
    "video play when",
    { Never => Never, OnActive => OnActive, OnPreview => OnPreview, Always => Always }
);
proto_enum!(
    trigger_when_to_pb,
    trigger_when_from_pb,
    VideoTriggerWhen,
    pb::VideoTriggerWhen,
    "video trigger when",
    { Never => Never, OnActive => OnActive, OnDeactivated => OnDeactivated, OnPreview => OnPreview }
);
proto_enum!(
    transport_to_pb,
    transport_from_pb,
    OutputTransport,
    pb::OutputTransport,
    "output transport",
    { Omt => Omt, Ndi => Ndi, DeckLink => DeckLink, Rtmp => Rtmp }
);
proto_enum!(
    source_kind_to_pb,
    source_kind_from_pb,
    OutputSourceKind,
    pb::OutputSourceKind,
    "output source kind",
    {
        Scene => Scene,
        MuPreview => MuPreview,
        MuProgram => MuProgram,
        Multiview => Multiview,
        Input => Input
    }
);
proto_enum!(
    mv_kind_to_pb,
    mv_kind_from_pb,
    MvSlotKind,
    pb::MvSlotKind,
    "multiview slot kind",
    { None => None, Input => Input, Scene => Scene, MuPreview => MuPreview, MuProgram => MuProgram }
);
proto_enum!(
    template_to_pb,
    template_from_pb,
    MultiviewTemplate,
    pb::MultiviewTemplate,
    "multiview template",
    {
        PreviewProgram8 => PreviewProgram8,
        PreviewProgram8Bottom => PreviewProgram8Bottom,
        PreviewProgram8Left => PreviewProgram8Left,
        PreviewProgram8Right => PreviewProgram8Right,
        PreviewProgram2 => PreviewProgram2,
        Quad4TopLeft => Quad4TopLeft,
        Quad4TopRight => Quad4TopRight,
        Quad4BottomLeft => Quad4BottomLeft,
        Quad4BottomRight => Quad4BottomRight,
        Large5TopLeft => Large5TopLeft,
        Large5TopRight => Large5TopRight,
        Large5BottomLeft => Large5BottomLeft,
        Large5BottomRight => Large5BottomRight,
        Grid2x2 => Grid2x2,
        Grid3x3 => Grid3x3,
        Grid4x4 => Grid4x4
    }
);
proto_enum!(
    device_kind_to_pb,
    device_kind_from_pb,
    AudioDeviceKind,
    pb::AudioDeviceKind,
    "audio device kind",
    { None => None, Wasapi => Wasapi, Asio => Asio, CoreAudio => CoreAudio }
);
proto_enum!(
    audio_capture_mode_to_pb,
    audio_capture_mode_from_pb,
    AudioCaptureMode,
    pb::AudioCaptureMode,
    "audio capture mode",
    { Mic => Mic, EndpointLoopback => EndpointLoopback, ProcessLoopback => ProcessLoopback }
);
proto_enum!(
    audio_link_to_pb,
    audio_link_from_pb,
    AudioLinkMode,
    pb::AudioLinkMode,
    "audio link",
    { Follow => Follow, Independent => Independent }
);
proto_enum!(
    filter_to_pb,
    filter_from_pb,
    SwitcherSceneFilter,
    pb::SwitcherSceneFilter,
    "switcher scene filter",
    { All => All, Include => Include, Exclude => Exclude }
);

fn replay_to_pb(replay: &crate::session::replay::ReplayDto) -> pb::Replay {
    use crate::session::replay::*;
    pb::Replay {
        enabled: replay.enabled,
        storage: replay
            .storage
            .iter()
            .map(|item| pb::ReplayStorage {
                path: item.path.clone(),
                angle_ids: item.angle_ids.clone(),
            })
            .collect(),
        format: Some(pb::ReplayFormat {
            width: replay.format.width,
            height: replay.format.height,
            fps_num: replay.format.fps_num,
            fps_den: replay.format.fps_den,
            profile: match replay.format.profile {
                ReplayProfile::Hq => pb::ReplayProfile::Hq as i32,
                ReplayProfile::Lq => pb::ReplayProfile::Lq as i32,
                ReplayProfile::Sq => pb::ReplayProfile::Sq as i32,
            },
            segment_frames: replay.format.segment_frames,
        }),
        buffering: Some(pb::ReplayBuffering {
            encode_queue_frames: replay.buffering.encode_queue_frames,
            write_buffer_mb: replay.buffering.write_buffer_mb,
            overflow_policy: match replay.buffering.overflow_policy {
                ReplayOverflowPolicy::StopRecording => pb::ReplayOverflow::Stop as i32,
                ReplayOverflowPolicy::DropFrames => pb::ReplayOverflow::Drop as i32,
            },
            reserve_mb: replay.buffering.reserve_mb,
        }),
        warnings: Some(pb::ReplayWarnings {
            remaining_warn_secs: Some(replay.warnings.remaining_warn_secs),
            remaining_alert_secs: Some(replay.warnings.remaining_alert_secs),
            remaining_critical_secs: Some(replay.warnings.remaining_critical_secs),
        }),
        retention: match replay.retention {
            ReplayRetention::Overwrite => pb::ReplayRetention::Overwrite as i32,
            ReplayRetention::OverwriteUnprotected => {
                pb::ReplayRetention::OverwriteUnprotected as i32
            }
            ReplayRetention::StopWhenFull => pb::ReplayRetention::StopWhenFull as i32,
        },
        angles: replay
            .angles
            .iter()
            .map(|angle| pb::Angle {
                id: angle.id,
                name: angle.name.clone(),
                scene_id: angle.scene_id,
                audio: match angle.audio {
                    crate::session::replay::AngleAudio::Silent => pb::AngleAudio::Silent as i32,
                    crate::session::replay::AngleAudio::Unit => pb::AngleAudio::Unit as i32,
                },
                record_mode: match angle.record_mode {
                    AngleRecordMode::Composite => pb::AngleRecordMode::Composite as i32,
                    AngleRecordMode::Passthrough => pb::AngleRecordMode::Passthrough as i32,
                },
                encoder: match angle.encoder {
                    crate::session::replay::AngleEncoder::Cpu => pb::AngleEncoder::Cpu as i32,
                    crate::session::replay::AngleEncoder::Gpu => pb::AngleEncoder::Gpu as i32,
                },
                audio_unit_id: angle.audio_unit_id,
                source: match angle.source {
                    crate::session::replay::AngleSourceKind::Scene => {
                        pb::AngleSourceKind::Scene as i32
                    }
                    crate::session::replay::AngleSourceKind::Input => {
                        pb::AngleSourceKind::Input as i32
                    }
                },
                input_id: angle.input_id,
            })
            .collect(),
        events: replay
            .events
            .iter()
            .map(|event| pb::ReplayEvent {
                id: event.id,
                recording_uuid: event.recording_uuid.clone(),
                in_frame: event.in_frame,
                out_frame: event.out_frame,
                name: event.name.clone(),
                tags: event.tags.clone(),
                protected: event.protected,
            })
            .collect(),
        playlists: replay
            .playlists
            .iter()
            .map(|list| pb::Playlist {
                id: list.id,
                name: list.name.clone(),
                revision: list.revision,
                clips: list
                    .clips
                    .iter()
                    .map(|clip| pb::Clip {
                        uid: clip.uid.clone(),
                        source_kind: match clip.source {
                            ClipSourceKind::Asset => pb::ClipSourceKind::Asset as i32,
                            ClipSourceKind::Angle => pb::ClipSourceKind::Angle as i32,
                        },
                        angle_id: clip.angle_id,
                        asset_path: clip.asset_path.clone(),
                        source_event: clip.source_event.unwrap_or(0),
                        has_source_event: clip.source_event.is_some(),
                        recording_uuid: clip.recording_uuid.clone(),
                        in_frame: clip.in_frame,
                        out_frame: clip.out_frame,
                        speed: clip.speed,
                        gain_db: clip.gain_db,
                        transition: Some(clip_transition_to_pb(clip.transition)),
                        tags: clip.tags.clone(),
                        preserve_pitch: clip.preserve_pitch,
                        audio: clip_audio_to_pb(clip.audio) as i32,
                        audio_angle_id: clip.audio_angle_id,
                    })
                    .collect(),
                default_transition: Some(clip_transition_to_pb(list.default_transition)),
            })
            .collect(),
        channels: replay
            .channels
            .iter()
            .map(|channel| pb::ReplayChannel {
                index: channel.index,
                playlist_id: channel.playlist_id.unwrap_or(0),
                has_playlist: channel.playlist_id.is_some(),
                input_id: channel.input_id,
            })
            .collect(),
        auto_add_to_playlist: replay.auto_add_to_playlist,
        active_angle_id: replay.active_angle_id.unwrap_or(0),
        has_active_angle: replay.active_angle_id.is_some(),
        next_angle_id: replay.next_angle_id,
        next_event_id: replay.next_event_id,
        next_playlist_id: replay.next_playlist_id,
    }
}

fn replay_from_pb(replay: pb::Replay) -> Result<crate::session::replay::ReplayDto, String> {
    use crate::session::replay::*;
    let format = replay.format.unwrap_or_default();
    let buffering = replay.buffering.unwrap_or_default();
    let warnings = replay.warnings.unwrap_or_default();
    Ok(ReplayDto {
        enabled: replay.enabled,
        storage: replay
            .storage
            .into_iter()
            .map(|item| ReplayStorageDto {
                path: item.path,
                angle_ids: item.angle_ids,
            })
            .collect(),
        format: ReplayFormatDto {
            width: format.width,
            height: format.height,
            fps_num: format.fps_num,
            fps_den: format.fps_den,
            profile: match pb::ReplayProfile::try_from(format.profile) {
                Ok(pb::ReplayProfile::Hq) => ReplayProfile::Hq,
                Ok(pb::ReplayProfile::Lq) => ReplayProfile::Lq,
                _ => ReplayProfile::Sq,
            },
            segment_frames: format.segment_frames,
        },
        buffering: ReplayBufferingDto {
            encode_queue_frames: buffering.encode_queue_frames,
            write_buffer_mb: buffering.write_buffer_mb,
            overflow_policy: match pb::ReplayOverflow::try_from(buffering.overflow_policy) {
                Ok(pb::ReplayOverflow::Stop) => ReplayOverflowPolicy::StopRecording,
                _ => ReplayOverflowPolicy::DropFrames,
            },
            reserve_mb: buffering.reserve_mb,
        },
        // A threshold that is absent takes the document's default; one that is
        // present is kept as written, including zero.
        warnings: ReplayWarningsDto {
            remaining_warn_secs: warnings
                .remaining_warn_secs
                .unwrap_or_else(|| ReplayWarningsDto::default().remaining_warn_secs),
            remaining_alert_secs: warnings
                .remaining_alert_secs
                .unwrap_or_else(|| ReplayWarningsDto::default().remaining_alert_secs),
            remaining_critical_secs: warnings
                .remaining_critical_secs
                .unwrap_or_else(|| ReplayWarningsDto::default().remaining_critical_secs),
        },
        retention: match pb::ReplayRetention::try_from(replay.retention) {
            Ok(pb::ReplayRetention::Overwrite) => ReplayRetention::Overwrite,
            Ok(pb::ReplayRetention::OverwriteUnprotected) => ReplayRetention::OverwriteUnprotected,
            _ => ReplayRetention::StopWhenFull,
        },
        angles: replay
            .angles
            .into_iter()
            .map(|angle| AngleDto {
                id: angle.id,
                name: angle.name,
                source: match pb::AngleSourceKind::try_from(angle.source) {
                    Ok(pb::AngleSourceKind::Input) => AngleSourceKind::Input,
                    _ => AngleSourceKind::Scene,
                },
                scene_id: angle.scene_id,
                input_id: angle.input_id,
                audio: match pb::AngleAudio::try_from(angle.audio) {
                    Ok(pb::AngleAudio::Unit) => AngleAudio::Unit,
                    _ => AngleAudio::Silent,
                },
                audio_unit_id: angle.audio_unit_id,
                record_mode: match pb::AngleRecordMode::try_from(angle.record_mode) {
                    Ok(pb::AngleRecordMode::Passthrough) => AngleRecordMode::Passthrough,
                    _ => AngleRecordMode::Composite,
                },
                encoder: match pb::AngleEncoder::try_from(angle.encoder) {
                    Ok(pb::AngleEncoder::Gpu) => AngleEncoder::Gpu,
                    _ => AngleEncoder::Cpu,
                },
            })
            .collect(),
        events: replay
            .events
            .into_iter()
            .map(|event| ReplayEventDto {
                id: event.id,
                recording_uuid: event.recording_uuid,
                in_frame: event.in_frame,
                out_frame: event.out_frame,
                name: event.name,
                tags: event.tags,
                protected: event.protected,
            })
            .collect(),
        playlists: replay
            .playlists
            .into_iter()
            .map(|list| PlaylistDto {
                id: list.id,
                name: list.name,
                revision: list.revision,
                clips: list
                    .clips
                    .into_iter()
                    .map(|clip| ClipDto {
                        uid: clip.uid,
                        source: match pb::ClipSourceKind::try_from(clip.source_kind) {
                            Ok(pb::ClipSourceKind::Asset) => ClipSourceKind::Asset,
                            _ => ClipSourceKind::Angle,
                        },
                        angle_id: clip.angle_id,
                        asset_path: clip.asset_path,
                        source_event: clip.has_source_event.then_some(clip.source_event),
                        recording_uuid: clip.recording_uuid,
                        in_frame: clip.in_frame,
                        out_frame: clip.out_frame,
                        speed: clip.speed,
                        gain_db: clip.gain_db,
                        transition: clip
                            .transition
                            .map(clip_transition_from_pb)
                            .unwrap_or_default(),
                        tags: clip.tags,
                        preserve_pitch: clip.preserve_pitch,
                        audio: clip_audio_from_pb(clip.audio),
                        audio_angle_id: clip.audio_angle_id,
                    })
                    .collect(),
                default_transition: list
                    .default_transition
                    .map(clip_transition_from_pb)
                    .unwrap_or_default(),
            })
            .collect(),
        channels: replay
            .channels
            .into_iter()
            .map(|channel| ChannelDto {
                index: channel.index,
                playlist_id: channel.has_playlist.then_some(channel.playlist_id),
                input_id: channel.input_id,
            })
            .collect(),
        auto_add_to_playlist: replay.auto_add_to_playlist,
        // The protobuf form predates these two; a document that came through it
        // plays with the defaults rather than losing the queue and the profiling flag.
        decode_ahead: true,
        profiling: false,
        active_angle_id: replay.has_active_angle.then_some(replay.active_angle_id),
        next_angle_id: replay.next_angle_id.max(1),
        next_event_id: replay.next_event_id.max(1),
        next_playlist_id: replay.next_playlist_id.max(1),
    })
}

fn clip_audio_to_pb(audio: crate::session::replay::ClipAudio) -> pb::ClipAudio {
    use crate::session::replay::ClipAudio;
    match audio {
        ClipAudio::FollowAngle => pb::ClipAudio::FollowAngle,
        ClipAudio::Silent => pb::ClipAudio::Silent,
        ClipAudio::Angle => pb::ClipAudio::Angle,
    }
}

fn clip_audio_from_pb(audio: i32) -> crate::session::replay::ClipAudio {
    use crate::session::replay::ClipAudio;
    match pb::ClipAudio::try_from(audio) {
        Ok(pb::ClipAudio::Silent) => ClipAudio::Silent,
        Ok(pb::ClipAudio::Angle) => ClipAudio::Angle,
        _ => ClipAudio::FollowAngle,
    }
}

fn clip_transition_to_pb(
    transition: crate::session::replay::ClipTransitionDto,
) -> pb::ClipTransition {
    use crate::session::replay::{
        ReplayTransitionDirection, ReplayTransitionKind, ReplayTransitionTiming,
    };
    pb::ClipTransition {
        kind: match transition.kind {
            ReplayTransitionKind::Cut => pb::ReplayTransitionKind::Cut as i32,
            ReplayTransitionKind::Dissolve => pb::ReplayTransitionKind::Dissolve as i32,
            ReplayTransitionKind::Dip => pb::ReplayTransitionKind::Dip as i32,
            ReplayTransitionKind::Wipe => pb::ReplayTransitionKind::Wipe as i32,
            ReplayTransitionKind::Slide => pb::ReplayTransitionKind::Slide as i32,
            ReplayTransitionKind::Push => pb::ReplayTransitionKind::Push as i32,
        },
        timing: match transition.timing {
            ReplayTransitionTiming::Overwrap => pb::ReplayTransitionTiming::Overwrap as i32,
            ReplayTransitionTiming::Additive => pb::ReplayTransitionTiming::Additive as i32,
        },
        duration_frames: transition.duration_frames,
        direction: match transition.direction {
            ReplayTransitionDirection::Left => pb::ReplayTransitionDirection::Left as i32,
            ReplayTransitionDirection::Right => pb::ReplayTransitionDirection::Right as i32,
            ReplayTransitionDirection::Up => pb::ReplayTransitionDirection::Up as i32,
            ReplayTransitionDirection::Down => pb::ReplayTransitionDirection::Down as i32,
        },
    }
}

fn clip_transition_from_pb(
    transition: pb::ClipTransition,
) -> crate::session::replay::ClipTransitionDto {
    use crate::session::replay::{
        ReplayTransitionDirection, ReplayTransitionKind, ReplayTransitionTiming,
    };
    crate::session::replay::ClipTransitionDto {
        kind: match pb::ReplayTransitionKind::try_from(transition.kind) {
            Ok(pb::ReplayTransitionKind::Dissolve) => ReplayTransitionKind::Dissolve,
            Ok(pb::ReplayTransitionKind::Dip) => ReplayTransitionKind::Dip,
            Ok(pb::ReplayTransitionKind::Wipe) => ReplayTransitionKind::Wipe,
            Ok(pb::ReplayTransitionKind::Slide) => ReplayTransitionKind::Slide,
            Ok(pb::ReplayTransitionKind::Push) => ReplayTransitionKind::Push,
            _ => ReplayTransitionKind::Cut,
        },
        direction: match pb::ReplayTransitionDirection::try_from(transition.direction) {
            Ok(pb::ReplayTransitionDirection::Right) => ReplayTransitionDirection::Right,
            Ok(pb::ReplayTransitionDirection::Up) => ReplayTransitionDirection::Up,
            Ok(pb::ReplayTransitionDirection::Down) => ReplayTransitionDirection::Down,
            _ => ReplayTransitionDirection::Left,
        },
        timing: match pb::ReplayTransitionTiming::try_from(transition.timing) {
            Ok(pb::ReplayTransitionTiming::Additive) => ReplayTransitionTiming::Additive,
            _ => ReplayTransitionTiming::Overwrap,
        },
        duration_frames: transition.duration_frames,
    }
}

fn input_kind_to_pb(kind: InputKind) -> pb::InputKind {
    match kind {
        InputKind::Color => pb::InputKind::Color,
        InputKind::Bars => pb::InputKind::Bars,
        InputKind::Black => pb::InputKind::Black,
        InputKind::Still => pb::InputKind::Still,
        InputKind::Video => pb::InputKind::Video,
        InputKind::OMT => pb::InputKind::Omt,
        InputKind::NDI => pb::InputKind::Ndi,
        InputKind::UVC => pb::InputKind::Uvc,
        InputKind::Mix => pb::InputKind::Mix,
        InputKind::Audio => pb::InputKind::Audio,
        InputKind::DeckLink => pb::InputKind::DeckLink,
        InputKind::Replay => pb::InputKind::Replay,
    }
}

fn input_kind_from_pb(value: i32) -> Result<InputKind, String> {
    match pb::InputKind::try_from(value) {
        Ok(pb::InputKind::Unspecified) => Err("input kind is required".into()),
        Ok(pb::InputKind::Color) => Ok(InputKind::Color),
        Ok(pb::InputKind::Bars) => Ok(InputKind::Bars),
        Ok(pb::InputKind::Black) => Ok(InputKind::Black),
        Ok(pb::InputKind::Still) => Ok(InputKind::Still),
        Ok(pb::InputKind::Video) => Ok(InputKind::Video),
        Ok(pb::InputKind::Omt) => Ok(InputKind::OMT),
        Ok(pb::InputKind::Ndi) => Ok(InputKind::NDI),
        Ok(pb::InputKind::Uvc) => Ok(InputKind::UVC),
        Ok(pb::InputKind::Mix) => Ok(InputKind::Mix),
        Ok(pb::InputKind::Audio) => Ok(InputKind::Audio),
        Ok(pb::InputKind::DeckLink) => Ok(InputKind::DeckLink),
        Ok(pb::InputKind::Replay) => Ok(InputKind::Replay),
        Err(_) => Err(unknown("input kind", value)),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn atomic_write_replaces_existing_file_without_leaving_temp() {
        let dir = std::env::temp_dir().join(format!("eiviz-atomic-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("session.eivz");
        super::atomic_write(&path, b"first").unwrap();
        super::atomic_write(&path, b"second").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"second");
        assert!(!super::tmp_path(&path).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    use super::*;
    use crate::session::{Renderer, parse, to_vec};

    fn sample_json() -> &'static [u8] {
        br#"{
  "version": 2,
  "settings": {
    "masterFpsNum": 60000,
    "masterFpsDen": 1001,
    "rebarOptimization": false,
    "renderer": "Vulkan",
    "lastSessionPath": "ignore-me.json"
  },
  "inputs": [{
    "id": 2,
    "name": "SMPTE Bars",
    "kind": "Bars",
    "tags": ["VTR"],
    "videoLoop": false
  }],
  "scenes": [{
    "id": 1,
    "name": "Scene 1",
    "layers": [{
      "inputId": 2,
      "width": 1,
      "height": 1,
      "sizeLinked": false,
      "cropX": 0.1,
      "cropWidth": 0.8
    }]
  }],
  "units": [{
    "id": 1,
    "name": "Mixing Unit 1",
    "alwaysOnTop": false,
    "previewSceneId": 1,
    "programSceneId": 1,
    "overlaysOnAir": [1],
    "audio": { "deviceKind": "Wasapi", "deviceId": "out", "mapRight": 1, "gain": 0.5 }
  }],
  "overlays": [{
    "id": 1,
    "sceneGpuId": 1,
    "sizeLinked": false,
    "cropY": 0.2,
    "cropHeight": 0.5
  }],
  "outputs": [{
    "id": 100,
    "name": "eiviz-pgm",
    "transport": "Omt",
    "sourceKind": "MuProgram",
    "skipEncodeWhenNoReceivers": false,
    "width": 1280,
    "height": 720,
    "fpsNum": 30,
    "fpsDen": 1,
    "audioUnitId": 1
  }],
  "headphone": { "deviceKind": "CoreAudio", "deviceId": "hp", "mapRight": 1 },
  "headphoneCopyMonitor": true,
  "inputTags": ["VTR"],
  "nextInputId": 10,
  "nextSceneId": 2,
  "nextUnitId": 2,
  "nextOutputId": 101,
  "selectedUnitId": 1
}"#
    }

    #[test]
    fn protobuf_roundtrip_keeps_host_fields() {
        let original = parse(sample_json()).unwrap();
        let encoded = encode_file(&original).unwrap();
        assert!(encoded.starts_with(MAGIC));
        let decoded = decode_file(&encoded).unwrap();
        assert_eq!(decoded.units[0].always_on_top, false);
        assert_eq!(decoded.units[0].preview_scene_id, 1);
        assert_eq!(decoded.units[0].program_scene_id, 1);
        assert!(!decoded.scenes[0].layers[0].size_linked);
        assert!((decoded.scenes[0].layers[0].crop_x - 0.1).abs() < f32::EPSILON);
        assert_eq!(decoded.units[0].overlays_on_air, vec![1]);
        assert!(!decoded.overlays[0].size_linked);
        assert!((decoded.overlays[0].crop_y - 0.2).abs() < f32::EPSILON);
        assert!(!decoded.outputs[0].skip_encode_when_no_receivers);
        assert_eq!(decoded.outputs[0].width, 1280);
        assert_eq!(decoded.outputs[0].height, 720);
        assert_eq!(decoded.outputs[0].fps_num, 30);
        assert_eq!(decoded.outputs[0].fps_den, 1);
        assert_eq!(decoded.outputs[0].audio_unit_id, 1);
        assert_eq!(decoded.units[0].audio.device_kind, AudioDeviceKind::Wasapi);
        assert_eq!(decoded.units[0].audio.device_id, "out");
        assert!((decoded.units[0].audio.gain - 0.5).abs() < f32::EPSILON);
        assert_eq!(decoded.headphone.device_kind, AudioDeviceKind::CoreAudio);
        assert!(decoded.headphone_copy_monitor);
        assert!(!decoded.settings.rebar_optimization);
        assert_eq!(decoded.settings.renderer, Renderer::Auto);
        assert_eq!(decoded.settings.last_session_path, None);
        let again = decode_file(&encode_file(&decoded).unwrap()).unwrap();
        let a = to_vec(&decoded).unwrap();
        let b = to_vec(&again).unwrap();
        assert_eq!(a, b);
        assert!(encoded.len() < to_vec(&original).unwrap().len());
    }

    #[test]
    fn json_payload_is_rejected() {
        assert!(decode_file(sample_json()).is_err());
        assert!(decode_file(br#"  {"version":2}"#).is_err());
    }

    /// The replay document is project data: angles, clips, transitions and the
    /// operator's selections have to survive a save and a load unchanged.
    #[test]
    fn replay_document_survives_a_save_and_a_load() {
        use crate::session::replay::{
            ReplayRetention, ReplayTransitionKind, ReplayTransitionTiming,
        };
        let json = br#"{
  "version": 2,
  "settings": { "masterFpsNum": 60000, "masterFpsDen": 1001 },
  "inputs": [
    { "id": 2, "name": "SMPTE Bars", "kind": "Bars" },
    { "id": 524289, "name": "Replay A", "kind": "Replay" }
  ],
  "scenes": [{ "id": 1, "name": "Scene 1", "layers": [{ "inputId": 2, "width": 1, "height": 1 }] }],
  "units": [{ "id": 1, "name": "Mixing Unit 1", "previewSceneId": 1, "programSceneId": 1 }],
  "replay": {
    "enabled": true,
    "storage": [{ "path": "D:/replay", "angleIds": [1, 2] }],
    "format": { "width": 1920, "height": 1080, "fpsNum": 60000, "fpsDen": 1001, "profile": "Hq", "segmentFrames": 300 },
    "buffering": { "encodeQueueFrames": 8, "writeBufferMb": 512, "overflowPolicy": "StopRecording", "reserveMb": 4096 },
    "warnings": { "remainingWarnSecs": 900, "remainingAlertSecs": 300, "remainingCriticalSecs": 60 },
    "retention": "OverwriteUnprotected",
    "angles": [
      { "id": 1, "name": "Cam 1", "sceneId": 1, "audio": "Unit", "audioUnitId": 1, "recordMode": "Composite" },
      { "id": 2, "name": "Cam 2", "sceneId": 1, "audio": "Silent", "recordMode": "Composite" }
    ],
    "events": [
      { "id": 4, "recordingUuid": "rec-1", "inFrame": 100, "outFrame": 400, "name": "Goal", "tags": ["Score"], "protected": true }
    ],
    "playlists": [{
      "id": 7,
      "name": "Highlights",
      "revision": 3,
      "defaultTransition": { "kind": "Dissolve", "timing": "Additive", "durationFrames": 25 },
      "clips": [{
        "uid": "clip-4", "source": "Angle", "angleId": 2, "sourceEvent": 4, "recordingUuid": "rec-1",
        "inFrame": 100, "outFrame": 400, "speed": 0.5, "gainDb": -6.0, "tags": ["Score"],
        "preservePitch": true, "audio": "Angle", "audioAngleId": 1,
        "transition": { "kind": "Dissolve", "timing": "Additive", "durationFrames": 25 }
      }]
    }],
    "channels": [{ "index": 0, "playlistId": 7, "inputId": 524289 }],
    "autoAddToPlaylist": true,
    "activeAngleId": 2,
    "nextAngleId": 3, "nextEventId": 5, "nextPlaylistId": 8
  },
  "nextInputId": 10, "nextSceneId": 2, "nextUnitId": 2
}"#;
        let original = parse(json).unwrap();
        let replay = original.replay.as_ref().expect("the session has Replay");
        assert_eq!(replay.channels[0].input_id, 524289);
        assert_eq!(replay.retention, ReplayRetention::OverwriteUnprotected);

        let decoded = decode_file(&encode_file(&original).unwrap()).unwrap();
        let round = decoded.replay.as_ref().expect("Replay survived the file");
        assert_eq!(round, replay, "the replay document must round-trip exactly");
        // The pieces an operator would notice missing.
        let clip = &round.playlists[0].clips[0];
        assert_eq!(clip.transition.kind, ReplayTransitionKind::Dissolve);
        assert_eq!(clip.transition.timing, ReplayTransitionTiming::Additive);
        assert_eq!(clip.transition.duration_frames, 25);
        assert_eq!(
            round.playlists[0].default_transition, clip.transition,
            "the playlist default is not lost"
        );
        assert!(clip.preserve_pitch);
        assert_eq!(clip.audio_angle_id, 1);
        assert_eq!(round.active_angle_id, Some(2));
        assert_eq!(round.channels[0].playlist_id, Some(7));
        assert_eq!(round.events[0].tags, ["Score"]);
        assert_eq!(round.angles[0].audio_unit_id, 1);
        assert_eq!(decoded.inputs.len(), 2);
    }

    #[test]
    fn magic_and_versions_are_rejected() {
        assert!(decode_file(b"").is_err());
        assert!(decode_file(b"not-a-session").is_err());
        let mut truncated = MAGIC.to_vec();
        truncated.extend_from_slice(&CONTAINER_VERSION.to_le_bytes());
        assert!(decode_file(&truncated).is_err());
        let mut bad_container = MAGIC.to_vec();
        bad_container.extend_from_slice(&2u16.to_le_bytes());
        bad_container.extend_from_slice(&[0u8; 4]);
        assert!(
            decode_file(&bad_container)
                .unwrap_err()
                .contains("container version")
        );
        let mut payload = encode_file(&parse(sample_json()).unwrap()).unwrap();
        payload.truncate(8);
        assert!(decode_file(&payload).is_err());
    }

    #[test]
    fn unknown_enum_is_rejected() {
        let doc = parse(sample_json()).unwrap();
        let mut file = pb::SessionFile {
            format_version: FORMAT_VERSION,
            document: Some(document_to_pb(&doc)),
            assets: Vec::new(),
            history: Vec::new(),
        };
        file.document.as_mut().unwrap().inputs[0].kind = 99;
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(&CONTAINER_VERSION.to_le_bytes());
        bytes.extend_from_slice(&file.encode_to_vec());
        let err = decode_file(&bytes).unwrap_err();
        assert!(err.contains("input kind"), "{err}");
    }

    #[test]
    fn future_format_version_is_rejected() {
        let doc = parse(sample_json()).unwrap();
        let file = pb::SessionFile {
            format_version: 99,
            document: Some(document_to_pb(&doc)),
            assets: Vec::new(),
            history: Vec::new(),
        };
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(&CONTAINER_VERSION.to_le_bytes());
        bytes.extend_from_slice(&file.encode_to_vec());
        assert!(
            decode_file(&bytes)
                .unwrap_err()
                .contains("format version 99")
        );
    }

    #[test]
    fn missing_document_is_rejected() {
        let file = pb::SessionFile {
            format_version: FORMAT_VERSION,
            document: None,
            assets: Vec::new(),
            history: Vec::new(),
        };
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(&CONTAINER_VERSION.to_le_bytes());
        bytes.extend_from_slice(&file.encode_to_vec());
        assert!(
            decode_file(&bytes)
                .unwrap_err()
                .contains("missing a document")
        );
    }

    #[test]
    fn large_canonical_json_roundtrips() {
        let mut doc = parse(sample_json()).unwrap();
        doc.inputs[0].name = "x".repeat(1_200_000);
        let json = to_vec(&doc).unwrap();
        assert!(json.len() > 1 << 20);
        let encoded = encode_file(&doc).unwrap();
        let decoded = decode_file(&encoded).unwrap();
        assert_eq!(decoded.inputs[0].name.len(), 1_200_000);
        assert!(to_vec(&decoded).unwrap().len() > 1 << 20);
    }

    #[test]
    fn export_embeds_media_and_load_extracts_it() {
        let dir = std::env::temp_dir().join(format!("eiviz-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("card.png");
        std::fs::write(&source, b"png-bytes").unwrap();
        let mut doc = parse(sample_json()).unwrap();
        doc.inputs[0].kind = InputKind::Still;
        doc.inputs[0].path_or_address = Some(source.to_string_lossy().into_owned());
        let exported = dir.join("show.eivz");
        export_document(&exported, &doc).unwrap();
        std::fs::remove_file(&source).unwrap();
        let loaded = read_document(&exported).unwrap();
        let extracted = loaded.inputs[0]
            .path_or_address
            .as_deref()
            .expect("extracted path");
        assert!(extracted.ends_with("2_card.png"), "{extracted}");
        assert_eq!(std::fs::read(extracted).unwrap(), b"png-bytes");
        assert!(has_embedded_assets(&exported).unwrap());
        let media = dir.join("picked-media");
        let dest = dir.join("imported.eivz");
        let imported = import_exported_session(&exported, &dest, &media).unwrap();
        let imported_path = imported.inputs[0]
            .path_or_address
            .as_deref()
            .expect("imported path");
        assert!(imported_path.contains("picked-media"), "{imported_path}");
        assert!(imported_path.ends_with("2_card.png"), "{imported_path}");
        assert_eq!(std::fs::read(imported_path).unwrap(), b"png-bytes");
        assert!(!has_embedded_assets(&dest).unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_appends_history_and_restores() {
        let dir = std::env::temp_dir().join(format!("eiviz-history-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("show.eivz");
        let mut first = parse(sample_json()).unwrap();
        first.inputs[0].name = "one".into();
        assert_eq!(write_document(&path, &first).unwrap(), 0);
        assert!(read_history(&path).unwrap().is_empty());
        let mut second = first.clone();
        second.inputs[0].name = "two".into();
        assert_eq!(write_document_rev(&path, &second, 4).unwrap(), 1);
        let history = read_history(&path).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].index, 0);
        assert_eq!(history[0].revision, 1);
        assert_eq!(extract_history(&path, 0).unwrap().inputs[0].name, "one");
        assert_eq!(read_document(&path).unwrap().inputs[0].name, "two");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn history_dedupes_unchanged_save() {
        let dir = std::env::temp_dir().join(format!("eiviz-history-dedupe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("show.eivz");
        let mut doc = parse(sample_json()).unwrap();
        doc.inputs[0].name = "keep".into();
        write_document(&path, &doc).unwrap();
        doc.inputs[0].name = "next".into();
        assert_eq!(write_document(&path, &doc).unwrap(), 1);
        assert_eq!(write_document(&path, &doc).unwrap(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn history_is_capped() {
        let dir = std::env::temp_dir().join(format!("eiviz-history-cap-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("show.eivz");
        let mut doc = parse(sample_json()).unwrap();
        for i in 0..=HISTORY_LIMIT {
            doc.inputs[0].name = format!("rev-{i}");
            write_document(&path, &doc).unwrap();
        }
        let history = read_history(&path).unwrap();
        assert_eq!(history.len(), HISTORY_LIMIT);
        assert_eq!(extract_history(&path, 0).unwrap().inputs[0].name, "rev-19");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn export_strips_history() {
        let dir = std::env::temp_dir().join(format!("eiviz-history-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("card.png");
        std::fs::write(&source, b"png-bytes").unwrap();
        let path = dir.join("show.eivz");
        let exported = dir.join("show.eivzx");
        let mut doc = parse(sample_json()).unwrap();
        doc.inputs[0].kind = InputKind::Still;
        doc.inputs[0].path_or_address = Some(source.to_string_lossy().into_owned());
        write_document(&path, &doc).unwrap();
        doc.inputs[0].name = "later".into();
        write_document(&path, &doc).unwrap();
        assert_eq!(read_history(&path).unwrap().len(), 1);
        export_document(&exported, &doc).unwrap();
        assert!(read_history(&exported).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_existing_file_is_not_overwritten() {
        let dir = std::env::temp_dir().join(format!("eiviz-corrupt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("show.eivz");
        std::fs::write(&path, b"not-an-eivz").unwrap();
        let doc = parse(sample_json()).unwrap();
        assert!(write_document(&path, &doc).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"not-an-eivz");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn extract_history_rejects_missing_index() {
        let dir = std::env::temp_dir().join(format!("eiviz-history-idx-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("show.eivz");
        write_document(&path, &parse(sample_json()).unwrap()).unwrap();
        let err = extract_history(&path, 0).unwrap_err();
        assert!(err.contains("history index 0 is missing"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn restored_bytes_have_empty_history() {
        let dir =
            std::env::temp_dir().join(format!("eiviz-history-restore-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("show.eivz");
        let standalone = dir.join("old.eivz");
        let mut first = parse(sample_json()).unwrap();
        first.inputs[0].name = "one".into();
        write_document(&path, &first).unwrap();
        let mut second = first.clone();
        second.inputs[0].name = "two".into();
        write_document(&path, &second).unwrap();
        let restored = extract_history(&path, 0).unwrap();
        assert_eq!(restored.inputs[0].name, "one");
        std::fs::write(&standalone, encode_file(&restored).unwrap()).unwrap();
        assert!(read_history(&standalone).unwrap().is_empty());
        assert_eq!(read_document(&standalone).unwrap().inputs[0].name, "one");
        assert_eq!(read_document(&path).unwrap().inputs[0].name, "two");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
