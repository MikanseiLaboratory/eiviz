//! Isolated record/stream sinks. A slow or failed sink never blocks Program.

use crc32fast::Hasher;
use eiviz_media::{AudioBuffer, MediaError, MediaSink, Result, VideoFrame};
use eiviz_time::MediaTime;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

pub struct RecordingSink {
    file: Mutex<File>,
    name: String,
    failed: Mutex<Option<String>>,
}

impl RecordingSink {
    pub fn create(path: PathBuf) -> Result<Self> {
        let mut file = File::create(&path).map_err(|e| MediaError::Other(e.to_string()))?;
        file.write_all(b"EIVZREC1")
            .map_err(|e| MediaError::Other(e.to_string()))?;
        Ok(Self {
            file: Mutex::new(file),
            name: path.display().to_string(),
            failed: Mutex::new(None),
        })
    }

    pub fn failed_reason(&self) -> Option<String> {
        self.failed.lock().unwrap().clone()
    }

    fn write_packet(&self, kind: u8, pts: MediaTime, payload: &[u8]) -> Result<()> {
        if self.failed.lock().unwrap().is_some() {
            return Err(MediaError::Disconnected(self.name.clone()));
        }
        let mut h = Hasher::new();
        h.update(payload);
        let crc = h.finalize();
        let mut hdr = Vec::new();
        hdr.push(kind);
        hdr.extend_from_slice(&pts.ticks().to_le_bytes());
        hdr.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        hdr.extend_from_slice(&crc.to_le_bytes());
        let mut g = self.file.lock().unwrap();
        if g.write_all(&hdr).is_err() || g.write_all(payload).is_err() {
            *self.failed.lock().unwrap() = Some("write failed".into());
            return Err(MediaError::Disconnected(self.name.clone()));
        }
        let _ = g.flush();
        Ok(())
    }
}

impl MediaSink for RecordingSink {
    fn name(&self) -> &str {
        &self.name
    }

    fn push_video(&self, frame: &VideoFrame) -> Result<()> {
        self.write_packet(1, frame.pts, &frame.data)
    }

    fn push_audio(&self, audio: &AudioBuffer) -> Result<()> {
        let mut bytes = Vec::new();
        for s in &audio.planes[0] {
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        self.write_packet(2, MediaTime::ZERO, &bytes)
    }
}

/// Minimal RTMP handshake writer used in protocol tests (C0/C1).
pub fn rtmp_c0c1() -> Vec<u8> {
    let mut v = vec![0x03];
    v.extend_from_slice(&[0u8; 4]); // time
    v.extend_from_slice(&[0u8; 4]);
    v.extend_from_slice(&[0xA5; 1528]);
    v
}

/// Fragmented MP4 recorder. Encode/mux failures stay local to this sink.
pub struct Fmp4RecordingSink {
    name: String,
    inner: Mutex<Fmp4Inner>,
    failed: Mutex<Option<String>>,
}

struct Fmp4Inner {
    file: File,
    mux: Option<eiviz_codec_software::FragmentedMp4>,
}

impl Fmp4RecordingSink {
    pub fn create(path: PathBuf) -> Result<Self> {
        let file = File::create(&path).map_err(|e| MediaError::Other(e.to_string()))?;
        Ok(Self {
            name: path.display().to_string(),
            inner: Mutex::new(Fmp4Inner { file, mux: None }),
            failed: Mutex::new(None),
        })
    }

    pub fn failed_reason(&self) -> Option<String> {
        self.failed.lock().unwrap().clone()
    }

    fn mark_failed(&self, reason: impl Into<String>) -> MediaError {
        let reason = reason.into();
        *self.failed.lock().unwrap() = Some(reason.clone());
        MediaError::Disconnected(format!("{}: {reason}", self.name))
    }
}

impl MediaSink for Fmp4RecordingSink {
    fn name(&self) -> &str {
        &self.name
    }

    fn push_video(&self, frame: &VideoFrame) -> Result<()> {
        if self.failed.lock().unwrap().is_some() {
            return Err(MediaError::Disconnected(self.name.clone()));
        }
        let au = eiviz_codec_software::encode_idr(frame);
        let (sps, pps) = eiviz_codec_software::extract_sps_pps(&au);
        let mut g = self.inner.lock().unwrap();
        if g.mux.is_none() {
            g.mux = Some(eiviz_codec_software::FragmentedMp4::new(
                60000,
                frame.width as u16,
                frame.height as u16,
                &sps,
                &pps,
            ));
        }
        let bytes = if let Some(mux) = g.mux.as_mut() {
            mux.write_sample(&au, 1001);
            let bytes = mux.bytes.clone();
            mux.bytes.clear();
            bytes
        } else {
            Vec::new()
        };
        if g.file.write_all(&bytes).is_err() {
            drop(g);
            return Err(self.mark_failed("disk write failed"));
        }
        let _ = g.file.flush();
        Ok(())
    }

    fn push_audio(&self, _audio: &AudioBuffer) -> Result<()> {
        Ok(())
    }
}

/// Always-failing sink used to prove Program isolation.
pub struct FailingSink {
    name: String,
}

impl FailingSink {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

impl MediaSink for FailingSink {
    fn name(&self) -> &str {
        &self.name
    }

    fn push_video(&self, _frame: &VideoFrame) -> Result<()> {
        Err(MediaError::Disconnected(self.name.clone()))
    }

    fn push_audio(&self, _audio: &AudioBuffer) -> Result<()> {
        Err(MediaError::Disconnected(self.name.clone()))
    }
}

pub fn flv_file_header() -> Vec<u8> {
    eiviz_codec_software::flv_header()
}

pub fn mpegts_pat() -> [u8; 188] {
    eiviz_codec_software::pat()
}

#[cfg(test)]
mod tests {
    use super::*;
    use eiviz_time::MediaTime;

    #[test]
    fn recording_writes_magic_and_survives_frame() {
        let path = std::env::temp_dir().join(format!("rec-{}.eivizbin", std::process::id()));
        let sink = RecordingSink::create(path.clone()).unwrap();
        let frame = VideoFrame::rgba_solid(1, MediaTime::ZERO, 2, 2, [1, 2, 3, 255]);
        sink.push_video(&frame).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[..8], b"EIVZREC1");
        assert_eq!(rtmp_c0c1()[0], 0x03);
        assert_eq!(rtmp_c0c1().len(), 1537);
    }

    #[test]
    fn fmp4_sink_writes_ftyp_and_failed_sink_does_not_panic() {
        let path = std::env::temp_dir().join(format!("rec-{}.mp4", std::process::id()));
        let sink = Fmp4RecordingSink::create(path.clone()).unwrap();
        let frame = VideoFrame::rgba_solid(2, MediaTime::ZERO, 16, 16, [10, 20, 30, 255]);
        sink.push_video(&frame).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert!(bytes.windows(4).any(|w| w == b"ftyp"));
        assert!(bytes.windows(4).any(|w| w == b"moof"));
        let fail = FailingSink::new("aux");
        assert!(fail.push_video(&frame).is_err());
        assert_eq!(&flv_file_header()[..3], b"FLV");
        assert_eq!(mpegts_pat()[0], 0x47);
    }
}
