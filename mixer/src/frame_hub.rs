use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::SendCmd;
use crate::clock::{PlayoutCursor, Rate};
use crate::output::release_send_cmd;
use crate::upload::AudioPacket;

const AUDIO_CAP: usize = 16;

#[derive(Debug, Default)]
pub struct OutputStats {
    pub video_sent: u64,
    pub video_repeat: u64,
    pub video_drop: u64,
    pub audio_drop: u64,
    pub last_pts: i64,
}

#[derive(Clone)]
pub struct OutputPace {
    fps_n: Arc<AtomicU32>,
    fps_d: Arc<AtomicU32>,
    video_sent: Arc<AtomicU64>,
    video_repeat: Arc<AtomicU64>,
    video_drop: Arc<AtomicU64>,
    audio_drop: Arc<AtomicU64>,
}

impl OutputPace {
    pub fn new(num: u32, den: u32) -> Self {
        let rate = Rate::or_default(num, den);
        Self {
            fps_n: Arc::new(AtomicU32::new(rate.num)),
            fps_d: Arc::new(AtomicU32::new(rate.den)),
            video_sent: Arc::new(AtomicU64::new(0)),
            video_repeat: Arc::new(AtomicU64::new(0)),
            video_drop: Arc::new(AtomicU64::new(0)),
            audio_drop: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn record_video(&self, sent: u64, repeat: u64, drop: u64) {
        self.video_sent.store(sent, Ordering::Relaxed);
        self.video_repeat.store(repeat, Ordering::Relaxed);
        self.video_drop.store(drop, Ordering::Relaxed);
    }

    pub fn record_audio_drop(&self, drop: u64) {
        self.audio_drop.store(drop, Ordering::Relaxed);
    }

    pub fn stats(&self) -> OutputStats {
        OutputStats {
            video_sent: self.video_sent.load(Ordering::Relaxed),
            video_repeat: self.video_repeat.load(Ordering::Relaxed),
            video_drop: self.video_drop.load(Ordering::Relaxed),
            audio_drop: self.audio_drop.load(Ordering::Relaxed),
            last_pts: 0,
        }
    }

    pub fn store(&self, num: u32, den: u32) {
        let rate = Rate::or_default(num, den);
        self.fps_n.store(rate.num, Ordering::Relaxed);
        self.fps_d.store(rate.den, Ordering::Relaxed);
    }

    pub fn rate(&self) -> Rate {
        Rate::or_default(
            self.fps_n.load(Ordering::Relaxed),
            self.fps_d.load(Ordering::Relaxed),
        )
    }
}

pub struct VideoMailbox {
    slot: Mutex<Option<SendCmd>>,
    drops: AtomicU64,
}

impl VideoMailbox {
    pub fn new() -> Self {
        Self {
            slot: Mutex::new(None),
            drops: AtomicU64::new(0),
        }
    }

    pub fn publish(&self, cmd: SendCmd) {
        if let Ok(mut slot) = self.slot.lock() {
            if let Some(prev) = slot.replace(cmd) {
                release_send_cmd(prev);
                self.drops.fetch_add(1, Ordering::Relaxed);
            }
        } else {
            release_send_cmd(cmd);
        }
    }

    pub fn take(&self) -> Option<SendCmd> {
        self.slot.lock().ok()?.take()
    }

    pub fn drops(&self) -> u64 {
        self.drops.load(Ordering::Relaxed)
    }
}

pub struct AudioMailbox {
    q: Mutex<VecDeque<AudioPacket>>,
    drops: AtomicU64,
}

impl AudioMailbox {
    pub fn new() -> Self {
        Self {
            q: Mutex::new(VecDeque::with_capacity(AUDIO_CAP)),
            drops: AtomicU64::new(0),
        }
    }

    pub fn push(&self, packet: AudioPacket) {
        let Ok(mut q) = self.q.lock() else {
            return;
        };
        if q.len() >= AUDIO_CAP {
            q.pop_front();
            self.drops.fetch_add(1, Ordering::Relaxed);
        }
        q.push_back(packet);
    }

    pub fn drain(&self) -> Vec<AudioPacket> {
        self.q
            .lock()
            .map(|mut q| q.drain(..).collect())
            .unwrap_or_default()
    }

    pub fn drops(&self) -> u64 {
        self.drops.load(Ordering::Relaxed)
    }
}

#[derive(Clone)]
pub struct OutputTx {
    pub video: Arc<VideoMailbox>,
    pub audio: Arc<AudioMailbox>,
    pub pace: Arc<OutputPace>,
    ctrl: std::sync::mpsc::Sender<SendCmd>,
}

impl OutputTx {
    pub fn new(
        video: Arc<VideoMailbox>,
        audio: Arc<AudioMailbox>,
        pace: Arc<OutputPace>,
        ctrl: std::sync::mpsc::Sender<SendCmd>,
    ) -> Self {
        Self {
            video,
            audio,
            pace,
            ctrl,
        }
    }

    pub fn stub() -> Self {
        let (ctrl, _) = std::sync::mpsc::channel();
        Self::new(
            Arc::new(VideoMailbox::new()),
            Arc::new(AudioMailbox::new()),
            Arc::new(OutputPace::new(60, 1)),
            ctrl,
        )
    }

    pub fn send(&self, cmd: SendCmd) -> Result<(), ()> {
        match cmd {
            SendCmd::Video { .. } | SendCmd::GpuVideo { .. } => {
                self.video.publish(cmd);
                Ok(())
            }
            SendCmd::Audio { packet } => {
                self.audio.push(packet);
                Ok(())
            }
            other => self.ctrl.send(other).map_err(|_| ()),
        }
    }
}

pub fn sleep_until_deadline(deadline: std::time::Instant, stop: &std::sync::atomic::AtomicBool) {
    use std::sync::atomic::Ordering;
    use std::thread;
    use std::time::{Duration, Instant};
    const SLICE: Duration = Duration::from_millis(2);
    while !stop.load(Ordering::Relaxed) && !crate::diag::is_fatal() {
        let now = Instant::now();
        if now >= deadline {
            return;
        }
        thread::sleep((deadline - now).min(SLICE));
    }
}

pub fn clone_video_cmd(cmd: &SendCmd) -> Option<SendCmd> {
    match cmd {
        SendCmd::Video {
            width,
            height,
            stride,
            pts,
            data,
            fps_n,
            fps_d,
        } => Some(SendCmd::Video {
            width: *width,
            height: *height,
            stride: *stride,
            pts: *pts,
            data: Arc::clone(data),
            fps_n: *fps_n,
            fps_d: *fps_d,
        }),
        SendCmd::GpuVideo { .. } => None,
        _ => None,
    }
}

pub fn retimestamp(cmd: SendCmd, pts: i64, fps_n: u32, fps_d: u32) -> SendCmd {
    match cmd {
        SendCmd::Video {
            width,
            height,
            stride,
            data,
            ..
        } => SendCmd::Video {
            width,
            height,
            stride,
            pts,
            data,
            fps_n,
            fps_d,
        },
        SendCmd::GpuVideo {
            texture,
            width,
            height,
            busy,
            ..
        } => SendCmd::GpuVideo {
            texture,
            width,
            height,
            pts,
            fps_n,
            fps_d,
            busy,
        },
        other => other,
    }
}

pub fn cursor_for_pace(pace: &OutputPace) -> PlayoutCursor {
    PlayoutCursor::new(pace.rate())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_mailbox_keeps_latest() {
        let box_ = VideoMailbox::new();
        box_.publish(SendCmd::Video {
            width: 2,
            height: 1,
            stride: 4,
            pts: 1,
            data: Arc::from([1u8, 2, 3, 4]),
            fps_n: 50,
            fps_d: 1,
        });
        box_.publish(SendCmd::Video {
            width: 2,
            height: 1,
            stride: 4,
            pts: 2,
            data: Arc::from([5u8, 6, 7, 8]),
            fps_n: 50,
            fps_d: 1,
        });
        match box_.take() {
            Some(SendCmd::Video { pts, .. }) if pts == 2 => {}
            _ => panic!("mailbox lost the latest frame"),
        }
        assert_eq!(box_.drops(), 1);
    }

    #[test]
    fn audio_mailbox_is_bounded() {
        let box_ = AudioMailbox::new();
        for i in 0..20 {
            box_.push(AudioPacket {
                timestamp: i,
                sample_rate: 48_000,
                channels: 2,
                samples_per_channel: 480,
                pcm_planar_f32: vec![0.0; 960],
            });
        }
        let drained = box_.drain();
        assert_eq!(drained.len(), AUDIO_CAP);
        assert_eq!(drained[0].timestamp, 4);
        assert_eq!(box_.drops(), 4);
    }

    #[test]
    fn mailboxes_are_isolated_across_outputs() {
        let a = OutputTx::stub();
        let b = OutputTx::stub();
        a.video.publish(SendCmd::Video {
            width: 2,
            height: 1,
            stride: 4,
            pts: 1,
            data: Arc::from([1u8, 2, 3, 4]),
            fps_n: 50,
            fps_d: 1,
        });
        b.video.publish(SendCmd::Video {
            width: 2,
            height: 1,
            stride: 4,
            pts: 9,
            data: Arc::from([9u8, 8, 7, 6]),
            fps_n: 25,
            fps_d: 1,
        });
        match a.video.take() {
            Some(SendCmd::Video { pts, .. }) if pts == 1 => {}
            _ => panic!("output A lost its frame"),
        }
        match b.video.take() {
            Some(SendCmd::Video { pts, .. }) if pts == 9 => {}
            _ => panic!("output B lost its frame"),
        }
    }

    #[test]
    fn encode_once_mailbox_does_not_clone_per_subscriber() {
        let tx = OutputTx::stub();
        let data: Arc<[u8]> = Arc::from([1u8, 2, 3, 4]);
        tx.video.publish(SendCmd::Video {
            width: 2,
            height: 1,
            stride: 4,
            pts: 1,
            data: Arc::clone(&data),
            fps_n: 60,
            fps_d: 1,
        });
        match tx.video.take() {
            Some(SendCmd::Video { data: taken, .. }) => {
                assert_eq!(Arc::strong_count(&taken), 2);
            }
            _ => panic!("missing frame"),
        }
        let stats = tx.pace.stats();
        assert_eq!(stats.video_sent, 0);
        assert_eq!(stats.video_repeat, 0);
        assert_eq!(stats.video_drop, 0);
        assert_eq!(stats.audio_drop, 0);
        assert_eq!(stats.last_pts, 0);
    }
}
