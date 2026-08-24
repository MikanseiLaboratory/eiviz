use eiviz_core::Playback;
use eiviz_media::{MediaError, Result};
use shiguredo_mp4::{
    BoxHeader, BoxSize, Decode,
    boxes::{ElstEntry, MoovBox},
};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct TrackEdit {
    pub media_start: u64,
    pub presentation_start_us: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct MovieTimeline {
    pub duration_us: u64,
    pub movie_timescale: u32,
    pub track_edits: HashMap<u32, TrackEdit>,
}

impl MovieTimeline {
    pub fn edit(&self, track_id: u32) -> TrackEdit {
        self.track_edits.get(&track_id).copied().unwrap_or_default()
    }

    pub fn presentation_ticks(
        &self,
        media_ticks: i64,
        track_timescale: u32,
        edit: TrackEdit,
    ) -> Result<i64> {
        let presentation_offset = scale_u64(
            edit.presentation_start_us,
            track_timescale as u64,
            1_000_000,
        )?;
        media_ticks
            .checked_sub(
                i64::try_from(edit.media_start).map_err(|_| {
                    MediaError::Other("MP4 edit-list media start exceeds i64".into())
                })?,
            )
            .and_then(|value| value.checked_add(i64::try_from(presentation_offset).ok()?))
            .ok_or_else(|| MediaError::Other("MP4 presentation timestamp overflow".into()))
    }
}

pub(crate) fn parse_movie_timeline(bytes: &[u8]) -> Result<MovieTimeline> {
    let mut offset = 0usize;
    let mut moov = None;
    while offset < bytes.len() {
        let remaining = &bytes[offset..];
        let (header, _) = BoxHeader::decode(remaining)
            .map_err(|error| MediaError::Other(format!("MP4 box header: {error}")))?;
        let box_len = if header.box_size == BoxSize::VARIABLE_SIZE {
            remaining.len()
        } else {
            usize::try_from(header.box_size.get())
                .map_err(|_| MediaError::Other("MP4 box size overflow".into()))?
        };
        if box_len < header.external_size() || box_len > remaining.len() {
            return Err(MediaError::Other("truncated MP4 top-level box".into()));
        }
        if header.box_type == MoovBox::TYPE {
            if moov.is_some() {
                return Err(MediaError::Unsupported(
                    "multiple MP4 movie boxes are unsupported".into(),
                ));
            }
            let (parsed, consumed) = MoovBox::decode(&remaining[..box_len])
                .map_err(|error| MediaError::Other(format!("MP4 movie box: {error}")))?;
            if consumed != box_len {
                return Err(MediaError::Other(
                    "MP4 movie box was not fully consumed".into(),
                ));
            }
            moov = Some(parsed);
        }
        offset = offset
            .checked_add(box_len)
            .ok_or_else(|| MediaError::Other("MP4 box offset overflow".into()))?;
    }
    let moov = moov.ok_or_else(|| MediaError::Unsupported("MP4 movie box is missing".into()))?;
    let movie_timescale = moov.mvhd_box.timescale.get();
    let duration_us = scale_u64(
        moov.mvhd_box.duration,
        1_000_000,
        u64::from(movie_timescale),
    )?;
    let mut track_edits = HashMap::new();
    for track in &moov.trak_boxes {
        let Some(entries) = track
            .edts_box
            .as_ref()
            .and_then(|edts| edts.elst_box.as_ref())
            .map(|elst| elst.entries.as_slice())
        else {
            continue;
        };
        let edit = normalize_edit_entries(entries, movie_timescale)?;
        track_edits.insert(track.tkhd_box.track_id, edit);
    }
    Ok(MovieTimeline {
        duration_us,
        movie_timescale,
        track_edits,
    })
}

fn normalize_edit_entries(entries: &[ElstEntry], movie_timescale: u32) -> Result<TrackEdit> {
    let mut presentation_start = 0u64;
    let mut media_start = None;
    for entry in entries {
        if entry.media_rate.integer != 1 || entry.media_rate.fraction != 0 {
            return Err(MediaError::Unsupported(
                "MP4 edit-list rates other than 1.0 are unsupported".into(),
            ));
        }
        if entry.media_time < 0 {
            if media_start.is_some() {
                return Err(MediaError::Unsupported(
                    "MP4 empty edit after a media edit is unsupported".into(),
                ));
            }
            presentation_start = presentation_start
                .checked_add(entry.edit_duration)
                .ok_or_else(|| MediaError::Other("MP4 edit duration overflow".into()))?;
        } else if media_start
            .replace(
                u64::try_from(entry.media_time).map_err(|_| {
                    MediaError::Other("MP4 edit media time conversion failed".into())
                })?,
            )
            .is_some()
        {
            return Err(MediaError::Unsupported(
                "multiple MP4 media edits are unsupported".into(),
            ));
        }
    }
    if !entries.is_empty() && media_start.is_none() {
        return Err(MediaError::Unsupported(
            "MP4 edit list contains no media edit".into(),
        ));
    }
    Ok(TrackEdit {
        media_start: media_start.unwrap_or(0),
        presentation_start_us: scale_u64(
            presentation_start,
            1_000_000,
            u64::from(movie_timescale),
        )?,
    })
}

pub(crate) fn scale_u64(value: u64, numerator: u64, denominator: u64) -> Result<u64> {
    if denominator == 0 {
        return Err(MediaError::Other("zero MP4 timescale".into()));
    }
    let scaled = u128::from(value)
        .checked_mul(u128::from(numerator))
        .ok_or_else(|| MediaError::Other("MP4 timestamp overflow".into()))?
        / u128::from(denominator);
    u64::try_from(scaled).map_err(|_| MediaError::Other("MP4 timestamp overflow".into()))
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TimelineStep {
    pub position_us: u64,
    pub generation: u64,
}

#[derive(Debug)]
pub(crate) struct PlaybackTimeline {
    config: Playback,
    media_end_us: u64,
    position_us: u64,
    last_clock_us: Option<u64>,
    fractional_us: f64,
    generation: u64,
}

impl PlaybackTimeline {
    pub fn new(config: Playback, media_end_us: u64, has_audio: bool) -> Result<Self> {
        validate_playback(&config, media_end_us, has_audio)?;
        let position_us = clamp_position(&config, media_end_us, config.position_us);
        Ok(Self {
            config,
            media_end_us,
            position_us,
            last_clock_us: None,
            fractional_us: 0.0,
            generation: 1,
        })
    }

    pub fn apply(&mut self, config: Playback, has_audio: bool) -> Result<()> {
        validate_playback(&config, self.media_end_us, has_audio)?;
        if config.position_us != self.config.position_us
            || config.in_us != self.config.in_us
            || config.out_us != self.config.out_us
        {
            self.position_us = clamp_position(&config, self.media_end_us, config.position_us);
            self.fractional_us = 0.0;
            self.last_clock_us = None;
            self.generation = self.generation.saturating_add(1);
        }
        self.config = config;
        Ok(())
    }

    pub fn playback(&self) -> Playback {
        let mut playback = self.config.clone();
        playback.position_us = self.position_us;
        playback
    }

    pub fn resolve(&mut self, clock_us: u64) -> TimelineStep {
        let elapsed = match self.last_clock_us {
            Some(last) if clock_us > last => {
                self.last_clock_us = Some(clock_us);
                clock_us - last
            }
            Some(_) => 0,
            None => {
                self.last_clock_us = Some(clock_us);
                0
            }
        };
        if self.config.playing && elapsed > 0 {
            let scaled = elapsed as f64 * f64::from(self.config.speed) + self.fractional_us;
            let advance = scaled.floor().max(0.0) as u64;
            self.fractional_us = scaled - advance as f64;
            let end = playback_end(&self.config, self.media_end_us);
            let next = self.position_us.saturating_add(advance);
            if next >= end {
                if self.config.loop_playback {
                    let length = end.saturating_sub(self.config.in_us);
                    self.position_us = if length == 0 {
                        self.config.in_us
                    } else {
                        self.config.in_us + next.saturating_sub(self.config.in_us) % length
                    };
                    self.generation = self.generation.saturating_add(1);
                } else {
                    self.position_us = end.saturating_sub(1).max(self.config.in_us);
                    self.config.playing = false;
                }
            } else {
                self.position_us = next;
            }
        }
        TimelineStep {
            position_us: self.position_us,
            generation: self.generation,
        }
    }
}

fn validate_playback(playback: &Playback, media_end_us: u64, has_audio: bool) -> Result<()> {
    if !playback.speed.is_finite() || playback.speed <= 0.0 {
        return Err(MediaError::Unsupported(
            "file playback speed must be finite and greater than zero".into(),
        ));
    }
    if has_audio && playback.speed != 1.0 {
        return Err(MediaError::Unsupported(
            "A/V file playback requires speed 1.0; time stretching is not an ASRC policy".into(),
        ));
    }
    let end = playback_end(playback, media_end_us);
    if playback.in_us >= end {
        return Err(MediaError::Unsupported(
            "file playback out point must be after in point".into(),
        ));
    }
    Ok(())
}

fn playback_end(playback: &Playback, media_end_us: u64) -> u64 {
    playback.out_us.unwrap_or(media_end_us).min(media_end_us)
}

fn clamp_position(playback: &Playback, media_end_us: u64, position_us: u64) -> u64 {
    let end = playback_end(playback, media_end_us);
    position_us
        .max(playback.in_us)
        .min(end.saturating_sub(1).max(playback.in_us))
}

#[cfg(test)]
mod tests {
    use super::*;
    use shiguredo_mp4::FixedPointNumber;

    fn entry(duration: u64, media_time: i64) -> ElstEntry {
        ElstEntry {
            edit_duration: duration,
            media_time,
            media_rate: FixedPointNumber::new(1, 0),
        }
    }

    #[test]
    fn edit_list_maps_empty_lead_and_aac_priming() {
        let edit = normalize_edit_entries(&[entry(500, -1), entry(9_500, 2_112)], 1_000).unwrap();
        assert_eq!(edit.media_start, 2_112);
        assert_eq!(edit.presentation_start_us, 500_000);
    }

    #[test]
    fn shared_cursor_reports_seek_and_loop_generation() {
        let mut cursor = PlaybackTimeline::new(
            Playback {
                playing: true,
                loop_playback: true,
                position_us: 100,
                in_us: 100,
                out_us: Some(400),
                speed: 1.0,
            },
            1_000,
            true,
        )
        .unwrap();
        let initial = cursor.resolve(1_000);
        assert_eq!(cursor.resolve(1_150).position_us, 250);
        let wrapped = cursor.resolve(1_350);
        assert_eq!(wrapped.position_us, 150);
        assert!(wrapped.generation > initial.generation);

        let mut seek = cursor.playback();
        seek.position_us = 300;
        cursor.apply(seek, true).unwrap();
        assert!(cursor.resolve(2_000).generation > wrapped.generation);
    }
}
