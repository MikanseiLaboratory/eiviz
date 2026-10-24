//! Replay transport and the picture boundary.
//!
//! Nothing in this crate names a mixer type. The mixer implements [`FrameSink`]
//! and [`FrameSource`] for whatever store it has open, and it uses [`channel`]
//! for playout.

pub mod channel;

/// One picture crossing the mixer boundary.
pub struct Picture<'a> {
    pub width: u32,
    pub height: u32,
    pub stride: usize,
    /// 0 is packed UYVY. 1 is BGRA. The mixer maps its own layout onto these.
    pub layout: u32,
    pub pixels: &'a [u8],
    pub pts_100ns: i64,
    pub fps_num: u32,
    pub fps_den: u32,
}

/// The recorder pushes a picture in. The implementor stores it.
pub trait FrameSink {
    fn write_picture(
        &mut self,
        angle_id: u32,
        frame: u64,
        picture: Picture<'_>,
    ) -> Result<(), String>;
}

/// Playout pulls a picture back out for a channel input.
pub trait FrameSource {
    fn read_picture(&mut self, angle_id: u32, frame: u64, dst: &mut [u8]) -> Result<u32, String>;
}

/// Hands a picture to a sink. The mixer calls this instead of the store directly.
pub fn deliver_picture<S: FrameSink + ?Sized>(
    sink: &mut S,
    angle_id: u32,
    frame: u64,
    picture: Picture<'_>,
) -> Result<(), String> {
    sink.write_picture(angle_id, frame, picture)
}

/// Reads a picture back. `Ok` is the byte count written into `dst`.
pub fn take_picture<S: FrameSource + ?Sized>(
    source: &mut S,
    angle_id: u32,
    frame: u64,
    dst: &mut [u8],
) -> Result<u32, String> {
    source.read_picture(angle_id, frame, dst)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Memory {
        written: Vec<u8>,
    }

    impl FrameSink for Memory {
        fn write_picture(
            &mut self,
            _angle_id: u32,
            _frame: u64,
            picture: Picture<'_>,
        ) -> Result<(), String> {
            self.written = picture.pixels.to_vec();
            Ok(())
        }
    }

    impl FrameSource for Memory {
        fn read_picture(
            &mut self,
            _angle_id: u32,
            _frame: u64,
            dst: &mut [u8],
        ) -> Result<u32, String> {
            let n = self.written.len().min(dst.len());
            dst[..n].copy_from_slice(&self.written[..n]);
            Ok(n as u32)
        }
    }

    #[test]
    fn a_picture_crosses_the_boundary_without_a_mixer_type() {
        let mut memory = Memory {
            written: Vec::new(),
        };
        let pixels = [16u8, 128, 16, 128];
        deliver_picture(
            &mut memory,
            1,
            4,
            Picture {
                width: 2,
                height: 1,
                stride: 4,
                layout: 0,
                pixels: &pixels,
                pts_100ns: 0,
                fps_num: 60,
                fps_den: 1,
            },
        )
        .unwrap();
        let mut out = [0u8; 4];
        assert_eq!(take_picture(&mut memory, 1, 4, &mut out).unwrap(), 4);
        assert_eq!(out, pixels);
    }
}
