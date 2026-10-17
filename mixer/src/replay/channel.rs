//! Channel transport. One step is one master frame.
//!
//! A channel plays a playlist snapshot. The clip that is on air is an immutable
//! snapshot, so editing the playlist from another channel can never corrupt the
//! picture that is already going out; the clip that follows always comes from the
//! newest list.

/// How a transition spends its time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransitionTiming {
    /// The two clips overlap, so the transition is taken out of both.
    Overwrap,
    /// The transition is inserted between the clips, holding the last frame of
    /// the outgoing clip and the first frame of the incoming one.
    Additive,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClipTransition {
    pub dissolve: bool,
    pub timing: TransitionTiming,
    pub frames: u32,
}

impl ClipTransition {
    pub const CUT: Self = Self {
        dissolve: false,
        timing: TransitionTiming::Overwrap,
        frames: 0,
    };

    /// Frames the transition occupies on the timeline.
    pub fn span(self) -> u32 {
        if self.dissolve {
            self.frames
        } else {
            0
        }
    }
}

/// Where a clip's audio comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipAudioSource {
    /// Whatever the clip's own angle recorded.
    Follow,
    Silent,
    /// Another angle's recording.
    Angle(u32),
}

/// Immutable snapshot of one playlist clip.
#[derive(Clone, Debug, PartialEq)]
pub struct Clip {
    pub uid: String,
    pub recording_uuid: String,
    pub angle_id: u32,
    pub in_frame: u64,
    pub out_frame: u64,
    pub speed: f64,
    /// Keep the pitch while the speed changes.
    pub preserve_pitch: bool,
    /// Clip gain, linear. Comes from the session's dB value.
    pub gain: f32,
    pub audio: ClipAudioSource,
    pub transition: ClipTransition,
}

impl Clip {
    /// Angle whose recording supplies this clip's audio. `None` means silence.
    pub fn audio_angle(&self) -> Option<u32> {
        match self.audio {
            ClipAudioSource::Follow => Some(self.angle_id),
            ClipAudioSource::Silent => None,
            ClipAudioSource::Angle(id) => Some(id),
        }
    }
}

impl Clip {
    /// Output frames the clip occupies on its own, before any transition.
    /// `in_frame` and `out_frame` are both inclusive, so a span of 2 is 3 frames.
    pub fn length(&self) -> f64 {
        let span = (self.out_frame as f64 - self.in_frame as f64).abs() + 1.0;
        let speed = self.speed.abs().max(1e-6);
        (span / speed).max(1.0)
    }

    /// Source frame `offset` output frames into the clip.
    /// Fractional source position, so playback audio can interpolate instead of
    /// stepping a whole frame at slow motion.
    pub fn source_position(&self, offset: f64) -> f64 {
        let speed = if self.speed.abs() < 1e-6 {
            1.0
        } else {
            self.speed
        };
        let value = self.in_frame as f64 + offset * speed;
        let low = self.in_frame.min(self.out_frame) as f64;
        let high = self.in_frame.max(self.out_frame) as f64;
        value.clamp(low, high).max(0.0)
    }

    pub fn source(&self, offset: f64) -> u64 {
        self.source_position(offset).round() as u64
    }
}

/// The frames one output frame is made of.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FramePlan {
    pub primary: Option<(u32, u64)>,
    pub secondary: Option<(u32, u64)>,
    /// 0 shows only `primary`, 1 shows only `secondary`.
    pub mix: f64,
}

#[derive(Clone, Debug)]
pub struct Cue {
    pub recording_uuid: String,
    pub angle_id: u32,
    pub in_frame: u64,
    pub out_frame: u64,
}

#[derive(Clone, Debug)]
pub struct Transport {
    /// The clip that is on air.
    current: Option<Clip>,
    /// Newest playlist snapshot, rebuilt whenever the session publishes one.
    list: Vec<Clip>,
    /// Where `current` sat in the list when it was picked.
    index: usize,
    /// Set when the on-air clip is gone from the list, so the clip that took its
    /// place is the one to play next.
    resume: Option<usize>,
    /// Output frames into the current clip's body.
    position: f64,
    /// Playout rate. Below 1 is slow motion.
    rate: f64,
    pub playing: bool,
    /// Follow the recording edge instead of a clip.
    live: bool,
    live_angle: u32,
    /// Frames the live view has fallen behind the recording edge.
    lag: f64,
    last_frame: u64,
    pub substituted: u64,
    pub decode_us_sum: u64,
    pub decode_samples: u64,
    shown: Option<FramePlan>,
}

impl Transport {
    pub fn new() -> Self {
        Self {
            current: None,
            list: Vec::new(),
            index: 0,
            resume: None,
            position: 0.0,
            rate: 1.0,
            playing: false,
            live: false,
            live_angle: 0,
            lag: 0.0,
            last_frame: 0,
            substituted: 0,
            decode_us_sum: 0,
            decode_samples: 0,
            shown: None,
        }
    }

    pub fn rate(&self) -> f64 {
        self.rate
    }

    pub fn is_live(&self) -> bool {
        self.live
    }

    /// Plays one clip that is not part of a playlist. The Replay API still needs
    /// this path.
    pub fn cue(&mut self, cue: Cue) {
        self.set_clip(
            Clip {
                uid: format!("cue-{}", cue.angle_id),
                recording_uuid: cue.recording_uuid,
                angle_id: cue.angle_id,
                in_frame: cue.in_frame,
                out_frame: cue.out_frame,
                speed: 1.0,
                preserve_pitch: false,
                audio: ClipAudioSource::Follow,
                gain: 1.0,
                transition: ClipTransition::CUT,
            },
            0,
        );
        self.list = Vec::new();
    }

    /// Clips on the channel, newest last. Used to resolve a cue by name.
    pub fn clips(&self) -> &[Clip] {
        &self.list
    }

    pub fn set_clip(&mut self, clip: Clip, index: usize) {
        self.current = Some(clip);
        self.index = index;
        self.resume = None;
        self.position = 0.0;
        self.live = false;
        self.lag = 0.0;
        self.playing = false;
        self.shown = None;
    }

    /// Publishes a new playlist. The on-air clip keeps playing from its snapshot;
    /// only what comes after it changes.
    pub fn set_playlist(&mut self, clips: Vec<Clip>) {
        let uid = self.current.as_ref().map(|clip| clip.uid.clone());
        self.resume = None;
        self.list = clips;
        if let Some(uid) = uid {
            match self.list.iter().position(|clip| clip.uid == uid) {
                Some(index) => self.index = index,
                // The clip that is on air was removed. Play it out, then resume
                // where it used to sit.
                None => self.resume = Some(self.index.min(self.list.len().saturating_sub(1))),
            }
        }
    }

    pub fn set_rate(&mut self, rate: f64) {
        if rate.abs() > 1e-6 {
            self.rate = rate;
        }
    }

    pub fn set_playing(&mut self, playing: bool) {
        self.playing = playing;
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// True while the picture is held past the clip's end, which is how an
    /// additive transition spends its inserted duration.
    pub fn is_holding(&self) -> bool {
        self.current
            .as_ref()
            .is_some_and(|clip| self.position >= clip.length())
    }

    /// Seeks inside the current clip to an absolute source frame.
    pub fn seek_source(&mut self, frame: u64) {
        if let Some(current) = &self.current {
            let speed = if current.speed.abs() < 1e-6 {
                1.0
            } else {
                current.speed
            };
            self.position = ((frame as f64 - current.in_frame as f64) / speed).max(0.0);
            self.shown = None;
        }
    }

    /// Follows the recording edge. `angle` is used when nothing is cued yet.
    pub fn jump_to_now(&mut self, last_frame: u64, angle: u32) {
        self.last_frame = last_frame;
        self.live = true;
        self.live_angle = self
            .current
            .as_ref()
            .map(|clip| clip.angle_id)
            .filter(|angle| *angle != 0)
            .unwrap_or(angle);
        self.lag = 0.0;
        self.shown = None;
    }

    pub fn note_recorded_frame(&mut self, last_frame: u64) {
        self.last_frame = last_frame;
    }

    pub fn clip(&self) -> Option<&Clip> {
        self.current.as_ref()
    }

    /// Frames the current clip plus its inserted transition occupies.
    fn span(&self) -> f64 {
        let Some(current) = &self.current else {
            return 0.0;
        };
        let frames = current.transition.span() as f64;
        let length = current.length();
        if frames <= 0.0 || frames >= length || self.peek().is_none() {
            return length;
        }
        match current.transition.timing {
            TransitionTiming::Overwrap => length,
            TransitionTiming::Additive => length + frames,
        }
    }

    fn peek(&self) -> Option<&Clip> {
        if let Some(index) = self.resume {
            return self.list.get(index);
        }
        self.list.get(self.index + 1)
    }

    fn take_next(&mut self) -> Option<Clip> {
        let index = self.resume.take().unwrap_or(self.index + 1);
        let clip = self.list.get(index).cloned()?;
        self.index = index;
        Some(clip)
    }

    fn plan(&self) -> Option<FramePlan> {
        let current = self.current.as_ref()?;
        let length = current.length();
        let frames = current.transition.span() as f64;
        let next = self.peek();
        if frames > 0.0 && frames < length && next.is_some() {
            match current.transition.timing {
                TransitionTiming::Overwrap => {
                    let start = length - frames;
                    if self.position >= start {
                        let mix = ((self.position - start) / frames).clamp(0.0, 1.0);
                        let next = next.expect("checked above");
                        return Some(FramePlan {
                            primary: Some((current.angle_id, current.source(self.position))),
                            secondary: Some((next.angle_id, next.source(self.position - start))),
                            mix,
                        });
                    }
                }
                TransitionTiming::Additive => {
                    if self.position >= length {
                        let mix = ((self.position - length) / frames).clamp(0.0, 1.0);
                        let next = next.expect("checked above");
                        return Some(FramePlan {
                            primary: Some((current.angle_id, current.source(length))),
                            secondary: Some((next.angle_id, next.source(0.0))),
                            mix,
                        });
                    }
                }
            }
        }
        Some(FramePlan {
            primary: Some((current.angle_id, current.source(self.position))),
            secondary: None,
            mix: 0.0,
        })
    }

    fn advance(&mut self, dt: f64) {
        self.position += dt;
        let mut guard = 0;
        while self.position >= self.span() && guard < 64 {
            let span = self.span();
            let carry = match self.current.as_ref().map(|clip| clip.transition) {
                Some(transition)
                    if transition.span() > 0
                        && transition.timing == TransitionTiming::Overwrap =>
                {
                    transition.span() as f64
                }
                _ => 0.0,
            };
            match self.take_next() {
                Some(clip) => {
                    self.current = Some(clip);
                    // The overlap already consumed `carry` frames of the new clip.
                    self.position = (self.position - span).max(0.0) + carry;
                    self.shown = None;
                }
                None => {
                    self.position = span;
                    self.playing = false;
                    break;
                }
            }
            guard += 1;
        }
    }

    /// Advances one output frame and returns what to show. `None` means "keep the
    /// picture that is already on screen", which is how pause and jog hold
    /// without flashing black.
    pub fn step(&mut self, dt: f64) -> Option<FramePlan> {
        if self.live {
            if self.playing {
                // A rate below 1 lets the view fall behind the recording edge.
                self.lag = (self.lag + (1.0 - self.rate).max(0.0) * dt).max(0.0);
            }
            let edge = self.last_frame.saturating_sub(3) as f64;
            let frame = (edge - self.lag).max(0.0).round() as u64;
            let plan = FramePlan {
                primary: (self.live_angle != 0).then_some((self.live_angle, frame)),
                secondary: None,
                mix: 0.0,
            };
            if plan.primary.is_none() || self.shown.as_ref() == Some(&plan) {
                return None;
            }
            self.shown = Some(plan.clone());
            return Some(plan);
        }

        self.current.as_ref()?;
        // Resolve this output frame before advancing, so the clip's first frame
        // is actually shown.
        let plan = self.plan()?;
        let changed = self.shown.as_ref() != Some(&plan);
        if self.playing && dt > 0.0 {
            self.advance(dt);
        }
        if !changed {
            return None;
        }
        self.shown = Some(plan.clone());
        Some(plan)
    }

    /// Source frame the channel is showing, for stats and the API.
    pub fn source_frame(&self) -> u64 {
        if self.live {
            // The live view sits a few frames behind the write head so the frames
            // it asks for are already on disk.
            return (self.last_frame as f64 - 3.0 - self.lag).max(0.0) as u64;
        }
        self.current
            .as_ref()
            .map(|clip| clip.source(self.position))
            .unwrap_or(0)
    }

    /// Fractional source position, used to place playback audio between frames.
    pub fn source_position(&self) -> f64 {
        if self.live {
            return (self.last_frame as f64 - 3.0 - self.lag).max(0.0);
        }
        self.current
            .as_ref()
            .map(|clip| clip.source_position(self.position))
            .unwrap_or(0.0)
    }

    /// Source frames per output frame, signed: negative plays backwards.
    pub fn playback_speed(&self) -> f64 {
        let clip_speed = self
            .current
            .as_ref()
            .map(|clip| clip.speed)
            .unwrap_or(1.0);
        if clip_speed.abs() < 1e-6 {
            1.0
        } else {
            clip_speed * self.rate
        }
    }

    /// True when the on-air clip asked for its pitch to be held.
    pub fn preserves_pitch(&self) -> bool {
        self.current
            .as_ref()
            .map(|clip| clip.preserve_pitch)
            .unwrap_or(false)
    }
}

impl Default for Transport {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(uid: &str, in_frame: u64, out_frame: u64, transition: ClipTransition) -> Clip {
        Clip {
            uid: uid.into(),
            recording_uuid: "rec".into(),
            angle_id: 1,
            in_frame,
            out_frame,
            speed: 1.0,
            preserve_pitch: false,
            gain: 1.0,
            audio: ClipAudioSource::Follow,
            transition,
        }
    }

    #[test]
    fn a_clip_resolves_its_audio_angle() {
        let mut clip = clip("a", 0, 10, ClipTransition::CUT);
        // Following the angle means following the picture.
        assert_eq!(clip.audio_angle(), Some(1));
        clip.audio = ClipAudioSource::Silent;
        assert_eq!(clip.audio_angle(), None);
        clip.audio = ClipAudioSource::Angle(3);
        assert_eq!(clip.audio_angle(), Some(3));
    }

    fn dissolve(timing: TransitionTiming, frames: u32) -> ClipTransition {
        ClipTransition {
            dissolve: true,
            timing,
            frames,
        }
    }

    /// Plays the playlist out and returns one entry per output frame it produced.
    /// A step that repaints the same picture still consumed a frame, so every step
    /// while the transport is playing is counted.
    fn play_out(transport: &mut Transport) -> Vec<FramePlan> {
        let mut frames = Vec::new();
        while transport.is_playing() && frames.len() < 500 {
            match transport.step(1.0) {
                Some(plan) => frames.push(plan),
                None => frames.push(FramePlan::default()),
            }
        }
        frames
    }

    #[test]
    fn forward_shows_every_frame_and_stops_at_the_end() {
        let mut transport = Transport::new();
        transport.cue(Cue {
            recording_uuid: "rec".into(),
            angle_id: 1,
            in_frame: 10,
            out_frame: 12,
        });
        transport.set_playing(true);
        assert_eq!(transport.step(1.0).unwrap().primary, Some((1, 10)));
        assert_eq!(transport.step(1.0).unwrap().primary, Some((1, 11)));
        assert_eq!(transport.step(1.0).unwrap().primary, Some((1, 12)));
        // The clip is over: the picture holds and playback stops.
        assert!(transport.step(1.0).is_none());
        assert!(!transport.playing);
    }

    #[test]
    fn pause_and_slow_motion_hold_the_picture_instead_of_going_black() {
        let mut transport = Transport::new();
        transport.cue(Cue {
            recording_uuid: "rec".into(),
            angle_id: 1,
            in_frame: 40,
            out_frame: 60,
        });
        transport.set_playing(true);
        assert_eq!(transport.step(1.0).unwrap().primary, Some((1, 40)));
        // Pausing shows the frame the transport already stepped into, then holds
        // instead of clearing the output.
        transport.set_playing(false);
        assert_eq!(transport.step(0.0).unwrap().primary, Some((1, 41)));
        assert!(transport.step(0.0).is_none());
        assert!(transport.step(0.0).is_none());
        // Half speed repaints only when the rounded source frame changes.
        transport.set_rate(0.5);
        transport.set_playing(true);
        assert!(transport.step(1.0).is_none());
        assert_eq!(transport.step(1.0).unwrap().primary, Some((1, 42)));
    }

    #[test]
    fn playlist_plays_on_to_the_next_clip() {
        let mut transport = Transport::new();
        let a = clip("a", 0, 2, ClipTransition::CUT);
        transport.set_clip(a.clone(), 0);
        transport.set_playlist(vec![a, clip("b", 10, 12, ClipTransition::CUT)]);
        transport.set_playing(true);
        assert_eq!(transport.step(1.0).unwrap().primary, Some((1, 0)));
        assert_eq!(transport.step(1.0).unwrap().primary, Some((1, 1)));
        assert_eq!(transport.step(1.0).unwrap().primary, Some((1, 2)));
        assert_eq!(transport.step(1.0).unwrap().primary, Some((1, 10)));
    }

    #[test]
    fn a_clip_appended_from_another_channel_is_picked_up() {
        let mut transport = Transport::new();
        let first = clip("a", 0, 2, ClipTransition::CUT);
        transport.set_clip(first.clone(), 0);
        transport.set_playlist(vec![first.clone()]);
        transport.set_playing(true);
        transport.step(1.0);
        transport.step(1.0);
        // Another channel appends Y while X is still playing.
        transport.set_playlist(vec![first, clip("y", 100, 102, ClipTransition::CUT)]);
        transport.step(1.0);
        assert_eq!(transport.step(1.0).unwrap().primary, Some((1, 100)));
    }

    #[test]
    fn overwrap_overlaps_by_taking_time_from_both_clips() {
        let mut transport = Transport::new();
        let a = clip("a", 0, 20, dissolve(TransitionTiming::Overwrap, 4));
        transport.set_clip(a.clone(), 0);
        transport.set_playlist(vec![a, clip("b", 100, 120, ClipTransition::CUT)]);
        transport.set_playing(true);
        let mut overlapped = None;
        let mut frames = 0;
        while let Some(plan) = transport.step(1.0) {
            if plan.secondary.is_some() {
                overlapped = Some(plan);
            }
            frames += 1;
            if frames > 60 {
                break;
            }
        }
        let plan = overlapped.expect("the overlap must produce two layers");
        assert!(plan.mix > 0.0 && plan.mix < 1.0);
        // A's own length is the budget: nothing was added to the timeline.
        assert_eq!(transport.clip().unwrap().uid, "b");
    }

    #[test]
    fn additive_inserts_the_duration_and_holds_both_edges() {
        let mut transport = Transport::new();
        let a = clip("a", 0, 20, dissolve(TransitionTiming::Additive, 4));
        transport.set_clip(a.clone(), 0);
        transport.set_playlist(vec![a, clip("b", 100, 120, ClipTransition::CUT)]);
        transport.set_playing(true);
        let mut plans = Vec::new();
        for _ in 0..30 {
            match transport.step(1.0) {
                Some(plan) => plans.push(plan),
                None => break,
            }
        }
        let inserted: Vec<_> = plans
            .iter()
            .filter(|plan| plan.secondary.is_some())
            .collect();
        assert_eq!(inserted.len(), 4, "the insertion lasts exactly its duration");
        assert!(inserted.iter().all(|plan| plan.primary == Some((1, 20))));
        assert!(inserted.iter().all(|plan| plan.secondary == Some((1, 100))));
        assert_eq!(inserted[0].mix, 0.0);
        assert_eq!(inserted[3].mix, 0.75);
    }

    /// The two timings differ in exactly one thing: where the transition's frames
    /// come from. Overwrap shares them between the clips, Additive inserts them,
    /// so the playlist is `A + Duration + B` long.
    #[test]
    fn the_timing_decides_whether_the_transition_extends_the_timeline() {
        let a = || clip("a", 0, 20, dissolve(TransitionTiming::Overwrap, 4));
        let additive = || clip("a", 0, 20, dissolve(TransitionTiming::Additive, 4));

        // Overwrap: A (21 frames) and B (21) share the 4 transition frames.
        let mut transport = Transport::new();
        let first = a();
        transport.set_clip(first.clone(), 0);
        transport.set_playlist(vec![first, clip("b", 100, 120, ClipTransition::CUT)]);
        transport.set_playing(true);
        let frames = play_out(&mut transport);
        assert_eq!(frames.len(), 21 + 21 - 4, "overwrap takes time from both");
        assert_eq!(transport.clip().unwrap().uid, "b");
        assert_eq!(transport.playing, false, "the playlist ran out");

        // Additive: the same clips, with the 4 frames inserted between them.
        let mut transport = Transport::new();
        let first = additive();
        transport.set_clip(first.clone(), 0);
        transport.set_playlist(vec![first, clip("b", 100, 120, ClipTransition::CUT)]);
        transport.set_playing(true);
        let frames = play_out(&mut transport);
        assert_eq!(frames.len(), 21 + 4 + 21, "additive inserts its duration");
        // B plays in full after the insertion, from its own first frame on.
        let after: Vec<u64> = frames
            .iter()
            .filter(|plan| plan.primary == Some((1, 100)) && plan.secondary.is_none())
            .map(|plan| plan.primary.unwrap().1)
            .collect();
        assert_eq!(after.len(), 1, "B's first frame is shown exactly once");
    }

    #[test]
    fn a_removed_clip_plays_out_then_resumes_at_the_next_one() {
        let mut transport = Transport::new();
        let a = clip("a", 0, 4, ClipTransition::CUT);
        let b = clip("b", 10, 14, ClipTransition::CUT);
        let c = clip("c", 20, 24, ClipTransition::CUT);
        transport.set_clip(a.clone(), 0);
        transport.set_playlist(vec![a.clone(), b, c.clone()]);
        transport.set_playing(true);
        transport.step(1.0);
        // B is deleted by another channel while A is on air.
        transport.set_playlist(vec![a, c.clone()]);
        let mut frames = Vec::new();
        for _ in 0..12 {
            if let Some(plan) = transport.step(1.0) {
                frames.push(plan.primary.unwrap().1);
            }
        }
        assert!(
            frames.contains(&c.in_frame),
            "C must play after A: {frames:?}"
        );
    }

    #[test]
    fn live_falls_behind_the_recording_edge_at_slow_motion() {
        let mut transport = Transport::new();
        transport.jump_to_now(1000, 6);
        transport.set_playing(true);
        let first = transport.step(1.0).unwrap();
        assert_eq!(first.primary, Some((6, 997)));
        // At full rate the view stays pinned to the edge.
        for _ in 0..10 {
            transport.note_recorded_frame(1010);
            transport.step(1.0);
        }
        assert_eq!(transport.source_frame(), 1007);
        // Half speed drifts behind it without leaving the live feed.
        transport.set_rate(0.5);
        for _ in 0..10 {
            transport.step(1.0);
        }
        assert!(transport.source_frame() < 1007, "{}", transport.source_frame());
    }
}


