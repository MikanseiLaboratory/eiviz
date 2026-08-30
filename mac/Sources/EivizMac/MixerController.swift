import AppKit
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
    @Published var showEditInput = false
    @Published var showMixingUnit = false
    @Published var showSceneEditor = false
    @Published var showOverlay = false
    @Published var showMultiview = false
    @Published var showResources = false
    @Published var editingUnit: MixingUnitEntry?
    @Published var editingScene: SceneEntry?

    let pumps = FramePump()
    private var booted = false
    private var tbarLatching = false
    private var meterTimer: Timer?
    private var previewByUnit: [UInt64: UInt64] = [:]
    private var programByUnit: [UInt64: UInt64] = [:]

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
        guard booted else { return }
        mixer_destroy()
        booted = false
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
        for output in session.outputs where output.transport != .deckLink {
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
        var entry = input
        if entry.id == 0 {
            entry.id = session.nextInputId
            session.nextInputId += 1
        }
        session.inputs.append(entry)
        attach(entry)
        selectedInputId = entry.id
    }

    func deleteSelectedInput() {
        guard let id = selectedInputId,
              let index = session.inputs.firstIndex(where: { $0.id == id }),
              !session.inputs[index].isBuiltin
        else { return }
        pumps.stop(id)
        _ = mixer_destroy_source(id)
        session.inputs.remove(at: index)
        selectedInputId = nil
    }

    func addScene() {
        let scene = session.addScene(name: "Scene \(session.scenes.count + 1)", input: selectedInputId ?? EIVIZ_SRC_BARS)
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
        guard let index = session.units.firstIndex(where: { $0.id == selectedUnitId }) else { return }
        if let slotIndex = session.units[index].overlays.firstIndex(where: { $0.id == slot.id }) {
            session.units[index].overlays[slotIndex].enabled.toggle()
            overlayOn[slot.id] = session.units[index].overlays[slotIndex].enabled
        }
        pushOverlays()
    }

    func addOutput(_ output: OutputEntry) {
        MixerFFI.withCString(output.name) { name in
            fail(
                mixer_output_add(
                    output.id,
                    output.transport.rawValueU32,
                    name,
                    output.sourceKind.rawValueU32,
                    output.sourceId,
                    output.unitId,
                    output.useGpu ? 1 : 0
                ),
                "Add output"
            )
        }
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

    func videoPlayToggle() {
        guard let id = pumps.activeFileId else { return }
        videoPlaying.toggle()
        pumps.setPlaying(id, playing: videoPlaying)
    }

    func videoRestart() {
        guard let id = pumps.activeFileId else { return }
        pumps.seek(id, fraction: 0)
        pumps.setPlaying(id, playing: true)
        videoPlaying = true
        videoFraction = 0
    }

    func videoSeek(_ value: Double) {
        guard let id = pumps.activeFileId else { return }
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
            if input.isBuiltin { return }
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
                pumps.startFile(id: input.id, path: path)
                videoTitle = input.name
                videoPlaying = true
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
                pumps.startCapture(id: input.id, deviceId: deviceId)
            }
        }
        _ = mixer_audio_set_input(input.id, input.busMask, input.gain, input.mute ? 1 : 0)
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
        fail(mixer_unit_set_state(unitId, &state), "Set Mixing Unit state")
    }

    private func currentState(_ unitId: UInt64) -> EivizUnitState {
        var state = MixerFFI.emptyState()
        _ = mixer_unit_get_state(unitId, &state)
        let unit = session.units.first { $0.id == unitId } ?? selectedUnit
        let enabled = unit.overlays.filter(\.enabled).prefix(8)
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
        return state
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
            resourceText = String(format: "GPU  %.1f / %.1f ms", stats.render_ms, stats.frame_budget_ms)
            warnText = stats.render_ms > stats.frame_budget_ms ? "compose over budget" : ""
        }
        if let id = pumps.activeFileId, let info = pumps.info(id) {
            videoPlaying = info.playing
            if info.duration > 0 {
                videoFraction = info.position / info.duration
            }
        }
        updateStatus()
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
