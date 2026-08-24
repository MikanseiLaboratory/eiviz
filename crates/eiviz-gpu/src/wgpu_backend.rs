//! Explicit wgpu 25 compositor. **No CPU fallback.**
//!
//! A hardware adapter is required. Every layer is sampled and alpha-composited
//! by a WGSL render pipeline. The desktop injects eframe's adapter/device/queue
//! and can display [`WgpuTextureFrame`] directly. Readback remains an explicit,
//! counted staging operation for sinks that still require [`VideoFrame`].

use crate::{Layer, RenderPlan};
use eiviz_core::{
    ColorConversionPolicy, ColorMatrix, ColorMetadata, ColorRange, FieldKind, InputId,
    ToneMapPolicy, TransferFunction, Transform2D, VideoFormat,
};
use eiviz_media::{PixelFormat, VideoFrame};
use eiviz_time::MediaTime;
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

const SHADER: &str = r#"
struct LayerUniform {
    rect: vec4<f32>,
    crop: vec4<f32>,
    rotation_opacity: vec4<f32>,
    color_flags: vec4<u32>,
    tone_map: vec4<f32>,
};

@group(0) @binding(0) var source_texture: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;
@group(0) @binding(2) var<uniform> layer: LayerUniform;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

fn decode_transfer(value: vec3<f32>, code: f32) -> vec3<f32> {
    let v = max(value, vec3<f32>(0.0));
    if code < 0.5 {
        return pow(v, vec3<f32>(2.2));
    }
    if code < 1.5 {
        return select(v / 4.5, pow((v + 0.099) / 1.099, vec3<f32>(1.0 / 0.45)), v >= vec3<f32>(0.081));
    }
    if code < 2.5 {
        let p = pow(v, vec3<f32>(1.0 / 78.84375));
        return pow(max((p - 0.8359375) / (18.8515625 - 18.6875 * p), vec3<f32>(0.0)), vec3<f32>(1.0 / 0.1593017578125));
    }
    if code < 3.5 {
        return select((v * v) / 3.0, (exp((v - 0.55991073) / 0.17883277) + 0.28466892) / 12.0, v > vec3<f32>(0.5));
    }
    return v;
}

fn encode_transfer(value: vec3<f32>, code: f32) -> vec3<f32> {
    let v = max(value, vec3<f32>(0.0));
    if code < 0.5 {
        return pow(v, vec3<f32>(1.0 / 2.2));
    }
    if code < 1.5 {
        return select(4.5 * v, 1.099 * pow(v, vec3<f32>(0.45)) - 0.099, v >= vec3<f32>(0.018));
    }
    if code < 2.5 {
        let p = pow(v, vec3<f32>(0.1593017578125));
        return pow((0.8359375 + 18.8515625 * p) / (1.0 + 18.6875 * p), vec3<f32>(78.84375));
    }
    if code < 3.5 {
        return select(sqrt(3.0 * v), 0.17883277 * log(12.0 * v - 0.28466892) + 0.55991073, v > vec3<f32>(1.0 / 12.0));
    }
    return v;
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var points = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
    );
    let local = points[vertex_index];
    let centered = (local - vec2<f32>(0.5, 0.5)) * layer.rect.zw;
    let angle = layer.rotation_opacity.x;
    let rotated = vec2<f32>(
        centered.x * cos(angle) - centered.y * sin(angle),
        centered.x * sin(angle) + centered.y * cos(angle),
    );
    let canvas = layer.rect.xy + layer.rect.zw * 0.5 + rotated;
    let ndc = vec2<f32>(canvas.x * 2.0 - 1.0, 1.0 - canvas.y * 2.0);
    let available = vec2<f32>(
        1.0 - layer.crop.x - layer.crop.z,
        1.0 - layer.crop.y - layer.crop.w,
    );

    var output: VertexOutput;
    output.position = vec4<f32>(ndc, 0.0, 1.0);
    output.uv = layer.crop.xy + local * available;
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    var color = textureSample(source_texture, source_sampler, input.uv);
    if layer.color_flags.x == 1u {
        let yuv = color.rgb;
        var y = yuv.x;
        var cb = yuv.y - 0.5;
        var cr = yuv.z - 0.5;
        if layer.color_flags.y == 1u {
            if (layer.color_flags.z & 16u) != 0u {
                y = (yuv.x * 1023.0 - 64.0) / 876.0;
                cb = (yuv.y * 1023.0 - 512.0) / 896.0;
                cr = (yuv.z * 1023.0 - 512.0) / 896.0;
            } else {
                y = (yuv.x * 255.0 - 16.0) / 219.0;
                cb = (yuv.y * 255.0 - 128.0) / 224.0;
                cr = (yuv.z * 255.0 - 128.0) / 224.0;
            }
        }
        let matrix_code = layer.color_flags.z & 3u;
        if matrix_code == 2u {
            color = vec4<f32>(
                y + 1.4020 * cr,
                y - 0.344136 * cb - 0.714136 * cr,
                y + 1.7720 * cb,
                color.a,
            );
        } else if matrix_code == 0u {
            color = vec4<f32>(
                y + 1.5748 * cr,
                y - 0.1873 * cb - 0.4681 * cr,
                y + 1.8556 * cb,
                color.a,
            );
        } else {
            color = vec4<f32>(
                y + 1.4746 * cr,
                y - 0.1646 * cb - 0.5714 * cr,
                y + 1.8814 * cb,
                color.a,
            );
        }
    }
    if (layer.color_flags.z & 4u) != 0u {
        var linear = decode_transfer(color.rgb, layer.tone_map.z);
        let source_2020 = (layer.color_flags.z & 3u) == 1u;
        let target_2020 = (layer.color_flags.z & 8u) != 0u;
        if source_2020 && !target_2020 {
            linear = mat3x3<f32>(
                vec3<f32>(1.6605, -0.1246, -0.0182),
                vec3<f32>(-0.5876, 1.1329, -0.1006),
                vec3<f32>(-0.0728, -0.0083, 1.1187),
            ) * linear;
        } else if !source_2020 && target_2020 {
            linear = mat3x3<f32>(
                vec3<f32>(0.6274, 0.0691, 0.0164),
                vec3<f32>(0.3293, 0.9195, 0.0880),
                vec3<f32>(0.0433, 0.0114, 0.8956),
            ) * linear;
        }
        color = vec4<f32>(linear, color.a);
    }
    if layer.color_flags.w == 1u {
        let peak = max(layer.tone_map.x, 1.0);
        let target_luminance = max(layer.tone_map.y, 1.0);
        let scaled = max(color.rgb, vec3<f32>(0.0)) * peak / target_luminance;
        color = vec4<f32>(scaled / (vec3<f32>(1.0) + scaled), color.a);
    }
    if (layer.color_flags.z & 4u) != 0u {
        color = vec4<f32>(encode_transfer(color.rgb, layer.tone_map.w), color.a);
    }
    color.a *= layer.rotation_opacity.y;
    return color;
}
"#;

#[derive(Debug, thiserror::Error)]
pub enum WgpuError {
    #[error("no hardware GPU adapter")]
    NoHardwareAdapter,
    #[error("wgpu request device: {0}")]
    Device(#[from] wgpu::RequestDeviceError),
    #[error("wgpu validation: {0}")]
    Validation(String),
    #[error("invalid render plan: {0}")]
    InvalidPlan(String),
    #[error("missing source frame for input {0}")]
    MissingSource(InputId),
    #[error("unsupported source format for input {input}: {format:?}")]
    UnsupportedFormat { input: InputId, format: PixelFormat },
    #[error("unsupported video profile: {0}")]
    UnsupportedProfile(String),
    #[error("color conversion rejected for input {input}: {detail}")]
    ColorConversion { input: InputId, detail: String },
    #[error("wgpu map readback failed: {0}")]
    Map(String),
    #[error("wgpu device was lost: {0}")]
    DeviceLost(String),
    #[error("GPU resource was not prewarmed: {0}")]
    UnpreparedResource(String),
    #[error(
        "GPU resource prewarm exceeds pool limits: requires {required_bytes} bytes/{required_resources} resources, limit is {limit_bytes} bytes/{limit_resources} resources"
    )]
    PoolLimit {
        required_bytes: u64,
        required_resources: usize,
        limit_bytes: u64,
        limit_resources: usize,
    },
}

/// A compositor result that remains on the shared GPU device.
#[derive(Clone)]
pub struct WgpuTextureFrame {
    resource: Arc<ResourceLease>,
    pub width: u32,
    pub height: u32,
    pub pts: MediaTime,
    pub frame_id: u64,
    pub format: PixelFormat,
    pub color: ColorMetadata,
    pub field: FieldKind,
}

impl std::fmt::Debug for WgpuTextureFrame {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WgpuTextureFrame")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("pts", &self.pts)
            .field("frame_id", &self.frame_id)
            .field("format", &self.format)
            .field("color", &self.color)
            .field("field", &self.field)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdapterCapabilities {
    pub max_texture_dimension_2d: u32,
    pub max_buffer_size: u64,
    pub rgba16_float_renderable: bool,
    pub rgba16_float_filterable: bool,
    pub rgba32_float_sampleable: bool,
}

impl AdapterCapabilities {
    fn detect(adapter: &wgpu::Adapter) -> Self {
        let limits = adapter.limits();
        let rgba16 = adapter.get_texture_format_features(wgpu::TextureFormat::Rgba16Float);
        let rgba32 = adapter.get_texture_format_features(wgpu::TextureFormat::Rgba32Float);
        Self {
            max_texture_dimension_2d: limits.max_texture_dimension_2d,
            max_buffer_size: limits.max_buffer_size,
            rgba16_float_renderable: rgba16
                .allowed_usages
                .contains(wgpu::TextureUsages::RENDER_ATTACHMENT),
            rgba16_float_filterable: rgba16
                .flags
                .contains(wgpu::TextureFormatFeatureFlags::FILTERABLE),
            rgba32_float_sampleable: rgba32
                .allowed_usages
                .contains(wgpu::TextureUsages::TEXTURE_BINDING),
        }
    }

    pub fn admit(&self, format: &VideoFormat) -> Result<(), WgpuError> {
        if format.width > self.max_texture_dimension_2d
            || format.height > self.max_texture_dimension_2d
        {
            return Err(WgpuError::UnsupportedProfile(format!(
                "{}x{} exceeds adapter max texture dimension {}",
                format.width, format.height, self.max_texture_dimension_2d
            )));
        }
        if format.bit_depth == 10
            && (!self.rgba16_float_renderable || !self.rgba16_float_filterable)
        {
            return Err(WgpuError::UnsupportedProfile(
                "10-bit/HDR requires renderable and filterable RGBA16Float".into(),
            ));
        }
        Ok(())
    }
}

impl WgpuTextureFrame {
    pub fn texture(&self) -> &wgpu::Texture {
        self.resource.output().0
    }

    pub fn view(&self) -> &wgpu::TextureView {
        self.resource.output().1
    }
}

/// Hard bounds for all reusable compositor allocations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResourcePoolLimits {
    pub max_bytes: u64,
    pub max_resources: usize,
}

impl Default for ResourcePoolLimits {
    fn default() -> Self {
        Self {
            max_bytes: 2 * 1024 * 1024 * 1024,
            max_resources: 16_384,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResourcePoolDiagnostics {
    pub limit_bytes: u64,
    pub limit_resources: usize,
    pub resident_bytes: u64,
    pub resident_resources: usize,
    pub idle_resources: usize,
    pub source_resources: usize,
    pub output_resources: usize,
    pub readback_resources: usize,
    pub allocations: u64,
    pub reuses: u64,
    pub evictions: u64,
    pub acquisition_misses: u64,
    pub prewarm_generations: u64,
    pub last_evicted: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum ResourceKind {
    Source,
    Output,
    Readback,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum PoolFormat {
    Rgba8,
    Bgra8,
    Rgba16Float,
}

impl PoolFormat {
    const fn bytes_per_pixel(self) -> u64 {
        match self {
            Self::Rgba8 | Self::Bgra8 => 4,
            Self::Rgba16Float => 8,
        }
    }

    const fn texture_format(self) -> wgpu::TextureFormat {
        match self {
            Self::Rgba8 => wgpu::TextureFormat::Rgba8Unorm,
            Self::Bgra8 => wgpu::TextureFormat::Bgra8Unorm,
            Self::Rgba16Float => wgpu::TextureFormat::Rgba16Float,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ResourceKey {
    kind: ResourceKind,
    format: PoolFormat,
    width: u32,
    height: u32,
}

impl ResourceKey {
    fn bytes(self) -> u64 {
        let row_bytes = u64::from(self.width).saturating_mul(self.format.bytes_per_pixel());
        match self.kind {
            ResourceKind::Readback => {
                let alignment = u64::from(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
                row_bytes
                    .div_ceil(alignment)
                    .saturating_mul(alignment)
                    .saturating_mul(u64::from(self.height))
            }
            ResourceKind::Source => row_bytes
                .saturating_mul(u64::from(self.height))
                .saturating_add(80),
            ResourceKind::Output => row_bytes.saturating_mul(u64::from(self.height)),
        }
    }
}

struct SourceResource {
    texture: wgpu::Texture,
    _view: wgpu::TextureView,
    uniform: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

struct OutputResource {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

struct ReadbackResource {
    buffer: wgpu::Buffer,
}

enum GpuResource {
    Source(SourceResource),
    Output(OutputResource),
    Readback(ReadbackResource),
}

struct IdleResource {
    sequence: u64,
    resource: GpuResource,
}

struct ResourcePool {
    limits: ResourcePoolLimits,
    idle: BTreeMap<ResourceKey, VecDeque<IdleResource>>,
    resident_by_key: BTreeMap<ResourceKey, usize>,
    resident_bytes: u64,
    resident_resources: usize,
    sequence: u64,
    diagnostics: ResourcePoolDiagnostics,
}

impl ResourcePool {
    fn new(limits: ResourcePoolLimits) -> Self {
        Self {
            limits,
            idle: BTreeMap::new(),
            resident_by_key: BTreeMap::new(),
            resident_bytes: 0,
            resident_resources: 0,
            sequence: 0,
            diagnostics: ResourcePoolDiagnostics::default(),
        }
    }

    fn idle_count(&self) -> usize {
        self.idle.values().map(VecDeque::len).sum()
    }

    fn diagnostics(&self) -> ResourcePoolDiagnostics {
        let mut value = self.diagnostics.clone();
        value.limit_bytes = self.limits.max_bytes;
        value.limit_resources = self.limits.max_resources;
        value.resident_bytes = self.resident_bytes;
        value.resident_resources = self.resident_resources;
        value.idle_resources = self.idle_count();
        value.source_resources = self
            .resident_by_key
            .iter()
            .filter(|(key, _)| key.kind == ResourceKind::Source)
            .map(|(_, count)| *count)
            .sum();
        value.output_resources = self
            .resident_by_key
            .iter()
            .filter(|(key, _)| key.kind == ResourceKind::Output)
            .map(|(_, count)| *count)
            .sum();
        value.readback_resources = self
            .resident_by_key
            .iter()
            .filter(|(key, _)| key.kind == ResourceKind::Readback)
            .map(|(_, count)| *count)
            .sum();
        value
    }

    fn acquire(pool: &Arc<Mutex<Self>>, key: ResourceKey) -> Result<ResourceLease, WgpuError> {
        let resource = {
            let mut state = pool
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let resource = state
                .idle
                .get_mut(&key)
                .and_then(VecDeque::pop_front)
                .map(|idle| idle.resource);
            if resource.is_some() {
                state.diagnostics.reuses = state.diagnostics.reuses.saturating_add(1);
            } else {
                state.diagnostics.acquisition_misses =
                    state.diagnostics.acquisition_misses.saturating_add(1);
            }
            resource
        };
        resource
            .map(|resource| ResourceLease {
                key,
                resource: Some(resource),
                pool: Arc::clone(pool),
            })
            .ok_or_else(|| {
                WgpuError::UnpreparedResource(format!(
                    "{:?}/{:?} {}x{}",
                    key.kind, key.format, key.width, key.height
                ))
            })
    }

    fn release(&mut self, key: ResourceKey, resource: GpuResource) {
        self.sequence = self.sequence.saturating_add(1);
        self.idle.entry(key).or_default().push_back(IdleResource {
            sequence: self.sequence,
            resource,
        });
    }

    fn evict_one(&mut self, required: &BTreeMap<ResourceKey, usize>) -> bool {
        let candidate = self
            .idle
            .iter()
            .filter(|(key, idle)| {
                let resident = self.resident_by_key.get(key).copied().unwrap_or(0);
                let protected = required.get(key).copied().unwrap_or(0);
                !idle.is_empty() && resident > protected
            })
            .flat_map(|(key, idle)| {
                idle.iter()
                    .enumerate()
                    .map(move |(index, value)| (value.sequence, *key, index))
            })
            .min();
        let Some((_, key, index)) = candidate else {
            return false;
        };
        let queue = self.idle.get_mut(&key).expect("candidate queue exists");
        let _ = queue.remove(index).expect("candidate entry exists");
        if queue.is_empty() {
            self.idle.remove(&key);
        }
        let count = self
            .resident_by_key
            .get_mut(&key)
            .expect("resident key exists");
        *count -= 1;
        if *count == 0 {
            self.resident_by_key.remove(&key);
        }
        self.resident_resources -= 1;
        self.resident_bytes = self.resident_bytes.saturating_sub(key.bytes());
        self.diagnostics.evictions = self.diagnostics.evictions.saturating_add(1);
        self.diagnostics.last_evicted = Some(format!(
            "{:?}/{:?} {}x{}",
            key.kind, key.format, key.width, key.height
        ));
        true
    }
}

struct ResourceLease {
    key: ResourceKey,
    resource: Option<GpuResource>,
    pool: Arc<Mutex<ResourcePool>>,
}

impl ResourceLease {
    fn source(&self) -> &SourceResource {
        match self.resource.as_ref().expect("live resource lease") {
            GpuResource::Source(resource) => resource,
            _ => unreachable!("source lease has source resource"),
        }
    }

    fn output(&self) -> (&wgpu::Texture, &wgpu::TextureView) {
        match self.resource.as_ref().expect("live resource lease") {
            GpuResource::Output(resource) => (&resource.texture, &resource.view),
            _ => unreachable!("output lease has output resource"),
        }
    }

    fn readback(&self) -> &wgpu::Buffer {
        match self.resource.as_ref().expect("live resource lease") {
            GpuResource::Readback(resource) => &resource.buffer,
            _ => unreachable!("readback lease has readback resource"),
        }
    }
}

impl Drop for ResourceLease {
    fn drop(&mut self) {
        if let Some(resource) = self.resource.take() {
            self.pool
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .release(self.key, resource);
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeviceLossReport {
    pub reason: String,
    pub message: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct WgpuDiagnostics {
    pub readbacks: u64,
    pub pass_nanos: u64,
    pub pass_total_nanos: u64,
    pub pass_max_nanos: u64,
    pub readback_nanos: u64,
    pub readback_max_nanos: u64,
    pub device_loss: Option<DeviceLossReport>,
    /// Device recreation is owned by the caller that supplied the device.
    pub automatic_recovery: bool,
    pub pool: ResourcePoolDiagnostics,
}

#[derive(Default)]
struct SharedDiagnostics {
    readbacks: AtomicU64,
    pass_nanos: AtomicU64,
    pass_total_nanos: AtomicU64,
    pass_max_nanos: AtomicU64,
    readback_nanos: AtomicU64,
    readback_max_nanos: AtomicU64,
    device_loss: Mutex<Option<DeviceLossReport>>,
}

/// Cloneable handles for lazily constructing the compositor on a GUI-owned
/// adapter/device/queue. Keeping this context does not select the Wgpu backend.
#[derive(Clone, Debug)]
pub struct SharedWgpuContext {
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

impl SharedWgpuContext {
    pub fn new(adapter: wgpu::Adapter, device: wgpu::Device, queue: wgpu::Queue) -> Self {
        Self {
            adapter,
            device,
            queue,
        }
    }

    pub fn create_compositor(&self) -> Result<WgpuCompositor, WgpuError> {
        WgpuCompositor::from_shared_device(&self.adapter, self.device.clone(), self.queue.clone())
    }
}

pub struct WgpuCompositor {
    // The headless profile owns an Instance. The injected desktop profile does not.
    _instance: Option<wgpu::Instance>,
    adapter_info: wgpu::AdapterInfo,
    capabilities: AdapterCapabilities,
    device: wgpu::Device,
    queue: wgpu::Queue,
    bind_group_layout: wgpu::BindGroupLayout,
    pipeline_8: wgpu::RenderPipeline,
    pipeline_16: Option<wgpu::RenderPipeline>,
    sampler: wgpu::Sampler,
    resources: Arc<Mutex<ResourcePool>>,
    diagnostics: Arc<SharedDiagnostics>,
    latest_output: Mutex<Option<WgpuTextureFrame>>,
}

impl WgpuCompositor {
    /// Construct a separate hardware-only device for headless workers and HIL.
    ///
    /// GUI applications must use [`Self::from_shared_device`] so eframe and the
    /// compositor do not create two logical devices for one physical GPU.
    pub fn new_headless_hardware() -> Result<Self, WgpuError> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
        }))
        .map_err(|_| WgpuError::NoHardwareAdapter)?;
        let info = adapter.get_info();
        if matches!(info.device_type, wgpu::DeviceType::Cpu) {
            return Err(WgpuError::NoHardwareAdapter);
        }
        let capabilities = AdapterCapabilities::detect(&adapter);
        let required_limits = adapter.limits();
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("eiviz-wgpu-compositor"),
                required_features: wgpu::Features::empty(),
                required_limits,
                memory_hints: wgpu::MemoryHints::Performance,
                trace: wgpu::Trace::Off,
            }))?;
        Self::build(Some(instance), info, capabilities, device, queue)
    }

    /// Build compositor resources on an adapter/device/queue owned by the GUI.
    ///
    /// This is the required desktop path for `CreationContext::wgpu_render_state`.
    pub fn from_shared_device(
        adapter: &wgpu::Adapter,
        device: wgpu::Device,
        queue: wgpu::Queue,
    ) -> Result<Self, WgpuError> {
        let info = adapter.get_info();
        if matches!(info.device_type, wgpu::DeviceType::Cpu) {
            return Err(WgpuError::NoHardwareAdapter);
        }
        let capabilities = AdapterCapabilities::detect(adapter);
        Self::build(None, info, capabilities, device, queue)
    }

    fn build(
        instance: Option<wgpu::Instance>,
        adapter_info: wgpu::AdapterInfo,
        capabilities: AdapterCapabilities,
        device: wgpu::Device,
        queue: wgpu::Queue,
    ) -> Result<Self, WgpuError> {
        Self::build_with_limits(
            instance,
            adapter_info,
            capabilities,
            device,
            queue,
            ResourcePoolLimits::default(),
        )
    }

    fn build_with_limits(
        instance: Option<wgpu::Instance>,
        adapter_info: wgpu::AdapterInfo,
        capabilities: AdapterCapabilities,
        device: wgpu::Device,
        queue: wgpu::Queue,
        pool_limits: ResourcePoolLimits,
    ) -> Result<Self, WgpuError> {
        let diagnostics = Arc::new(SharedDiagnostics::default());
        let device_loss = diagnostics.clone();
        device.set_device_lost_callback(move |reason, message| {
            tracing::error!(reason = ?reason, message = %message, "GPU device lost");
            *device_loss
                .device_loss
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(DeviceLossReport {
                reason: format!("{reason:?}"),
                message,
            });
        });
        device.push_error_scope(wgpu::ErrorFilter::Validation);
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("eiviz-layer-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("eiviz-layer-shader"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(SHADER)),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("eiviz-compositor-pipeline-layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });
        let create_pipeline = |label, format| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview: None,
                cache: None,
            })
        };
        let pipeline_8 = create_pipeline(
            "eiviz-compositor-rgba8-pipeline",
            wgpu::TextureFormat::Rgba8Unorm,
        );
        let pipeline_16 = capabilities.rgba16_float_renderable.then(|| {
            create_pipeline(
                "eiviz-compositor-rgba16f-pipeline",
                wgpu::TextureFormat::Rgba16Float,
            )
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("eiviz-nearest-sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let _ = device.poll(wgpu::PollType::Wait);
        if let Some(error) = pollster::block_on(device.pop_error_scope()) {
            return Err(WgpuError::Validation(error.to_string()));
        }
        Ok(Self {
            _instance: instance,
            adapter_info,
            capabilities,
            device,
            queue,
            bind_group_layout,
            pipeline_8,
            pipeline_16,
            sampler,
            resources: Arc::new(Mutex::new(ResourcePool::new(pool_limits))),
            diagnostics,
            latest_output: Mutex::new(None),
        })
    }

    pub fn adapter_info(&self) -> wgpu::AdapterInfo {
        self.adapter_info.clone()
    }

    pub fn capabilities(&self) -> &AdapterCapabilities {
        &self.capabilities
    }

    pub fn admit_video_format(&self, format: &VideoFormat) -> Result<(), WgpuError> {
        self.capabilities.admit(format)
    }

    /// Materialize every resident resource needed by an immutable runtime
    /// snapshot. Rendering only acquires from this pool; it never grows it.
    pub fn prewarm_snapshot(
        &self,
        plans: &[RenderPlan],
        source_width: u32,
        source_height: u32,
        source_slots: usize,
    ) -> Result<(), WgpuError> {
        self.ensure_device_available()?;
        let mut required = BTreeMap::<ResourceKey, usize>::new();
        for plan in plans {
            validate_plan(plan)?;
            let format = pool_format(plan.output_format)?;
            let output = ResourceKey {
                kind: ResourceKind::Output,
                format,
                width: plan.width,
                height: plan.height,
            };
            *required.entry(output).or_default() += 1;
            required.insert(
                ResourceKey {
                    kind: ResourceKind::Readback,
                    ..output
                },
                1,
            );
        }
        // One spare output per key allows the next frame to render while the
        // previous native GUI texture remains leased.
        for count in required
            .iter_mut()
            .filter_map(|(key, count)| (key.kind == ResourceKind::Output).then_some(count))
        {
            *count = count.saturating_add(1);
        }
        if source_slots > 0 {
            for format in [
                PoolFormat::Rgba8,
                PoolFormat::Bgra8,
                PoolFormat::Rgba16Float,
            ] {
                required.insert(
                    ResourceKey {
                        kind: ResourceKind::Source,
                        format,
                        width: source_width,
                        height: source_height,
                    },
                    source_slots,
                );
            }
        }
        let required_resources = required.values().copied().sum::<usize>();
        let required_bytes = required.iter().fold(0_u64, |total, (key, count)| {
            total.saturating_add(key.bytes().saturating_mul(*count as u64))
        });
        let mut pool = self
            .resources
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if required_bytes > pool.limits.max_bytes || required_resources > pool.limits.max_resources
        {
            return Err(WgpuError::PoolLimit {
                required_bytes,
                required_resources,
                limit_bytes: pool.limits.max_bytes,
                limit_resources: pool.limits.max_resources,
            });
        }
        let additions = required
            .iter()
            .map(|(key, count)| {
                let resident = pool.resident_by_key.get(key).copied().unwrap_or(0);
                (*key, count.saturating_sub(resident))
            })
            .filter(|(_, count)| *count > 0)
            .collect::<Vec<_>>();
        let addition_resources = additions.iter().map(|(_, count)| *count).sum::<usize>();
        let addition_bytes = additions.iter().fold(0_u64, |total, (key, count)| {
            total.saturating_add(key.bytes().saturating_mul(*count as u64))
        });
        while pool.resident_bytes.saturating_add(addition_bytes) > pool.limits.max_bytes
            || pool.resident_resources.saturating_add(addition_resources)
                > pool.limits.max_resources
        {
            if !pool.evict_one(&required) {
                return Err(WgpuError::PoolLimit {
                    required_bytes: pool.resident_bytes.saturating_add(addition_bytes),
                    required_resources: pool.resident_resources.saturating_add(addition_resources),
                    limit_bytes: pool.limits.max_bytes,
                    limit_resources: pool.limits.max_resources,
                });
            }
        }
        for (key, count) in additions {
            for _ in 0..count {
                let resource = self.create_resource(key);
                pool.resident_bytes = pool.resident_bytes.saturating_add(key.bytes());
                pool.resident_resources = pool.resident_resources.saturating_add(1);
                *pool.resident_by_key.entry(key).or_default() += 1;
                pool.diagnostics.allocations = pool.diagnostics.allocations.saturating_add(1);
                pool.release(key, resource);
            }
        }
        pool.diagnostics.prewarm_generations =
            pool.diagnostics.prewarm_generations.saturating_add(1);
        drop(pool);
        let _ = self.device.poll(wgpu::PollType::Wait);
        Ok(())
    }

    fn create_resource(&self, key: ResourceKey) -> GpuResource {
        match key.kind {
            ResourceKind::Source => {
                let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("eiviz-pooled-source"),
                    size: wgpu::Extent3d {
                        width: key.width,
                        height: key.height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: key.format.texture_format(),
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                });
                let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                let uniform = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("eiviz-pooled-layer-uniform"),
                    size: 80,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("eiviz-pooled-layer-bind-group"),
                    layout: &self.bind_group_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&self.sampler),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: uniform.as_entire_binding(),
                        },
                    ],
                });
                GpuResource::Source(SourceResource {
                    texture,
                    _view: view,
                    uniform,
                    bind_group,
                })
            }
            ResourceKind::Output => {
                let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("eiviz-pooled-output"),
                    size: wgpu::Extent3d {
                        width: key.width,
                        height: key.height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: key.format.texture_format(),
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::COPY_SRC
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                });
                let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                GpuResource::Output(OutputResource { texture, view })
            }
            ResourceKind::Readback => {
                let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("eiviz-pooled-readback"),
                    size: key.bytes(),
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                });
                GpuResource::Readback(ReadbackResource { buffer })
            }
        }
    }

    /// Release the compositor's own native-texture lease before a snapshot or
    /// device boundary. Runtime stream leases are released separately.
    pub fn clear_latest_output(&self) {
        self.latest_output
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
    }

    pub fn diagnostics(&self) -> WgpuDiagnostics {
        WgpuDiagnostics {
            readbacks: self.diagnostics.readbacks.load(Ordering::Relaxed),
            pass_nanos: self.diagnostics.pass_nanos.load(Ordering::Relaxed),
            pass_total_nanos: self.diagnostics.pass_total_nanos.load(Ordering::Relaxed),
            pass_max_nanos: self.diagnostics.pass_max_nanos.load(Ordering::Relaxed),
            readback_nanos: self.diagnostics.readback_nanos.load(Ordering::Relaxed),
            readback_max_nanos: self.diagnostics.readback_max_nanos.load(Ordering::Relaxed),
            device_loss: self
                .diagnostics
                .device_loss
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone(),
            automatic_recovery: false,
            pool: self
                .resources
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .diagnostics(),
        }
    }

    /// Most recently submitted compositor texture. Runtime code snapshots this
    /// into stream-specific slots immediately after each composition.
    pub fn latest_output(&self) -> Option<WgpuTextureFrame> {
        self.latest_output
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn ensure_device_available(&self) -> Result<(), WgpuError> {
        if let Some(loss) = self.diagnostics().device_loss {
            return Err(WgpuError::DeviceLost(format!(
                "{}: {}",
                loss.reason, loss.message
            )));
        }
        Ok(())
    }

    /// Composite and explicitly read back for a sink that requires `VideoFrame`.
    ///
    /// GUI preview code should use [`Self::composite_texture`] and register its
    /// texture view with egui instead of performing a GPU→CPU→GUI GPU round trip.
    pub fn composite(
        &self,
        plan: &RenderPlan,
        sources: &HashMap<InputId, VideoFrame>,
        pts: MediaTime,
        frame_id: u64,
    ) -> Result<VideoFrame, WgpuError> {
        let texture = self.composite_texture(plan, sources, pts, frame_id)?;
        self.readback(&texture)
    }

    /// Composite all layers and retain the output on the shared GPU device.
    pub fn composite_texture(
        &self,
        plan: &RenderPlan,
        sources: &HashMap<InputId, VideoFrame>,
        pts: MediaTime,
        frame_id: u64,
    ) -> Result<WgpuTextureFrame, WgpuError> {
        let started = std::time::Instant::now();
        let span = tracing::info_span!(
            "gpu_pass",
            frame_id,
            width = plan.width,
            height = plan.height,
            layers = plan.layers.len()
        );
        let _entered = span.enter();
        self.ensure_device_available()?;
        validate_plan(plan)?;
        let max_dimension = self.device.limits().max_texture_dimension_2d;
        if plan.width > max_dimension || plan.height > max_dimension {
            return Err(WgpuError::InvalidPlan(format!(
                "output {}x{} exceeds GPU max dimension {max_dimension}",
                plan.width, plan.height
            )));
        }
        if plan.output_format == PixelFormat::Rgba16Float && self.pipeline_16.is_none() {
            return Err(WgpuError::UnsupportedProfile(
                "render plan requires RGBA16Float but adapter cannot render it".into(),
            ));
        }
        self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let layers = match self.prepare_layers(plan, sources, frame_id) {
            Ok(layers) => layers,
            Err(error) => {
                let _ = self.device.poll(wgpu::PollType::Wait);
                let _ = pollster::block_on(self.device.pop_error_scope());
                return Err(error);
            }
        };
        let output = ResourcePool::acquire(
            &self.resources,
            ResourceKey {
                kind: ResourceKind::Output,
                format: pool_format(plan.output_format)?,
                width: plan.width,
                height: plan.height,
            },
        )?;
        let (_, output_view) = output.output();
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("eiviz-compositor-encoder"),
            });
        {
            let color_attachment = Some(wgpu::RenderPassColorAttachment {
                view: output_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("eiviz-compositor-pass"),
                color_attachments: &[color_attachment],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(match plan.output_format {
                PixelFormat::Rgba8 => &self.pipeline_8,
                PixelFormat::Rgba16Float => self
                    .pipeline_16
                    .as_ref()
                    .expect("validated RGBA16Float pipeline above"),
                _ => unreachable!("validated output format above"),
            });
            for layer in &layers {
                pass.set_bind_group(0, &layer.resource.source().bind_group, &[]);
                pass.draw(0..6, 0..1);
            }
        }
        self.queue.submit(Some(encoder.finish()));
        let _ = self.device.poll(wgpu::PollType::Wait);
        if let Some(error) = pollster::block_on(self.device.pop_error_scope()) {
            return Err(WgpuError::Validation(error.to_string()));
        }
        let frame = WgpuTextureFrame {
            resource: Arc::new(output),
            width: plan.width,
            height: plan.height,
            pts,
            frame_id,
            format: plan.output_format,
            color: plan.color,
            field: plan.field_at(frame_id),
        };
        *self
            .latest_output
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(frame.clone());
        let elapsed = started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
        self.diagnostics
            .pass_nanos
            .store(elapsed, Ordering::Relaxed);
        self.diagnostics
            .pass_total_nanos
            .fetch_add(elapsed, Ordering::Relaxed);
        self.diagnostics
            .pass_max_nanos
            .fetch_max(elapsed, Ordering::Relaxed);
        tracing::info!(frame_id, pass_nanos = elapsed, "GPU pass completed");
        Ok(frame)
    }

    /// Counted GPU staging readback for CPU-frame sinks.
    pub fn readback(&self, frame: &WgpuTextureFrame) -> Result<VideoFrame, WgpuError> {
        let started = std::time::Instant::now();
        let span = tracing::info_span!(
            "gpu_readback",
            frame_id = frame.frame_id,
            width = frame.width,
            height = frame.height
        );
        let _entered = span.enter();
        self.ensure_device_available()?;
        let bytes_per_pixel = match frame.format {
            PixelFormat::Rgba8 => 4,
            PixelFormat::Rgba16Float => 8,
            other => {
                return Err(WgpuError::InvalidPlan(format!(
                    "unsupported readback format {other:?}"
                )));
            }
        };
        let unpadded_bytes_per_row = frame
            .width
            .checked_mul(bytes_per_pixel)
            .ok_or_else(|| WgpuError::InvalidPlan("readback row byte overflow".into()))?;
        let padded_bytes_per_row = unpadded_bytes_per_row
            .div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let readback_size = u64::from(padded_bytes_per_row) * u64::from(frame.height);
        let readback = ResourcePool::acquire(
            &self.resources,
            ResourceKey {
                kind: ResourceKind::Readback,
                format: pool_format(frame.format)?,
                width: frame.width,
                height: frame.height,
            },
        )?;
        debug_assert_eq!(readback.key.bytes(), readback_size);
        self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("eiviz-compositor-readback-encoder"),
            });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: frame.texture(),
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: readback.readback(),
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bytes_per_row),
                    rows_per_image: Some(frame.height),
                },
            },
            wgpu::Extent3d {
                width: frame.width,
                height: frame.height,
                depth_or_array_layers: 1,
            },
        );
        self.diagnostics.readbacks.fetch_add(1, Ordering::Relaxed);
        self.queue.submit(Some(encoder.finish()));
        let slice = readback.readback().slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        let _ = self.device.poll(wgpu::PollType::Wait);
        let map_result = receiver
            .recv()
            .map_err(|error| WgpuError::Map(error.to_string()))?
            .map_err(|error| WgpuError::Map(error.to_string()));
        let validation_error = pollster::block_on(self.device.pop_error_scope());
        map_result?;
        if let Some(error) = validation_error {
            return Err(WgpuError::Validation(error.to_string()));
        }
        let mapped = slice.get_mapped_range();
        let mut rgba = Vec::with_capacity(unpadded_bytes_per_row as usize * frame.height as usize);
        for row in mapped.chunks_exact(padded_bytes_per_row as usize) {
            rgba.extend_from_slice(&row[..unpadded_bytes_per_row as usize]);
        }
        drop(mapped);
        readback.readback().unmap();
        let elapsed = started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
        self.diagnostics
            .readback_nanos
            .store(elapsed, Ordering::Relaxed);
        self.diagnostics
            .readback_max_nanos
            .fetch_max(elapsed, Ordering::Relaxed);
        tracing::info!(
            frame_id = frame.frame_id,
            readback_nanos = elapsed,
            "GPU staging readback completed"
        );
        Ok(VideoFrame {
            id: frame.frame_id,
            source: None,
            pts: frame.pts,
            capture_domain: eiviz_time::ClockDomain::Virtual,
            clock_observation: None,
            width: frame.width,
            height: frame.height,
            format: frame.format,
            color: frame.color,
            field: frame.field,
            data: Arc::from(rgba),
            discontinuity: false,
        })
    }

    /// GPU transition. Both frames are uploaded and blended by the same WGSL
    /// pipeline; the CPU reference mixer is never called.
    pub fn mix(
        &self,
        a: &VideoFrame,
        b: &VideoFrame,
        factor: f32,
        pts: MediaTime,
        frame_id: u64,
    ) -> Result<VideoFrame, WgpuError> {
        let a_id = InputId::from_u128(1);
        let b_id = InputId::from_u128(2);
        let sources = HashMap::from([(a_id, a.clone()), (b_id, b.clone())]);
        let plan = RenderPlan {
            width: a.width,
            height: a.height,
            output_format: a.format,
            color: a.color,
            field_order: None,
            color_conversion: ColorConversionPolicy::Exact,
            vram_bytes: RenderPlan::estimate_vram_bytes(a.width, a.height, a.format, 2),
            layers: vec![
                Layer {
                    input: a_id,
                    transform: Transform2D::fullscreen(),
                    opacity: 1.0,
                },
                Layer {
                    input: b_id,
                    transform: Transform2D::fullscreen(),
                    opacity: factor.clamp(0.0, 1.0),
                },
            ],
        };
        self.composite(&plan, &sources, pts, frame_id)
    }

    fn prepare_layers(
        &self,
        plan: &RenderPlan,
        sources: &HashMap<InputId, VideoFrame>,
        frame_id: u64,
    ) -> Result<Vec<PreparedLayer>, WgpuError> {
        let mut prepared = Vec::with_capacity(plan.layers.len());
        for layer in &plan.layers {
            let source = sources
                .get(&layer.input)
                .ok_or(WgpuError::MissingSource(layer.input))?;
            let expected_field = plan.field_at(frame_id);
            if source.field != expected_field {
                return Err(WgpuError::UnsupportedProfile(format!(
                    "input {} field {:?} does not match render boundary {:?}; implicit scan conversion is forbidden",
                    layer.input, source.field, expected_field
                )));
            }
            validate_source(source, layer.input)?;
            let upload = prepare_upload(source, layer.input)?;
            if upload.texture_format == wgpu::TextureFormat::Rgba16Float
                && !self.capabilities.rgba16_float_filterable
            {
                return Err(WgpuError::UnsupportedProfile(
                    "P010/P216/RGBA16Float input requires filterable RGBA16Float sampling".into(),
                ));
            }
            let max_dimension = self.device.limits().max_texture_dimension_2d;
            if source.width > max_dimension || source.height > max_dimension {
                return Err(WgpuError::InvalidPlan(format!(
                    "source {} {}x{} exceeds GPU max dimension {max_dimension}",
                    layer.input, source.width, source.height
                )));
            }
            let resource = ResourcePool::acquire(
                &self.resources,
                ResourceKey {
                    kind: ResourceKind::Source,
                    format: pool_format_from_wgpu(upload.texture_format)?,
                    width: source.width,
                    height: source.height,
                },
            )?;
            let source_resource = resource.source();
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &source_resource.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &upload.data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(upload.bytes_per_row),
                    rows_per_image: Some(source.height),
                },
                wgpu::Extent3d {
                    width: source.width,
                    height: source.height,
                    depth_or_array_layers: 1,
                },
            );
            let uniform_data = layer_uniform_bytes(plan, layer, source, upload.yuv)?;
            self.queue
                .write_buffer(&source_resource.uniform, 0, &uniform_data);
            prepared.push(PreparedLayer { resource });
        }
        Ok(prepared)
    }
}

struct PreparedLayer {
    resource: ResourceLease,
}

fn pool_format(format: PixelFormat) -> Result<PoolFormat, WgpuError> {
    match format {
        PixelFormat::Rgba8 | PixelFormat::Nv12 => Ok(PoolFormat::Rgba8),
        PixelFormat::Bgra8 => Ok(PoolFormat::Bgra8),
        PixelFormat::Rgba16Float | PixelFormat::P010 | PixelFormat::P216 => {
            Ok(PoolFormat::Rgba16Float)
        }
    }
}

fn pool_format_from_wgpu(format: wgpu::TextureFormat) -> Result<PoolFormat, WgpuError> {
    match format {
        wgpu::TextureFormat::Rgba8Unorm => Ok(PoolFormat::Rgba8),
        wgpu::TextureFormat::Bgra8Unorm => Ok(PoolFormat::Bgra8),
        wgpu::TextureFormat::Rgba16Float => Ok(PoolFormat::Rgba16Float),
        other => Err(WgpuError::InvalidPlan(format!(
            "unsupported pooled texture format {other:?}"
        ))),
    }
}

fn validate_plan(plan: &RenderPlan) -> Result<(), WgpuError> {
    if plan.width == 0 || plan.height == 0 {
        return Err(WgpuError::InvalidPlan("zero-sized output".into()));
    }
    if plan.width > u32::MAX / 4 {
        return Err(WgpuError::InvalidPlan(
            "output row byte count overflows u32".into(),
        ));
    }
    let bytes = u64::from(plan.width)
        .checked_mul(u64::from(plan.height))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| WgpuError::InvalidPlan("output size overflow".into()))?;
    if bytes > usize::MAX as u64 {
        return Err(WgpuError::InvalidPlan(
            "output exceeds addressable memory".into(),
        ));
    }
    Ok(())
}

fn validate_source(source: &VideoFrame, input: InputId) -> Result<(), WgpuError> {
    if source.width == 0 || source.height == 0 {
        return Err(WgpuError::InvalidPlan(format!(
            "source {input} is zero-sized"
        )));
    }
    source
        .validate_layout()
        .map_err(|detail| WgpuError::InvalidPlan(format!("source {input}: {detail}")))?;
    Ok(())
}

struct PreparedUpload {
    texture_format: wgpu::TextureFormat,
    bytes_per_row: u32,
    data: Vec<u8>,
    yuv: bool,
}

fn prepare_upload(source: &VideoFrame, input: InputId) -> Result<PreparedUpload, WgpuError> {
    let pixels = source.width as usize * source.height as usize;
    match source.format {
        PixelFormat::Rgba8 => Ok(PreparedUpload {
            texture_format: wgpu::TextureFormat::Rgba8Unorm,
            bytes_per_row: source.width * 4,
            data: source.data.to_vec(),
            yuv: false,
        }),
        PixelFormat::Bgra8 => Ok(PreparedUpload {
            texture_format: wgpu::TextureFormat::Bgra8Unorm,
            bytes_per_row: source.width * 4,
            data: source.data.to_vec(),
            yuv: false,
        }),
        PixelFormat::Rgba16Float => Ok(PreparedUpload {
            texture_format: wgpu::TextureFormat::Rgba16Float,
            bytes_per_row: source.width * 8,
            data: source.data.to_vec(),
            yuv: false,
        }),
        PixelFormat::Nv12 => {
            let width = source.width as usize;
            let height = source.height as usize;
            let uv_offset = pixels;
            let mut rgba = Vec::with_capacity(pixels * 4);
            for y in 0..height {
                for x in 0..width {
                    let uv = uv_offset + (y / 2) * width + (x & !1);
                    rgba.extend_from_slice(&[
                        source.data[y * width + x],
                        source.data[uv],
                        source.data[uv + 1],
                        255,
                    ]);
                }
            }
            Ok(PreparedUpload {
                texture_format: wgpu::TextureFormat::Rgba8Unorm,
                bytes_per_row: source.width * 4,
                data: rgba,
                yuv: true,
            })
        }
        PixelFormat::P010 | PixelFormat::P216 => {
            let width = source.width as usize;
            let height = source.height as usize;
            let y_bytes = pixels * 2;
            let mut rgba = Vec::with_capacity(pixels * 8);
            for y in 0..height {
                for x in 0..width {
                    let y_code = ten_bit_word(&source.data, (y * width + x) * 2, input)?;
                    let chroma_word = match source.format {
                        PixelFormat::P010 => ((y / 2) * width + (x & !1)) * 2,
                        PixelFormat::P216 => (y * width + (x & !1)) * 2,
                        _ => unreachable!(),
                    };
                    let u_code = ten_bit_word(&source.data, y_bytes + chroma_word, input)?;
                    let v_code = ten_bit_word(&source.data, y_bytes + chroma_word + 2, input)?;
                    for value in [
                        y_code as f32 / 1023.0,
                        u_code as f32 / 1023.0,
                        v_code as f32 / 1023.0,
                        1.0,
                    ] {
                        rgba.extend_from_slice(&f32_to_f16_bits(value).to_le_bytes());
                    }
                }
            }
            Ok(PreparedUpload {
                texture_format: wgpu::TextureFormat::Rgba16Float,
                bytes_per_row: source.width * 8,
                data: rgba,
                yuv: true,
            })
        }
    }
}

fn ten_bit_word(data: &[u8], offset: usize, input: InputId) -> Result<u16, WgpuError> {
    let bytes = data.get(offset..offset + 2).ok_or_else(|| {
        WgpuError::InvalidPlan(format!("source {input} has truncated 10-bit data"))
    })?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]) >> 6)
}

fn f32_to_f16_bits(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xff) as i32 - 127 + 15;
    let mantissa = bits & 0x7f_ffff;
    if exponent <= 0 {
        if exponent < -10 {
            return sign;
        }
        let mantissa = (mantissa | 0x80_0000) >> (1 - exponent);
        return sign | ((mantissa + 0x1000) >> 13) as u16;
    }
    if exponent >= 31 {
        return sign | 0x7c00;
    }
    sign | ((exponent as u16) << 10) | ((mantissa + 0x1000) >> 13) as u16
}

fn layer_uniform_bytes(
    plan: &RenderPlan,
    layer: &Layer,
    source: &VideoFrame,
    yuv: bool,
) -> Result<Vec<u8>, WgpuError> {
    let transform = layer.transform;
    let (x, y, width, height) = if transform.pixel_space {
        (
            transform.x / plan.width as f32,
            transform.y / plan.height as f32,
            transform.width / plan.width as f32,
            transform.height / plan.height as f32,
        )
    } else {
        (transform.x, transform.y, transform.width, transform.height)
    };
    let crop = transform.crop;
    let crop_values = [
        crop.left.clamp(0.0, 1.0),
        crop.top.clamp(0.0, 1.0),
        crop.right.clamp(0.0, 1.0),
        crop.bottom.clamp(0.0, 1.0),
    ];
    if crop_values[0] + crop_values[2] >= 1.0 || crop_values[1] + crop_values[3] >= 1.0 {
        return Err(WgpuError::InvalidPlan(
            "crop removes the entire source".into(),
        ));
    }
    let values = [
        x,
        y,
        width,
        height,
        crop_values[0],
        crop_values[1],
        crop_values[2],
        crop_values[3],
        transform.rotation_deg.to_radians(),
        layer.opacity.clamp(0.0, 1.0),
        0.0,
        0.0,
    ];
    let mut bytes = Vec::with_capacity(80);
    for value in values {
        bytes.extend_from_slice(&value.to_ne_bytes());
    }
    let mismatch = source.color != plan.color;
    let tone_map = match (mismatch, plan.color_conversion) {
        (false, _) => None,
        (true, ColorConversionPolicy::Exact) => {
            return Err(WgpuError::ColorConversion {
                input: layer.input,
                detail: format!(
                    "source {:?} does not match render plan {:?} and policy is Exact",
                    source.color, plan.color
                ),
            });
        }
        (
            true,
            ColorConversionPolicy::Gpu {
                tone_map: ToneMapPolicy::Disabled,
            },
        ) if is_hdr(source.color.transfer) && !is_hdr(plan.color.transfer) => {
            return Err(WgpuError::ColorConversion {
                input: layer.input,
                detail: "HDR-to-SDR conversion requires an explicit tone-map policy".into(),
            });
        }
        (
            true,
            ColorConversionPolicy::Gpu {
                tone_map:
                    ToneMapPolicy::HdrToSdr {
                        source_peak_nits,
                        target_nits,
                    },
            },
        ) if is_hdr(source.color.transfer) && !is_hdr(plan.color.transfer) => {
            Some((f32::from(source_peak_nits), f32::from(target_nits)))
        }
        (
            true,
            ColorConversionPolicy::Gpu {
                tone_map: ToneMapPolicy::HdrToSdr { .. },
            },
        ) => {
            return Err(WgpuError::ColorConversion {
                input: layer.input,
                detail: "HDR-to-SDR tone map was selected for a conversion that is not HDR-to-SDR"
                    .into(),
            });
        }
        (true, ColorConversionPolicy::Gpu { .. }) => None,
    };
    let source_matrix = match source.color.matrix {
        ColorMatrix::Bt709 => 0,
        ColorMatrix::Bt2020NonConstantLuminance => 1,
        ColorMatrix::Bt601 => 2,
    };
    let target_2020 = plan.color.matrix == ColorMatrix::Bt2020NonConstantLuminance;
    let color_mode = source_matrix
        | (u32::from(mismatch) << 2)
        | (u32::from(target_2020) << 3)
        | (u32::from(source.format.bit_depth() > 8) << 4);
    let flags = [
        u32::from(yuv),
        u32::from(source.color.range == ColorRange::Limited),
        color_mode,
        u32::from(tone_map.is_some()),
    ];
    for flag in flags {
        bytes.extend_from_slice(&flag.to_ne_bytes());
    }
    let (source_peak, target) = tone_map.unwrap_or((0.0, 0.0));
    for value in [
        source_peak,
        target,
        transfer_code(source.color.transfer),
        transfer_code(plan.color.transfer),
    ] {
        bytes.extend_from_slice(&value.to_ne_bytes());
    }
    Ok(bytes)
}

const fn is_hdr(transfer: TransferFunction) -> bool {
    matches!(transfer, TransferFunction::Pq | TransferFunction::Hlg)
}

const fn transfer_code(transfer: TransferFunction) -> f32 {
    match transfer {
        TransferFunction::Srgb => 0.0,
        TransferFunction::Bt709 => 1.0,
        TransferFunction::Pq => 2.0,
        TransferFunction::Hlg => 3.0,
        TransferFunction::Linear => 4.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noop_compositor(limits: ResourcePoolLimits) -> WgpuCompositor {
        let (device, queue) = wgpu::Device::noop(&wgpu::DeviceDescriptor {
            label: Some("eiviz-injected-device-test"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        });
        WgpuCompositor::build_with_limits(
            None,
            wgpu::AdapterInfo {
                name: "injected-noop".into(),
                vendor: 0,
                device: 0,
                device_type: wgpu::DeviceType::Other,
                driver: "noop".into(),
                driver_info: String::new(),
                backend: wgpu::Backend::Noop,
            },
            AdapterCapabilities {
                max_texture_dimension_2d: 8192,
                max_buffer_size: 256 * 1024 * 1024,
                rgba16_float_renderable: true,
                rgba16_float_filterable: true,
                rgba32_float_sampleable: true,
            },
            device,
            queue,
            limits,
        )
        .unwrap()
    }

    fn empty_plan(width: u32, height: u32) -> RenderPlan {
        RenderPlan {
            width,
            height,
            output_format: PixelFormat::Rgba8,
            color: eiviz_core::ColorSpace::Bt709Sdr.metadata(),
            field_order: None,
            color_conversion: ColorConversionPolicy::Exact,
            vram_bytes: RenderPlan::estimate_vram_bytes(width, height, PixelFormat::Rgba8, 0),
            layers: Vec::new(),
        }
    }

    #[test]
    fn wgsl_parses_and_validates_without_an_adapter() {
        let module = naga::front::wgsl::parse_str(SHADER).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
    }

    #[test]
    fn hardware_adapter_or_explicit_error() {
        match WgpuCompositor::new_headless_hardware() {
            Ok(_) | Err(WgpuError::NoHardwareAdapter) | Err(WgpuError::Device(_)) => {}
            Err(other) => panic!("unexpected {other}"),
        }
    }

    #[test]
    fn injected_device_path_builds_without_requesting_another_device() {
        let compositor = noop_compositor(ResourcePoolLimits::default());
        assert_eq!(compositor.adapter_info().name, "injected-noop");
        assert_eq!(compositor.diagnostics().readbacks, 0);
        assert!(!compositor.diagnostics().automatic_recovery);
    }

    #[test]
    fn prewarmed_steady_state_reuses_every_resident_resource() {
        let compositor = noop_compositor(ResourcePoolLimits::default());
        let plan = empty_plan(16, 16);
        compositor
            .prewarm_snapshot(std::slice::from_ref(&plan), 16, 16, 0)
            .unwrap();
        let allocations = compositor.diagnostics().pool.allocations;
        for frame_id in 0..3 {
            compositor
                .composite(&plan, &HashMap::new(), MediaTime::ZERO, frame_id)
                .unwrap();
        }
        let diagnostics = compositor.diagnostics();
        assert_eq!(diagnostics.pool.allocations, allocations);
        assert_eq!(diagnostics.pool.acquisition_misses, 0);
        assert!(diagnostics.pool.reuses >= 6);
        assert_eq!(diagnostics.readbacks, 3);
    }

    #[test]
    fn bounded_pool_evicts_oldest_idle_key_deterministically() {
        let compositor = noop_compositor(ResourcePoolLimits {
            max_bytes: 1024 * 1024,
            max_resources: 3,
        });
        compositor
            .prewarm_snapshot(&[empty_plan(16, 16)], 16, 16, 0)
            .unwrap();
        let first = compositor.diagnostics().pool;
        assert_eq!(first.resident_resources, 3);
        assert_eq!(first.resident_bytes, 6_144);
        compositor
            .prewarm_snapshot(&[empty_plan(32, 32)], 32, 32, 0)
            .unwrap();
        let pool = compositor.diagnostics().pool;
        assert_eq!(pool.resident_resources, 3);
        assert_eq!(pool.evictions, 3);
        assert_eq!(pool.last_evicted.as_deref(), Some("Readback/Rgba8 16x16"));
    }

    #[test]
    fn prewarm_rejects_snapshot_over_pool_limit() {
        let compositor = noop_compositor(ResourcePoolLimits {
            max_bytes: 1,
            max_resources: 1,
        });
        assert!(matches!(
            compositor.prewarm_snapshot(&[empty_plan(16, 16)], 16, 16, 0),
            Err(WgpuError::PoolLimit { .. })
        ));
        assert_eq!(compositor.diagnostics().pool.allocations, 0);
    }

    #[test]
    fn frame_never_allocates_an_unprepared_resource() {
        let compositor = noop_compositor(ResourcePoolLimits::default());
        let error = compositor
            .composite_texture(&empty_plan(16, 16), &HashMap::new(), MediaTime::ZERO, 1)
            .unwrap_err();
        assert!(matches!(error, WgpuError::UnpreparedResource(_)));
        let pool = compositor.diagnostics().pool;
        assert_eq!(pool.allocations, 0);
        assert_eq!(pool.acquisition_misses, 1);
    }

    #[test]
    fn injected_loss_stops_gpu_operations_without_cpu_fallback() {
        let compositor = noop_compositor(ResourcePoolLimits::default());
        let plan = empty_plan(16, 16);
        compositor
            .prewarm_snapshot(std::slice::from_ref(&plan), 16, 16, 0)
            .unwrap();
        *compositor
            .diagnostics
            .device_loss
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(DeviceLossReport {
            reason: "Injected".into(),
            message: "mock device removed".into(),
        });
        assert!(matches!(
            compositor.composite_texture(&plan, &HashMap::new(), MediaTime::ZERO, 1),
            Err(WgpuError::DeviceLost(_))
        ));
        assert_eq!(
            compositor.diagnostics().device_loss,
            Some(DeviceLossReport {
                reason: "Injected".into(),
                message: "mock device removed".into(),
            })
        );
    }

    #[test]
    fn uniform_converts_pixel_space_and_crop() {
        let plan = RenderPlan {
            width: 1920,
            height: 1080,
            output_format: PixelFormat::Rgba8,
            color: eiviz_core::ColorSpace::Bt709Sdr.metadata(),
            field_order: None,
            color_conversion: ColorConversionPolicy::Exact,
            vram_bytes: RenderPlan::estimate_vram_bytes(1920, 1080, PixelFormat::Rgba8, 0),
            layers: vec![],
        };
        let layer = Layer {
            input: InputId::from_u128(1),
            transform: Transform2D {
                x: 192.0,
                y: 108.0,
                width: 960.0,
                height: 540.0,
                crop: eiviz_core::Crop {
                    left: 0.1,
                    top: 0.2,
                    right: 0.3,
                    bottom: 0.1,
                },
                pixel_space: true,
                ..Transform2D::default()
            },
            opacity: 0.5,
        };
        let source = VideoFrame::rgba_solid(0, MediaTime::ZERO, 4, 4, [0, 0, 0, 255]);
        let bytes = layer_uniform_bytes(&plan, &layer, &source, false).unwrap();
        let values = bytes
            .chunks_exact(4)
            .map(|bytes| f32::from_ne_bytes(bytes.try_into().unwrap()))
            .collect::<Vec<_>>();
        assert_eq!(&values[..4], &[0.1, 0.1, 0.5, 0.5]);
        assert_eq!(&values[4..8], &[0.1, 0.2, 0.3, 0.1]);
        assert_eq!(values[9], 0.5);
    }

    #[test]
    fn p010_unpack_preserves_ten_bit_codes_for_shader_conversion() {
        let words = [64_u16, 940, 64, 940, 512, 512];
        let data = words
            .into_iter()
            .flat_map(|word| (word << 6).to_le_bytes())
            .collect::<Vec<_>>();
        let frame = VideoFrame {
            id: 1,
            source: None,
            pts: MediaTime::ZERO,
            capture_domain: eiviz_time::ClockDomain::Virtual,
            clock_observation: None,
            width: 2,
            height: 2,
            format: PixelFormat::P010,
            color: eiviz_core::ColorSpace::Bt2020Pq.metadata(),
            field: FieldKind::Progressive,
            data: data.into(),
            discontinuity: false,
        };
        let upload = prepare_upload(&frame, InputId::from_u128(1)).unwrap();
        assert_eq!(upload.texture_format, wgpu::TextureFormat::Rgba16Float);
        assert_eq!(upload.data.len(), 2 * 2 * 8);
    }

    #[test]
    fn adapter_profile_rejection_is_explicit() {
        let capabilities = AdapterCapabilities {
            max_texture_dimension_2d: 4096,
            max_buffer_size: 256 * 1024 * 1024,
            rgba16_float_renderable: false,
            rgba16_float_filterable: false,
            rgba32_float_sampleable: false,
        };
        assert!(matches!(
            capabilities.admit(&VideoFormat::uhd_5994_hdr10_pq()),
            Err(WgpuError::UnsupportedProfile(_))
        ));
        capabilities.admit(&VideoFormat::uhd_5994_sdr()).unwrap();
    }

    #[test]
    fn hdr_to_sdr_requires_and_encodes_explicit_policy() {
        let layer = Layer {
            input: InputId::from_u128(7),
            transform: Transform2D::fullscreen(),
            opacity: 1.0,
        };
        let mut source = VideoFrame::rgba_solid(1, MediaTime::ZERO, 2, 2, [128, 128, 128, 255]);
        source.color = eiviz_core::ColorSpace::Bt2020Pq.metadata();
        let mut plan = RenderPlan {
            width: 2,
            height: 2,
            output_format: PixelFormat::Rgba8,
            color: eiviz_core::ColorSpace::Bt709Sdr.metadata(),
            field_order: None,
            color_conversion: ColorConversionPolicy::Exact,
            vram_bytes: RenderPlan::estimate_vram_bytes(2, 2, PixelFormat::Rgba8, 1),
            layers: vec![layer.clone()],
        };
        assert!(matches!(
            layer_uniform_bytes(&plan, &layer, &source, false),
            Err(WgpuError::ColorConversion { .. })
        ));
        plan.color_conversion = ColorConversionPolicy::Gpu {
            tone_map: ToneMapPolicy::Disabled,
        };
        assert!(
            layer_uniform_bytes(&plan, &layer, &source, false)
                .unwrap_err()
                .to_string()
                .contains("tone-map")
        );
        plan.color_conversion = ColorConversionPolicy::Gpu {
            tone_map: ToneMapPolicy::HdrToSdr {
                source_peak_nits: 1_000,
                target_nits: 100,
            },
        };
        let uniform = layer_uniform_bytes(&plan, &layer, &source, false).unwrap();
        let flags = uniform[48..64]
            .chunks_exact(4)
            .map(|bytes| u32::from_ne_bytes(bytes.try_into().unwrap()))
            .collect::<Vec<_>>();
        let policy = uniform[64..80]
            .chunks_exact(4)
            .map(|bytes| f32::from_ne_bytes(bytes.try_into().unwrap()))
            .collect::<Vec<_>>();
        assert_eq!(flags[3], 1);
        assert_eq!(policy, [1_000.0, 100.0, 2.0, 1.0]);
    }
}
