use std::collections::HashMap;
use std::panic::{self, AssertUnwindSafe};

use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

use crate::abi::NativeSurface;
use crate::device::{self, GpuDevice};

/// wgpu surface created (and configured) before the render thread sees it.
/// On macOS this must happen on the AppKit main thread.
pub(crate) struct PreparedSurface {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
}

#[cfg(any(windows, target_os = "macos"))]
#[derive(Clone)]
pub(crate) struct SurfaceGpu {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
}

pub struct Presenter {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    present: wgpu::RenderPipeline,
    uyvy: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    params: wgpu::Buffer,
    bind_key: u64,
    bind: Option<wgpu::BindGroup>,
    pending: Option<(u32, u32)>,
    ready: bool,
    occluded_streak: u32,
    native: NativeSurface,
}

/// Drops the state that references frame textures and views: the bind group
/// above all, which is what keeps a bygone generation's `TextureView` resource
/// reachable from a presenter. Called when a surface is detached and when a
/// present panicked, so a stale view never survives into the next attach or the
/// rebuild attempt; the presenter then rebinds (or is torn down) from scratch.
///
/// `ready` is left alone on purpose: forcing it off would have the next tick
/// `Surface::configure` a surface that may belong to a dead device generation, and a
/// wgpu configure against a foreign device raises a device error, where the panic we
/// are riding out came before any encoder work at all.
impl Presenter {
    fn teardown(&mut self) {
        self.bind = None;
        self.bind_key = 0;
        self.pending = None;
    }
}

/// What the present loop may do with one monitor this tick, given how it fared before.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MonitorAction {
    /// Draw as usual.
    Draw,
    /// One rebuild attempt: the presenter's frame references were cleared first, so
    /// the draw starts without a possibly stale view in its bind group.
    Retry,
    /// Give up: detach the monitor and stop presenting until the host attaches it
    /// again. Pushing on could only panic every frame.
    Detach,
}

/// Per-monitor recovery ledger around a present-time panic.
///
/// A present panic means a texture view the current wgpu hub cannot resolve — the
/// epoch assert in wgpu-core's storage ("Cannot get TextureView(..,..), found other
/// resource ..") fires when a render pass or bind group resolves a view that belongs
/// to another mixer generation. Reconfiguring the surface cannot rebuild a view whose
/// hub is gone, so the presenter gets exactly one cleared-out retry; a second failure
/// detaches it. That turns a monitor that panicked every frame into at most two
/// silent failures and a stop, until the host re-attaches it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MonitorRecovery {
    panics: u8,
}

impl MonitorRecovery {
    /// Two panics are needed to reach `Detach` — the second is the rebuild retry.
    const MAX: u8 = 2;

    fn new() -> Self {
        Self { panics: 0 }
    }

    fn action(&self) -> MonitorAction {
        match self.panics {
            0 => MonitorAction::Draw,
            1 => MonitorAction::Retry,
            _ => MonitorAction::Detach,
        }
    }

    fn note_panic(&mut self) {
        self.panics = self.panics.saturating_add(1).min(Self::MAX);
    }

    fn note_recovered(&mut self) {
        *self = Self::new();
    }
}

#[derive(Default)]
pub struct Presenters {
    by_key: HashMap<(u64, u32, NativeSurface), Presenter>,
    monitors: HashMap<u64, Presenter>,
    monitor_sources: HashMap<u64, u64>,
    monitor_intervals: HashMap<u64, u32>,
    monitor_recover: HashMap<u64, MonitorRecovery>,
}

impl Presenters {
    pub fn attach(
        &mut self,
        device: &GpuDevice,
        unit_id: u64,
        kind: u32,
        surface: NativeSurface,
        width: u32,
        height: u32,
        prepared: Option<PreparedSurface>,
    ) -> Result<(), String> {
        let presenter = match prepared {
            Some(prepared) => presenter_from_prepared(device, prepared, surface)?,
            None => create_presenter(device, surface, width, height)?,
        };
        self.by_key.insert((unit_id, kind, surface), presenter);
        Ok(())
    }

    pub fn resize(
        &mut self,
        device: &GpuDevice,
        unit_id: u64,
        kind: u32,
        surface: NativeSurface,
        width: u32,
        height: u32,
    ) {
        resize_presenter_for(
            device,
            self.by_key.get_mut(&(unit_id, kind, surface)),
            width,
            height,
        );
    }

    pub fn detach(&mut self, unit_id: u64, kind: u32, surface: NativeSurface) {
        if let Some(mut presenter) = self.by_key.remove(&(unit_id, kind, surface)) {
            presenter.teardown();
        }
    }

    pub fn detach_unit(&mut self, unit_id: u64) {
        let mut stale = self
            .by_key
            .extract_if(|(id, _, _), _| *id == unit_id)
            .map(|(_, presenter)| presenter)
            .collect::<Vec<_>>();
        for presenter in &mut stale {
            presenter.teardown();
        }
    }

    pub fn attach_monitor(
        &mut self,
        device: &GpuDevice,
        monitor_id: u64,
        source_id: u64,
        surface: NativeSurface,
        width: u32,
        height: u32,
        prepared: Option<PreparedSurface>,
    ) -> Result<(), String> {
        let presenter = match prepared {
            Some(prepared) => presenter_from_prepared(device, prepared, surface)?,
            None => create_presenter(device, surface, width, height)?,
        };
        if let Some(mut stale) = self.monitors.remove(&monitor_id) {
            stale.teardown();
        }
        // A fresh attach is a clean start even for a monitor that had panicked its
        // way to Detach.
        self.monitor_recover.remove(&monitor_id);
        self.monitors.insert(monitor_id, presenter);
        self.monitor_sources.insert(monitor_id, source_id);
        self.monitor_intervals.entry(monitor_id).or_insert(1);
        Ok(())
    }

    pub fn resize_monitor(&mut self, device: &GpuDevice, monitor_id: u64, width: u32, height: u32) {
        resize_presenter_for(device, self.monitors.get_mut(&monitor_id), width, height);
    }

    pub fn detach_monitor(&mut self, monitor_id: u64) {
        if let Some(mut presenter) = self.monitors.remove(&monitor_id) {
            presenter.teardown();
        }
        self.monitor_sources.remove(&monitor_id);
        self.monitor_intervals.remove(&monitor_id);
        self.monitor_recover.remove(&monitor_id);
    }

    pub fn set_monitor_source(&mut self, monitor_id: u64, source_id: u64) {
        if self.monitors.contains_key(&monitor_id) {
            self.monitor_sources.insert(monitor_id, source_id);
        }
    }

    pub fn set_monitor_interval(&mut self, monitor_id: u64, frames: u32) {
        self.monitor_intervals
            .insert(monitor_id, frames.clamp(1, 8));
    }

    fn interval_for(&self, monitor_id: u64) -> u32 {
        self.monitor_intervals
            .get(&monitor_id)
            .copied()
            .unwrap_or(1)
            .clamp(1, 8)
    }

    pub fn attached_monitor_sources(&self) -> Vec<u64> {
        self.monitor_sources.values().copied().collect()
    }

    pub fn attached_monitor_sources_due(&self, frame_i: u64) -> Vec<u64> {
        self.monitor_sources
            .iter()
            .filter(|(id, _)| frame_i % u64::from(self.interval_for(**id)) == 0)
            .map(|(_, source)| *source)
            .collect()
    }

    pub fn any_monitor_due(&self, frame_i: u64) -> bool {
        self.monitors
            .keys()
            .any(|id| frame_i % u64::from(self.interval_for(*id)) == 0)
    }

    pub fn present_unit_buses(
        &mut self,
        device: &GpuDevice,
        encoder: &mut wgpu::CommandEncoder,
        epoch: u64,
        view_for: impl Fn(u64, u32) -> Option<wgpu::TextureView>,
    ) -> Result<Vec<(wgpu::SurfaceTexture, wgpu::TextureView)>, String> {
        let planned: Vec<_> = self
            .by_key
            .keys()
            .copied()
            .map(|(unit_id, kind, surface)| {
                let cache_key = unit_id ^ (u64::from(kind) << 48) ^ epoch.rotate_left(8);
                (unit_id, kind, surface, cache_key, view_for(unit_id, kind))
            })
            .collect();
        present_pass(self, device, encoder, &planned)
    }

    pub fn present_monitors(
        &mut self,
        device: &GpuDevice,
        encoder: &mut wgpu::CommandEncoder,
        epoch: u64,
        frame_i: u64,
        source_view: impl Fn(u64) -> Option<(wgpu::TextureView, bool)>,
    ) -> Result<Vec<(wgpu::SurfaceTexture, wgpu::TextureView)>, String> {
        let due: Vec<u64> = self
            .monitors
            .keys()
            .copied()
            .filter(|id| frame_i % u64::from(self.interval_for(*id)) == 0)
            .collect();
        let (tiles, previews): (Vec<u64>, Vec<u64>) = due.into_iter().partition(|id| {
            self.monitors.get(id).is_some_and(|presenter| {
                !is_preview_surface(presenter.config.width, presenter.config.height)
            })
        });
        let mut acquired =
            present_monitor_group(self, device, encoder, epoch, &tiles, &source_view)?;
        for id in previews {
            acquired.extend(present_monitor_group(
                self,
                device,
                encoder,
                epoch,
                &[id],
                &source_view,
            )?);
        }
        Ok(acquired)
    }

    pub fn reconfigure_pending(&mut self, device: &GpuDevice) {
        // Apply on the render thread. Surface *creation* already ran on the
        // AppKit main thread. dispatch_sync back to main from here deadlocks
        // when the host (or cargo test) is on main waiting for this thread
        // — attach reply, mixer_destroy join, or a sleeping integration test.
        reconfigure_pending_inner(device, self);
    }
}

fn reconfigure_pending_inner(device: &GpuDevice, presenters: &mut Presenters) {
    for presenter in presenters.by_key.values_mut() {
        apply_pending_size(device, presenter);
    }
    for presenter in presenters.monitors.values_mut() {
        apply_pending_size(device, presenter);
    }
}

#[cfg(windows)]
fn win32_hinstance(hwnd: std::num::NonZeroIsize) -> Option<std::num::NonZeroIsize> {
    const GWLP_HINSTANCE: i32 = -6;
    unsafe extern "system" {
        fn GetWindowLongPtrW(hwnd: isize, index: i32) -> isize;
        fn GetModuleHandleW(name: *const u16) -> isize;
    }
    let from_window = unsafe { GetWindowLongPtrW(hwnd.get(), GWLP_HINSTANCE) };
    std::num::NonZeroIsize::new(from_window)
        .or_else(|| std::num::NonZeroIsize::new(unsafe { GetModuleHandleW(std::ptr::null()) }))
}

#[cfg(windows)]
fn win32_window_handle(hwnd: std::num::NonZeroIsize) -> raw_window_handle::Win32WindowHandle {
    let mut handle = raw_window_handle::Win32WindowHandle::new(hwnd);
    handle.hinstance = win32_hinstance(hwnd);
    handle
}

fn raw_handles(surface: NativeSurface) -> Result<(RawDisplayHandle, RawWindowHandle), String> {
    match surface.kind {
        #[cfg(windows)]
        crate::abi::NATIVE_WIN32_HWND => {
            use std::num::NonZeroIsize;

            use raw_window_handle::WindowsDisplayHandle;

            let hwnd = NonZeroIsize::new(surface.handle).ok_or("HWND cannot be null")?;
            Ok((
                RawDisplayHandle::Windows(WindowsDisplayHandle::new()),
                RawWindowHandle::Win32(win32_window_handle(hwnd)),
            ))
        }
        #[cfg(target_os = "macos")]
        crate::abi::NATIVE_APPKIT_NSVIEW => {
            use std::ptr::NonNull;

            use raw_window_handle::{AppKitDisplayHandle, AppKitWindowHandle};

            let ns_view = NonNull::new(surface.handle as *mut core::ffi::c_void)
                .ok_or("NSView cannot be null")?;
            Ok((
                RawDisplayHandle::AppKit(AppKitDisplayHandle::new()),
                RawWindowHandle::AppKit(AppKitWindowHandle::new(ns_view)),
            ))
        }
        _ => Err("native surface is not supported on this OS".into()),
    }
}

fn create_presenter(
    device: &GpuDevice,
    surface: NativeSurface,
    width: u32,
    height: u32,
) -> Result<Presenter, String> {
    let prepared = prepare_surface(
        &device.instance,
        &device.adapter,
        &device.device,
        surface,
        width,
        height,
    )?;
    presenter_from_prepared(device, prepared, surface)
}

pub(crate) fn prepare_surface(
    instance: &wgpu::Instance,
    adapter: &wgpu::Adapter,
    device: &wgpu::Device,
    native: NativeSurface,
    width: u32,
    height: u32,
) -> Result<PreparedSurface, String> {
    let (raw_display_handle, raw_window_handle) = raw_handles(native)?;
    // SAFETY: the host owns the HWND / NSView and detaches the surface before destroying it.
    let surface = unsafe {
        instance
            .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: Some(raw_display_handle),
                raw_window_handle,
            })
            .map_err(|error| format!("create_surface: {error}"))?
    };
    let mut config = surface
        .get_default_config(adapter, width.max(2), height.max(2))
        .ok_or("surface exposes no compatible configuration")?;
    let caps = surface.get_capabilities(adapter);
    config.format = pick_surface_format(&caps.formats);
    config.alpha_mode = pick_alpha_mode(&caps.alpha_modes);
    config.present_mode = pick_present_mode(&caps.present_modes);
    crate::diag::info(&format!("surface present_mode={:?}", config.present_mode));
    configure_surface(device, &surface, &config)?;
    Ok(PreparedSurface { surface, config })
}

fn presenter_from_prepared(
    device: &GpuDevice,
    prepared: PreparedSurface,
    native: NativeSurface,
) -> Result<Presenter, String> {
    let (layout, present, uyvy, sampler) = present_pipelines(device, prepared.config.format)?;
    let params = device.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("present params"),
        size: 256,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    Ok(Presenter {
        surface: prepared.surface,
        config: prepared.config,
        present,
        uyvy,
        layout,
        sampler,
        params,
        bind_key: 0,
        bind: None,
        pending: None,
        ready: true,
        occluded_streak: 0,
        native,
    })
}

fn resize_presenter_for(
    _device: &GpuDevice,
    presenter: Option<&mut Presenter>,
    width: u32,
    height: u32,
) {
    let Some(presenter) = presenter else {
        return;
    };
    presenter.pending = Some((width.max(2), height.max(2)));
}

fn is_preview_surface(width: u32, height: u32) -> bool {
    width >= 256 && height >= 256
}

fn present_monitor_group(
    presenters: &mut Presenters,
    device: &GpuDevice,
    encoder: &mut wgpu::CommandEncoder,
    epoch: u64,
    ids: &[u64],
    source_view: &impl Fn(u64) -> Option<(wgpu::TextureView, bool)>,
) -> Result<Vec<(wgpu::SurfaceTexture, wgpu::TextureView)>, String> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let planned: Vec<_> = ids
        .iter()
        .copied()
        .filter_map(|monitor_id| {
            let source_id = *presenters.monitor_sources.get(&monitor_id)?;
            let (view, packed) = match source_view(source_id) {
                Some((view, packed)) => (Some(view), packed),
                None => (None, false),
            };
            Some((monitor_id, source_id ^ epoch.rotate_left(8), view, packed))
        })
        .collect();
    present_monitor_pass(presenters, device, encoder, &planned)
}

fn present_pass(
    presenters: &mut Presenters,
    device: &GpuDevice,
    encoder: &mut wgpu::CommandEncoder,
    planned: &[(
        u64,
        u32,
        crate::abi::NativeSurface,
        u64,
        Option<wgpu::TextureView>,
    )],
) -> Result<Vec<(wgpu::SurfaceTexture, wgpu::TextureView)>, String> {
    let mut acquired = Vec::new();
    for (unit_id, kind, surface, cache_key, view) in planned {
        let Some(presenter) = presenters.by_key.get_mut(&(*unit_id, *kind, *surface)) else {
            continue;
        };
        if let Some(item) =
            draw_presenter(device, presenter, *cache_key, view.as_ref(), false, encoder)
        {
            acquired.push(item);
        }
    }
    Ok(acquired)
}

fn present_monitor_pass(
    presenters: &mut Presenters,
    device: &GpuDevice,
    encoder: &mut wgpu::CommandEncoder,
    planned: &[(u64, u64, Option<wgpu::TextureView>, bool)],
) -> Result<Vec<(wgpu::SurfaceTexture, wgpu::TextureView)>, String> {
    let mut acquired = Vec::new();
    for (monitor_id, cache_key, view, packed) in planned {
        let monitor_id = *monitor_id;
        // One panic here is a texture view the current wgpu hub cannot resolve, so
        // the recovery ledger decides whether this monitor may draw at all.
        let action = presenters
            .monitor_recover
            .get(&monitor_id)
            .map_or(MonitorAction::Draw, MonitorRecovery::action);
        match action {
            MonitorAction::Detach => {
                if let Some(mut presenter) = presenters.monitors.remove(&monitor_id) {
                    presenter.teardown();
                }
                presenters.monitor_sources.remove(&monitor_id);
                presenters.monitor_intervals.remove(&monitor_id);
                presenters.monitor_recover.remove(&monitor_id);
                crate::diag::error(&format!(
                    "monitor present {monitor_id:#x} detached: stale texture view survived one rebuild"
                ));
                // The host watches this counter and drops the last attached flip
                // budget surface, which makes it re-attach.
                crate::diag::note_surface_lost();
            }
            MonitorAction::Retry => {
                // Quiet on purpose: the panic notes below log what happened, and a
                // retry that keeps failing quietly (an acquire that gives up instead
                // of panicking) must not speak once per frame.
                if let Some(presenter) = presenters.monitors.get_mut(&monitor_id) {
                    presenter.teardown();
                }
            }
            MonitorAction::Draw => {}
        }
        // The Detach branch above may already have removed the presenter.
        let Some(presenter) = presenters.monitors.get_mut(&monitor_id) else {
            continue;
        };
        let drawn = panic::catch_unwind(AssertUnwindSafe(|| {
            draw_presenter(
                device,
                presenter,
                *cache_key,
                view.as_ref(),
                *packed,
                encoder,
            )
        }));
        match drawn {
            Ok(Some(item)) => {
                if let Some(recovery) = presenters.monitor_recover.get_mut(&monitor_id) {
                    recovery.note_recovered();
                }
                acquired.push(item);
            }
            Ok(None) => {}
            Err(_) => {
                if let Some(presenter) = presenters.monitors.get_mut(&monitor_id) {
                    // Any stale view the presenter resolved goes with this teardown,
                    // so the one rebuild attempt — or the host's next attach — starts
                    // without it.
                    presenter.teardown();
                }
                let panics = match presenters.monitor_recover.get_mut(&monitor_id) {
                    Some(recovery) => {
                        recovery.note_panic();
                        recovery.panics
                    }
                    None => {
                        presenters
                            .monitor_recover
                            .insert(monitor_id, MonitorRecovery { panics: 1 });
                        1
                    }
                };
                if panics == 1 {
                    crate::diag::error(&format!(
                        "monitor present {monitor_id:#x} panicked; stale view reference cleared, one rebuild follows"
                    ));
                } else {
                    crate::diag::error(&format!(
                        "monitor present {monitor_id:#x} panicked again; it detaches on the next present tick"
                    ));
                }
            }
        }
    }
    Ok(acquired)
}

fn live_extent(presenter: &Presenter) -> (u32, u32) {
    native_client_size(presenter.native)
        .or(presenter.pending)
        .unwrap_or((
            presenter.config.width.max(2),
            presenter.config.height.max(2),
        ))
}

fn apply_pending_size(device: &GpuDevice, presenter: &mut Presenter) {
    let (width, height) = live_extent(presenter);
    if presenter.ready && presenter.config.width == width && presenter.config.height == height {
        presenter.pending = None;
        return;
    }
    let mut config = presenter.config.clone();
    config.width = width;
    config.height = height;
    if configure_surface(&device.device, &presenter.surface, &config).is_ok() {
        presenter.config = config;
        presenter.bind = None;
        presenter.bind_key = 0;
        presenter.pending = None;
        presenter.ready = true;
    }
}

fn configure_surface(
    device: &wgpu::Device,
    surface: &wgpu::Surface,
    config: &wgpu::SurfaceConfiguration,
) -> Result<(), String> {
    // Do not wait for the whole mix queue to drain. GridSplitter drags resize the
    // HWND every layout tick; a 50ms+ idle wait guarantees the next acquire is
    // already Outdated. wgpu's configure waits on this swapchain's in-flight
    // presents. lock_gpu_queue still serializes against Queue::submit.
    for _ in 0..4 {
        {
            let _guard = device::lock_gpu_queue();
            let _ = device.poll(wgpu::PollType::Poll);
            let (_, failed) = device::with_surface_configure(|| {
                surface.configure(device, config);
            });
            if !failed {
                return Ok(());
            }
        }
        std::thread::yield_now();
    }
    Err("surface configure failed".into())
}

fn pick_present_mode(modes: &[wgpu::PresentMode]) -> wgpu::PresentMode {
    const PREFERRED: [wgpu::PresentMode; 3] = [
        wgpu::PresentMode::Mailbox,
        wgpu::PresentMode::Fifo,
        wgpu::PresentMode::FifoRelaxed,
    ];
    PREFERRED
        .into_iter()
        .find(|mode| modes.contains(mode))
        .unwrap_or(wgpu::PresentMode::Fifo)
}

fn pick_surface_format(formats: &[wgpu::TextureFormat]) -> wgpu::TextureFormat {
    const PREFERRED: [wgpu::TextureFormat; 2] = [
        wgpu::TextureFormat::Bgra8Unorm,
        wgpu::TextureFormat::Rgba8Unorm,
    ];
    PREFERRED
        .into_iter()
        .find(|format| formats.contains(format))
        .or_else(|| formats.first().copied())
        .unwrap_or(wgpu::TextureFormat::Bgra8Unorm)
}

fn pick_alpha_mode(modes: &[wgpu::CompositeAlphaMode]) -> wgpu::CompositeAlphaMode {
    const PREFERRED: [wgpu::CompositeAlphaMode; 2] = [
        wgpu::CompositeAlphaMode::Opaque,
        wgpu::CompositeAlphaMode::Auto,
    ];
    PREFERRED
        .into_iter()
        .find(|mode| modes.contains(mode))
        .unwrap_or(wgpu::CompositeAlphaMode::Auto)
}

fn mark_rebuild(presenter: &mut Presenter) {
    presenter.ready = false;
    presenter.pending = Some(native_client_size(presenter.native).unwrap_or((
        presenter.config.width.max(2),
        presenter.config.height.max(2),
    )));
}

#[cfg(windows)]
fn native_client_size(surface: NativeSurface) -> Option<(u32, u32)> {
    if surface.kind != crate::abi::NATIVE_WIN32_HWND {
        return None;
    }
    #[repr(C)]
    struct WinRect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }
    unsafe extern "system" {
        fn GetClientRect(hwnd: isize, rect: *mut WinRect) -> i32;
    }
    let mut rect = WinRect {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    if unsafe { GetClientRect(surface.handle, &mut rect) } == 0 {
        return None;
    }
    let width = rect.right.saturating_sub(rect.left).max(0) as u32;
    let height = rect.bottom.saturating_sub(rect.top).max(0) as u32;
    (width >= 8 && height >= 8).then_some((width, height))
}

#[cfg(not(windows))]
fn native_client_size(_surface: NativeSurface) -> Option<(u32, u32)> {
    None
}

fn sync_presenter_extent(device: &GpuDevice, presenter: &mut Presenter) {
    let extent = live_extent(presenter);
    if presenter.ready
        && presenter.pending.is_none()
        && presenter.config.width == extent.0
        && presenter.config.height == extent.1
    {
        return;
    }
    presenter.pending = Some(extent);
    apply_pending_size(device, presenter);
}

fn acquire_surface_texture(
    device: &GpuDevice,
    presenter: &mut Presenter,
) -> Option<wgpu::SurfaceTexture> {
    // While a GridSplitter is dragged the HWND extent changes every layout
    // tick, so Vulkan WSI returns Outdated. Rebuild from GetClientRect and
    // retry in this call; skipping the frame leaves Preview/Program frozen
    // for the whole drag.
    for _ in 0..8 {
        sync_presenter_extent(device, presenter);
        if !presenter.ready {
            return None;
        }
        // wgpu 30 panics in Surface::get_current_texture when the HAL surface was
        // never configured (error_sink is None). Device error scopes do not catch that.
        let acquired = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            presenter.surface.get_current_texture()
        })) {
            Ok(acquired) => acquired,
            Err(_) => {
                crate::diag::error("surface get_current_texture panicked");
                presenter.ready = false;
                presenter.pending = None;
                return None;
            }
        };
        match acquired {
            wgpu::CurrentSurfaceTexture::Success(texture) => {
                // HWND can still move between configure and acquire. Presenting
                // a texture whose extent no longer matches the window makes
                // Vulkan WSI scale the image (the "broken" look on resize).
                if native_client_size(presenter.native)
                    .is_some_and(|size| size != (presenter.config.width, presenter.config.height))
                {
                    drop(texture);
                    mark_rebuild(presenter);
                    continue;
                }
                presenter.occluded_streak = 0;
                return Some(texture);
            }
            wgpu::CurrentSurfaceTexture::Suboptimal(texture) => {
                // Must drop before configure: wgpu panics if a SurfaceTexture
                // for this surface is still alive. Suboptimal on Win32 Vulkan
                // is a resize; presenting it stretches the frame to the HWND.
                drop(texture);
                mark_rebuild(presenter);
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                crate::diag::note_surface_lost();
                mark_rebuild(presenter);
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Validation => {
                mark_rebuild(presenter);
            }
            wgpu::CurrentSurfaceTexture::Timeout => return None,
            wgpu::CurrentSurfaceTexture::Occluded => {
                mark_rebuild(presenter);
            }
        }
    }
    None
}

fn draw_presenter(
    device: &GpuDevice,
    presenter: &mut Presenter,
    cache_key: u64,
    src: Option<&wgpu::TextureView>,
    packed: bool,
    encoder: &mut wgpu::CommandEncoder,
) -> Option<(wgpu::SurfaceTexture, wgpu::TextureView)> {
    let texture = acquire_surface_texture(device, presenter)?;
    let dest = texture.texture.create_view(&Default::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("present"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &dest,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
            multiview_mask: None,
        });
        if let Some(src) = src {
            if presenter.bind_key != cache_key || presenter.bind.is_none() {
                let params = BlitParams {
                    dst: [0.0, 0.0, 1.0, 1.0],
                    src: [0.0, 0.0, 1.0, 1.0],
                    opacity: 1.0,
                    pad: [0.0; 3],
                };
                device
                    .queue
                    .write_buffer(&presenter.params, 0, bytemuck::bytes_of(&params));
                presenter.bind =
                    Some(device.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("present bg"),
                        layout: &presenter.layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::TextureView(src),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::Sampler(&presenter.sampler),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: presenter.params.as_entire_binding(),
                            },
                        ],
                    }));
                presenter.bind_key = cache_key;
            }
            pass.set_pipeline(if packed {
                &presenter.uyvy
            } else {
                &presenter.present
            });
            pass.set_bind_group(0, presenter.bind.as_ref().expect("present bind"), &[]);
            pass.draw(0..6, 0..1);
        }
    }
    Some((texture, dest))
}

/// Flips the swapchain images this frame drew into.
///
/// The command buffer holding the blits was already submitted by the caller, so this only presents.
/// Doing it in that one submission is what stops a frame from reaching the GPU three times and
/// waiting for queue room on the second and third.
pub(crate) fn present_acquired(
    device: &GpuDevice,
    acquired: Vec<(wgpu::SurfaceTexture, wgpu::TextureView)>,
) -> Result<(), String> {
    for (texture, dest) in acquired {
        drop(dest);
        device.queue.present(texture);
    }
    Ok(())
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BlitParams {
    dst: [f32; 4],
    src: [f32; 4],
    opacity: f32,
    pad: [f32; 3],
}

fn present_pipelines(
    device: &GpuDevice,
    format: wgpu::TextureFormat,
) -> Result<
    (
        wgpu::BindGroupLayout,
        wgpu::RenderPipeline,
        wgpu::RenderPipeline,
        wgpu::Sampler,
    ),
    String,
> {
    let layout = device
        .device
        .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("present"),
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
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
    let pipeline_layout = device
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("present"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
    let present = make_present_pipeline(
        device,
        &pipeline_layout,
        format,
        include_str!("../shaders/present.wgsl"),
        "present",
    );
    let uyvy = make_present_pipeline(
        device,
        &pipeline_layout,
        format,
        include_str!("../shaders/uyvy_to_rgba.wgsl"),
        "present uyvy",
    );
    let sampler = device.device.create_sampler(&wgpu::SamplerDescriptor {
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    Ok((layout, present, uyvy, sampler))
}

fn make_present_pipeline(
    device: &GpuDevice,
    pipeline_layout: &wgpu::PipelineLayout,
    format: wgpu::TextureFormat,
    source: &str,
    label: &str,
) -> wgpu::RenderPipeline {
    let shader = device
        .device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(label),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
    device
        .device
        .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        })
}

#[cfg(test)]
mod tests {
    use super::{BlitParams, MonitorAction, MonitorRecovery, pick_present_mode};

    #[test]
    fn present_params_match_uyvy_shader() {
        assert_eq!(std::mem::size_of::<BlitParams>(), 48);
        assert_eq!(std::mem::offset_of!(BlitParams, src), 16);
    }

    #[test]
    fn prefers_fifo_over_immediate() {
        let modes = [wgpu::PresentMode::Fifo, wgpu::PresentMode::Immediate];
        assert_eq!(pick_present_mode(&modes), wgpu::PresentMode::Fifo);
    }

    #[test]
    fn prefers_mailbox_when_available() {
        let modes = [
            wgpu::PresentMode::Mailbox,
            wgpu::PresentMode::Fifo,
            wgpu::PresentMode::Immediate,
        ];
        assert_eq!(pick_present_mode(&modes), wgpu::PresentMode::Mailbox);
    }

    #[cfg(windows)]
    #[test]
    fn win32_surface_handle_sets_hinstance() {
        let hwnd = std::num::NonZeroIsize::new(1).expect("hwnd");
        let handle = super::win32_window_handle(hwnd);
        assert!(
            handle.hinstance.is_some(),
            "Vulkan WSI requires Win32 hinstance"
        );
    }

    /// A monitor whose present panicked gets exactly one rebuild attempt and is then
    /// left detached: the sequence of actions must be Draw, Retry, Detach, with the
    /// Detach state never handed out twice because the presenter is removed on it.
    ///
    /// This is the headless model of the real-machine failure: SwiftUI opens and
    /// closes scene preview windows and replaces the session, the engine hands the
    /// present loop a texture view from a bygone wgpu hub, and without this ledger
    /// the "present monitors panicked" guard caught the same panic every frame for
    /// the rest of the session (62,878 lines in one log).
    #[test]
    fn monitor_recovery_panics_once_then_stops() {
        let mut recovery = MonitorRecovery::new();
        assert_eq!(
            recovery.action(),
            MonitorAction::Draw,
            "a fresh monitor draws"
        );

        recovery.note_panic();
        assert_eq!(
            recovery.action(),
            MonitorAction::Retry,
            "the first panic earns a single rebuild attempt"
        );

        // A tick where the draw does not panic restores the monitor.
        recovery.note_recovered();
        assert_eq!(recovery.action(), MonitorAction::Draw);

        recovery.note_panic();
        recovery.note_panic();
        assert_eq!(
            recovery.action(),
            MonitorAction::Detach,
            "the retry failed too, so the monitor detaches and stops"
        );

        // Beyond the cap the action stays Detach (defensive saturation).
        recovery.note_panic();
        assert_eq!(recovery.action(), MonitorAction::Detach);
    }

    /// A monitor that recovered mid-way (its rebuild attempt drew fine) is reset back
    /// to a fresh one, and re-attaching a monitor resets its ledger: attach, fail,
    /// detach, attach, draw — the operator's open, close and re-open of a preview
    /// window must work again without a manual session reload.
    #[test]
    fn monitor_recovery_resets_on_recovery_and_reattach() {
        let mut recovery = MonitorRecovery::new();
        recovery.note_panic();
        assert_eq!(recovery.action(), MonitorAction::Retry);
        recovery.note_recovered();
        assert_eq!(
            recovery.action(),
            MonitorAction::Draw,
            "recovery clears the ledger"
        );

        // One panic then a detached rebuild, then a fresh attach resets again.
        recovery.note_panic();
        recovery.note_panic();
        assert_eq!(recovery.action(), MonitorAction::Detach);
        recovery.note_recovered();
        assert_eq!(recovery.action(), MonitorAction::Draw);
    }
}
