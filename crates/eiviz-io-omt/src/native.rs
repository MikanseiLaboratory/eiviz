use eiviz_core::InputId;
use eiviz_media::{AdapterHealth, AudioBuffer, MediaError, MediaSink, MediaSource, VideoFrame};
use eiviz_time::{ClockDomain, FrameRate, MediaTime, Rational};
use libloading::Library;
use parking_lot::Mutex;
use std::collections::VecDeque;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::path::PathBuf;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::{SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const OMT_FRAME_METADATA: c_int = 1;
const OMT_FRAME_VIDEO: c_int = 2;
const OMT_FRAME_AUDIO: c_int = 4;
const OMT_FRAME_ALL: c_int = OMT_FRAME_METADATA | OMT_FRAME_VIDEO | OMT_FRAME_AUDIO;
const OMT_FORMAT_BGRA: c_int = 2;
const OMT_RECEIVE_FLAGS_NONE: c_int = 0;
const OMT_CODEC_FPA1: c_int = 0x3141_5046;
const OMT_CODEC_UYVY: c_int = 0x5956_5955;
const OMT_CODEC_BGRA: c_int = 0x4152_4742;
const OMT_COLORSPACE_BT709: c_int = 709;
const OMT_VIDEO_FLAG_ALPHA: c_int = 2;
const OMT_VIDEO_FLAG_PREMULTIPLIED: c_int = 4;
const OMT_TICKS_PER_SECOND: i64 = 10_000_000;
const RECEIVE_TIMEOUT_MS: c_int = 20;
const AUDIO_QUEUE_CAPACITY: usize = 8;
const MAX_DIMENSION: c_int = 16_384;
const MAX_AUDIO_CHANNELS: c_int = 32;
const MAX_AUDIO_SAMPLES: c_int = 1_000_000;

type DiscoveryGetAddresses = unsafe extern "C" fn(*mut c_int) -> *mut *mut c_char;
type ReceiveCreate = unsafe extern "C" fn(*const c_char, c_int, c_int, c_int) -> *mut c_void;
type ReceiveDestroy = unsafe extern "C" fn(*mut c_void);
type Receive = unsafe extern "C" fn(*mut c_void, c_int, c_int) -> *mut OmtMediaFrame;
type ReceiveStatistics = unsafe extern "C" fn(*mut c_void, *mut OmtStatistics);
type SendCreate = unsafe extern "C" fn(*const c_char, c_int) -> *mut c_void;
type SendDestroy = unsafe extern "C" fn(*mut c_void);
type SendFrame = unsafe extern "C" fn(*mut c_void, *mut OmtMediaFrame) -> c_int;

#[repr(C)]
#[derive(Clone, Copy)]
struct OmtMediaFrame {
    frame_type: c_int,
    timestamp: i64,
    codec: c_int,
    width: c_int,
    height: c_int,
    stride: c_int,
    flags: c_int,
    frame_rate_n: c_int,
    frame_rate_d: c_int,
    aspect_ratio: f32,
    color_space: c_int,
    sample_rate: c_int,
    channels: c_int,
    samples_per_channel: c_int,
    data: *mut c_void,
    data_length: c_int,
    compressed_data: *mut c_void,
    compressed_length: c_int,
    frame_metadata: *mut c_void,
    frame_metadata_length: c_int,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct OmtStatistics {
    bytes_sent: i64,
    bytes_received: i64,
    bytes_sent_since_last: i64,
    bytes_received_since_last: i64,
    frames: i64,
    frames_since_last: i64,
    frames_dropped: i64,
    codec_time: i64,
    codec_time_since_last: i64,
    reserved1: i64,
    reserved2: i64,
    reserved3: i64,
    reserved4: i64,
    reserved5: i64,
    reserved6: i64,
    reserved7: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum OmtError {
    #[error("OMT native library unavailable; set EIVIZ_OMT_LIBRARY to libomt: {0}")]
    LibraryUnavailable(String),
    #[error("OMT native symbol {symbol}: {source}")]
    Symbol {
        symbol: &'static str,
        source: libloading::Error,
    },
    #[error("OMT address contains NUL")]
    AddressNul,
    #[error("OMT receiver creation failed for {0}")]
    ReceiverCreate(String),
    #[error("invalid OMT frame: {0}")]
    InvalidFrame(String),
    #[error("unsupported OMT codec {0:#010x}")]
    UnsupportedCodec(c_int),
}

type Result<T> = std::result::Result<T, OmtError>;

struct NativeApi {
    _library: Library,
    _vmx_library: Library,
    discovery_get_addresses: DiscoveryGetAddresses,
    discovery_lock: Mutex<()>,
    receive_create: ReceiveCreate,
    receive_destroy: ReceiveDestroy,
    receive: Receive,
    receive_video_statistics: ReceiveStatistics,
    receive_audio_statistics: ReceiveStatistics,
    send_create: SendCreate,
    send_destroy: SendDestroy,
    send: SendFrame,
    loaded_from: PathBuf,
}

impl NativeApi {
    fn load() -> Result<Self> {
        let candidates = library_candidates()?;
        let mut errors = Vec::new();
        for path in candidates {
            let vmx_path = sibling_vmx_path(&path);
            // SAFETY: libvmx is an official native dependency loaded from the
            // same explicit directory as libomt and retained for process life.
            let vmx_library = match unsafe { Library::new(&vmx_path) } {
                Ok(library) => library,
                Err(error) => {
                    errors.push(format!("{}: {error}", vmx_path.display()));
                    continue;
                }
            };
            // SAFETY: Loading a vendor library is inherently unsafe. All resolved
            // symbols are checked below against libomt.h's C ABI and the Library
            // is retained for at least as long as any copied function pointer.
            let library = match unsafe { Library::new(&path) } {
                Ok(library) => library,
                Err(error) => {
                    errors.push(format!("{}: {error}", path.display()));
                    continue;
                }
            };
            // SAFETY: Symbol names and signatures are copied verbatim from the
            // MIT-licensed official libomt.h.
            let discovery_get_addresses =
                unsafe { load_symbol(&library, b"omt_discovery_getaddresses\0") }.map_err(
                    |source| OmtError::Symbol {
                        symbol: "omt_discovery_getaddresses",
                        source,
                    },
                )?;
            // SAFETY: See above.
            let receive_create = unsafe { load_symbol(&library, b"omt_receive_create\0") }
                .map_err(|source| OmtError::Symbol {
                    symbol: "omt_receive_create",
                    source,
                })?;
            // SAFETY: See above.
            let receive_destroy = unsafe { load_symbol(&library, b"omt_receive_destroy\0") }
                .map_err(|source| OmtError::Symbol {
                    symbol: "omt_receive_destroy",
                    source,
                })?;
            // SAFETY: See above.
            let receive = unsafe { load_symbol(&library, b"omt_receive\0") }.map_err(|source| {
                OmtError::Symbol {
                    symbol: "omt_receive",
                    source,
                }
            })?;
            // SAFETY: See above.
            let receive_video_statistics =
                unsafe { load_symbol(&library, b"omt_receive_getvideostatistics\0") }.map_err(
                    |source| OmtError::Symbol {
                        symbol: "omt_receive_getvideostatistics",
                        source,
                    },
                )?;
            // SAFETY: See above.
            let receive_audio_statistics =
                unsafe { load_symbol(&library, b"omt_receive_getaudiostatistics\0") }.map_err(
                    |source| OmtError::Symbol {
                        symbol: "omt_receive_getaudiostatistics",
                        source,
                    },
                )?;
            // SAFETY: See above.
            let send_create =
                unsafe { load_symbol(&library, b"omt_send_create\0") }.map_err(|source| {
                    OmtError::Symbol {
                        symbol: "omt_send_create",
                        source,
                    }
                })?;
            // SAFETY: See above.
            let send_destroy =
                unsafe { load_symbol(&library, b"omt_send_destroy\0") }.map_err(|source| {
                    OmtError::Symbol {
                        symbol: "omt_send_destroy",
                        source,
                    }
                })?;
            // SAFETY: See above.
            let send = unsafe { load_symbol(&library, b"omt_send\0") }.map_err(|source| {
                OmtError::Symbol {
                    symbol: "omt_send",
                    source,
                }
            })?;
            return Ok(Self {
                _library: library,
                _vmx_library: vmx_library,
                discovery_get_addresses,
                discovery_lock: Mutex::new(()),
                receive_create,
                receive_destroy,
                receive,
                receive_video_statistics,
                receive_audio_statistics,
                send_create,
                send_destroy,
                send,
                loaded_from: path,
            });
        }
        Err(OmtError::LibraryUnavailable(errors.join("; ")))
    }

    fn discover(&self) -> Result<Vec<String>> {
        let _guard = self.discovery_lock.lock();
        let mut count = 0;
        // SAFETY: The API owns the returned array until the next discovery call.
        // We copy every string before returning and serialize calls through the
        // process-wide API instance.
        let addresses = unsafe { (self.discovery_get_addresses)(&mut count) };
        if !(0..=4096).contains(&count) {
            return Err(OmtError::InvalidFrame(format!(
                "discovery count out of range: {count}"
            )));
        }
        if count == 0 {
            return Ok(Vec::new());
        }
        let addresses = NonNull::new(addresses)
            .ok_or_else(|| OmtError::InvalidFrame("null discovery array".into()))?;
        let mut result = Vec::with_capacity(count as usize);
        for index in 0..count as usize {
            // SAFETY: `count` is supplied by libomt for this array and is bounded
            // above. Each non-null pointer is documented as a UTF-8 C string.
            let ptr = unsafe { *addresses.as_ptr().add(index) };
            let ptr = NonNull::new(ptr)
                .ok_or_else(|| OmtError::InvalidFrame("null discovery address".into()))?;
            // SAFETY: libomt documents each entry as a NUL-terminated string.
            let value = unsafe { CStr::from_ptr(ptr.as_ptr()) }
                .to_string_lossy()
                .into_owned();
            result.push(value);
        }
        Ok(result)
    }
}

unsafe fn load_symbol<T: Copy>(
    library: &Library,
    name: &'static [u8],
) -> std::result::Result<T, libloading::Error> {
    // SAFETY: The caller supplies the exact C ABI type from official libomt.h.
    let symbol = unsafe { library.get::<T>(name)? };
    Ok(*symbol)
}

fn library_candidates() -> Result<Vec<PathBuf>> {
    if let Some(path) = std::env::var_os("EIVIZ_OMT_LIBRARY") {
        let path = PathBuf::from(path);
        if !path.is_absolute() {
            return Err(OmtError::LibraryUnavailable(
                "EIVIZ_OMT_LIBRARY must be an absolute path".into(),
            ));
        }
        return Ok(vec![path]);
    }
    let executable_dir = std::env::current_exe()
        .map_err(|error| OmtError::LibraryUnavailable(error.to_string()))?
        .parent()
        .ok_or_else(|| OmtError::LibraryUnavailable("executable has no parent".into()))?
        .to_path_buf();
    #[cfg(target_os = "windows")]
    {
        Ok(vec![executable_dir.join("libomt.dll")])
    }
    #[cfg(target_os = "macos")]
    {
        Ok(vec![executable_dir.join("libomt.dylib")])
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Ok(vec![executable_dir.join("libomt.so")])
    }
}

fn sibling_vmx_path(omt_path: &std::path::Path) -> PathBuf {
    let parent = omt_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    #[cfg(target_os = "windows")]
    {
        parent.join("libvmx.dll")
    }
    #[cfg(target_os = "macos")]
    {
        parent.join("libvmx.dylib")
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        parent.join("libvmx.so")
    }
}

fn api() -> Result<Arc<NativeApi>> {
    static API: OnceLock<std::result::Result<Arc<NativeApi>, String>> = OnceLock::new();
    match API.get_or_init(|| {
        NativeApi::load()
            .map(Arc::new)
            .map_err(|error| error.to_string())
    }) {
        Ok(api) => Ok(api.clone()),
        Err(error) => Err(OmtError::LibraryUnavailable(error.clone())),
    }
}

struct Receiver {
    api: Arc<NativeApi>,
    handle: NonNull<c_void>,
}

// SAFETY: libomt documents a receiver instance as usable by a receive worker;
// this wrapper moves the instance to exactly one worker thread.
unsafe impl Send for Receiver {}

impl Receiver {
    fn connect(address: &str) -> Result<Self> {
        let api = api()?;
        let address_c = CString::new(address).map_err(|_| OmtError::AddressNul)?;
        // SAFETY: The CString remains alive for the call and arguments match
        // libomt.h. The returned pointer is checked before wrapping.
        let handle = unsafe {
            (api.receive_create)(
                address_c.as_ptr(),
                OMT_FRAME_ALL,
                OMT_FORMAT_BGRA,
                OMT_RECEIVE_FLAGS_NONE,
            )
        };
        let handle =
            NonNull::new(handle).ok_or_else(|| OmtError::ReceiverCreate(address.to_owned()))?;
        Ok(Self { api, handle })
    }

    fn receive(&mut self) -> Option<OmtMediaFrame> {
        // SAFETY: This worker is the sole owner of the live receiver handle.
        // The frame struct is copied immediately; pointed-to data is copied by
        // conversion before this function is called again.
        let frame =
            unsafe { (self.api.receive)(self.handle.as_ptr(), OMT_FRAME_ALL, RECEIVE_TIMEOUT_MS) };
        NonNull::new(frame).map(|frame| {
            // SAFETY: libomt returned a non-null pointer valid until next receive.
            unsafe { *frame.as_ptr() }
        })
    }

    fn statistics(&self) -> (OmtStatistics, OmtStatistics) {
        let mut video = OmtStatistics::default();
        let mut audio = OmtStatistics::default();
        // SAFETY: The receiver handle is live and the output structs are valid
        // writable C-layout storage.
        unsafe {
            (self.api.receive_video_statistics)(self.handle.as_ptr(), &mut video);
            (self.api.receive_audio_statistics)(self.handle.as_ptr(), &mut audio);
        }
        (video, audio)
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        // SAFETY: The handle was created by this API and is destroyed exactly once
        // after the worker loop has stopped using it.
        unsafe { (self.api.receive_destroy)(self.handle.as_ptr()) };
    }
}

struct Sender {
    api: Arc<NativeApi>,
    handle: NonNull<c_void>,
}

// SAFETY: The native sender is moved to and used by exactly one output thread.
unsafe impl Send for Sender {}

impl Sender {
    fn create(name: &str) -> Result<Self> {
        let api = api()?;
        let name_c = CString::new(name).map_err(|_| OmtError::AddressNul)?;
        // SAFETY: CString and C ABI arguments are valid for this call. Quality 0
        // is OMTQuality_Default, allowing receivers to negotiate.
        let handle = unsafe { (api.send_create)(name_c.as_ptr(), 0) };
        let handle =
            NonNull::new(handle).ok_or_else(|| OmtError::ReceiverCreate(name.to_owned()))?;
        Ok(Self { api, handle })
    }

    fn send(&mut self, frame: &mut OmtMediaFrame) {
        // SAFETY: The frame and all pointed-to owned bytes remain alive for this
        // synchronous call. This worker exclusively owns the native sender.
        let _ = unsafe { (self.api.send)(self.handle.as_ptr(), frame) };
    }
}

impl Drop for Sender {
    fn drop(&mut self) {
        // SAFETY: Created by the matching API and destroyed exactly once.
        unsafe { (self.api.send_destroy)(self.handle.as_ptr()) };
    }
}

/// Real OMT receiver backed by the official `libomt` C ABI.
///
/// The native receive call runs on a dedicated thread. Runtime pulls only
/// already-copied frames, so no network or native blocking occurs on a tick.
pub struct OmtSource {
    id: InputId,
    address: String,
    video: Arc<Mutex<Option<VideoFrame>>>,
    audio: Arc<Mutex<VecDeque<AudioBuffer>>>,
    health: Arc<AtomicU8>,
    last_error: Arc<Mutex<Option<String>>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl OmtSource {
    pub fn connect(id: InputId, address: impl Into<String>) -> Result<Self> {
        let address = address.into();
        let receiver = Receiver::connect(&address)?;
        let video = Arc::new(Mutex::new(None));
        let audio = Arc::new(Mutex::new(VecDeque::with_capacity(AUDIO_QUEUE_CAPACITY)));
        let health = Arc::new(AtomicU8::new(health_to_u8(AdapterHealth::Degraded)));
        let last_error = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let worker = {
            let video = video.clone();
            let audio = audio.clone();
            let health = health.clone();
            let last_error = last_error.clone();
            let stop = stop.clone();
            thread::Builder::new()
                .name(format!("omt-receive-{id}"))
                .spawn(move || {
                    receive_loop(receiver, id, &video, &audio, &health, &last_error, &stop);
                })
                .map_err(|error| OmtError::ReceiverCreate(error.to_string()))?
        };
        Ok(Self {
            id,
            address,
            video,
            audio,
            health,
            last_error,
            stop,
            worker: Some(worker),
        })
    }

    pub fn address(&self) -> &str {
        &self.address
    }

    pub fn health(&self) -> AdapterHealth {
        u8_to_health(self.health.load(Ordering::Acquire))
    }

    pub fn last_error(&self) -> Option<String> {
        self.last_error.lock().clone()
    }
}

impl Drop for OmtSource {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl MediaSource for OmtSource {
    fn id(&self) -> InputId {
        self.id
    }

    fn pull_video(
        &self,
        _pts: MediaTime,
        _rate: FrameRate,
    ) -> eiviz_media::Result<Option<VideoFrame>> {
        Ok(self.video.lock().clone())
    }

    fn pull_audio(
        &self,
        _sample_index: u64,
        _frames: usize,
    ) -> eiviz_media::Result<Option<AudioBuffer>> {
        Ok(self.audio.lock().pop_front())
    }
}

enum OutputPacket {
    Video(VideoFrame),
    Audio(AudioBuffer),
}

/// Bounded asynchronous OMT output backed by the official `libomt` sender.
pub struct OmtSink {
    name: String,
    queue: SyncSender<OutputPacket>,
}

impl OmtSink {
    pub fn create(name: impl Into<String>, frame_rate: FrameRate) -> Result<Self> {
        let name = name.into();
        let sender = Sender::create(&name)?;
        let (queue, receiver) = sync_channel(4);
        thread::Builder::new()
            .name(format!("omt-send-{name}"))
            .spawn(move || {
                let mut sender = sender;
                while let Ok(packet) = receiver.recv() {
                    match packet {
                        OutputPacket::Video(frame) => {
                            if let Ok((mut native, _storage)) = video_to_native(&frame, frame_rate)
                            {
                                sender.send(&mut native);
                            }
                        }
                        OutputPacket::Audio(audio) => {
                            if let Ok((mut native, _storage)) = audio_to_native(&audio) {
                                sender.send(&mut native);
                            }
                        }
                    }
                }
            })
            .map_err(|error| OmtError::ReceiverCreate(error.to_string()))?;
        Ok(Self { name, queue })
    }
}

impl MediaSink for OmtSink {
    fn name(&self) -> &str {
        &self.name
    }

    fn push_video(&self, frame: &VideoFrame) -> eiviz_media::Result<()> {
        self.queue
            .try_send(OutputPacket::Video(frame.clone()))
            .map_err(map_output_queue_error)
    }

    fn push_audio(&self, audio: &AudioBuffer) -> eiviz_media::Result<()> {
        self.queue
            .try_send(OutputPacket::Audio(audio.clone()))
            .map_err(map_output_queue_error)
    }
}

fn map_output_queue_error(error: TrySendError<OutputPacket>) -> MediaError {
    match error {
        TrySendError::Full(_) => MediaError::QueueFull("omt-output"),
        TrySendError::Disconnected(_) => {
            MediaError::Disconnected("OMT output worker stopped".into())
        }
    }
}

pub fn discover_sources() -> Result<Vec<String>> {
    api()?.discover()
}

pub fn loaded_library() -> Result<PathBuf> {
    Ok(api()?.loaded_from.clone())
}

fn receive_loop(
    mut receiver: Receiver,
    id: InputId,
    video: &Mutex<Option<VideoFrame>>,
    audio: &Mutex<VecDeque<AudioBuffer>>,
    health: &AtomicU8,
    last_error: &Mutex<Option<String>>,
    stop: &AtomicBool,
) {
    let mut frame_id = 0u64;
    let mut last_frame = Instant::now();
    let mut discontinuity = false;
    let mut last_video_timestamp = None;
    while !stop.load(Ordering::Acquire) {
        let Some(frame) = receiver.receive() else {
            if last_frame.elapsed() >= Duration::from_secs(2) {
                discontinuity = true;
                let (video_stats, audio_stats) = receiver.statistics();
                health.store(health_to_u8(AdapterHealth::Degraded), Ordering::Release);
                *last_error.lock() = Some(format!(
                    "OMT receive timeout; video frames={} dropped={}, audio frames={} dropped={}",
                    video_stats.frames,
                    video_stats.frames_dropped,
                    audio_stats.frames,
                    audio_stats.frames_dropped
                ));
            }
            continue;
        };
        if frame.frame_type == OMT_FRAME_VIDEO {
            if let Some(previous) = last_video_timestamp {
                let expected = if frame.frame_rate_n > 0 && frame.frame_rate_d > 0 {
                    OMT_TICKS_PER_SECOND.saturating_mul(i64::from(frame.frame_rate_d))
                        / i64::from(frame.frame_rate_n)
                } else {
                    0
                };
                if frame.timestamp <= previous
                    || (expected > 0 && frame.timestamp - previous > expected.saturating_mul(4))
                {
                    discontinuity = true;
                }
            }
            last_video_timestamp = Some(frame.timestamp);
        }
        let mut degraded_reason = None;
        let converted = match frame.frame_type {
            OMT_FRAME_VIDEO => convert_video(&frame, id, frame_id).map(|mut frame| {
                frame.discontinuity = discontinuity;
                discontinuity = false;
                *video.lock() = Some(frame);
            }),
            OMT_FRAME_AUDIO => convert_audio(&frame).map(|frame| {
                let mut queue = audio.lock();
                if queue.len() == AUDIO_QUEUE_CAPACITY {
                    queue.pop_front();
                    degraded_reason = Some("OMT audio queue overflow; oldest packet dropped");
                }
                queue.push_back(frame);
            }),
            OMT_FRAME_METADATA => Ok(()),
            other => Err(OmtError::InvalidFrame(format!(
                "unknown frame type {other}"
            ))),
        };
        match converted {
            Ok(()) => {
                frame_id = frame_id.saturating_add(1);
                last_frame = Instant::now();
                if let Some(reason) = degraded_reason {
                    health.store(health_to_u8(AdapterHealth::Degraded), Ordering::Release);
                    *last_error.lock() = Some(reason.into());
                } else {
                    health.store(health_to_u8(AdapterHealth::Running), Ordering::Release);
                    *last_error.lock() = None;
                }
            }
            Err(error) => {
                health.store(health_to_u8(AdapterHealth::Degraded), Ordering::Release);
                *last_error.lock() = Some(error.to_string());
            }
        }
    }
}

fn convert_video(frame: &OmtMediaFrame, id: InputId, frame_id: u64) -> Result<VideoFrame> {
    validate_video_shape(frame)?;
    if frame.timestamp < 0 {
        return Err(OmtError::InvalidFrame("negative source timestamp".into()));
    }
    let width = frame.width as usize;
    let height = frame.height as usize;
    let stride = frame.stride as usize;
    let data_length = frame.data_length as usize;
    let data = NonNull::new(frame.data.cast::<u8>())
        .ok_or_else(|| OmtError::InvalidFrame("null video data".into()))?;
    // SAFETY: Shape validation proves DataLength is non-negative and bounded by
    // the dimensions/stride required below. We copy before the next receive call.
    let source = unsafe { std::slice::from_raw_parts(data.as_ptr(), data_length) };
    let mut rgba = vec![0u8; width * height * 4];
    match frame.codec {
        OMT_CODEC_BGRA => {
            let alpha = frame.flags & OMT_VIDEO_FLAG_ALPHA != 0;
            let premultiplied = frame.flags & OMT_VIDEO_FLAG_PREMULTIPLIED != 0;
            for row in 0..height {
                let src = &source[row * stride..row * stride + width * 4];
                let dst = &mut rgba[row * width * 4..(row + 1) * width * 4];
                for (input, output) in src.chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
                    let a = if alpha { input[3] } else { 255 };
                    let channels = [input[2], input[1], input[0]];
                    for (target, channel) in output[..3].iter_mut().zip(channels) {
                        *target = if premultiplied && alpha {
                            unpremultiply(channel, a)
                        } else {
                            channel
                        };
                    }
                    output[3] = a;
                }
            }
        }
        OMT_CODEC_UYVY => {
            for row in 0..height {
                let src = &source[row * stride..row * stride + width * 2];
                let dst = &mut rgba[row * width * 4..(row + 1) * width * 4];
                for (pair, output) in src.chunks_exact(4).zip(dst.chunks_exact_mut(8)) {
                    let u = pair[0];
                    let y0 = pair[1];
                    let v = pair[2];
                    let y1 = pair[3];
                    output[..4].copy_from_slice(&yuv_to_rgba(y0, u, v, frame.color_space));
                    output[4..].copy_from_slice(&yuv_to_rgba(y1, u, v, frame.color_space));
                }
            }
        }
        codec => return Err(OmtError::UnsupportedCodec(codec)),
    }
    Ok(VideoFrame {
        id: frame_id,
        source: Some(id),
        pts: MediaTime::new(
            frame.timestamp,
            Rational::new(1, OMT_TICKS_PER_SECOND).expect("constant timebase"),
        ),
        capture_domain: ClockDomain::SourceMedia,
        width: frame.width as u32,
        height: frame.height as u32,
        format: eiviz_media::PixelFormat::Rgba8,
        data: rgba.into(),
        discontinuity: false,
    })
}

fn unpremultiply(channel: u8, alpha: u8) -> u8 {
    if alpha == 0 {
        0
    } else {
        ((u32::from(channel) * 255 + u32::from(alpha) / 2) / u32::from(alpha)).min(255) as u8
    }
}

fn validate_video_shape(frame: &OmtMediaFrame) -> Result<()> {
    if frame.width <= 0
        || frame.height <= 0
        || frame.width > MAX_DIMENSION
        || frame.height > MAX_DIMENSION
        || frame.stride <= 0
        || frame.data_length <= 0
    {
        return Err(OmtError::InvalidFrame(format!(
            "video shape {}x{}, stride {}, length {}",
            frame.width, frame.height, frame.stride, frame.data_length
        )));
    }
    let bytes_per_pixel = match frame.codec {
        OMT_CODEC_BGRA => 4i64,
        OMT_CODEC_UYVY if frame.width % 2 == 0 => 2i64,
        OMT_CODEC_UYVY => {
            return Err(OmtError::InvalidFrame("UYVY width must be even".into()));
        }
        codec => return Err(OmtError::UnsupportedCodec(codec)),
    };
    let row_bytes = i64::from(frame.width)
        .checked_mul(bytes_per_pixel)
        .ok_or_else(|| OmtError::InvalidFrame("row size overflow".into()))?;
    if i64::from(frame.stride) < row_bytes {
        return Err(OmtError::InvalidFrame("video stride is too small".into()));
    }
    let required = i64::from(frame.stride)
        .checked_mul(i64::from(frame.height))
        .ok_or_else(|| OmtError::InvalidFrame("video buffer size overflow".into()))?;
    if i64::from(frame.data_length) < required {
        return Err(OmtError::InvalidFrame("video buffer is truncated".into()));
    }
    Ok(())
}

fn convert_audio(frame: &OmtMediaFrame) -> Result<AudioBuffer> {
    if frame.codec != OMT_CODEC_FPA1 {
        return Err(OmtError::UnsupportedCodec(frame.codec));
    }
    if frame.timestamp < 0
        || frame.sample_rate <= 0
        || frame.channels <= 0
        || frame.channels > MAX_AUDIO_CHANNELS
        || frame.samples_per_channel <= 0
        || frame.samples_per_channel > MAX_AUDIO_SAMPLES
        || frame.data_length <= 0
    {
        return Err(OmtError::InvalidFrame("invalid audio shape".into()));
    }
    let sample_count = (frame.channels as usize)
        .checked_mul(frame.samples_per_channel as usize)
        .ok_or_else(|| OmtError::InvalidFrame("audio sample count overflow".into()))?;
    let required = sample_count
        .checked_mul(std::mem::size_of::<f32>())
        .ok_or_else(|| OmtError::InvalidFrame("audio byte count overflow".into()))?;
    if (frame.data_length as usize) < required {
        return Err(OmtError::InvalidFrame("audio buffer is truncated".into()));
    }
    let data = NonNull::new(frame.data.cast::<u8>())
        .ok_or_else(|| OmtError::InvalidFrame("null audio data".into()))?;
    // SAFETY: The validated length covers the exact planar sample payload and
    // bytes are copied before the next receive call.
    let source = unsafe { std::slice::from_raw_parts(data.as_ptr(), required) };
    let samples_per_channel = frame.samples_per_channel as usize;
    let mut planes = vec![vec![0.0f32; samples_per_channel]; frame.channels as usize];
    for (channel, plane) in planes.iter_mut().enumerate() {
        let start = channel * samples_per_channel * 4;
        for (sample, bytes) in plane
            .iter_mut()
            .zip(source[start..start + samples_per_channel * 4].chunks_exact(4))
        {
            *sample = f32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        }
    }
    let sample_index = ((frame.timestamp as u128) * (frame.sample_rate as u128)
        / OMT_TICKS_PER_SECOND as u128)
        .min(u64::MAX as u128) as u64;
    Ok(AudioBuffer {
        sample_index,
        sample_rate: frame.sample_rate as u32,
        channels: frame.channels as u16,
        planes,
    })
}

fn video_to_native(frame: &VideoFrame, frame_rate: FrameRate) -> Result<(OmtMediaFrame, Vec<u8>)> {
    if frame.format != eiviz_media::PixelFormat::Rgba8 {
        return Err(OmtError::InvalidFrame(format!(
            "OMT output requires RGBA8, got {:?}",
            frame.format
        )));
    }
    if frame.width > c_int::MAX as u32 || frame.height > c_int::MAX as u32 {
        return Err(OmtError::InvalidFrame(
            "output video dimensions exceed C ABI".into(),
        ));
    }
    if frame_rate.numerator() > c_int::MAX as u32 || frame_rate.denominator() > c_int::MAX as u32 {
        return Err(OmtError::InvalidFrame(
            "output frame rate exceeds C ABI".into(),
        ));
    }
    let required = (frame.width as usize)
        .checked_mul(frame.height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| OmtError::InvalidFrame("output video size overflow".into()))?;
    if frame.data.len() < required {
        return Err(OmtError::InvalidFrame(
            "output video buffer is truncated".into(),
        ));
    }
    if required > c_int::MAX as usize {
        return Err(OmtError::InvalidFrame(
            "output video buffer exceeds C ABI".into(),
        ));
    }
    let mut bgra = vec![0u8; required];
    for (input, output) in frame.data.chunks_exact(4).zip(bgra.chunks_exact_mut(4)) {
        output.copy_from_slice(&[input[2], input[1], input[0], input[3]]);
    }
    let timestamp = media_time_to_omt(frame.pts)?;
    let native = OmtMediaFrame {
        frame_type: OMT_FRAME_VIDEO,
        timestamp,
        codec: OMT_CODEC_BGRA,
        width: frame.width as c_int,
        height: frame.height as c_int,
        stride: frame.width.saturating_mul(4) as c_int,
        flags: OMT_VIDEO_FLAG_ALPHA,
        frame_rate_n: frame_rate.numerator() as c_int,
        frame_rate_d: frame_rate.denominator() as c_int,
        aspect_ratio: frame.width as f32 / frame.height.max(1) as f32,
        color_space: OMT_COLORSPACE_BT709,
        sample_rate: 0,
        channels: 0,
        samples_per_channel: 0,
        data: bgra.as_mut_ptr().cast(),
        data_length: bgra.len() as c_int,
        compressed_data: std::ptr::null_mut(),
        compressed_length: 0,
        frame_metadata: std::ptr::null_mut(),
        frame_metadata_length: 0,
    };
    Ok((native, bgra))
}

fn audio_to_native(audio: &AudioBuffer) -> Result<(OmtMediaFrame, Vec<u8>)> {
    let samples_per_channel = audio.planes.first().map_or(0, Vec::len);
    if audio.sample_rate == 0
        || audio.sample_rate > c_int::MAX as u32
        || audio.channels == 0
        || audio.channels as usize > MAX_AUDIO_CHANNELS as usize
        || audio.channels as usize != audio.planes.len()
        || samples_per_channel > c_int::MAX as usize
        || audio
            .planes
            .iter()
            .any(|plane| plane.len() != samples_per_channel)
    {
        return Err(OmtError::InvalidFrame(
            "inconsistent output audio planes".into(),
        ));
    }
    let mut data = Vec::with_capacity(samples_per_channel * audio.planes.len() * 4);
    for plane in &audio.planes {
        for sample in plane {
            data.extend_from_slice(&sample.to_ne_bytes());
        }
    }
    if data.len() > c_int::MAX as usize {
        return Err(OmtError::InvalidFrame(
            "output audio buffer exceeds C ABI".into(),
        ));
    }
    let timestamp = ((audio.sample_index as u128) * OMT_TICKS_PER_SECOND as u128
        / audio.sample_rate as u128)
        .min(i64::MAX as u128) as i64;
    let native = OmtMediaFrame {
        frame_type: OMT_FRAME_AUDIO,
        timestamp,
        codec: OMT_CODEC_FPA1,
        width: 0,
        height: 0,
        stride: 0,
        flags: 0,
        frame_rate_n: 0,
        frame_rate_d: 0,
        aspect_ratio: 0.0,
        color_space: 0,
        sample_rate: audio.sample_rate as c_int,
        channels: audio.channels as c_int,
        samples_per_channel: samples_per_channel as c_int,
        data: data.as_mut_ptr().cast(),
        data_length: data.len() as c_int,
        compressed_data: std::ptr::null_mut(),
        compressed_length: 0,
        frame_metadata: std::ptr::null_mut(),
        frame_metadata_length: 0,
    };
    Ok((native, data))
}

fn media_time_to_omt(time: MediaTime) -> Result<i64> {
    let ticks = time.ticks() as i128;
    let timebase = time.timebase();
    let numerator = ticks
        .checked_mul(timebase.numerator() as i128)
        .and_then(|value| value.checked_mul(OMT_TICKS_PER_SECOND as i128))
        .ok_or_else(|| OmtError::InvalidFrame("timestamp overflow".into()))?;
    let value = numerator / timebase.denominator() as i128;
    i64::try_from(value).map_err(|_| OmtError::InvalidFrame("timestamp overflow".into()))
}

fn yuv_to_rgba(y: u8, u: u8, v: u8, color_space: c_int) -> [u8; 4] {
    let c = i32::from(y) - 16;
    let d = i32::from(u) - 128;
    let e = i32::from(v) - 128;
    let (r_e, g_d, g_e, b_d) = if color_space == OMT_COLORSPACE_BT709 {
        (459, 55, 136, 541)
    } else {
        (409, 100, 208, 516)
    };
    let r = (298 * c + r_e * e + 128) >> 8;
    let g = (298 * c - g_d * d - g_e * e + 128) >> 8;
    let b = (298 * c + b_d * d + 128) >> 8;
    [
        r.clamp(0, 255) as u8,
        g.clamp(0, 255) as u8,
        b.clamp(0, 255) as u8,
        255,
    ]
}

fn health_to_u8(health: AdapterHealth) -> u8 {
    match health {
        AdapterHealth::Running => 0,
        AdapterHealth::Degraded => 1,
        AdapterHealth::Unavailable => 2,
        AdapterHealth::Failed => 3,
    }
}

fn u8_to_health(value: u8) -> AdapterHealth {
    match value {
        0 => AdapterHealth::Running,
        1 => AdapterHealth::Degraded,
        2 => AdapterHealth::Unavailable,
        _ => AdapterHealth::Failed,
    }
}

impl From<OmtError> for MediaError {
    fn from(value: OmtError) -> Self {
        match value {
            OmtError::LibraryUnavailable(message) => MediaError::Unsupported(message),
            OmtError::ReceiverCreate(message) => MediaError::Disconnected(message),
            other => MediaError::Other(other.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame_with_data(codec: c_int, data: &mut [u8]) -> OmtMediaFrame {
        OmtMediaFrame {
            frame_type: OMT_FRAME_VIDEO,
            timestamp: 10_000_000,
            codec,
            width: 2,
            height: 1,
            stride: data.len() as c_int,
            flags: OMT_VIDEO_FLAG_ALPHA,
            frame_rate_n: 60_000,
            frame_rate_d: 1001,
            aspect_ratio: 16.0 / 9.0,
            color_space: OMT_COLORSPACE_BT709,
            sample_rate: 0,
            channels: 0,
            samples_per_channel: 0,
            data: data.as_mut_ptr().cast(),
            data_length: data.len() as c_int,
            compressed_data: std::ptr::null_mut(),
            compressed_length: 0,
            frame_metadata: std::ptr::null_mut(),
            frame_metadata_length: 0,
        }
    }

    #[test]
    fn bgra_is_copied_to_rgba_with_source_timestamp() {
        let mut data = [3, 2, 1, 4, 30, 20, 10, 40];
        let frame = frame_with_data(OMT_CODEC_BGRA, &mut data);
        let output = convert_video(&frame, InputId::new(), 7).unwrap();
        assert_eq!(output.pixel(0, 0), [1, 2, 3, 4]);
        assert_eq!(output.pixel(1, 0), [10, 20, 30, 40]);
        assert_eq!(output.pts.ticks(), 10_000_000);
        assert_eq!(output.pts.timebase(), Rational::new(1, 10_000_000).unwrap());
    }

    #[test]
    fn bgra_without_alpha_forces_opaque() {
        let mut data = [3, 2, 1, 0, 30, 20, 10, 0];
        let mut frame = frame_with_data(OMT_CODEC_BGRA, &mut data);
        frame.flags = 0;
        let output = convert_video(&frame, InputId::new(), 0).unwrap();
        assert_eq!(output.pixel(0, 0), [1, 2, 3, 255]);
    }

    #[test]
    fn premultiplied_bgra_is_converted_to_straight_alpha() {
        let mut data = [25, 50, 100, 128, 0, 0, 0, 0];
        let mut frame = frame_with_data(OMT_CODEC_BGRA, &mut data);
        frame.flags = OMT_VIDEO_FLAG_ALPHA | OMT_VIDEO_FLAG_PREMULTIPLIED;
        let output = convert_video(&frame, InputId::new(), 0).unwrap();
        assert_eq!(output.pixel(0, 0), [199, 100, 50, 128]);
        assert_eq!(output.pixel(1, 0), [0, 0, 0, 0]);
    }

    #[test]
    fn uyvy_bt709_converts_black_and_white() {
        let mut data = [128, 16, 128, 235];
        let frame = frame_with_data(OMT_CODEC_UYVY, &mut data);
        let output = convert_video(&frame, InputId::new(), 0).unwrap();
        assert_eq!(output.pixel(0, 0), [0, 0, 0, 255]);
        assert_eq!(output.pixel(1, 0), [255, 255, 255, 255]);
    }

    #[test]
    fn fpa1_planar_audio_is_copied() {
        let samples = [0.25f32, -0.5, 0.75, -1.0];
        let mut bytes = Vec::new();
        for sample in samples {
            bytes.extend_from_slice(&sample.to_ne_bytes());
        }
        let mut frame = frame_with_data(OMT_CODEC_FPA1, &mut bytes);
        frame.frame_type = OMT_FRAME_AUDIO;
        frame.sample_rate = 48_000;
        frame.channels = 2;
        frame.samples_per_channel = 2;
        let output = convert_audio(&frame).unwrap();
        assert_eq!(output.sample_index, 48_000);
        assert_eq!(output.planes, vec![vec![0.25, -0.5], vec![0.75, -1.0]]);
    }

    #[test]
    fn rgba_output_is_bgra_with_exact_omt_timestamp() {
        let frame = VideoFrame::rgba_solid(
            1,
            MediaTime::new(1, Rational::new(1001, 60_000).unwrap()),
            2,
            1,
            [1, 2, 3, 4],
        );
        let (native, data) = video_to_native(&frame, eiviz_time::NTSC_5994).unwrap();
        assert_eq!(native.timestamp, 166_833);
        assert_eq!(data, vec![3, 2, 1, 4, 3, 2, 1, 4]);
    }

    #[test]
    fn planar_audio_output_keeps_channel_order() {
        let audio = AudioBuffer {
            sample_index: 48_000,
            sample_rate: 48_000,
            channels: 2,
            planes: vec![vec![0.25, -0.5], vec![0.75, -1.0]],
        };
        let (native, data) = audio_to_native(&audio).unwrap();
        assert_eq!(native.timestamp, 10_000_000);
        let values = data
            .chunks_exact(4)
            .map(|bytes| f32::from_ne_bytes(bytes.try_into().unwrap()))
            .collect::<Vec<_>>();
        assert_eq!(values, vec![0.25, -0.5, 0.75, -1.0]);
    }

    #[test]
    fn truncated_frame_is_rejected() {
        let mut data = [0u8; 7];
        let frame = frame_with_data(OMT_CODEC_BGRA, &mut data);
        assert!(matches!(
            convert_video(&frame, InputId::new(), 0),
            Err(OmtError::InvalidFrame(_))
        ));
    }

    #[test]
    fn media_frame_layout_matches_official_64_bit_c_abi() {
        assert_eq!(std::mem::size_of::<OmtMediaFrame>(), 112);
        assert_eq!(std::mem::offset_of!(OmtMediaFrame, frame_type), 0);
        assert_eq!(std::mem::offset_of!(OmtMediaFrame, timestamp), 8);
        assert_eq!(std::mem::offset_of!(OmtMediaFrame, codec), 16);
        assert_eq!(std::mem::offset_of!(OmtMediaFrame, data), 64);
        assert_eq!(std::mem::offset_of!(OmtMediaFrame, data_length), 72);
        assert_eq!(std::mem::offset_of!(OmtMediaFrame, compressed_data), 80);
        assert_eq!(std::mem::offset_of!(OmtMediaFrame, frame_metadata), 96);
        assert_eq!(
            std::mem::offset_of!(OmtMediaFrame, frame_metadata_length),
            104
        );
    }
}
