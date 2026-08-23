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
}
