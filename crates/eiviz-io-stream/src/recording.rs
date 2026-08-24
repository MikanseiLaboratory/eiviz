use crate::fanout::EncodedSink;
use eiviz_media::{EncodedAccessUnit, EncodedKind, EncodedStreamConfig, MediaError, Result};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryReport {
    pub original_len: u64,
    pub recovered_len: u64,
    pub truncated_bytes: u64,
    pub complete_boxes: u64,
}

pub struct FragmentedMp4Sink {
    path: PathBuf,
    name: String,
    recover_incomplete_tail: bool,
    file: Option<File>,
    muxer: Option<eiviz_codec_software::FragmentedMp4>,
    video_sample_duration: u32,
}

impl FragmentedMp4Sink {
    pub fn new(path: PathBuf, recover_incomplete_tail: bool) -> Self {
        Self {
            name: format!("fMP4 {}", path.display()),
            path,
            recover_incomplete_tail,
            file: None,
            muxer: None,
            video_sample_duration: 0,
        }
    }
}

impl EncodedSink for FragmentedMp4Sink {
    fn name(&self) -> &str {
        &self.name
    }

    fn connect(&mut self, config: &EncodedStreamConfig) -> Result<()> {
        if self.recover_incomplete_tail && self.path.exists() {
            recover_fragmented_mp4(&self.path)?;
        }
        if let Some(parent) = self
            .path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)
                .map_err(|error| MediaError::Other(error.to_string()))?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&self.path)
            .map_err(|error| MediaError::Other(error.to_string()))?;
        let length = file
            .metadata()
            .map_err(|error| MediaError::Other(error.to_string()))?
            .len();
        let mut muxer = eiviz_codec_software::FragmentedMp4::new_av(config.clone());
        if length == 0 {
            file.write_all(&muxer.bytes)
                .map_err(|error| MediaError::Disconnected(error.to_string()))?;
            file.sync_data()
                .map_err(|error| MediaError::Disconnected(error.to_string()))?;
        } else {
            validate_fmp4_prefix(&mut file)?;
        }
        muxer.bytes.clear();
        file.seek(SeekFrom::End(0))
            .map_err(|error| MediaError::Other(error.to_string()))?;
        self.file = Some(file);
        self.muxer = Some(muxer);
        self.video_sample_duration = config.video_sample_duration;
        Ok(())
    }

    fn send(&mut self, access_unit: &Arc<EncodedAccessUnit>) -> Result<()> {
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| MediaError::Disconnected(self.name.clone()))?;
        let muxer = self
            .muxer
            .as_mut()
            .ok_or_else(|| MediaError::Disconnected(self.name.clone()))?;
        let duration = match access_unit.kind {
            EncodedKind::Avc => self.video_sample_duration,
            EncodedKind::Aac => 1024,
            EncodedKind::Pcm => {
                return Err(MediaError::Unsupported(
                    "fragmented MP4 baseline requires AAC, not PCM".into(),
                ));
            }
        };
        muxer.write_sample(access_unit, duration);
        file.write_all(&muxer.bytes)
            .map_err(|error| MediaError::Disconnected(error.to_string()))?;
        muxer.bytes.clear();
        if access_unit.keyframe {
            file.sync_data()
                .map_err(|error| MediaError::Disconnected(error.to_string()))?;
        }
        Ok(())
    }

    fn disconnect(&mut self) {
        if let Some(file) = self.file.take() {
            let _ = file.sync_data();
        }
        self.muxer = None;
    }
}

pub fn recover_fragmented_mp4(path: &Path) -> Result<RecoveryReport> {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|error| MediaError::Other(error.to_string()))?;
    let original_len = file
        .metadata()
        .map_err(|error| MediaError::Other(error.to_string()))?
        .len();
    let mut offset = 0u64;
    let mut complete_boxes = 0u64;
    let mut header = [0u8; 8];
    while offset.saturating_add(8) <= original_len {
        file.seek(SeekFrom::Start(offset))
            .and_then(|_| file.read_exact(&mut header))
            .map_err(|error| MediaError::Other(error.to_string()))?;
        let size = u32::from_be_bytes(header[..4].try_into().expect("four bytes")) as u64;
        if size < 8 {
            break;
        }
        let Some(end) = offset.checked_add(size) else {
            break;
        };
        if end > original_len {
            break;
        }
        offset = end;
        complete_boxes += 1;
    }
    if offset < original_len {
        file.set_len(offset)
            .map_err(|error| MediaError::Other(error.to_string()))?;
        file.sync_data()
            .map_err(|error| MediaError::Other(error.to_string()))?;
    }
    Ok(RecoveryReport {
        original_len,
        recovered_len: offset,
        truncated_bytes: original_len - offset,
        complete_boxes,
    })
}

fn validate_fmp4_prefix(file: &mut File) -> Result<()> {
    let mut header = [0u8; 8];
    file.seek(SeekFrom::Start(0))
        .and_then(|_| file.read_exact(&mut header))
        .map_err(|error| MediaError::Other(error.to_string()))?;
    if &header[4..] != b"ftyp" {
        return Err(MediaError::Unsupported(
            "refusing to append: recording does not start with an ftyp box".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use eiviz_time::MediaTime;

    #[test]
    fn recovery_truncates_only_incomplete_tail() {
        let path =
            std::env::temp_dir().join(format!("eiviz-fmp4-recover-{}.mp4", std::process::id()));
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&12u32.to_be_bytes());
        bytes.extend_from_slice(b"ftyp");
        bytes.extend_from_slice(b"test");
        bytes.extend_from_slice(&20u32.to_be_bytes());
        bytes.extend_from_slice(b"moof");
        bytes.extend_from_slice(&[1, 2, 3]);
        std::fs::write(&path, bytes).unwrap();
        let report = recover_fragmented_mp4(&path).unwrap();
        assert_eq!(report.recovered_len, 12);
        assert_eq!(report.truncated_bytes, 11);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn recording_contains_avc_aac_tracks_and_fragments() {
        let path = std::env::temp_dir().join(format!("eiviz-fmp4-av-{}.mp4", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let config = EncodedStreamConfig {
            h264_sps: vec![0x67, 66, 0, 31].into(),
            h264_pps: vec![0x68, 0].into(),
            aac_audio_specific_config: vec![0x11, 0x90].into(),
            video_width: 1920,
            video_height: 1080,
            video_timescale: 60_000,
            video_sample_duration: 1001,
            audio_sample_rate: 48_000,
            audio_channels: 2,
        };
        let mut sink = FragmentedMp4Sink::new(path.clone(), true);
        sink.connect(&config).unwrap();
        sink.send(&Arc::new(EncodedAccessUnit {
            pts: MediaTime::ZERO,
            dts: Some(MediaTime::ZERO),
            keyframe: true,
            bytes: vec![0, 0, 0, 1, 0x65, 1].into(),
            kind: EncodedKind::Avc,
        }))
        .unwrap();
        sink.send(&Arc::new(EncodedAccessUnit {
            pts: MediaTime::ZERO,
            dts: Some(MediaTime::ZERO),
            keyframe: false,
            bytes: vec![0x21, 0x10].into(),
            kind: EncodedKind::Aac,
        }))
        .unwrap();
        sink.disconnect();
        let bytes = std::fs::read(&path).unwrap();
        for box_type in [b"ftyp", b"moov", b"avc1", b"mp4a", b"moof", b"mdat"] {
            assert!(
                bytes.windows(4).any(|window| window == box_type),
                "missing {}",
                String::from_utf8_lossy(box_type)
            );
        }
        assert_eq!(
            bytes.windows(4).filter(|window| *window == b"moof").count(),
            2
        );
        let _ = std::fs::remove_file(path);
    }
}
