use super::*;

pub(crate) fn render_loop(
    device: GpuDevice,
    fps_num: u32,
    fps_den: u32,
    shared: Arc<Mutex<Shared>>,
    uploads: Arc<Mutex<UploadStore>>,
    telemetry: Arc<Mutex<Telemetry>>,
    thumb_pixels: Arc<Mutex<HashMap<u64, crate::thumb::ThumbPixels>>>,
    cmds: mpsc::Receiver<GpuCmd>,
    stop: Arc<AtomicBool>,
) {
    let mut composer = match Composer::new(&device) {
        Ok(composer) => composer,
        Err(error) => {
            set_error(&telemetry, error);
            return;
        }
    };
    let mut presenters = Presenters::default();
    let mut thumbs = crate::thumb::ThumbStore::new(thumb_pixels);
    let mut readbacks = ReadbackStore::default();
    let mut gpu_sends = GpuSendStore::default();
    let mut frame_delay = FrameDelay::new(3);
    let mut next = Instant::now();
    let clock_start = Instant::now();
    let mut frame_i = 0u64;
    let mut video_due: HashMap<u64, Instant> = HashMap::new();
    let mut last_bus: HashMap<u64, (u64, u64, u32, u64)> = HashMap::new();
    let mut snapshot = Vec::new();
    let mut scene_specs = Vec::new();
    let mut scene_labels = HashMap::new();
    let mut generators = Vec::new();
    let mut outputs_snap = Vec::new();
    let mut cached_mem = (0u64, 0u64);
    let mut cached_adapter = 0u64;
    let mut mem_at = Instant::now()
        .checked_sub(Duration::from_secs(1))
        .unwrap_or_else(Instant::now);
    let mut pending_snapshots: Vec<(u64, u32, String, mpsc::Sender<i32>)> = Vec::new();
    while !stop.load(Ordering::Relaxed) && !crate::diag::is_fatal() {
        while let Ok(cmd) = cmds.try_recv() {
            match cmd {
                GpuCmd::Attach {
                    unit_id,
                    kind,
                    surface,
                    width,
                    height,
                    prepared,
                    reply,
                } => {
                    let code = match panic::catch_unwind(AssertUnwindSafe(|| {
                        presenters.attach(&device, unit_id, kind, surface, width, height, prepared)
                    })) {
                        Ok(Ok(())) => {
                            shared.lock().expect("shared").compose_dirty = true;
                            OK
                        }
                        Ok(Err(error)) => {
                            crate::diag::error(&format!("attach surface: {error}"));
                            set_error(&telemetry, error);
                            ERR_DEVICE
                        }
                        Err(_) => {
                            crate::diag::error("attach surface panicked");
                            set_error(&telemetry, "attach surface panicked");
                            ERR_DEVICE
                        }
                    };
                    let _ = reply.send(code);
                }
                GpuCmd::Resize {
                    unit_id,
                    kind,
                    surface,
                    width,
                    height,
                } => presenters.resize(&device, unit_id, kind, surface, width, height),
                GpuCmd::Detach {
                    unit_id,
                    kind,
                    surface,
                    reply,
                } => {
                    presenters.detach(unit_id, kind, surface);
                    let _ = reply.send(OK);
                }
                GpuCmd::DetachUnit { unit_id, reply } => {
                    presenters.detach_unit(unit_id);
                    let _ = reply.send(OK);
                }
                GpuCmd::AttachMonitor {
                    monitor_id,
                    source_id,
                    surface,
                    width,
                    height,
                    prepared,
                    reply,
                } => {
                    let code = match panic::catch_unwind(AssertUnwindSafe(|| {
                        presenters.attach_monitor(
                            &device, monitor_id, source_id, surface, width, height, prepared,
                        )
                    })) {
                        Ok(Ok(())) => OK,
                        Ok(Err(error)) => {
                            crate::diag::error(&format!("attach monitor: {error}"));
                            set_error(&telemetry, error);
                            ERR_DEVICE
                        }
                        Err(_) => {
                            crate::diag::error("attach monitor panicked");
                            set_error(&telemetry, "attach monitor panicked");
                            ERR_DEVICE
                        }
                    };
                    let _ = reply.send(code);
                }
                GpuCmd::ResizeMonitor {
                    monitor_id,
                    width,
                    height,
                } => presenters.resize_monitor(&device, monitor_id, width, height),
                GpuCmd::DetachMonitor { monitor_id, reply } => {
                    presenters.detach_monitor(monitor_id);
                    let _ = reply.send(OK);
                }
                GpuCmd::SetMonitorSource {
                    monitor_id,
                    source_id,
                } => presenters.set_monitor_source(monitor_id, source_id),
                GpuCmd::SetMonitorInterval { monitor_id, frames } => {
                    presenters.set_monitor_interval(monitor_id, frames)
                }
                GpuCmd::Snapshot {
                    unit_id,
                    kind,
                    path,
                    reply,
                } => pending_snapshots.push((unit_id, kind, path, reply)),
                GpuCmd::Shutdown => {
                    for (_, _, _, reply) in pending_snapshots.drain(..) {
                        let _ = reply.send(ERR_DEVICE);
                    }
                    drop(presenters);
                    let _ = device.device.poll(wgpu::PollType::Wait {
                        submission_index: None,
                        timeout: Some(Duration::from_millis(200)),
                    });
                    return;
                }
            }
        }
        if crate::diag::take_gpu_fault() {
            crate::diag::mark_fatal("GPU device fault");
            set_error(&telemetry, "GPU device fault");
            break;
        }
        let frame = panic::catch_unwind(AssertUnwindSafe(|| {
            presenters.reconfigure_pending(&device);
        }));
        if frame.is_err() {
            crate::diag::error("presenter reconfigure panicked");
            set_error(&telemetry, "presenter reconfigure panicked");
            crate::diag::mark_fatal("presenter reconfigure panicked");
            break;
        }
        let (buffer_frames, use_rebar, direct_sample, fps_num, fps_den) = {
            let guard = shared.lock().expect("shared");
            let use_rebar = guard.rebar.available && guard.rebar_optimization;
            let direct_sample = use_rebar && cfg!(target_os = "macos");
            (
                { guard.frame_buffer_frames.clamp(1, 8) },
                use_rebar,
                direct_sample,
                if guard.master_fps_num > 0 {
                    guard.master_fps_num
                } else {
                    fps_num
                },
                if guard.master_fps_den > 0 {
                    guard.master_fps_den
                } else {
                    fps_den
                },
            )
        };
        let frame_dt = frame_period(fps_num, fps_den);
        frame_delay.set_depth(buffer_frames);
        shared
            .lock()
            .expect("shared")
            .audio
            .set_video_delay(buffer_frames, fps_num, fps_den);
        let now = Instant::now();
        if next + frame_dt.saturating_mul(buffer_frames) < now {
            next = now;
        }
        if next > Instant::now() {
            thread::sleep(next.saturating_duration_since(Instant::now()));
        }
        {
            let mut guard = shared.lock().expect("shared");
            for unit in guard.units.values_mut() {
                tick_unit_transitions(unit);
            }
            snapshot.clear();
            snapshot.extend(guard.units.iter().map(|(id, unit)| {
                let mix_preview = snapshot_mix_preview(unit);
                let mut state = unit.state;
                state.incoming_source = mix_preview;
                (
                    *id,
                    unit.width,
                    unit.height,
                    unit.fps_num,
                    unit.fps_den,
                    state,
                    mix_preview,
                    unit.custom_wgsl.clone(),
                )
            }));
            scene_specs.clear();
            scene_specs.extend(guard.scenes.iter().map(|(id, spec)| {
                (
                    *id,
                    spec.width,
                    spec.height,
                    Arc::clone(&spec.layers),
                    spec.mv_label,
                )
            }));
            scene_labels.clear();
            scene_labels.extend(
                guard
                    .scenes
                    .iter()
                    .map(|(id, spec)| (*id, Arc::clone(&spec.labels))),
            );
            let bus_colors = guard.bus_colors;
            generators.clear();
            generators.extend(guard.generators.iter().map(|(id, spec)| (*id, *spec)));
            outputs_snap.clear();
            outputs_snap.extend(guard.outputs.iter().map(|(id, output)| {
                let unit = guard.units.get(&output.unit_id);
                let (fps_n, fps_d) = if output.fps_num > 0 && output.fps_den > 0 {
                    (output.fps_num, output.fps_den)
                } else {
                    (
                        unit.map(|item| item.fps_num).unwrap_or(fps_num),
                        unit.map(|item| item.fps_den).unwrap_or(fps_den),
                    )
                };
                OutputSnap {
                    output_id: *id,
                    source_kind: output.source_kind,
                    source_id: output.source_id,
                    unit_id: output.unit_id,
                    audio_bus_id: output.audio_bus_id,
                    width: output.width,
                    height: output.height,
                    fps_n,
                    fps_d,
                    video_sub: Arc::clone(&output.video_sub),
                    use_gpu: output.use_gpu,
                    skip_idle_encode: output.skip_idle_encode,
                    tx: output.tx.clone(),
                    audio_send: output.audio_send.clone(),
                }
            }));
            let compose_dirty = guard.compose_dirty;
            guard.compose_dirty = false;
            let thumbs_snap = guard.thumbs.clone();
            let mix_inputs = guard.mix_inputs.clone();
            let audio_routes: Vec<audio::AudioOutputRoute> = outputs_snap
                .iter()
                .filter(|output| {
                    output.audio_bus_id != 0 && output.source_kind != SRC_KIND_MU_MULTIVIEW
                })
                .map(|output| {
                    let tx = output.tx.clone();
                    let audio_send = output.audio_send.clone();
                    audio::AudioOutputRoute {
                        audio_bus_id: output.audio_bus_id,
                        source_kind: output.source_kind,
                        send: Arc::new(move |packet| {
                            if let Some(send) = &audio_send {
                                send(packet);
                            } else {
                                let _ = tx.send(SendCmd::Audio { packet });
                            }
                        }),
                    }
                })
                .collect();
            *guard.audio_snap.lock().expect("audio snap") = audio::AudioMixSnapshot {
                units: snapshot.clone(),
                scenes: scene_specs.clone(),
                mix_inputs: mix_inputs.clone(),
                fps_num,
                fps_den,
                generators: generators
                    .iter()
                    .map(|(id, spec)| (*id, (spec.tone_hz, spec.tone_level_dbfs)))
                    .collect(),
                outputs: audio_routes,
                buffer_frames: guard.frame_buffer_frames,
            };
            drop(guard);
            let changed_units: Vec<u64> = snapshot
                .iter()
                .filter(|(id, _, _, _, _, state, mix_preview, _)| {
                    last_bus
                        .get(id)
                        .is_none_or(|(program, preview, mix, incoming)| {
                            *program != state.program_source
                                || *preview != state.preview_source
                                || *mix != state.mix.to_bits()
                                || *incoming != *mix_preview
                        })
                })
                .map(|(id, ..)| *id)
                .collect();
            if !changed_units.is_empty() {
                frame_delay.discard(changed_units);
            }
            let tallies: Vec<(u64, u64)> = snapshot
                .iter()
                .map(|item| (item.5.preview_source, item.5.program_source))
                .collect();
            frame_i = frame_i.wrapping_add(1);
            // Three lanes:
            // 1. On-air playout/upload — every master frame (FIFOs and live pixels).
            // 2. On-air compose (Preview/Program/outputs) — every master frame.
            // 3. Monitor compose and GUI-only upload — present_interval only.
            // Save roles still see every attached monitor/thumb so OMT quality
            // does not flap on skipped present ticks.
            let due_monitors = presenters.attached_monitor_sources_due(frame_i);
            let due_thumbs: Vec<u64> = thumbs_snap
                .iter()
                .filter(|(_, sub)| frame_i % u64::from(sub.interval) == 0)
                .map(|(id, _)| *id)
                .collect();
            let mut compose_sources = due_monitors;
            compose_sources.extend_from_slice(&due_thumbs);
            for (id, ..) in &pending_snapshots {
                if snapshot.iter().any(|(unit_id, ..)| *unit_id == *id) {
                    continue;
                }
                compose_sources.push(*id);
            }
            let (mut used_scenes, used_uploads) = collect_frame_live_ids(
                &scene_specs,
                &snapshot,
                &compose_sources,
                &outputs_snap,
                &mix_inputs,
                compose_dirty,
            );
            let mut role_sources = presenters.attached_monitor_sources();
            role_sources.extend(thumbs_snap.keys().copied());
            let output_refs: Vec<(u32, u64)> = outputs_snap
                .iter()
                .map(|item| (item.source_kind, item.source_id))
                .collect();
            let roles = collect_source_roles(&scene_specs, &snapshot, &role_sources, &output_refs);
            {
                let guard = shared.lock().expect("shared");
                for (id, receiver) in &guard.receivers {
                    let save = guard.live_save.get(id).copied().unwrap_or_default();
                    let role = roles.get(id).copied().unwrap_or_default();
                    receiver.apply_save(want_full(save, role), role.on_program, role.on_preview);
                }
            }
            let frame_begin = Instant::now();
            let pts = (clock_start.elapsed().as_nanos() / 100) as i64;
            let secs = frame_i as f64 * f64::from(fps_den) / f64::from(fps_num.max(1));
            let phase = ((secs * 0.12) % 1.0) as f32;
            let phase_y = ((secs * 0.07) % 1.0) as f32;
            let composed = panic::catch_unwind(AssertUnwindSafe(|| {
                composer.begin_frame();
                composer.ensure_builtins(&device);
                composer.sync_generators(&generators, phase, phase_y);
                let need_bake = composer.generators_need_rebake();
                if need_bake {
                    for (id, ..) in &scene_specs {
                        used_scenes.insert(*id);
                    }
                }
                let snaps = {
                    let mut upload_guard = uploads.lock().expect("uploads");
                    upload_guard.advance_playout(&used_uploads);
                    upload_guard.snapshot(&used_uploads)
                };
                composer.upload_sources(&device, &snaps, use_rebar, direct_sample);
                need_bake
            }));
            let need_gen_bake = match composed {
                Ok(need_bake) => need_bake,
                Err(_) => {
                    crate::diag::error("compose panicked");
                    set_error(&telemetry, "compose panicked");
                    crate::diag::mark_fatal("compose panicked");
                    false
                }
            };
            if need_gen_bake {
                frame_delay.discard(snapshot.iter().map(|(id, ..)| *id));
            }
            let need_prv = outputs_snap
                .iter()
                .any(|item| item.source_kind == SRC_KIND_MU_PREVIEW && item.cpu_video());
            let present_epoch = composer.gpu_epoch() ^ frame_delay.epoch().rotate_left(8);
            match panic::catch_unwind(AssertUnwindSafe(|| {
                presenters.present_unit_buses(&device, present_epoch, |unit_id, kind| {
                    frame_delay
                        .view(unit_id, kind)
                        .or_else(|| composer.unit_view(unit_id, kind))
                })
            })) {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    set_error(&telemetry, error.clone());
                    if error.contains("unconfigured") {
                        crate::diag::mark_fatal(error);
                        break;
                    }
                }
                Err(_) => {
                    crate::diag::error("present unit buses panicked");
                    set_error(&telemetry, "present panicked");
                    crate::diag::mark_fatal("present unit buses panicked");
                    break;
                }
            }
            {
                let mut encoder =
                    device
                        .device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("eiviz delay out"),
                        });
                let mut packed_copies: Vec<(u64, u32, u32)> = Vec::new();
                let mut gpu_copies: Vec<GpuEncodeCopy> = Vec::new();
                let send_now = Instant::now();
                for output in &outputs_snap {
                    if output.source_kind != SRC_KIND_MU_PROGRAM
                        && output.source_kind != SRC_KIND_MU_PREVIEW
                    {
                        continue;
                    }
                    if !output.wants_video() || !output_due(output, send_now, &mut video_due) {
                        continue;
                    }
                    let kind = if output.source_kind == SRC_KIND_MU_PREVIEW {
                        OUTPUT_PREVIEW
                    } else {
                        OUTPUT_PROGRAM
                    };
                    // Mix/T-bar ticks discard the delay ring so present can
                    // show the live compose. Program send must do the same
                    // or NDI/OMT freeze until mix is stable again.
                    let src = frame_delay
                        .rgba(output.unit_id, kind)
                        .or_else(|| composer.rgba_texture(output.unit_id, kind))
                        .cloned();
                    let Some(src) = src else {
                        continue;
                    };
                    if output.cpu_video() {
                        push_cpu_packed(
                            &mut composer,
                            &device,
                            &mut encoder,
                            &mut readbacks,
                            &mut packed_copies,
                            output,
                            &src,
                        );
                    } else if output.gpu_video() {
                        push_scaled_gpu(
                            &mut composer,
                            &mut gpu_sends,
                            &device,
                            &mut encoder,
                            &mut gpu_copies,
                            output,
                            &src,
                            false,
                        );
                    }
                }
                device.submit(Some(encoder.finish()));
                emit_packed(&mut readbacks, &device, &packed_copies, &outputs_snap, pts);
                emit_gpu_encode(&gpu_copies, pts);
            }
            frame_delay.consume_display(false);
            composer.set_bus_colors(bus_colors.preview, bus_colors.program, bus_colors.inactive);
            composer.sync_scenes(&device, &scene_specs, &scene_labels);
            let mut encoder =
                device
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("eiviz compose"),
                    });
            {
                let mix_owned = resolve_mix_sources(&mix_inputs, &composer);
                let mix_sources: HashMap<u64, &wgpu::Texture> =
                    mix_owned.iter().map(|(id, tex)| (*id, tex)).collect();
                composer.stage_mix_inputs(
                    &device,
                    &mut encoder,
                    mix_inputs.keys().copied(),
                    &mix_sources,
                );
            }
            if let Err(error) =
                composer.render_scenes(&device, &used_scenes, &mut encoder, &tallies)
            {
                set_error(&telemetry, error);
            }
            let mut packed_copies: Vec<(u64, u32, u32)> = Vec::new();
            let mut gpu_copies: Vec<GpuEncodeCopy> = Vec::new();
            let send_now = Instant::now();
            for (unit_id, width, height, _, _, state, mix_preview, custom) in &snapshot {
                composer.ensure_unit(&device, *unit_id, *width, *height);
                if let Err(error) =
                    composer.set_custom_mix(&device, *unit_id, custom.as_deref().unwrap_or(""))
                {
                    crate::diag::error(&format!("custom wgsl: {error}"));
                }
                let pack_pgm = outputs_snap.iter().any(|item| {
                    item.unit_id == *unit_id
                        && item.source_kind == SRC_KIND_MU_PROGRAM
                        && item.cpu_video()
                });
                if let Err(error) = composer.render_unit(
                    &device,
                    *unit_id,
                    state,
                    *mix_preview,
                    &mut encoder,
                    pack_pgm,
                ) {
                    set_error(&telemetry, error);
                }
                composer.pack_aux(&device, &mut encoder, *unit_id, need_prv);
            }
            for output in &outputs_snap {
                if output.source_kind == SRC_KIND_MU_PROGRAM
                    || output.source_kind == SRC_KIND_MU_PREVIEW
                {
                    continue;
                }
                if !output.wants_video() || !output_due(output, send_now, &mut video_due) {
                    continue;
                }
                let packed_src = output.source_kind == SRC_KIND_INPUT
                    && composer.source_is_packed(output.source_id);
                let src = match output.source_kind {
                    SRC_KIND_INPUT => composer
                        .mix_texture(output.source_id)
                        .or_else(|| composer.source_texture(output.source_id))
                        .cloned(),
                    SRC_KIND_SCENE | SRC_KIND_MU_MULTIVIEW => {
                        composer.scene_texture(output.source_id).cloned()
                    }
                    _ => None,
                };
                let Some(src) = src else {
                    continue;
                };
                if output.cpu_video() {
                    push_cpu_packed(
                        &mut composer,
                        &device,
                        &mut encoder,
                        &mut readbacks,
                        &mut packed_copies,
                        output,
                        &src,
                    );
                } else if output.gpu_video() {
                    push_scaled_gpu(
                        &mut composer,
                        &mut gpu_sends,
                        &device,
                        &mut encoder,
                        &mut gpu_copies,
                        output,
                        &src,
                        packed_src,
                    );
                }
            }
            frame_delay.capture(
                &device,
                &mut encoder,
                &composer,
                snapshot.iter().map(|(id, ..)| *id),
            );
            thumbs.capture(&device, &mut composer, &mut encoder, frame_i, &thumbs_snap);
            device.submit(Some(encoder.finish()));
            flush_snapshots(
                &device,
                &mut composer,
                &frame_delay,
                &telemetry,
                &mut pending_snapshots,
            );
            thumbs.advance(&device);
            emit_packed(&mut readbacks, &device, &packed_copies, &outputs_snap, pts);
            emit_gpu_encode(&gpu_copies, pts);
            for (id, _, _, _, _, state, mix_preview, ..) in &snapshot {
                last_bus.insert(
                    *id,
                    (
                        state.program_source,
                        state.preview_source,
                        state.mix.to_bits(),
                        *mix_preview,
                    ),
                );
            }
            if presenters.any_monitor_due(frame_i) {
                let monitor_epoch = composer.gpu_epoch() ^ frame_delay.epoch().rotate_left(8);
                if panic::catch_unwind(AssertUnwindSafe(|| {
                    if let Err(error) =
                        presenters.present_monitors(&device, monitor_epoch, frame_i, |source_id| {
                            frame_delay
                                .view_for_source(source_id)
                                .or_else(|| composer.view_for_source(source_id))
                                .map(|view| (view, composer.source_is_packed(source_id)))
                        })
                    {
                        set_error(&telemetry, error);
                    }
                }))
                .is_err()
                {
                    crate::diag::error("present monitors panicked");
                    set_error(&telemetry, "present monitors panicked");
                }
            }
            let compose_vram = composer.vram_bytes();
            let delay_vram = frame_delay.vram_bytes();
            let send_vram = gpu_sends.vram_bytes();
            if mem_at.elapsed() >= Duration::from_millis(500) {
                cached_mem = uploads.lock().expect("uploads").memory_bytes();
                cached_adapter = crate::rebar::adapter_usage_bytes(&device.device);
                mem_at = Instant::now();
            }
            let (ram, source_vram) = cached_mem;
            let accounted = source_vram
                .saturating_add(compose_vram)
                .saturating_add(delay_vram)
                .saturating_add(send_vram);
            {
                let mut guard = telemetry.lock().expect("telemetry");
                guard.last_render_ms = frame_begin.elapsed().as_secs_f32() * 1000.0;
                guard.last_ram_bytes = ram;
                guard.last_compose_vram = compose_vram;
                guard.last_delay_vram = delay_vram;
                guard.last_vram_bytes = cached_adapter.max(accounted);
                guard.scene_usage = composer.scene_usages();
                if crate::diag::profile_send() && frame_i % 60 == 0 {
                    let readback = crate::diag::take_readback_avg_ms().unwrap_or(0.0);
                    crate::diag::info(&format!(
                        "profile render={:.2}ms budget={:.2}ms readback={readback:.2}ms outputs={}",
                        guard.last_render_ms,
                        1000.0 * fps_den as f32 / fps_num.max(1) as f32,
                        outputs_snap.len()
                    ));
                }
            }
        }
        next += frame_dt;
    }
}

pub(crate) fn snapshot_texture<'a>(
    composer: &'a Composer,
    frame_delay: &'a FrameDelay,
    source_id: u64,
    kind: u32,
) -> Option<&'a wgpu::Texture> {
    if crate::abi::is_scene(source_id) {
        return composer.scene_texture(source_id);
    }
    if kind == OUTPUT_SOURCE {
        return composer.mix_texture(source_id).or_else(|| {
            composer
                .source_can_copy(source_id)
                .then(|| composer.source_texture(source_id))
                .flatten()
        });
    }
    composer
        .rgba_texture(source_id, kind)
        .or_else(|| frame_delay.rgba(source_id, kind))
}

pub(crate) fn flush_snapshots(
    device: &GpuDevice,
    composer: &mut Composer,
    frame_delay: &FrameDelay,
    telemetry: &Mutex<Telemetry>,
    pending: &mut Vec<(u64, u32, String, mpsc::Sender<i32>)>,
) {
    for (source_id, kind, path, reply) in pending.drain(..) {
        let blitted = (kind == OUTPUT_SOURCE
            && snapshot_texture(composer, frame_delay, source_id, kind).is_none())
        .then(|| composer.blit_source_rgba(device, source_id));
        let code = match snapshot_texture(composer, frame_delay, source_id, kind)
            .or(blitted.as_ref().and_then(|item| item.as_ref()))
        {
            Some(tex) => match crate::snapshot::save_texture(device, tex, &path) {
                Ok(()) => OK,
                Err(error) => {
                    set_error(telemetry, error);
                    ERR_IO
                }
            },
            None => {
                set_error(telemetry, "snapshot source not ready");
                ERR_IO
            }
        };
        let _ = reply.send(code);
    }
}

pub(crate) fn emit_packed(
    readbacks: &mut ReadbackStore,
    device: &GpuDevice,
    packed_copies: &[(u64, u32, u32)],
    outputs_snap: &[OutputSnap],
    pts: i64,
) {
    for (key, width, height) in packed_copies {
        if let Some(rb) = readbacks.get_mut(*key) {
            if crate::diag::profile_send() {
                let started = Instant::now();
                rb.advance(device);
                crate::diag::add_readback(started.elapsed());
            } else {
                rb.advance(device);
            }
            if let Some(packed) = rb.latest() {
                let data: Arc<[u8]> = packed.to_vec().into();
                last_frames().lock().expect("frames").insert(
                    *key,
                    Acquired {
                        data: Arc::clone(&data),
                        stride: width * 2,
                        pts,
                    },
                );
                for output in outputs_snap {
                    if !output.cpu_video() || output.output_id != *key {
                        continue;
                    }
                    let _ = output.tx.send(SendCmd::Video {
                        width: *width,
                        height: *height,
                        stride: width * 2,
                        pts,
                        data: Arc::clone(&data),
                        fps_n: output.fps_n,
                        fps_d: output.fps_d,
                    });
                }
            }
        }
    }
}

pub(crate) fn push_gpu_encode(
    gpu_sends: &mut GpuSendStore,
    device: &GpuDevice,
    encoder: &mut wgpu::CommandEncoder,
    copies: &mut Vec<GpuEncodeCopy>,
    output: &OutputSnap,
    src: &wgpu::Texture,
) {
    let Some((texture, width, height, busy)) =
        gpu_sends.copy(device, encoder, output.output_id, src)
    else {
        return;
    };
    copies.push(GpuEncodeCopy {
        tx: output.tx.clone(),
        texture,
        width,
        height,
        busy,
        fps_n: output.fps_n,
        fps_d: output.fps_d,
    });
}

fn frame_period(fps_n: u32, fps_d: u32) -> Duration {
    let n = u64::from(fps_n.max(1));
    let d = u64::from(fps_d.max(1));
    Duration::from_nanos(1_000_000_000u64.saturating_mul(d) / n)
}

fn video_send_due(now: Instant, next_due: &mut Instant, period: Duration) -> bool {
    if now < *next_due {
        return false;
    }
    *next_due = now.checked_add(period).unwrap_or(now);
    true
}

fn output_due(output: &OutputSnap, now: Instant, due: &mut HashMap<u64, Instant>) -> bool {
    video_send_due(
        now,
        due.entry(output.output_id).or_insert(now),
        frame_period(output.fps_n, output.fps_d),
    )
}

fn push_cpu_packed(
    composer: &mut Composer,
    device: &GpuDevice,
    encoder: &mut wgpu::CommandEncoder,
    readbacks: &mut ReadbackStore,
    packed_copies: &mut Vec<(u64, u32, u32)>,
    output: &OutputSnap,
    src: &wgpu::Texture,
) {
    let (width, height) = output.video_size(src.size().width, src.size().height);
    let src_view = src.create_view(&Default::default());
    let Some(packed) =
        composer.pack_rgba_sized(device, encoder, output.output_id, &src_view, width, height)
    else {
        return;
    };
    let packed_w = packed.size().width.saturating_mul(2).max(2);
    let packed_h = packed.size().height.max(1);
    let rb = readbacks.ensure(device, output.output_id, packed_w, packed_h);
    rb.copy_from(encoder, packed);
    packed_copies.push((output.output_id, packed_w, packed_h));
}

fn push_scaled_gpu(
    composer: &mut Composer,
    gpu_sends: &mut GpuSendStore,
    device: &GpuDevice,
    encoder: &mut wgpu::CommandEncoder,
    copies: &mut Vec<GpuEncodeCopy>,
    output: &OutputSnap,
    src: &wgpu::Texture,
    packed_src: bool,
) {
    let (width, height) = output.video_size(src.size().width, src.size().height);
    if let Some(scaled) = composer.scale_rgba(
        device,
        encoder,
        output.output_id,
        src,
        width,
        height,
        packed_src,
    ) {
        push_gpu_encode(gpu_sends, device, encoder, copies, output, scaled);
    } else {
        push_gpu_encode(gpu_sends, device, encoder, copies, output, src);
    }
}

pub(crate) fn emit_gpu_encode(copies: &[GpuEncodeCopy], pts: i64) {
    for copy in copies {
        if copy
            .tx
            .send(SendCmd::GpuVideo {
                texture: copy.texture.clone(),
                width: copy.width,
                height: copy.height,
                pts,
                fps_n: copy.fps_n,
                fps_d: copy.fps_d,
                busy: Arc::clone(&copy.busy),
            })
            .is_err()
        {
            copy.busy.store(false, Ordering::Release);
        }
    }
}

#[allow(dead_code)]
pub(crate) fn follow_gains(
    snapshot: &[UnitSnap],
    scenes: &[(u64, u32, u32, Arc<[OverlayDesc]>, MvLabelStyle)],
    _uploads: &UploadStore,
) -> Vec<(u64, f32)> {
    let spec_map: HashMap<u64, &[OverlayDesc]> = scenes
        .iter()
        .map(|spec| (spec.0, spec.3.as_ref()))
        .collect();
    let mut gains = HashMap::<u64, f32>::new();
    fn add(
        id: u64,
        gain: f32,
        spec_map: &HashMap<u64, &[OverlayDesc]>,
        gains: &mut HashMap<u64, f32>,
    ) {
        if gain.abs() < 1e-4 {
            return;
        }
        if crate::abi::is_scene(id) {
            if let Some(layers) = spec_map.get(&id) {
                for layer in *layers {
                    if layer.audio_follow == 0 {
                        continue;
                    }
                    add(
                        layer.source_id,
                        gain * layer.opacity.max(0.0),
                        spec_map,
                        gains,
                    );
                }
            }
            return;
        }
        if crate::abi::mixing_unit_from_source(id).is_some() {
            return;
        }
        if id > 0 {
            gains
                .entry(id)
                .and_modify(|current| *current = (*current).max(gain))
                .or_insert(gain);
        }
    }
    for (_, _, _, _, _, state, mix_preview, _) in snapshot {
        let mix = state.mix.clamp(0.0, 1.0);
        let incoming = if *mix_preview != 0 {
            *mix_preview
        } else {
            state.mix_incoming()
        };
        add(state.program_source, 1.0 - mix, &spec_map, &mut gains);
        add(incoming, mix, &spec_map, &mut gains);
        for overlay in state.overlays.iter().take(state.overlay_count as usize) {
            if overlay.audio_follow == 0 {
                continue;
            }
            add(
                overlay.source_id,
                overlay.opacity.max(0.0),
                &spec_map,
                &mut gains,
            );
        }
    }
    gains.into_iter().filter(|(_, gain)| *gain > 1e-4).collect()
}

#[allow(dead_code)]
pub(crate) fn audio_for_source(
    uploads: &UploadStore,
    scenes: &HashMap<u64, SceneSpec>,
    source_id: u64,
) -> Option<AudioPacket> {
    if let Some(audio) = uploads
        .audio_store()
        .lock()
        .ok()
        .and_then(|store| store.latest_packet(source_id))
    {
        return Some(audio);
    }
    if crate::abi::is_scene(source_id)
        && let Some(spec) = scenes.get(&source_id)
    {
        for layer in spec.layers.iter() {
            if let Some(audio) = audio_for_source(uploads, scenes, layer.source_id) {
                return Some(audio);
            }
        }
    }
    None
}

/// Scenes and CPU/GPU uploads that must be current for the given buses.
/// Pass only *due* monitor ids for compose; pass every attached monitor when
/// collecting uploads so tile-only sources keep their FIFOs moving.
pub(crate) fn unit_uses_mix_cycle(
    unit_id: u64,
    state: &UnitState,
    mix_inputs: &HashMap<u64, MixInputSpec>,
    scenes: &HashMap<u64, SceneSpec>,
) -> bool {
    let mut seen = HashSet::new();
    let incoming = state.mix_incoming();
    mix_source_cycles(state.program_source, unit_id, mix_inputs, scenes, &mut seen)
        || mix_source_cycles(state.preview_source, unit_id, mix_inputs, scenes, &mut seen)
        || mix_source_cycles(incoming, unit_id, mix_inputs, scenes, &mut seen)
        || state
            .overlays
            .iter()
            .take(state.overlay_count as usize)
            .any(|overlay| {
                mix_source_cycles(overlay.source_id, unit_id, mix_inputs, scenes, &mut seen)
            })
}

pub(crate) fn mix_source_cycles(
    source_id: u64,
    unit_id: u64,
    mix_inputs: &HashMap<u64, MixInputSpec>,
    scenes: &HashMap<u64, SceneSpec>,
    seen: &mut HashSet<u64>,
) -> bool {
    if !seen.insert(source_id) {
        return false;
    }
    if let Some(spec) = mix_inputs.get(&source_id)
        && !spec.is_session_multiview()
        && spec.target_id == unit_id
    {
        return true;
    }
    if let Some(scene) = scenes.get(&source_id) {
        return scene
            .layers
            .iter()
            .any(|layer| mix_source_cycles(layer.source_id, unit_id, mix_inputs, scenes, seen));
    }
    false
}

pub(crate) fn resolve_mix_sources(
    mix_inputs: &HashMap<u64, MixInputSpec>,
    composer: &Composer,
) -> HashMap<u64, wgpu::Texture> {
    let mut sources = HashMap::new();
    for id in mix_inputs.keys() {
        if let Some(texture) = mix_rgba_at(mix_inputs, composer, *id) {
            sources.insert(*id, texture);
        }
    }
    sources
}

pub(crate) fn mix_rgba_at(
    mix_inputs: &HashMap<u64, MixInputSpec>,
    composer: &Composer,
    source_id: u64,
) -> Option<wgpu::Texture> {
    let spec = mix_inputs.get(&source_id)?;
    // Always the previous compose. Auto/T-bar discards the delay ring, and
    // hopping between ring and live looks like a cut. Frame Buffer does not
    // apply to Mix Input video.
    if spec.is_session_multiview() {
        composer.scene_texture(spec.target_id).cloned()
    } else if let Some(bus) = spec.unit_bus() {
        composer.rgba_texture(spec.target_id, bus).cloned()
    } else {
        None
    }
}

pub(crate) fn collect_frame_live_ids(
    scene_specs: &[(u64, u32, u32, Arc<[OverlayDesc]>, MvLabelStyle)],
    snapshot: &[UnitSnap],
    due_gui: &[u64],
    outputs: &[OutputSnap],
    mix_inputs: &HashMap<u64, MixInputSpec>,
    compose_dirty: bool,
) -> (HashSet<u64>, HashSet<u64>) {
    let (mut scenes, mut uploads) =
        collect_live_ids(scene_specs, snapshot, due_gui, outputs, mix_inputs);
    if compose_dirty {
        let dirty: Vec<u64> = scene_specs.iter().map(|spec| spec.0).collect();
        scenes.extend(dirty.iter().copied());
        let (_, dirty_uploads) = collect_live_ids(scene_specs, &[], &dirty, &[], mix_inputs);
        uploads.extend(dirty_uploads);
    }
    (scenes, uploads)
}

pub(crate) fn collect_live_ids(
    scene_specs: &[(u64, u32, u32, Arc<[OverlayDesc]>, MvLabelStyle)],
    snapshot: &[UnitSnap],
    monitor_sources: &[u64],
    outputs: &[OutputSnap],
    mix_inputs: &HashMap<u64, MixInputSpec>,
) -> (HashSet<u64>, HashSet<u64>) {
    let spec_map: HashMap<u64, &[OverlayDesc]> = scene_specs
        .iter()
        .map(|spec| (spec.0, spec.3.as_ref()))
        .collect();
    let mut scenes = HashSet::new();
    let mut uploads = HashSet::new();
    fn add(
        id: u64,
        spec_map: &HashMap<u64, &[OverlayDesc]>,
        mix_inputs: &HashMap<u64, MixInputSpec>,
        scenes: &mut HashSet<u64>,
        uploads: &mut HashSet<u64>,
    ) {
        if let Some(spec) = mix_inputs.get(&id) {
            if spec.is_session_multiview() {
                add(spec.target_id, spec_map, mix_inputs, scenes, uploads);
            }
            return;
        }
        if crate::abi::is_scene(id) || crate::abi::is_multiview(id) {
            if !scenes.insert(id) {
                return;
            }
            if let Some(layers) = spec_map.get(&id) {
                for layer in *layers {
                    add(layer.source_id, spec_map, mix_inputs, scenes, uploads);
                }
            }
            return;
        }
        if crate::abi::mixing_unit_from_source(id).is_some() {
            return;
        }
        if id > 0 {
            uploads.insert(id);
        }
    }
    for (_, _, _, _, _, state, mix_preview, _) in snapshot {
        add(
            state.program_source,
            &spec_map,
            mix_inputs,
            &mut scenes,
            &mut uploads,
        );
        add(
            state.preview_source,
            &spec_map,
            mix_inputs,
            &mut scenes,
            &mut uploads,
        );
        let incoming = if *mix_preview != 0 {
            *mix_preview
        } else {
            state.mix_incoming()
        };
        add(incoming, &spec_map, mix_inputs, &mut scenes, &mut uploads);
        for overlay in state.overlays.iter().take(state.overlay_count as usize) {
            add(
                overlay.source_id,
                &spec_map,
                mix_inputs,
                &mut scenes,
                &mut uploads,
            );
        }
    }
    for &id in monitor_sources {
        add(id, &spec_map, mix_inputs, &mut scenes, &mut uploads);
    }
    for output in outputs {
        match output.source_kind {
            SRC_KIND_SCENE | SRC_KIND_MU_MULTIVIEW => add(
                output.source_id,
                &spec_map,
                mix_inputs,
                &mut scenes,
                &mut uploads,
            ),
            SRC_KIND_INPUT => add(
                output.source_id,
                &spec_map,
                mix_inputs,
                &mut scenes,
                &mut uploads,
            ),
            _ => {}
        }
    }
    (scenes, uploads)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_period_matches_session_rate() {
        assert_eq!(
            frame_period(60_000, 1_001),
            Duration::from_nanos(1_000_000_000u64 * 1_001 / 60_000)
        );
        assert_eq!(
            frame_period(30, 1),
            Duration::from_nanos(1_000_000_000 / 30)
        );
    }

    #[test]
    fn video_send_due_paces_and_does_not_burst() {
        let start = Instant::now();
        let period = Duration::from_millis(16);
        let mut due = start;
        assert!(video_send_due(start, &mut due, period));
        assert!(!video_send_due(
            start + Duration::from_millis(1),
            &mut due,
            period
        ));
        assert!(video_send_due(
            start + Duration::from_millis(16),
            &mut due,
            period
        ));
        // A hitch must not emit a catch-up burst; the next slot is from now.
        let late = start + Duration::from_millis(80);
        assert!(video_send_due(late, &mut due, period));
        assert!(!video_send_due(
            late + Duration::from_millis(1),
            &mut due,
            period
        ));
    }
}
