use eiviz_core::InputId;
use eiviz_media::{MediaError, MediaSource, Result, VideoFrame};
use eiviz_time::{FrameRate, MediaTime};
use std::path::PathBuf;
use std::sync::Mutex;

pub struct ImageSource {
    id: InputId,
    path: PathBuf,
    cached: Mutex<Option<VideoFrame>>,
}

impl ImageSource {
    pub fn new(id: InputId, path: PathBuf) -> Self {
        Self {
            id,
            path,
            cached: Mutex::new(None),
        }
    }

    fn load(&self, pts: MediaTime) -> Result<VideoFrame> {
        let bytes = std::fs::read(&self.path).map_err(|e| MediaError::Other(e.to_string()))?;
        let img = image::load_from_memory(&bytes)
            .map_err(|e| MediaError::Other(e.to_string()))?
            .to_rgba8();
        let (w, h) = img.dimensions();
        Ok(VideoFrame {
            id: 0,
            source: Some(self.id),
            pts,
            capture_domain: eiviz_time::ClockDomain::SourceMedia,
            width: w,
            height: h,
            format: eiviz_media::PixelFormat::Rgba8,
            data: img.into_raw().into(),
            discontinuity: false,
        })
    }
}

impl MediaSource for ImageSource {
    fn id(&self) -> InputId {
        self.id
    }

    fn pull_video(&self, pts: MediaTime, _rate: FrameRate) -> Result<Option<VideoFrame>> {
        let mut g = self.cached.lock().unwrap();
        if g.is_none() {
            *g = Some(self.load(pts)?);
        }
        if let Some(frame) = g.as_mut() {
            frame.pts = pts;
        }
        Ok(g.clone())
    }

    fn pull_audio(
        &self,
        _sample_index: u64,
        _frames: usize,
    ) -> Result<Option<eiviz_media::AudioBuffer>> {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eiviz_core::InputId;
    use eiviz_time::NTSC_5994;

    #[test]
    fn loads_png() {
        let dir = std::env::temp_dir().join(format!("eiviz-img-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("px.png");
        let img = image::RgbaImage::from_pixel(2, 2, image::Rgba([9, 8, 7, 255]));
        img.save(&path).unwrap();
        let src = ImageSource::new(InputId::new(), path);
        let frame = src.pull_video(MediaTime::ZERO, NTSC_5994).unwrap().unwrap();
        assert_eq!(frame.pixel(0, 0), [9, 8, 7, 255]);
    }
}
