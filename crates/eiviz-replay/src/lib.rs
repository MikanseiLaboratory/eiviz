//! Replay transport and the picture boundary.
//!
//! Nothing in this crate names a mixer type. The mixer implements [`FrameSink`]
//! for the store it is recording into, and uses [`channel`] for playout.

pub mod channel;

/// One picture crossing the mixer boundary.
///
/// The pixels are owned. The readback hands the recorder a buffer it has already allocated,
/// and borrowing it only forced the sink to copy the whole picture a second time on its way
/// into the encoder queue: at eight 1080p angles that was 33 MB a frame through one thread.
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub stride: usize,
    /// 0 is packed UYVY. 1 is BGRA. The mixer maps its own layout onto these.
    pub layout: u32,
    pub pixels: Vec<u8>,
    pub pts_100ns: i64,
    pub fps_num: u32,
    pub fps_den: u32,
}

/// The recorder pushes a picture in. The implementor stores it.
pub trait FrameSink {
    fn write_picture(&mut self, angle_id: u32, frame: u64, picture: Picture) -> Result<(), String>;
}

/// Hands a picture to a sink. The mixer calls this instead of the store directly, so the
/// two crates hand pictures over the same way.
pub fn deliver_picture<S: FrameSink + ?Sized>(
    sink: &mut S,
    angle_id: u32,
    frame: u64,
    picture: Picture,
) -> Result<(), String> {
    sink.write_picture(angle_id, frame, picture)
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
            picture: Picture,
        ) -> Result<(), String> {
            self.written = picture.pixels.to_vec();
            Ok(())
        }
    }

    #[test]
    fn a_picture_crosses_the_boundary_without_a_mixer_type() {
        let mut memory = Memory {
            written: Vec::new(),
        };
        let pixels = [16u8, 128, 16, 128];
        memory
            .write_picture(
                1,
                4,
                Picture {
                    width: 2,
                    height: 1,
                    stride: 4,
                    layout: 0,
                    pixels: pixels.to_vec(),
                    pts_100ns: 0,
                    fps_num: 60,
                    fps_den: 1,
                },
            )
            .unwrap();
        assert_eq!(memory.written, pixels);
    }
}
