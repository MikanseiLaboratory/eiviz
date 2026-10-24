//! Instant-replay session model. Stage 1 records one angle and plays one clip on channel A.

use serde::{Deserialize, Serialize};

use super::{Document, InputDto, InputKind};

/// Source-id band for Replay channel inputs. `0x0005_0000` holds the host's
/// remote preview/program/multiview video sources and `0x0007_0000` holds
/// remote mixing unit ids, so Replay takes `0x0008_0000`.
pub const REPLAY_CHANNEL_BASE: u64 = 0x0008_0000;

session_string_enum! {
    #[derive(Default)]
    pub enum ReplayProfile {
        Hq,
        #[default]
        Sq,
        Lq,
    }
}

session_string_enum! {
    #[derive(Default)]
    pub enum ReplayOverflowPolicy {
        #[default]
        DropFrames,
        StopRecording,
    }
}

session_string_enum! {
    #[derive(Default)]
    pub enum ReplayRetention {
        #[default]
        StopWhenFull,
        Overwrite,
        OverwriteUnprotected,
    }
}

session_string_enum! {
    /// Where an angle's audio comes from. `Unit` records the mixing unit's
    /// program mix; its id lives on `AngleDto::audio_unit_id`.
    #[derive(Default)]
    pub enum AngleAudio {
        #[default]
        Silent,
        Unit,
    }
}

session_string_enum! {
    /// What an angle records. A `Scene` angle records that scene's composited
    /// output; an `Input` angle records the input's own picture, without composing
    /// a scene first.
    #[derive(Default)]
    pub enum AngleSourceKind {
        #[default]
        Scene,
        Input,
    }
}

session_string_enum! {
    #[derive(Default)]
    pub enum AngleRecordMode {
        #[default]
        Composite,
        /// Keep the frames as they arrive, without re-encoding them. Only an input
        /// that already delivers compressed frames (OMT) can do this, and the
        /// operator chooses it per angle: nothing switches to it on its own.
        Passthrough,
    }
}

session_string_enum! {
    #[derive(Default)]
    pub enum ClipSourceKind {
        #[default]
        Angle,
        Asset,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayDto {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub storage: Vec<ReplayStorageDto>,
    #[serde(default)]
    pub format: ReplayFormatDto,
    #[serde(default)]
    pub buffering: ReplayBufferingDto,
    #[serde(default)]
    pub warnings: ReplayWarningsDto,
    #[serde(default)]
    pub retention: ReplayRetention,
    #[serde(default)]
    pub angles: Vec<AngleDto>,
    #[serde(default)]
    pub events: Vec<ReplayEventDto>,
    #[serde(default)]
    pub playlists: Vec<PlaylistDto>,
    #[serde(default)]
    pub channels: Vec<ChannelDto>,
    #[serde(default)]
    pub auto_add_to_playlist: bool,
    /// Decode a frame or two ahead of the picture, so a decode that runs long does
    /// not push the picture past its frame time. On unless the operator turned it
    /// off, which is the first thing to try when a playback problem is blamed on it.
    #[serde(default = "yes")]
    pub decode_ahead: bool,
    /// Write per-stage playout timings to the mixer log (decode, publish, readback,
    /// how many frames came from the queue, late ticks). Meant for measuring a
    /// machine, not for everyday use.
    #[serde(default)]
    pub profiling: bool,
    /// Angle the operator is watching. Event creation and playlist auto-add use
    /// it, so the control layer never guesses which camera a clip came from.
    #[serde(default)]
    pub active_angle_id: Option<u32>,
    #[serde(default = "one_u32")]
    pub next_angle_id: u32,
    #[serde(default = "one_u64")]
    pub next_event_id: u64,
    #[serde(default = "one_u64")]
    pub next_playlist_id: u64,
    /// Recordings this session uses, so a clip's recording is looked up by the
    /// session rather than discovered by scanning the drives. A recording that is not
    /// listed here is not played.
    #[serde(default)]
    pub recordings: Vec<ReplayRecordingDto>,
    /// Scene states the replay timeline fires. A cue names either a clip, in which
    /// case `frame` is that clip's in point, or an absolute recording frame when
    /// `clip_uid` is empty. Distinct from a scene sequence's replay cue.
    #[serde(default)]
    pub timeline_cues: Vec<ReplayTimelineCueDto>,
}

fn one_u32() -> u32 {
    1
}

fn yes() -> bool {
    true
}

fn one_u64() -> u64 {
    1
}

impl Default for ReplayDto {
    fn default() -> Self {
        Self {
            enabled: false,
            storage: Vec::new(),
            format: ReplayFormatDto::default(),
            buffering: ReplayBufferingDto::default(),
            warnings: ReplayWarningsDto::default(),
            retention: ReplayRetention::StopWhenFull,
            angles: Vec::new(),
            events: Vec::new(),
            playlists: Vec::new(),
            channels: Vec::new(),
            auto_add_to_playlist: false,
            decode_ahead: true,
            profiling: false,
            active_angle_id: None,
            next_angle_id: 1,
            next_event_id: 1,
            next_playlist_id: 1,
            timeline_cues: Vec::new(),
            recordings: Vec::new(),
        }
    }
}

impl ReplayDto {
    /// Keep explicitly created channels well-formed. Replay channels are never
    /// invented while loading a session.
    pub fn normalize(&mut self) {
        // One camera is not a choice, so a document with exactly one angle selects
        // it. Automation (the vMix API, an animation step) marks clips without ever
        // pressing a tile, and without this the mark would be refused for want of a
        // selection nobody could make. A missing or dangling id is repaired the same
        // way; with two or more cameras nothing is guessed.
        let known = self
            .active_angle_id
            .is_some_and(|id| self.angles.iter().any(|angle| angle.id == id));
        if !known && let [only] = self.angles.as_slice() {
            self.active_angle_id = Some(only.id);
        }
        self.buffering.encode_queue_frames = self.buffering.encode_queue_frames.clamp(0, 60);
        if self.buffering.encode_queue_frames != 0 && self.buffering.encode_queue_frames < 2 {
            self.buffering.encode_queue_frames = 2;
        }
        // One registry entry per recording, and none without an id: a duplicate would
        // make "where is this recording" depend on the order of the list.
        self.recordings
            .retain(|recording| !recording.uuid.trim().is_empty());
        let mut seen_recordings = std::collections::HashSet::new();
        self.recordings
            .retain(|recording| seen_recordings.insert(recording.uuid.clone()));
        self.channels.sort_by_key(|channel| channel.index);
        // Hosts may publish a Replay document without the counters (the win32
        // window builds the settings JSON by hand), so repair them from the ids
        // that already exist. Without this the next mark-out reuses id 1.
        self.next_event_id = self
            .next_event_id
            .max(highest(self.events.iter().map(|event| event.id)).saturating_add(1))
            .max(1);
        self.next_playlist_id = self
            .next_playlist_id
            .max(highest(self.playlists.iter().map(|list| list.id)).saturating_add(1))
            .max(1);
        self.next_angle_id = self
            .next_angle_id
            .max(
                u32::try_from(
                    highest(self.angles.iter().map(|angle| u64::from(angle.id))).saturating_add(1),
                )
                .unwrap_or(u32::MAX),
            )
            .max(1);
    }

    /// Allocates the next event id and advances the counter.
    pub fn reserve_event_id(&mut self) -> u64 {
        let highest = highest(self.events.iter().map(|event| event.id));
        let id = self.next_event_id.max(highest.saturating_add(1)).max(1);
        self.next_event_id = id.saturating_add(1);
        id
    }

    /// Allocates the next playlist id and advances the counter.
    pub fn reserve_playlist_id(&mut self) -> u64 {
        let highest = highest(self.playlists.iter().map(|list| list.id));
        let id = self.next_playlist_id.max(highest.saturating_add(1)).max(1);
        self.next_playlist_id = id.saturating_add(1);
        id
    }

    /// Allocates the next angle id and advances the counter. Angles are added by
    /// the hosts and by the API, so the id has to come from the document rather
    /// than from a count that a deleted angle would skew.
    pub fn reserve_angle_id(&mut self) -> u32 {
        let highest = self.angles.iter().map(|angle| angle.id).max().unwrap_or(0);
        let id = self.next_angle_id.max(highest.saturating_add(1)).max(1);
        self.next_angle_id = id.saturating_add(1);
        id
    }

    /// Records a Mark Out. The event is always filed; the clip is filed only when
    /// the document asks for it, and it inherits the playlist's default
    /// transition. The UI offers that setting as "new clips", so it has to mean
    /// the same thing on this path as it does in the hosts.
    ///
    /// Returns the new event id.
    pub fn note_mark_out(
        &mut self,
        uuid: &str,
        in_frame: u64,
        out_frame: u64,
        name: &str,
    ) -> Result<u64, String> {
        let id = self.reserve_event_id();
        // A mark taken from the API or an automation panel often carries no name. An
        // unnamed row in the event list says nothing, so it gets the same shape of
        // name the deck gives one, without the locale.
        let name = if name.trim().is_empty() {
            format!("Replay {id}")
        } else {
            name.trim().to_string()
        };
        self.events.push(ReplayEventDto {
            id,
            recording_uuid: uuid.to_string(),
            in_frame,
            out_frame,
            name,
            tags: Vec::new(),
            protected: false,
        });
        if !self.auto_add_to_playlist {
            return Ok(id);
        }
        // The operator's current angle is explicit state. Guessing between cameras
        // would silently file the clip against the wrong one; a document with one
        // camera has that selection made for it by `normalize`.
        let angle_id = self.active_angle_id.ok_or_else(|| {
            "Select a Replay angle before adding events to a playlist".to_string()
        })?;
        if !self.angles.iter().any(|angle| angle.id == angle_id) {
            return Err(format!("Replay angle {angle_id} does not exist"));
        }
        // A playlist without an id is not a playlist the operator made, so it is
        // replaced rather than appended to.
        let playlist_id = match self.playlists.first() {
            Some(list) if list.id != 0 => list.id,
            _ => {
                let playlist_id = self.reserve_playlist_id();
                self.playlists.push(PlaylistDto {
                    id: playlist_id,
                    name: "Replay".into(),
                    revision: 1,
                    clips: Vec::new(),
                    default_transition: ClipTransitionDto::default(),
                });
                playlist_id
            }
        };
        let Some(list) = self
            .playlists
            .iter_mut()
            .find(|list| list.id == playlist_id)
        else {
            return Ok(id);
        };
        list.revision = list.revision.saturating_add(1);
        let transition = list.default_transition;
        list.clips.push(ClipDto {
            uid: format!("clip-{id}"),
            source: ClipSourceKind::Angle,
            angle_id,

            recording_uuid: uuid.to_string(),
            in_frame,
            out_frame,
            transition,
            ..ClipDto::default()
        });
        Ok(id)
    }
}

fn highest(values: impl Iterator<Item = u64>) -> u64 {
    values.max().unwrap_or(0)
}

/// The part of a Replay document that Replay Setup owns.
///
/// Settings are written on their own so that applying them cannot overwrite the
/// events and playlists an operator is building at the same time. The hosts send
/// this, and the whole-document `UpsertReplaySettings` stays for clients that
/// really do mean to replace the document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplaySettingsDto {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub storage: Vec<ReplayStorageDto>,
    #[serde(default)]
    pub format: ReplayFormatDto,
    #[serde(default)]
    pub buffering: ReplayBufferingDto,
    #[serde(default)]
    pub retention: ReplayRetention,
    /// Remaining-time warnings. Replay Setup owns them, so a deck save cannot
    /// put them back to the defaults.
    #[serde(default)]
    pub warnings: ReplayWarningsDto,
    #[serde(default)]
    pub angles: Vec<AngleDto>,
    #[serde(default)]
    pub auto_add_to_playlist: bool,
    /// Decode a frame or two ahead of the picture, so a decode that runs long does
    /// not push the picture past its frame time. On unless the operator turned it
    /// off, which is the first thing to try when a playback problem is blamed on it.
    #[serde(default = "yes")]
    pub decode_ahead: bool,
    /// Write per-stage playout timings to the mixer log (decode, publish, readback,
    /// how many frames came from the queue, late ticks). Meant for measuring a
    /// machine, not for everyday use.
    #[serde(default)]
    pub profiling: bool,
}

impl ReplaySettingsDto {
    /// The settings of an existing document, for a host that read it and wants to
    /// send back only what it edited.
    pub fn from_document(replay: &ReplayDto) -> Self {
        Self {
            enabled: replay.enabled,
            storage: replay.storage.clone(),
            format: replay.format.clone(),
            buffering: replay.buffering.clone(),
            retention: replay.retention,
            warnings: replay.warnings.clone(),
            angles: replay.angles.clone(),
            auto_add_to_playlist: replay.auto_add_to_playlist,
            decode_ahead: replay.decode_ahead,
            profiling: replay.profiling,
        }
    }

    /// Writes these settings onto a document, leaving everything else in place.
    pub fn apply_to(self, replay: &mut ReplayDto) {
        replay.enabled = self.enabled;
        replay.storage = self.storage;
        replay.format = self.format;
        replay.buffering = self.buffering;
        replay.retention = self.retention;
        replay.warnings = self.warnings;
        replay.angles = self.angles;
        replay.auto_add_to_playlist = self.auto_add_to_playlist;
        replay.decode_ahead = self.decode_ahead;
        replay.profiling = self.profiling;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayStorageDto {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub angle_ids: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayFormatDto {
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    #[serde(default)]
    pub fps_num: u32,
    #[serde(default)]
    pub fps_den: u32,
    #[serde(default)]
    pub profile: ReplayProfile,
    #[serde(default)]
    pub segment_frames: u32,
}

impl Default for ReplayFormatDto {
    fn default() -> Self {
        Self {
            width: 0,
            height: 0,
            fps_num: 0,
            fps_den: 0,
            profile: ReplayProfile::Sq,
            segment_frames: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayBufferingDto {
    #[serde(default = "encode_queue_default")]
    pub encode_queue_frames: u32,
    #[serde(default = "write_buffer_default")]
    pub write_buffer_mb: u32,
    #[serde(default)]
    pub overflow_policy: ReplayOverflowPolicy,
    #[serde(default = "reserve_default")]
    pub reserve_mb: u32,
}

fn encode_queue_default() -> u32 {
    6
}
fn write_buffer_default() -> u32 {
    512
}
fn reserve_default() -> u32 {
    2048
}

impl Default for ReplayBufferingDto {
    fn default() -> Self {
        Self {
            encode_queue_frames: 6,
            write_buffer_mb: 512,
            overflow_policy: ReplayOverflowPolicy::DropFrames,
            reserve_mb: 2048,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayWarningsDto {
    #[serde(default = "warn_secs")]
    pub remaining_warn_secs: u32,
    #[serde(default = "alert_secs")]
    pub remaining_alert_secs: u32,
    #[serde(default = "critical_secs")]
    pub remaining_critical_secs: u32,
}

fn warn_secs() -> u32 {
    600
}
fn alert_secs() -> u32 {
    300
}
fn critical_secs() -> u32 {
    60
}

impl Default for ReplayWarningsDto {
    fn default() -> Self {
        Self {
            remaining_warn_secs: 600,
            remaining_alert_secs: 300,
            remaining_critical_secs: 60,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AngleDto {
    pub id: u32,
    #[serde(default)]
    pub name: String,
    /// What this angle records. `Scene` uses `scene_id`; `Input` uses `input_id`.
    #[serde(default)]
    pub source: AngleSourceKind,
    /// Scene this angle records when `source` is `Scene`. Zero for an input angle.
    #[serde(default)]
    pub scene_id: u64,
    /// Input this angle records when `source` is `Input`. Zero for a scene angle.
    #[serde(default)]
    pub input_id: u64,
    #[serde(default)]
    pub audio: AngleAudio,
    /// Mixing unit whose program mix feeds this angle when `audio` is `Unit`.
    #[serde(default)]
    pub audio_unit_id: u64,
    #[serde(default)]
    pub record_mode: AngleRecordMode,
    /// Where this angle is encoded. `Gpu` keeps the picture on the GPU from scene to
    /// stored packet; `Cpu` reads it back and encodes on the CPU. The choice is per
    /// angle because the CPU cost is per angle: two angles at 1080p60 already exceed
    /// what one CPU encoder can do.
    #[serde(default)]
    pub encoder: AngleEncoder,
}

session_string_enum! {
    /// Where a replay angle is encoded.
    #[derive(Default)]
    pub enum AngleEncoder {
        /// Read the picture back and encode on the CPU. Works everywhere.
        #[default]
        Cpu,
        /// Encode from the scene texture on the GPU, so the frame is never copied back.
        Gpu,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayEventDto {
    pub id: u64,
    #[serde(default)]
    pub recording_uuid: String,
    #[serde(default)]
    pub in_frame: u64,
    #[serde(default)]
    pub out_frame: u64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub protected: bool,
}

session_string_enum! {
    /// Which recording a clip's audio comes from. `Angle` names another angle,
    /// so a clip can play one angle's picture with a different angle's sound.
    #[derive(Default)]
    pub enum ClipAudio {
        /// Whatever the clip's own angle recorded.
        #[default]
        FollowAngle,
        Silent,
        Angle,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipDto {
    #[serde(default)]
    pub uid: String,
    #[serde(default)]
    pub source: ClipSourceKind,
    #[serde(default)]
    pub angle_id: u32,
    #[serde(default)]
    pub asset_path: String,
    #[serde(default)]
    pub recording_uuid: String,
    #[serde(default)]
    pub in_frame: u64,
    #[serde(default)]
    pub out_frame: u64,
    #[serde(default = "speed_one")]
    pub speed: f32,
    #[serde(default)]
    pub gain_db: f32,
    /// Free-form tags, with suggestions from the session's replay catalog.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Hold the pitch when the clip plays at something other than normal speed.
    /// Defaults to on: a replay that drops in speed should not sound like a tape
    /// running down.
    #[serde(default = "preserve_pitch_default")]
    pub preserve_pitch: bool,
    /// Audio source for this clip.
    #[serde(default)]
    pub audio: ClipAudio,
    /// Angle whose recording supplies the audio when `audio` is `Angle`.
    #[serde(default)]
    pub audio_angle_id: u32,
    #[serde(default)]
    pub transition: ClipTransitionDto,
}

fn speed_one() -> f32 {
    1.0
}

/// Pitch is held unless the operator turns it off.
fn preserve_pitch_default() -> bool {
    true
}

impl Default for ClipDto {
    fn default() -> Self {
        Self {
            uid: String::new(),
            source: ClipSourceKind::Angle,
            angle_id: 0,
            asset_path: String::new(),
            recording_uuid: String::new(),
            in_frame: 0,
            out_frame: 0,
            speed: 1.0,
            gain_db: 0.0,
            tags: Vec::new(),
            preserve_pitch: true,
            audio: ClipAudio::FollowAngle,
            audio_angle_id: 0,
            transition: ClipTransitionDto::default(),
        }
    }
}

session_string_enum! {
    /// How one clip gives way to the next. The names are the mixing unit's names, so an
    /// operator picks the same word in both places.
    ///
    /// Only the ones the replay path can draw are listed. A clip transition is composed
    /// from the two decoded frames, and the mixing unit's transitions are GPU shaders
    /// that warp the picture; a warp here would cost a frame's worth of work per pixel
    /// on the CPU and would not keep up with playout.
    #[derive(Default)]
    pub enum ReplayTransitionKind {
        #[default]
        Cut,
        Dissolve,
        /// Fade out through black, then in.
        Dip,
        /// A hard edge sweeps across the picture.
        Wipe,
        /// The incoming clip slides in over the outgoing one.
        Slide,
        /// The outgoing clip slides out as the incoming one slides in.
        Push,
    }
}

session_string_enum! {
    /// Which way a directional transition travels.
    #[derive(Default)]
    pub enum ReplayTransitionDirection {
        #[default]
        Left,
        Right,
        Up,
        Down,
    }
}

session_string_enum! {
    /// How the transition spends its time. `Overwrap` takes the duration out of
    /// the two clips by overlapping them; `Additive` inserts it between them.
    #[derive(Default)]
    pub enum ReplayTransitionTiming {
        #[default]
        Overwrap,
        Additive,
    }
}

/// Transition out of one clip and into the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipTransitionDto {
    #[serde(default)]
    pub kind: ReplayTransitionKind,
    /// Which way a `Wipe`, `Slide` or `Push` travels. Ignored by the others.
    #[serde(default)]
    pub direction: ReplayTransitionDirection,
    #[serde(default)]
    pub timing: ReplayTransitionTiming,
    #[serde(default)]
    pub duration_frames: u32,
}

/// Accepts both the structured form and the bare kind name that earlier sessions
/// stored (`"Cut"`, `"Dissolve"`).
impl<'de> Deserialize<'de> for ClipTransitionDto {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Kind(String),
            #[serde(rename_all = "camelCase")]
            Full {
                #[serde(default)]
                kind: ReplayTransitionKind,
                #[serde(default)]
                direction: ReplayTransitionDirection,
                #[serde(default)]
                timing: ReplayTransitionTiming,
                #[serde(default)]
                duration_frames: u32,
            },
        }
        Ok(match Raw::deserialize(deserializer)? {
            Raw::Kind(name) => Self {
                kind: ReplayTransitionKind::from_str_loose(&name).unwrap_or_default(),
                ..Self::default()
            },
            Raw::Full {
                kind,
                direction,
                timing,
                duration_frames,
            } => Self {
                kind,
                direction,
                timing,
                duration_frames,
            },
        })
    }
}

impl Default for ClipTransitionDto {
    fn default() -> Self {
        Self {
            kind: ReplayTransitionKind::Cut,
            direction: ReplayTransitionDirection::Left,
            timing: ReplayTransitionTiming::Overwrap,
            duration_frames: 0,
        }
    }
}

impl ClipTransitionDto {
    pub fn is_cut(self) -> bool {
        self.kind == ReplayTransitionKind::Cut || self.duration_frames == 0
    }

    /// Frames the transition occupies on the timeline. A cut takes none.
    pub fn frames(self) -> u32 {
        if self.is_cut() {
            0
        } else {
            self.duration_frames
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistDto {
    pub id: u64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub revision: u64,
    #[serde(default)]
    pub clips: Vec<ClipDto>,
    /// Transition for clips that leave their own at the default `Cut`.
    #[serde(default)]
    pub default_transition: ClipTransitionDto,
}

/// One recording this session uses.
///
/// The clip carries the recording's id; this entry says where that recording lives,
/// so playback opens a path the session knows instead of searching the drives. The
/// fields after `storage` describe the recording as it was written, which is what the
/// deck shows when the files are no longer there.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayRecordingDto {
    /// `rec-...`, the same id the clips and events carry.
    #[serde(default)]
    pub uuid: String,
    /// Directory the recording lives in (the storage path it was written to).
    #[serde(default)]
    pub storage: String,
    #[serde(default)]
    pub angles: Vec<u32>,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    #[serde(default)]
    pub fps_num: u32,
    #[serde(default)]
    pub fps_den: u32,
    /// Frames written, as the manifest reports them when recording stops.
    #[serde(default)]
    pub first_frame: u64,
    #[serde(default)]
    pub last_frame: u64,
    /// Wall clock (milliseconds since the epoch) for ordering; 0 when unknown.
    #[serde(default)]
    pub created_ms: u64,
}

impl ReplayRecordingDto {
    /// The full directory of this recording.
    pub fn path(&self) -> std::path::PathBuf {
        std::path::Path::new(&self.storage).join(&self.uuid)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelDto {
    pub index: u32,
    #[serde(default)]
    pub playlist_id: Option<u64>,
    #[serde(default)]
    pub input_id: u64,
}

/// A scene state fired from the replay timeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayTimelineCueDto {
    #[serde(default)]
    pub id: u64,
    /// Recording frame, or the clip's in point when `clip_uid` is set.
    #[serde(default)]
    pub frame: u64,
    #[serde(default)]
    pub scene_id: u64,
    #[serde(default)]
    pub state_id: u64,
    /// Empty when `frame` is an absolute recording timecode.
    #[serde(default)]
    pub clip_uid: String,
}

/// The part of a Replay document that the deck owns.
///
/// Events, playlists, the selected angle and which playlist a channel plays.
/// Channels themselves are added and removed only with the Replay input.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayDeckDto {
    #[serde(default)]
    pub events: Vec<ReplayEventDto>,
    #[serde(default)]
    pub playlists: Vec<PlaylistDto>,
    #[serde(default)]
    pub active_angle_id: Option<u32>,
    #[serde(default)]
    pub next_event_id: u64,
    #[serde(default)]
    pub next_playlist_id: u64,
    #[serde(default)]
    pub channels: Vec<ChannelDto>,
    #[serde(default)]
    pub timeline_cues: Vec<ReplayTimelineCueDto>,
}

impl ReplayDeckDto {
    /// Writes the deck onto a document. Channel rows only update the playlist a
    /// channel already has; a deck snapshot cannot add or remove channels.
    pub fn apply_to(self, replay: &mut ReplayDto) {
        replay.events = self.events;
        replay.playlists = self.playlists;
        // A deck that never picked a camera sends none. Clearing the selection because
        // of that silently moved the operator's angle to whatever normalize picks next.
        if self.active_angle_id.is_some() {
            replay.active_angle_id = self.active_angle_id;
        }
        if self.next_event_id != 0 {
            replay.next_event_id = self.next_event_id;
        }
        if self.next_playlist_id != 0 {
            replay.next_playlist_id = self.next_playlist_id;
        }
        for incoming in self.channels {
            if let Some(channel) = replay
                .channels
                .iter_mut()
                .find(|channel| channel.index == incoming.index)
            {
                channel.playlist_id = incoming.playlist_id;
            }
        }
        replay.timeline_cues = self.timeline_cues;
    }
}

/// True when playback moved across `boundary` since the previous frame.
///
/// Sitting on the boundary does not fire again. A jog that crosses it in either
/// direction fires once for that crossing.
pub fn boundary_crossed(previous: u64, current: u64, boundary: u64) -> bool {
    if previous == current {
        return false;
    }
    let low = previous.min(current);
    let high = previous.max(current);
    boundary > low && boundary <= high
}

/// Scene states whose boundary the playhead just crossed, in playback order.
///
/// A cue with a clip id fires at that clip's in point. A cue without one fires
/// at its own recording frame. Only forward movement fires: a rewind that crosses
/// a boundary again would run the states backwards, which is not what the operator
/// recorded them for.
///
/// `max_step` is the largest frame-to-frame move that counts as playback. A loop
/// back to the start of a playlist, a cue or a jump to live moves the playhead by
/// thousands of frames in one tick; firing every state in between made the program
/// and the scenes change by themselves at the loop point.
pub fn due_timeline_cues(
    previous: u64,
    current: u64,
    cues: &[ReplayTimelineCueDto],
    playlists: &[PlaylistDto],
    max_step: u64,
) -> Vec<(u64, u64)> {
    if current <= previous || current - previous > max_step {
        return Vec::new();
    }
    let mut due = Vec::new();
    for cue in cues {
        let boundary = if cue.clip_uid.is_empty() {
            cue.frame
        } else {
            let Some(frame) = playlists
                .iter()
                .flat_map(|playlist| playlist.clips.iter())
                .find(|clip| clip.uid == cue.clip_uid)
                .map(|clip| clip.in_frame)
            else {
                continue;
            };
            frame
        };
        if boundary > previous && boundary <= current {
            due.push((cue.scene_id, cue.state_id));
        }
    }
    due
}

/// A clip whose out point is before every segment that is still on disk was
/// overwritten. `None` means the recording is not open, which is a missing file
/// rather than an expiry.
pub fn clip_overwritten(out_frame: u64, earliest_remaining: Option<u64>) -> bool {
    earliest_remaining.is_some_and(|earliest| out_frame < earliest)
}

/// Passthrough stores the sender's bitstream. Playback stays refused.
pub fn passthrough_playback_refused(mode: AngleRecordMode) -> bool {
    mode == AngleRecordMode::Passthrough
}

pub fn replay_channel_gpu_id(index: u32) -> u64 {
    REPLAY_CHANNEL_BASE | u64::from(index.saturating_add(1))
}

pub fn channel_letter(index: u32) -> char {
    char::from_u32(u32::from('A') + index).unwrap_or('A')
}

/// Recording format a fresh Replay document starts with.
///
/// The resolution and rate come from the first Mixing Unit, because Replay records
/// one format for every angle and the operator is already working in that format.
/// A document created without one can never record, and the mixer would only be
/// able to say "the format is missing" much later.
pub fn seed_format(doc: &Document) -> Result<ReplayFormatDto, String> {
    let Some(unit) = doc.units.first() else {
        return Err("A Mixing Unit is required for Replay".to_string());
    };
    Ok(ReplayFormatDto {
        width: unit.width,
        height: unit.height,
        fps_num: unit.fps_num,
        fps_den: unit.fps_den,
        profile: ReplayProfile::Sq,
        segment_frames: DEFAULT_SEGMENT_FRAMES,
    })
}

/// Two seconds at 60 fps. The operator can change it in Replay Setup.
pub const DEFAULT_SEGMENT_FRAMES: u32 = 120;

/// Segment length bounds, in frames. A recording that quietly picks its own
/// segment length is not the recording the operator asked for.
pub const SEGMENT_FRAMES_MIN: u32 = 30;
pub const SEGMENT_FRAMES_MAX: u32 = 1800;
/// Encode queue bounds, in frames per angle.
pub const ENCODE_QUEUE_MIN: u32 = 2;
pub const ENCODE_QUEUE_MAX: u32 = 60;
/// Write buffer bounds, in MiB.
pub const WRITE_BUFFER_MIN_MB: u32 = 64;
pub const WRITE_BUFFER_MAX_MB: u32 = 8192;

/// Add one explicit Replay channel input. The first call creates the replay
/// document; no Replay state is present in a default session.
pub fn add_channel_input(doc: &mut Document) -> Result<u32, String> {
    if doc.replay.is_none() {
        let format = seed_format(doc)?;
        doc.replay = Some(ReplayDto {
            format,
            ..ReplayDto::default()
        });
    }
    let replay = doc.replay.get_or_insert_with(ReplayDto::default);
    replay.normalize();
    let index = (0..26u32)
        .find(|index| {
            !replay
                .channels
                .iter()
                .any(|channel| channel.index == *index)
        })
        .ok_or_else(|| "all Replay channels A-Z already exist".to_string())?;
    let id = replay_channel_gpu_id(index);
    if doc.inputs.iter().any(|input| input.id == id) {
        return Err(format!(
            "Replay channel {} input already exists",
            channel_letter(index)
        ));
    }
    replay.channels.push(ChannelDto {
        index,
        playlist_id: None,
        input_id: id,
    });
    replay.normalize();
    doc.inputs.push(replay_input(
        id,
        format!("Replay {}", channel_letter(index)),
    ));
    Ok(index)
}

/// Channel index an input id names, when it sits in the reserved band.
pub fn replay_channel_index(input_id: u64) -> Option<u32> {
    let offset = input_id.checked_sub(REPLAY_CHANNEL_BASE)?;
    let index = u32::try_from(offset.checked_sub(1)?).ok()?;
    (index < 26).then_some(index)
}

/// Gives an explicitly added Replay input the channel it owns.
///
/// A client can add the input itself (the session API's `UpsertInput`), and Replay
/// exists only because that input does, so the document follows the input instead
/// of the other way round. Idempotent: an input that already owns its channel is
/// left alone.
pub fn adopt_channel_input(doc: &mut Document, input_id: u64) -> Result<u32, String> {
    // The id decides the channel, so the letter the operator sees matches the
    // input they added. An id outside the band is not a Replay channel.
    let index = replay_channel_index(input_id)
        .ok_or_else(|| format!("input {input_id} is not a Replay channel id"))?;
    if doc.replay.is_none() {
        // The client added the input, so the document has to be usable: a format
        // seeded from the first Mixing Unit is what makes recording possible.
        let format = seed_format(doc)?;
        doc.replay = Some(ReplayDto {
            format,
            ..ReplayDto::default()
        });
    }
    let replay = doc.replay.get_or_insert_with(ReplayDto::default);
    replay.normalize();
    if let Some(channel) = replay
        .channels
        .iter()
        .find(|channel| channel.index == index)
    {
        if channel.input_id == input_id {
            return Ok(index);
        }
        return Err(format!(
            "Replay channel {} already belongs to another input",
            channel_letter(index)
        ));
    }
    replay.channels.push(ChannelDto {
        index,
        playlist_id: None,
        input_id,
    });
    replay.normalize();
    Ok(index)
}

/// Drops every animation cue that names a channel, because the channel it cues is
/// gone. A cue without its channel makes the session unloadable, so it goes with
/// the channel.
pub fn clear_cues_for_channel(doc: &mut Document, channel: u32) -> usize {
    let mut cleared = 0;
    for scene in &mut doc.scenes {
        for sequence in &mut scene.sequences {
            for step in &mut sequence.steps {
                if step
                    .replay
                    .as_ref()
                    .is_some_and(|action| action.channel == channel)
                {
                    step.replay = None;
                    cleared += 1;
                }
            }
        }
    }
    cleared
}

/// Drops every animation cue, for when Replay itself is removed. The step still
/// changes the scene; only the cue to a channel that no longer exists goes away.
pub fn clear_all_cues(doc: &mut Document) -> usize {
    let mut cleared = 0;
    for scene in &mut doc.scenes {
        for sequence in &mut scene.sequences {
            for step in &mut sequence.steps {
                if step.replay.take().is_some() {
                    cleared += 1;
                }
            }
        }
    }
    cleared
}

/// Remove the input's channel metadata. The caller decides whether removing
/// the last channel has been explicitly confirmed; this function removes the
/// whole optional replay document when it becomes empty.
pub fn remove_channel_input(doc: &mut Document, input_id: u64) -> bool {
    let Some(replay) = doc.replay.as_mut() else {
        return false;
    };
    let before = replay.channels.len();
    replay
        .channels
        .retain(|channel| channel.input_id != input_id);
    if before == replay.channels.len() {
        return false;
    }
    if replay.channels.is_empty() {
        doc.replay = None;
    }
    true
}

fn replay_input(id: u64, name: String) -> InputDto {
    InputDto {
        id,
        guid: format!("replay-{id:x}"),
        name,
        kind: InputKind::Replay,
        path_or_address: None,
        color_r: 0.0,
        color_g: 0.0,
        color_b: 0.0,
        scroll: false,
        tone_hz: 0.0,
        tone_level_dbfs: -20.0,
        audio_units: Vec::new(),
        gain: 1.0,
        mute: false,
        use_gpu: false,
        frame_buffer_frames: 1,
        bandwidth_save: Default::default(),
        keep_full_on_multiview: false,
        omt_quality: Default::default(),
        ndi_bandwidth: Default::default(),
        video_loop: false,
        video_play_when: Default::default(),
        video_restart_when: Default::default(),
        video_pause_when: Default::default(),
        capture_width: 0,
        capture_height: 0,
        capture_fps_num: 0,
        capture_fps_den: 0,
        tags: Vec::new(),
        mix_source: Default::default(),
        mix_target_id: 0,
        audio_capture_mode: Default::default(),
        audio_device_kind: Default::default(),
        audio_device_id: String::new(),
        audio_map_left: 0,
        audio_map_right: 1,
        audio_process_exe: String::new(),
        audio_process_aumid: String::new(),
        decklink_mode: String::new(),
    }
}

pub fn profile_code(profile: ReplayProfile) -> u32 {
    match profile {
        ReplayProfile::Hq => 0,
        ReplayProfile::Sq => 1,
        ReplayProfile::Lq => 2,
    }
}

pub fn overflow_code(policy: ReplayOverflowPolicy) -> u32 {
    match policy {
        ReplayOverflowPolicy::DropFrames => 0,
        ReplayOverflowPolicy::StopRecording => 1,
    }
}

pub fn retention_code(retention: ReplayRetention) -> u32 {
    match retention {
        ReplayRetention::StopWhenFull => 0,
        ReplayRetention::Overwrite => 1,
        ReplayRetention::OverwriteUnprotected => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(id: u64) -> ReplayEventDto {
        ReplayEventDto {
            id,
            recording_uuid: "rec".into(),
            in_frame: 10,
            out_frame: 20,
            name: format!("Replay {id}"),
            tags: Vec::new(),
            protected: false,
        }
    }

    #[test]
    fn normalize_repairs_a_counter_a_host_left_at_one() {
        // The win32 window builds the settings JSON without the counters, so the
        // next mark-out would otherwise reuse an id that already exists.
        let mut replay = ReplayDto::default();
        replay.events.push(event(1));
        replay.events.push(event(4));
        replay.playlists.push(PlaylistDto {
            id: 9,
            name: "Replay".into(),
            revision: 1,
            clips: Vec::new(),
            default_transition: ClipTransitionDto::default(),
        });
        replay.normalize();
        assert_eq!(replay.next_event_id, 5);
        assert_eq!(replay.next_playlist_id, 10);
    }

    #[test]
    fn reserve_event_id_never_repeats_an_existing_id() {
        let mut replay = ReplayDto::default();
        replay.next_event_id = 1;
        replay.events.push(event(7));
        assert_eq!(replay.reserve_event_id(), 8);
        assert_eq!(replay.reserve_event_id(), 9);
        assert_eq!(replay.next_event_id, 10);
    }

    #[test]
    fn channel_inputs_use_the_replay_band_only() {
        // A Mixing Unit is what gives the new document its recording format.
        let mut doc: Document =
            serde_json::from_str(r#"{"version":2,"units":[{"id":1,"name":"MU 1"}]}"#).unwrap();
        let index = add_channel_input(&mut doc).unwrap();
        assert_eq!(index, 0);
        let id = doc.inputs[0].id;
        assert!(id >= REPLAY_CHANNEL_BASE && id < REPLAY_CHANNEL_BASE + 0x1_0000);
        assert_eq!(channel_letter(index), 'A');
        assert_eq!(doc.inputs[0].kind, InputKind::Replay);
        // The document arrives ready to record: the format comes from the unit.
        let format = &doc.replay.as_ref().unwrap().format;
        assert_eq!(format.width, 1920);
        assert_eq!(format.height, 1080);
        assert_eq!(format.segment_frames, DEFAULT_SEGMENT_FRAMES);
    }

    /// A document with no Mixing Unit has no format to record in. Creating one
    /// anyway would leave a project that can never record, so it is refused with
    /// the reason.
    #[test]
    fn a_replay_document_needs_a_mixing_unit() {
        let mut doc: Document = serde_json::from_str(r#"{"version":2}"#).unwrap();
        let error = add_channel_input(&mut doc).unwrap_err();
        assert!(error.contains("Mixing Unit"), "{error}");
        assert!(doc.replay.is_none(), "nothing was created");
    }

    #[test]
    fn transition_accepts_the_bare_kind_name_and_the_full_form() {
        // Earlier sessions stored just the kind name.
        let bare: ClipTransitionDto = serde_json::from_str("\"Dissolve\"").unwrap();
        assert_eq!(bare.kind, ReplayTransitionKind::Dissolve);
        assert_eq!(bare.timing, ReplayTransitionTiming::Overwrap);
        assert_eq!(bare.duration_frames, 0);

        let full: ClipTransitionDto =
            serde_json::from_str(r#"{"kind":"Dissolve","timing":"Additive","durationFrames":24}"#)
                .unwrap();
        assert_eq!(full.kind, ReplayTransitionKind::Dissolve);
        assert_eq!(full.timing, ReplayTransitionTiming::Additive);
        assert_eq!(full.duration_frames, 24);
        assert_eq!(full.frames(), 24);
    }

    #[test]
    fn a_cut_never_occupies_timeline_frames() {
        let cut = ClipTransitionDto {
            kind: ReplayTransitionKind::Cut,
            direction: ReplayTransitionDirection::Left,
            timing: ReplayTransitionTiming::Additive,
            duration_frames: 30,
        };
        assert!(cut.is_cut());
        assert_eq!(cut.frames(), 0);

        let dissolve = ClipTransitionDto {
            kind: ReplayTransitionKind::Dissolve,
            ..cut
        };
        assert!(!dissolve.is_cut());
        assert_eq!(dissolve.frames(), 30);
    }

    /// The playlist's default transition is what the UI offers as "new clips", so
    /// a clip added by Mark Out has to come out with it.
    #[test]
    fn a_marked_out_clip_inherits_the_playlists_default_transition() {
        let mut replay = ReplayDto::default();
        replay.auto_add_to_playlist = true;
        replay.active_angle_id = Some(1);
        replay.angles.push(AngleDto {
            id: 1,
            name: "A".into(),
            source: crate::session::replay::AngleSourceKind::Scene,
            input_id: 0,
            scene_id: 1,
            audio: AngleAudio::Silent,
            audio_unit_id: 0,
            record_mode: AngleRecordMode::Composite,
            encoder: crate::session::replay::AngleEncoder::Gpu,
        });
        let default = ClipTransitionDto {
            direction: ReplayTransitionDirection::Left,
            kind: ReplayTransitionKind::Dissolve,
            timing: ReplayTransitionTiming::Additive,
            duration_frames: 25,
        };
        replay.playlists.push(PlaylistDto {
            id: 3,
            name: "Replay".into(),
            revision: 1,
            clips: Vec::new(),
            default_transition: default,
        });

        let id = replay.note_mark_out("rec", 10, 40, "Replay 1").unwrap();
        assert_eq!(id, 1);
        assert_eq!(replay.events.len(), 1);
        assert_eq!(replay.playlists[0].revision, 2, "the playlist changed");
        let clip = &replay.playlists[0].clips[0];
        assert_eq!(clip.transition, default, "the default transition applies");
        assert_eq!(clip.angle_id, 1);
        assert_eq!(clip.recording_uuid, "rec");
        assert_eq!((clip.in_frame, clip.out_frame), (10, 40));
    }

    /// Without the setting the event is still filed, and the playlist untouched:
    /// the operator asked for an event, not for a clip.
    #[test]
    fn a_marked_out_event_stays_out_of_the_playlist_when_not_asked() {
        let mut replay = ReplayDto::default();
        replay.auto_add_to_playlist = false;
        replay.playlists.push(PlaylistDto {
            id: 3,
            name: "Replay".into(),
            revision: 1,
            clips: Vec::new(),
            default_transition: ClipTransitionDto::default(),
        });

        replay.note_mark_out("rec", 0, 30, "Replay 1").unwrap();
        assert_eq!(replay.events.len(), 1);
        assert!(replay.playlists[0].clips.is_empty());
        assert_eq!(replay.playlists[0].revision, 1);
    }

    /// A mark taken without a name, as the API and an animation step do, still reads
    /// as an event: the deck shows the name it was given.
    #[test]
    fn a_marked_out_event_without_a_name_is_named_after_its_id() {
        let mut replay = ReplayDto::default();
        replay.auto_add_to_playlist = false;
        let id = replay.note_mark_out("rec", 0, 30, "   ").unwrap();
        assert_eq!(replay.events[0].name, format!("Replay {id}"));
        let id = replay.note_mark_out("rec", 40, 70, " Goal ").unwrap();
        assert_eq!(
            replay.events[1].name, "Goal",
            "a given name is kept, trimmed"
        );
        assert_ne!(replay.events[0].id, id);
    }

    /// One camera is not a choice: a document with a single angle adopts it, so
    /// automation that never presses a tile can still file clips. Two cameras stay
    /// unselected until the operator says which one.
    #[test]
    fn a_single_angle_becomes_the_selection() {
        let mut replay = ReplayDto::default();
        replay.angles.push(AngleDto {
            id: 4,
            name: "Cam".into(),
            source: crate::session::replay::AngleSourceKind::Scene,
            input_id: 0,
            scene_id: 1,
            audio: AngleAudio::Silent,
            audio_unit_id: 0,
            record_mode: AngleRecordMode::Composite,
            encoder: crate::session::replay::AngleEncoder::Cpu,
        });
        replay.normalize();
        assert_eq!(replay.active_angle_id, Some(4));
        // A stored selection that points at a deleted angle is repaired the same way.
        replay.active_angle_id = Some(9);
        replay.normalize();
        assert_eq!(replay.active_angle_id, Some(4));
        // Two cameras are a real choice.
        replay.angles.push(AngleDto {
            id: 5,
            name: "Cam 2".into(),
            source: crate::session::replay::AngleSourceKind::Scene,
            input_id: 0,
            scene_id: 2,
            audio: AngleAudio::Silent,
            audio_unit_id: 0,
            record_mode: AngleRecordMode::Composite,
            encoder: crate::session::replay::AngleEncoder::Cpu,
        });
        replay.active_angle_id = None;
        replay.normalize();
        assert_eq!(
            replay.active_angle_id, None,
            "two cameras must not be guessed"
        );
    }

    /// The clip's camera is the operator's selection. A document that never said
    /// which angle is watched cannot file one.
    #[test]
    fn a_marked_out_clip_needs_a_selected_angle() {
        let mut replay = ReplayDto::default();
        replay.auto_add_to_playlist = true;
        replay.angles.push(AngleDto {
            id: 1,
            name: "A".into(),
            source: crate::session::replay::AngleSourceKind::Scene,
            input_id: 0,
            scene_id: 1,
            audio: AngleAudio::Silent,
            audio_unit_id: 0,
            record_mode: AngleRecordMode::Composite,
            encoder: crate::session::replay::AngleEncoder::Gpu,
        });

        let error = replay.note_mark_out("rec", 0, 30, "Replay 1").unwrap_err();
        assert!(error.contains("angle"), "{error}");
        assert!(replay.playlists.is_empty(), "no playlist was invented");
        assert_eq!(replay.events.len(), 1, "the event is still filed");

        // A selection that names an angle the document does not have is refused
        // rather than filed against the wrong camera.
        replay.active_angle_id = Some(7);
        assert!(
            replay
                .note_mark_out("rec", 0, 30, "Replay 2")
                .unwrap_err()
                .contains("7")
        );
        assert!(replay.playlists.is_empty());
        assert_eq!(replay.events.len(), 2);
    }

    /// The first marked-out clip creates the playlist it is filed in.
    #[test]
    fn a_marked_out_clip_creates_the_playlist_it_needs() {
        let mut replay = ReplayDto::default();
        replay.auto_add_to_playlist = true;
        replay.active_angle_id = Some(2);
        replay.angles.push(AngleDto {
            id: 2,
            name: "B".into(),
            source: crate::session::replay::AngleSourceKind::Scene,
            input_id: 0,
            scene_id: 2,
            audio: AngleAudio::Silent,
            audio_unit_id: 0,
            record_mode: AngleRecordMode::Composite,
            encoder: crate::session::replay::AngleEncoder::Gpu,
        });

        replay.note_mark_out("rec", 5, 35, "Replay 1").unwrap();
        assert_eq!(replay.playlists.len(), 1);
        assert_eq!(replay.playlists[0].id, 1);
        assert_eq!(replay.playlists[0].clips.len(), 1);
        assert_eq!(replay.playlists[0].clips[0].angle_id, 2);
        assert!(
            replay.playlists[0].clips[0].preserve_pitch,
            "a new clip holds its pitch"
        );
    }

    #[test]
    fn a_deck_edit_leaves_settings_where_setup_put_them() {
        let mut replay = ReplayDto::default();
        replay.storage.push(ReplayStorageDto {
            path: "D:/replay".into(),
            angle_ids: vec![1],
        });
        replay.format.segment_frames = 300;
        replay.warnings.remaining_warn_secs = 90;
        replay.decode_ahead = false;
        replay.profiling = true;
        replay.channels.push(ChannelDto {
            index: 0,
            playlist_id: None,
            input_id: 7,
        });
        replay.events.push(event(3));
        let deck = ReplayDeckDto {
            events: vec![event(3), event(4)],
            playlists: vec![PlaylistDto {
                id: 2,
                name: "Show".into(),
                revision: 1,
                clips: Vec::new(),
                default_transition: ClipTransitionDto::default(),
            }],
            active_angle_id: Some(1),
            next_event_id: 5,
            next_playlist_id: 3,
            channels: vec![ChannelDto {
                index: 0,
                playlist_id: Some(2),
                input_id: 0,
            }],
            timeline_cues: Vec::new(),
        };
        deck.apply_to(&mut replay);
        assert_eq!(replay.storage[0].path, "D:/replay");
        assert_eq!(replay.format.segment_frames, 300);
        assert_eq!(replay.warnings.remaining_warn_secs, 90);
        assert!(!replay.decode_ahead);
        assert!(replay.profiling);
        assert_eq!(replay.channels.len(), 1);
        assert_eq!(replay.channels[0].input_id, 7);
        assert_eq!(replay.channels[0].playlist_id, Some(2));
        assert_eq!(replay.events.len(), 2);
    }

    #[test]
    fn applying_settings_leaves_the_deck_alone() {
        let mut replay = ReplayDto::default();
        replay.events.push(event(8));
        replay.next_event_id = 9;
        replay.channels.push(ChannelDto {
            index: 0,
            playlist_id: Some(1),
            input_id: 4,
        });
        let mut settings = ReplaySettingsDto::from_document(&replay);
        settings.storage.push(ReplayStorageDto {
            path: "E:/takes".into(),
            angle_ids: vec![1],
        });
        settings.warnings.remaining_critical_secs = 15;
        settings.apply_to(&mut replay);
        assert_eq!(replay.events.len(), 1);
        assert_eq!(replay.events[0].id, 8);
        assert_eq!(replay.next_event_id, 9);
        assert_eq!(replay.channels[0].playlist_id, Some(1));
        assert_eq!(replay.storage[0].path, "E:/takes");
        assert_eq!(replay.warnings.remaining_critical_secs, 15);
    }

    #[test]
    fn normalize_does_not_rewrite_the_operators_segment_length() {
        let mut replay = ReplayDto::default();
        replay.format.segment_frames = 300;
        replay.normalize();
        assert_eq!(replay.format.segment_frames, 300);
    }

    #[test]
    fn a_timeline_boundary_fires_once_per_crossing() {
        assert!(!boundary_crossed(10, 10, 12));
        assert!(boundary_crossed(10, 12, 12));
        assert!(!boundary_crossed(12, 12, 12));
        assert!(boundary_crossed(14, 11, 12));
        assert!(!boundary_crossed(13, 14, 12));
        let mut clip = ClipDto::default();
        clip.uid = "clip-a".into();
        clip.in_frame = 40;
        let playlist = PlaylistDto {
            id: 1,
            name: "Show".into(),
            revision: 1,
            clips: vec![clip],
            default_transition: ClipTransitionDto::default(),
        };
        let cues = vec![
            ReplayTimelineCueDto {
                id: 1,
                frame: 12,
                scene_id: 7,
                state_id: 3,
                clip_uid: String::new(),
            },
            ReplayTimelineCueDto {
                id: 2,
                frame: 0,
                scene_id: 8,
                state_id: 4,
                clip_uid: "clip-a".into(),
            },
        ];
        assert_eq!(
            due_timeline_cues(10, 12, &cues, &[playlist.clone()], 8),
            vec![(7, 3)]
        );
        assert!(due_timeline_cues(12, 12, &cues, &[playlist.clone()], 8).is_empty());
        // Playback order only: a rewind must not run the states backwards.
        assert!(due_timeline_cues(14, 11, &cues, &[playlist.clone()], 8).is_empty());
        // A loop back to the start, or a jump to live: the playhead moved across
        // the cue without playing over it, so no state fires.
        assert!(due_timeline_cues(10, 900, &cues, &[playlist.clone()], 8).is_empty());
        assert_eq!(
            due_timeline_cues(30, 40, &cues, &[playlist], 16),
            vec![(8, 4)]
        );
    }

    #[test]
    fn an_overwritten_range_is_expired_and_passthrough_does_not_play() {
        assert!(clip_overwritten(40, Some(50)));
        assert!(!clip_overwritten(50, Some(50)));
        assert!(!clip_overwritten(40, None));
        assert!(passthrough_playback_refused(AngleRecordMode::Passthrough));
        assert!(!passthrough_playback_refused(AngleRecordMode::Composite));
    }

    #[test]
    fn a_deck_snapshot_without_an_angle_keeps_the_selection() {
        // The remote deck sends the camera it has selected. A deck that never picked
        // one must not clear the choice another deck already made.
        let mut replay = ReplayDto::default();
        replay.angles = vec![
            AngleDto {
                id: 1,
                name: String::new(),
                source: AngleSourceKind::Scene,
                scene_id: 0,
                input_id: 0,
                audio: AngleAudio::Silent,
                audio_unit_id: 0,
                record_mode: AngleRecordMode::Composite,
                encoder: AngleEncoder::Cpu,
            },
            AngleDto {
                id: 2,
                name: String::new(),
                source: AngleSourceKind::Scene,
                scene_id: 0,
                input_id: 0,
                audio: AngleAudio::Silent,
                audio_unit_id: 0,
                record_mode: AngleRecordMode::Composite,
                encoder: AngleEncoder::Cpu,
            },
        ];
        replay.active_angle_id = Some(2);
        let deck = ReplayDeckDto {
            events: Vec::new(),
            playlists: Vec::new(),
            active_angle_id: None,
            next_event_id: 0,
            next_playlist_id: 0,
            channels: Vec::new(),
            timeline_cues: Vec::new(),
        };
        deck.apply_to(&mut replay);
        assert_eq!(replay.active_angle_id, Some(2));
    }

    #[test]
    fn a_recording_is_listed_once_and_never_without_an_id() {
        let mut replay = ReplayDto::default();
        replay.recordings = vec![
            ReplayRecordingDto {
                uuid: "rec-a".into(),
                ..Default::default()
            },
            ReplayRecordingDto {
                uuid: "rec-a".into(),
                ..Default::default()
            },
            ReplayRecordingDto {
                uuid: "   ".into(),
                ..Default::default()
            },
        ];
        replay.normalize();
        assert_eq!(replay.recordings.len(), 1);
        assert_eq!(replay.recordings[0].uuid, "rec-a");
    }
    #[test]
    fn a_missing_preserve_pitch_field_defaults_on() {
        let clip: ClipDto =
            serde_json::from_str(r#"{"uid":"c","inFrame":0,"outFrame":10}"#).unwrap();
        assert!(clip.preserve_pitch);
    }
}
