import AppKit
import AVFoundation
import Combine
import EivizMixer
import Foundation
import SwiftUI

@MainActor
final class MixerController: ObservableObject {
    @Published var session = MixerSessionData.default()
    @Published var selectedUnitId: UInt64 = 1
    @Published var selectedSceneId: UInt64?
    @Published var selectedInputId: UInt64?
    @Published var mix: Float = 0
    @Published var tbarLocked = false
    @Published var tbarPresetIndex = 0
    @Published var status = ""
    @Published var errorText = ""
    @Published var warnText = ""
    @Published var resourceText = ""
    @Published var peaks: [UInt64: (Float, Float)] = [:]
    @Published var overlayOn: [UUID: Bool] = [:]
    @Published var videoFraction: Double = 0
    @Published var videoPlaying = false
    @Published var videoTitle = ""
    @Published var showSettings = false
    @Published var showAddInput = false
    @Published var editingInput: InputEntry?
    @Published var showMixingUnit = false
    @Published var showSceneEditor = false
    @Published var showOverlay = false
    @Published var showMultiview = false
    @Published var showMultiviewSlots = false
    @Published var showResources = false
    @Published var editingUnit: MixingUnitEntry?
    @Published var editingScene: SceneEntry?
    @Published var openMultiview: MultiviewLayout?
    @Published var expandedTransitions: Set<UUID> = []

    let pumps = FramePump()
    private var booted = false
    private var tbarLatching = false
    private var meterTimer: Timer?
    private var previewByUnit: [UInt64: UInt64] = [:]
    private var programByUnit: [UInt64: UInt64] = [:]
    private var inputPreviewWindows: [UInt64: NSWindow] = [:]
    private var inputPreviewMonitorIds: [UInt64: UInt64] = [:]
    private let inputPreviewCloser = InputPreviewCloser()
    private var videoRoles: [UInt64: (program: Bool, preview: Bool)] = [:]

    var selectedUnit: MixingUnitEntry {
        session.units.first { $0.id == selectedUnitId } ?? session.units[0]
    }

    func boot() {
        guard !booted else { return }
        guard mixer_ping() == 0x4549_5649 else {
            errorText = "The Rust mixer ABI does not match this host."
            return
        }
        fail(mixer_create(0, session.settings.masterFpsNum, session.settings.masterFpsDen), "Metal mixer initialization")
        fail(mixer_set_frame_buffer(min(8, max(1, session.settings.frameBufferFrames))), "Set frame buffer")
        applySession()
        meterTimer = Timer.scheduledTimer(withTimeInterval: 0.05, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.tick() }
        }
        booted = true
        updateStatus()
    }

    func shutdown() {
        meterTimer?.invalidate()
        meterTimer = nil
        closeAllInputPreviews()
        guard booted else { return }
        mixer_destroy()
        booted = false
    }

    func openInputPreview(inputId: UInt64, name: String) {
        if let existing = inputPreviewWindows[inputId] {
            existing.makeKeyAndOrderFront(nil)
            return
        }
        let monitorId = session.nextMonitorId
        session.nextMonitorId += 1
        inputPreviewMonitorIds[inputId] = monitorId
        let unit = selectedUnit
        let host = NSHostingController(rootView: InputPreviewView(
            monitorId: monitorId,
            sourceId: inputId,
            ratioWidth: unit.width,
            ratioHeight: unit.height
        ))
        let width = CGFloat(960)
        let height = width * CGFloat(max(1, unit.height)) / CGFloat(max(1, unit.width))
        let window = InputPreviewHostWindow(
            contentRect: NSRect(x: 0, y: 0, width: width, height: height),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        window.title = name
        window.identifier = NSUserInterfaceItemIdentifier("input-preview-\(inputId)")
        window.contentViewController = host
        window.isReleasedWhenClosed = false
        window.backgroundColor = NSColor(calibratedWhite: 17 / 255, alpha: 1)
        window.contentAspectRatio = NSSize(width: CGFloat(max(1, unit.width)), height: CGFloat(max(1, unit.height)))
        var behavior = window.collectionBehavior
        behavior.insert(.fullScreenPrimary)
        window.collectionBehavior = behavior
        window.center()
        inputPreviewCloser.onClose = { [weak self] closedId in
            Task { @MainActor in
                self?.inputPreviewDidClose(closedId)
            }
        }
        window.delegate = inputPreviewCloser
        inputPreviewWindows[inputId] = window
        window.makeKeyAndOrderFront(nil)
        _ = mixer_set_monitor_present_interval(monitorId, 1)
    }

    func closeInputPreview(_ inputId: UInt64) {
        let window = inputPreviewWindows.removeValue(forKey: inputId)
        if let monitorId = inputPreviewMonitorIds.removeValue(forKey: inputId) {
            _ = mixer_detach_monitor(monitorId)
        }
        window?.delegate = nil
        window?.close()
    }

    func closeAllInputPreviews() {
        for id in Array(inputPreviewWindows.keys) {
            closeInputPreview(id)
        }
    }

    private func inputPreviewDidClose(_ inputId: UInt64) {
        inputPreviewWindows.removeValue(forKey: inputId)
        if let monitorId = inputPreviewMonitorIds.removeValue(forKey: inputId) {
            _ = mixer_detach_monitor(monitorId)
        }
    }

    func applySession() {
        session.assignMonitors()
        for unit in session.units {
            fail(mixer_create_unit(unit.id, unit.width, unit.height), "Create Mixing Unit")
            fail(mixer_unit_configure(unit.id, unit.width, unit.height, unit.fpsNum, unit.fpsDen), "Configure Mixing Unit")
        }
        pushAudio()
        for scene in session.scenes {
            pushScene(scene)
        }
        let preview = session.scenes.first?.gpuId ?? EIVIZ_SRC_BARS
        let program = session.scenes.count > 1 ? session.scenes[1].gpuId : preview
        for unit in session.units {
            previewByUnit[unit.id] = preview
            programByUnit[unit.id] = program
            pushState(unitId: unit.id, program: program, preview: preview, mix: 0, kind: EIVIZ_TRANSITION_FADE)
        }
        attachInputs()
        for layout in session.multiviews {
            pushMultiview(layout)
        }
        for output in session.outputs where output.enabled && output.transport != .deckLink {
            addOutput(output)
        }
        selectedSceneId = session.scenes.first?.id
        selectedUnitId = session.selectedUnitId == 0 ? (session.units.first?.id ?? 1) : session.selectedUnitId
    }

    func pushAudio() {
        var live: [UInt64] = []
        let n = mixer_audio_bus_count()
        if n > 0 {
            for i in 0..<UInt32(n) {
                var info = MixerFFI.zeroed() as EivizAudioBusInfo
                if mixer_audio_bus_get(i, &info) != 0 { continue }
                live.append(info.id)
            }
        }
        let keep = Set(session.buses.map(\.id))
        for id in live where !keep.contains(id) {
            _ = mixer_audio_bus_remove(id)
        }
        for bus in session.buses {
            MixerFFI.withCString(bus.name) { name in
                MixerFFI.withCString(bus.deviceId) { device in
                    _ = mixer_audio_bus_upsert(
                        bus.id,
                        name,
                        bus.role.rawUInt,
                        bus.deviceKind.rawUInt,
                        device,
                        bus.mapLeft,
                        bus.mapRight,
                        bus.exclusive ? 1 : 0
                    )
                }
            }
            _ = mixer_audio_set_bus_gain(bus.id, max(0, bus.gain), bus.mute ? 1 : 0)
        }
        for input in session.inputs {
            _ = mixer_audio_set_input(input.id, input.busMask == 0 ? 1 : input.busMask, max(0, input.gain), input.mute ? 1 : 0)
        }
        for unit in session.units {
            _ = mixer_audio_set_unit_link(unit.id, unit.audioBusId == 0 ? 1 : unit.audioBusId, unit.audioLink.rawUInt)
        }
        _ = mixer_audio_set_headphone_cue(selectedUnitId)
        _ = mixer_audio_set_headphone_copy_master(session.headphoneCopyMaster ? 1 : 0)
    }

    func cut() {
        takeCut()
        mix = 0
        tbarLocked = false
        updateStatus()
    }

    func takeCut() {
        let preset = tbarPreset()
        fail(mixer_unit_cut(selectedUnit.id, preset.swap ? 1 : 0), "CUT")
    }

    func auto() {
        let preset = tbarPreset()
        if preset.kind == EIVIZ_TRANSITION_CUT || preset.durationFrames <= 1 {
            cut()
            return
        }
        fail(mixer_unit_auto(selectedUnit.id, selectedUnit.durationMs(preset.durationFrames), preset.swap ? 1 : 0), "AUTO")
        mix = 0
        tbarLocked = false
    }

    func firePreset(_ preset: TransitionPreset, index: Int) {
        tbarPresetIndex = index
        if preset.kind == EIVIZ_TRANSITION_CUT || preset.durationFrames <= 1 {
            fail(mixer_unit_cut(selectedUnit.id, preset.swap ? 1 : 0), "TAKE")
        } else {
            fail(
                mixer_unit_auto(selectedUnit.id, selectedUnit.durationMs(preset.durationFrames), preset.swap ? 1 : 0),
                "TAKE"
            )
        }
        mix = 0
        tbarLocked = false
    }

    func previewScene(_ scene: SceneEntry) {
        selectedSceneId = scene.id
        previewByUnit[selectedUnit.id] = scene.gpuId
        var state = currentState(selectedUnit.id)
        state.preview_source = scene.gpuId
        state.mix = mix
        fail(mixer_unit_set_state(selectedUnit.id, &state), "Preview scene")
    }

    func setMix(_ value: Float) {
        if tbarLatching { return }
        if tbarLocked {
            if value < 1 {
                tbarLatching = true
                mix = 1
                tbarLatching = false
            }
            return
        }
        if value >= 0.999 {
            tbarLocked = true
            tbarLatching = true
            mix = 1
            tbarLatching = false
            takeCut()
            return
        }
        mix = value
        var state = currentState(selectedUnit.id)
        state.mix = value
        fail(mixer_unit_set_state(selectedUnit.id, &state), "T-bar")
    }

    func finishTBar() {
        guard tbarLocked else { return }
        tbarLatching = true
        mix = 0
        tbarLatching = false
        tbarLocked = false
    }

    func addInput(_ input: InputEntry) {
        upsertInput(input, replacing: nil)
    }

    func upsertInput(_ input: InputEntry, replacing: UInt64?) {
        var entry = input
        if let id = replacing, let index = session.inputs.firstIndex(where: { $0.id == id }) {
            if !session.inputs[index].isBuiltin {
                pumps.stop(id)
                _ = mixer_destroy_source(id)
            }
            entry.id = id
            session.inputs[index] = entry
        } else {
            if entry.id == 0 || session.inputs.contains(where: { $0.id == entry.id }) {
                entry.id = session.nextInputId
                session.nextInputId += 1
            }
            session.inputs.append(entry)
        }
        attach(entry)
        selectedInputId = entry.id
        objectWillChange.send()
    }

    func deleteSelectedInput() {
        guard let id = selectedInputId,
              let index = session.inputs.firstIndex(where: { $0.id == id }),
              !session.inputs[index].isBuiltin
        else { return }
        closeInputPreview(id)
        videoRoles.removeValue(forKey: id)
        pumps.stop(id)
        _ = mixer_destroy_source(id)
        session.inputs.remove(at: index)
        selectedInputId = nil
    }

    func addScene() {
        let scene = session.addScene(name: "Scene \(session.nextSceneId)", input: nil)
        pushScene(scene)
        previewScene(scene)
    }

    func removeScene() {
        guard session.scenes.count > 1, let id = selectedSceneId,
              let index = session.scenes.firstIndex(where: { $0.id == id })
        else { return }
        _ = mixer_destroy_scene(session.scenes[index].gpuId)
        session.scenes.remove(at: index)
        if let next = session.scenes.first {
            previewScene(next)
        }
    }

    func saveScene(_ scene: SceneEntry) {
        if let index = session.scenes.firstIndex(where: { $0.id == scene.id }) {
            session.scenes[index] = scene
        }
        pushScene(scene)
    }

    func addUnit() {
        let id = session.nextUnitId
        session.nextUnitId += 1
        var unit = MixingUnitEntry(id: id, name: "Mixing Unit \(id)")
        unit.transitions = [
            TransitionPreset(kind: EIVIZ_TRANSITION_CUT, durationFrames: 1, swap: true),
            TransitionPreset(kind: EIVIZ_TRANSITION_FADE, durationFrames: 30, swap: true)
        ]
        session.units.append(unit)
        fail(mixer_create_unit(id, unit.width, unit.height), "Create Mixing Unit")
        selectedUnitId = id
    }

    func deleteUnit() {
        guard session.units.count > 1 else { return }
        let id = selectedUnitId
        _ = mixer_destroy_unit(id)
        session.units.removeAll { $0.id == id }
        selectedUnitId = session.units[0].id
    }

    func saveUnit(_ unit: MixingUnitEntry) {
        if let index = session.units.firstIndex(where: { $0.id == unit.id }) {
            session.units[index] = unit
        }
        fail(mixer_unit_configure(unit.id, unit.width, unit.height, unit.fpsNum, unit.fpsDen), "Configure Mixing Unit")
        fail(mixer_audio_set_unit_link(unit.id, unit.audioBusId, unit.audioLink.rawUInt), "Audio link")
        selectedUnitId = unit.id
        updateStatus()
    }

    func toggleOverlay(_ slot: OverlaySlot) {
        guard let index = session.units.firstIndex(where: { $0.id == selectedUnitId }),
              let slotIndex = session.units[index].overlays.firstIndex(where: { $0.id == slot.id })
        else { return }
        let enabled = !session.units[index].overlays[slotIndex].enabled
        setOverlayEnabled(slot.id, enabled: enabled)
    }

    func addOutput(_ output: OutputEntry) {
        var entry = output
        if entry.transport != .omt {
            entry.useGpu = false
        }
        if let index = session.outputs.firstIndex(where: { $0.id == entry.id }) {
            session.outputs[index] = entry
        }
        _ = mixer_output_remove(entry.id)
        guard entry.enabled else { return }
        MixerFFI.withCString(entry.name) { name in
            fail(
                mixer_output_add(
                    entry.id,
                    entry.transport.rawValueU32,
                    name,
                    entry.sourceKind.rawValueU32,
                    entry.sourceId,
                    entry.unitId,
                    entry.useGpu ? 1 : 0
                ),
                "Add output"
            )
        }
    }

    func openNewMultiview() {
        let unitId = session.settings.defaultMultiviewUnitId == 0
            ? selectedUnitId
            : session.settings.defaultMultiviewUnitId
        let layout = session.addMultiview(unitId: unitId)
        pushMultiview(layout)
        openMultiviewWindow(layout)
    }

    func openMultiviewWindow(_ layout: MultiviewLayout) {
        pushMultiview(layout)
        openMultiview = layout
        showMultiview = true
    }

    func deleteMultiview(_ id: UInt64) {
        if let layout = session.multiviews.first(where: { $0.id == id }) {
            _ = mixer_destroy_scene(layout.gpuId)
            _ = mixer_detach_monitor(layout.monitorId)
        }
        session.multiviews.removeAll { $0.id == id }
        if openMultiview?.id == id {
            showMultiview = false
            openMultiview = nil
        }
    }

    func pushMultiview(_ layout: MultiviewLayout) {
        guard let index = session.multiviews.firstIndex(where: { $0.id == layout.id }) else { return }
        var item = layout
        if item.tiles.count < 8 {
            item.tiles.append(contentsOf: Array(repeating: MvSlot(), count: 8 - item.tiles.count))
        }
        session.multiviews[index] = item
        var layers: [EivizOverlayDesc] = []
        func layer(_ source: UInt64, _ x: Float, _ y: Float, _ w: Float, _ h: Float, _ z: Int32) {
            var desc = MixerFFI.emptyOverlay()
            desc.source_id = source
            desc.rect = EivizRect(x: x, y: y, width: w, height: h)
            desc.opacity = 1
            desc.z = z
            layers.append(desc)
        }
        layer(MvSlotKind.muPreview.encoded(item.previewUnitId), 0, 0, 0.5, 0.5, 0)
        layer(MvSlotKind.muProgram.encoded(item.programUnitId), 0.5, 0, 0.5, 0.5, 1)
        for i in 0..<8 {
            let col = Float(i % 4)
            let row = Float(i / 4)
            layer(item.tiles[i].kind.encoded(item.tiles[i].sourceId), col / 4, 0.5 + row / 4, 0.25, 0.25, Int32(2 + i))
        }
        layers.withUnsafeMutableBufferPointer { ptr in
            fail(
                mixer_define_scene(item.gpuId, selectedUnit.width, selectedUnit.height, UInt32(ptr.count), ptr.baseAddress),
                "Define Multiview"
            )
        }
        fail(mixer_bind_multiview(item.gpuId, item.previewUnitId, item.programUnitId), "Bind Multiview")
        let interval = item.presentInterval == 0 ? session.settings.defaultPresentInterval : item.presentInterval
        _ = mixer_set_monitor_present_interval(item.monitorId, max(1, interval))
    }

    func setOverlayEnabled(_ id: UUID, enabled: Bool) {
        guard let unitIndex = session.units.firstIndex(where: { $0.id == selectedUnitId }),
              let slotIndex = session.units[unitIndex].overlays.firstIndex(where: { $0.id == id })
        else { return }
        session.units[unitIndex].overlays[slotIndex].enabled = enabled
        overlayOn[id] = enabled
        pushOverlays()
    }

    func saveSession() {
        let panel = NSSavePanel()
        panel.allowedContentTypes = [.json]
        panel.nameFieldStringValue = "eiviz.json"
        guard panel.runModal() == .OK, let url = panel.url else { return }
        session.selectedUnitId = selectedUnitId
        session.settings.lastSessionPath = url.path
        do {
            let json = try SessionFile.encode(session)
            MixerFFI.withCString(url.path) { path in
                json.withUnsafeBytes { ptr in
                    fail(
                        mixer_session_save(path, ptr.bindMemory(to: UInt8.self).baseAddress, json.count),
                        "Save session"
                    )
                }
            }
        } catch {
            errorText = error.localizedDescription
        }
    }

    func loadSession() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [.json]
        panel.allowsMultipleSelection = false
        guard panel.runModal() == .OK, let url = panel.url else { return }
        var buffer = [UInt8](repeating: 0, count: 1 << 20)
        let n = MixerFFI.withCString(url.path) { path in
            buffer.withUnsafeMutableBufferPointer { ptr in
                mixer_session_load(path, ptr.baseAddress, ptr.count)
            }
        }
        guard n >= 0 else {
            fail(n, "Load session")
            return
        }
        do {
            let loaded = try SessionFile.decode(Data(buffer.prefix(Int(n))))
            mixer_destroy()
            session = loaded
            selectedUnitId = loaded.selectedUnitId == 0 ? 1 : loaded.selectedUnitId
            mix = 0
            fail(mixer_create(0, session.settings.masterFpsNum, session.settings.masterFpsDen), "Metal mixer initialization")
            fail(mixer_set_frame_buffer(min(8, max(1, session.settings.frameBufferFrames))), "Set frame buffer")
            applySession()
        } catch {
            errorText = error.localizedDescription
        }
    }

    var selectedVideoId: UInt64? {
        guard let id = selectedInputId,
              session.inputs.first(where: { $0.id == id })?.kind == .video
        else { return nil }
        return id
    }

    func videoPlayToggle() {
        guard let id = selectedVideoId ?? pumps.activeFileId else { return }
        videoPlaying.toggle()
        pumps.setPlaying(id, playing: videoPlaying)
    }

    func videoRestart() {
        guard let id = selectedVideoId ?? pumps.activeFileId else { return }
        pumps.seek(id, fraction: 0)
        pumps.setPlaying(id, playing: true)
        videoPlaying = true
        videoFraction = 0
    }

    func videoSeek(_ value: Double) {
        guard let id = selectedVideoId ?? pumps.activeFileId else { return }
        pumps.seek(id, fraction: value)
        videoFraction = value
    }

    private func attachInputs() {
        for input in session.inputs {
            attach(input)
        }
    }

    private func attach(_ input: InputEntry) {
        switch input.kind {
        case .color, .bars:
            if !(input.isBuiltin && !input.scroll) {
                fail(
                    mixer_define_generator(
                        input.id,
                        input.kind == .bars ? EIVIZ_GEN_BARS : EIVIZ_GEN_SOLID,
                        input.colorR,
                        input.colorG,
                        input.colorB,
                        1,
                        input.scroll ? 1 : 0
                    ),
                    "Define colour generator"
                )
            }
            _ = mixer_generator_set_tone(input.id, input.toneHz, input.toneLevelDbfs)
        case .black:
            break
        case .still:
            if let path = input.pathOrAddress {
                MixerFFI.withCString(path) { cstr in
                    fail(mixer_load_still(input.id, cstr), "Still load")
                }
            }
        case .video:
            if let path = input.pathOrAddress {
                pumps.startFile(id: input.id, path: path, loop: input.videoLoop, playing: input.videoStartsPlaying)
                videoTitle = input.name
                videoPlaying = input.videoStartsPlaying
            }
        case .omt:
            if let address = input.pathOrAddress {
                MixerFFI.withCString(address) { cstr in
                    fail(
                        mixer_omt_connect(
                            input.id,
                            cstr,
                            input.useGpu ? 1 : 0,
                            max(1, min(8, input.frameBufferFrames)),
                            input.omtQuality.rawUInt
                        ),
                        "OMT connect"
                    )
                }
                _ = mixer_set_live_save(
                    input.id,
                    input.bandwidthSave.rawUInt,
                    input.keepFullOnMultiview ? EIVIZ_SAVE_FLAG_MULTIVIEW : 0
                )
            }
        case .ndi:
            if let address = input.pathOrAddress {
                MixerFFI.withCString(address) { cstr in
                    fail(
                        mixer_ndi_connect(
                            input.id,
                            cstr,
                            max(1, min(8, input.frameBufferFrames)),
                            input.ndiBandwidth.rawUInt
                        ),
                        "NDI connect"
                    )
                }
            }
        case .uvc:
            if let deviceId = input.pathOrAddress {
                startCapture(id: input.id, deviceId: deviceId)
            }
        }
        _ = mixer_audio_set_input(input.id, input.busMask, input.gain, input.mute ? 1 : 0)
    }

    private func startCapture(id: UInt64, deviceId: String) {
        switch AVCaptureDevice.authorizationStatus(for: .video) {
        case .authorized:
            pumps.startCapture(id: id, deviceId: deviceId)
        case .notDetermined:
            Task { @MainActor in
                if await AVCaptureDevice.requestAccess(for: .video) {
                    pumps.startCapture(id: id, deviceId: deviceId)
                } else {
                    errorText = "Camera access was denied."
                }
            }
        default:
            errorText = "Camera access was denied."
        }
    }

    func pushScene(_ scene: SceneEntry) {
        var layers = scene.layers.map { layer -> EivizOverlayDesc in
            var desc = MixerFFI.emptyOverlay()
            desc.source_id = layer.inputId
            desc.rect = EivizRect(x: layer.x, y: layer.y, width: layer.width, height: layer.height)
            desc.opacity = layer.opacity
            desc.z = layer.z
            desc.audio_follow = layer.audioFollow ? 1 : 0
            return desc
        }
        let count = UInt32(layers.count)
        layers.withUnsafeMutableBufferPointer { ptr in
            fail(
                mixer_define_scene(scene.gpuId, selectedUnit.width, selectedUnit.height, count, ptr.baseAddress),
                "Define scene"
            )
        }
    }

    func pushOverlays() {
        var state = currentState(selectedUnit.id)
        fail(mixer_unit_set_state(selectedUnit.id, &state), "Overlays")
    }

    private func pushState(unitId: UInt64, program: UInt64, preview: UInt64, mix: Float, kind: UInt32) {
        var state = MixerFFI.emptyState()
        state.program_source = program
        state.preview_source = preview
        state.mix = mix
        state.transition_kind = kind
        let unit = session.units.first { $0.id == unitId } ?? selectedUnit
        fillAux(&state, unit: unit)
        fail(mixer_unit_set_state(unitId, &state), "Set Mixing Unit state")
    }

    private func currentState(_ unitId: UInt64) -> EivizUnitState {
        var state = MixerFFI.emptyState()
        _ = mixer_unit_get_state(unitId, &state)
        let unit = session.units.first { $0.id == unitId } ?? selectedUnit
        fillAux(&state, unit: unit)
        return state
    }

    private func fillAux(_ state: inout EivizUnitState, unit: MixingUnitEntry) {
        let enabled = unit.overlays.filter { $0.enabled && $0.sceneGpuId != 0 }.prefix(8)
        state.overlay_count = UInt32(enabled.count)
        for (index, slot) in enabled.enumerated() {
            var desc = MixerFFI.emptyOverlay()
            desc.source_id = slot.sceneGpuId
            desc.rect = EivizRect(x: slot.x, y: slot.y, width: slot.width, height: slot.height)
            desc.opacity = slot.opacity
            desc.z = slot.z
            desc.audio_follow = 1
            MixerFFI.setOverlay(&state, index: index, desc)
        }
        let tiles = unit.multiviewTiles
        state.mv_slot_count = UInt32(min(16, tiles.count))
        for (index, tile) in tiles.prefix(16).enumerated() {
            MixerFFI.setMv(&state, index: index, tile.kind.encoded(tile.sourceId))
        }
    }

    private func tbarPreset() -> TransitionPreset {
        let list = selectedUnit.transitions
        guard !list.isEmpty else {
            return TransitionPreset(kind: EIVIZ_TRANSITION_CUT, durationFrames: 1, swap: true)
        }
        return list[min(tbarPresetIndex, list.count - 1)]
    }

    private func tick() {
        var buffer = [EivizAudioPeak](repeating: MixerFFI.zeroed(), count: 32)
        let n = buffer.withUnsafeMutableBufferPointer { ptr in
            mixer_copy_audio_peaks(ptr.baseAddress, UInt32(ptr.count))
        }
        if n > 0 {
            var next: [UInt64: (Float, Float)] = [:]
            for peak in buffer.prefix(Int(n)) {
                next[peak.source_id] = (peak.left, peak.right)
            }
            peaks = next
        }
        var stats = EivizMixerStats(render_ms: 0, frame_budget_ms: 0)
        _ = mixer_copy_stats(&stats)
        if stats.frame_budget_ms > 0 {
            let hud = HostResources.hud(renderMs: stats.render_ms, budgetMs: stats.frame_budget_ms)
            resourceText = hud.text
            warnText = hud.warn
        }
        if let id = selectedVideoId ?? pumps.activeFileId, let info = pumps.info(id) {
            videoPlaying = info.playing
            videoTitle = session.inputs.first { $0.id == id }?.name ?? videoTitle
            if info.duration > 0 {
                videoFraction = info.position / info.duration
            }
        }
        tickVideoTransport()
        updateStatus()
    }

    private func tickVideoTransport() {
        var roles: [UInt64: (program: Bool, preview: Bool)] = [:]
        for unit in session.units {
            var state = MixerFFI.emptyState()
            _ = mixer_unit_get_state(unit.id, &state)
            markVideoRole(&roles, state.program_source, program: true, preview: false)
            markVideoRole(&roles, state.preview_source, program: false, preview: true)
            if state.mix > 0.001 {
                markVideoRole(&roles, state.preview_source, program: true, preview: false)
            }
            for slot in unit.overlays where slot.enabled && slot.sceneGpuId != 0 {
                markVideoRole(&roles, slot.sceneGpuId, program: true, preview: false)
            }
        }
        for id in inputPreviewWindows.keys {
            markVideoRole(&roles, id, program: false, preview: true)
        }
        for input in session.inputs where input.kind == .video {
            let now = roles[input.id] ?? (false, false)
            let prev = videoRoles[input.id] ?? (false, false)
            let roseProgram = now.program && !prev.program
            let fellProgram = !now.program && prev.program
            let rosePreview = now.preview && !prev.preview
            let paused = matchesTrigger(input.videoPauseWhen, roseProgram: roseProgram, fellProgram: fellProgram, rosePreview: rosePreview)
            let restarted = matchesTrigger(input.videoRestartWhen, roseProgram: roseProgram, fellProgram: fellProgram, rosePreview: rosePreview)
            if paused && !restarted {
                pumps.setPlaying(input.id, playing: false)
            }
            if restarted {
                pumps.seek(input.id, fraction: 0)
                pumps.setPlaying(input.id, playing: true)
            } else if shouldPlay(input.videoPlayWhen, roseProgram: roseProgram, rosePreview: rosePreview, now: now) {
                pumps.setPlaying(input.id, playing: true)
            }
            videoRoles[input.id] = now
        }
    }

    private func markVideoRole(
        _ roles: inout [UInt64: (program: Bool, preview: Bool)],
        _ id: UInt64,
        program: Bool,
        preview: Bool
    ) {
        guard id != 0, id < EIVIZ_MULTIVIEW_BASE else { return }
        if id >= EIVIZ_SCENE_BASE {
            guard let scene = session.scenes.first(where: { $0.gpuId == id }) else { return }
            for layer in scene.layers {
                markVideoRole(&roles, layer.inputId, program: program, preview: preview)
            }
            return
        }
        let current = roles[id] ?? (false, false)
        roles[id] = (current.program || program, current.preview || preview)
    }

    private func shouldPlay(
        _ when: VideoPlayWhen,
        roseProgram: Bool,
        rosePreview: Bool,
        now: (program: Bool, preview: Bool)
    ) -> Bool {
        switch when {
        case .onActive: return roseProgram
        case .onPreview: return rosePreview
        case .always: return now.program || now.preview
        case .never: return false
        }
    }

    private func matchesTrigger(
        _ when: VideoTriggerWhen,
        roseProgram: Bool,
        fellProgram: Bool,
        rosePreview: Bool
    ) -> Bool {
        switch when {
        case .onActive: return roseProgram
        case .onDeactivated: return fellProgram
        case .onPreview: return rosePreview
        case .never: return false
        }
    }

    private func updateStatus() {
        let unit = selectedUnit
        status = "\(unit.width)x\(unit.height) \(unit.fpsLabel)   \(unit.name)"
    }

    private func fail(_ code: Int32, _ action: String) {
        if let message = MixerFFI.check(code, action) {
            errorText = message
        }
    }
}

private final class InputPreviewCloser: NSObject, NSWindowDelegate {
    var onClose: ((UInt64) -> Void)?

    func windowWillClose(_ notification: Notification) {
        guard let window = notification.object as? NSWindow,
              let raw = window.identifier?.rawValue,
              raw.hasPrefix("input-preview-"),
              let inputId = UInt64(raw.dropFirst("input-preview-".count))
        else { return }
        onClose?(inputId)
    }
}
