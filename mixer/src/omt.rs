use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use openmediatransport::{
    Codec, DecodedAudioFrame, Discovery, FrameType, MediaFrame, ReceiverConfig, ReceiverSession,
    Sender,
};

use crate::abi::FMT_BGRA;
use crate::upload::{AudioPacket, CpuFormat, UploadStore};

pub struct OmtReceiver {
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl OmtReceiver {
    pub fn start(
        source_id: u64,
        address: String,
        uploads: Arc<Mutex<UploadStore>>,
    ) -> Result<Self, String> {
        let config = ReceiverConfig {
            frame_types: FrameType::VIDEO | FrameType::AUDIO,
            connect_timeout: Duration::from_secs(5),
            ..ReceiverConfig::default()
        };
        let session = connect_receiver(&address, config)?;
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = Arc::clone(&stop);
        let join = thread::Builder::new()
            .name(format!("eiviz-omt-{source_id}"))
            .spawn(move || {
                {
                    let mut store = uploads.lock().expect("uploads lock");
                    store.ensure(
                        source_id,
                        16,
                        16,
                        CpuFormat::from_abi(FMT_BGRA).expect("BGRA"),
                    );
                }
                while !stop_thread.load(Ordering::Relaxed) {
                    if let Some(frame) = session.recv_video_timeout(Duration::from_millis(4)) {
                        let mut store = uploads.lock().expect("uploads lock");
                        store.ensure(
                            source_id,
                            frame.width.max(2),
                            frame.height.max(2),
                            CpuFormat::from_abi(FMT_BGRA).expect("BGRA"),
                        );
                        let stride = frame.stride.max(frame.width * 4) as usize;
                        store
                            .push(source_id, &frame.pixels, stride, frame.timestamp)
                            .ok();
                    }
                    let mut store = uploads.lock().expect("uploads lock");
                    while let Some(audio) = session.try_recv_audio() {
                        store.ingest_audio(source_id, to_audio(audio));
                    }
                }
                session.disconnect();
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            stop,
            join: Some(join),
        })
    }
}

impl Drop for OmtReceiver {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

pub struct ProgramSender {
    pub sender: Sender,
    pub unit_id: u64,
    name: String,
    discovery: Option<Discovery>,
}

impl ProgramSender {
    pub fn start(name: &str) -> Result<Self, String> {
        let sender = Sender::create(name, FrameType::VIDEO | FrameType::AUDIO)
            .map_err(|error| error.to_string())?;
        let port = sender.port();
        let advertised = name.to_string();
        let discovery = panic::catch_unwind(AssertUnwindSafe(|| {
            Discovery::new().ok().and_then(|mut discovery| {
                discovery.register(&advertised, port).ok()?;
                Some(discovery)
            })
        }))
        .ok()
        .flatten();
        Ok(Self {
            sender,
            unit_id: 0,
            name: name.to_string(),
            discovery,
        })
    }

    pub fn pump(&mut self) -> Result<(), String> {
        self.sender.poll_accept().map_err(|e| e.to_string())?;
        self.sender.poll_peer_metadata().map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn send_video_uyvy(
        &mut self,
        width: u32,
        height: u32,
        stride: u32,
        pts: i64,
        pixels: Arc<[u8]>,
        fps_num: u32,
        fps_den: u32,
    ) -> Result<(), String> {
        let data = pixels.to_vec();
        let frame = MediaFrame {
            frame_type: FrameType::VIDEO,
            timestamp: pts,
            codec: Codec::Uyvy as i32,
            width: width as i32,
            height: height as i32,
            stride: stride as i32,
            frame_rate_n: fps_num as i32,
            frame_rate_d: fps_den as i32,
            aspect_ratio: width as f32 / height.max(1) as f32,
            data,
            ..Default::default()
        };
        self.sender.send_video(frame).map_err(|e| e.to_string())
    }

    pub fn video_subscribed(&self) -> bool {
        self.sender.video_subscribed()
    }

    pub fn send_audio(&mut self, audio: &AudioPacket) -> Result<(), String> {
        let frame = MediaFrame {
            frame_type: FrameType::AUDIO,
            timestamp: audio.timestamp,
            codec: Codec::Fpa1 as i32,
            sample_rate: audio.sample_rate,
            channels: audio.channels,
            samples_per_channel: audio.samples_per_channel,
            data: audio.pcm_planar_f32.clone(),
            ..Default::default()
        };
        self.sender.send_audio(frame).map_err(|e| e.to_string())
    }
}

impl Drop for ProgramSender {
    fn drop(&mut self) {
        if let Some(discovery) = self.discovery.as_mut() {
            let _ = discovery.deregister(&self.name);
        }
    }
}

pub fn discover_addresses() -> Result<Vec<String>, String> {
    let mut discovery = Discovery::new().map_err(|e| e.to_string())?;
    discovery.refresh().map_err(|e| e.to_string())?;
    Ok(discovery
        .sources()
        .into_iter()
        .map(|source| source.to_url())
        .collect())
}

fn connect_receiver(address: &str, config: ReceiverConfig) -> Result<ReceiverSession, String> {
    let trimmed = address.trim();
    if let Ok(mut discovery) = Discovery::new()
        && discovery.refresh().is_ok()
        && let Some(source) = discovery.sources().iter().find(|source| {
            source.to_url() == trimmed
                || source.to_string() == trimmed
                || source.instance_name() == trimmed
        })
    {
        return ReceiverSession::connect_from_address(source, config).map_err(|e| e.to_string());
    }
    if trimmed.starts_with("omt://") {
        return ReceiverSession::connect(trimmed, config).map_err(|e| e.to_string());
    }
    Err(format!("OMT source not found: {trimmed}"))
}

fn to_audio(frame: DecodedAudioFrame) -> AudioPacket {
        let channels = frame.channels.max(1);
        let samples = if frame.samples_per_channel > 0 {
            frame.samples_per_channel
        } else {
            (frame.pcm_planar_f32.len() as i32 / 4 / channels).max(1)
        };
        AudioPacket {
            timestamp: frame.timestamp,
            sample_rate: frame.sample_rate,
            channels: frame.channels,
            samples_per_channel: samples,
            pcm_planar_f32: frame.pcm_planar_f32.to_vec(),
        }
}
