import AppKit
import AVFoundation
import Combine
import Darwin
import EivizMixer
import EivizRemote
import Foundation
import SwiftUI
import UniformTypeIdentifiers

extension MixerController {
    func cut() {
        takeCut(unitId: selectedUnit.id)
        mix = 0
        tbarLocked = false
        updateStatus()
    }

    private func fireAuto(_ preset: TransitionPreset, unit: MixingUnitEntry, reason: String) {
        if isRemote {
            _ = mixer_remote_auto(
                remoteHandle,
                unit.id,
                preset.kind,
                unit.durationMs(for: preset),
                preset.swap ? 1 : 0,
                preset.keepPreview ? 1 : 0,
                preset.easing,
                preset.direction,
                preset.dipR,
                preset.dipG,
                preset.dipB,
                preset.dipA <= 0 ? 1 : preset.dipA,
                preset.softness,
                preset.param
            )
            return
        }
        let wgsl = resolvedCustomWgsl(preset)
        if wgsl.isEmpty {
            fail(mixer_unit_set_custom_wgsl(unit.id, nil), "custom wgsl")
        } else {
            wgsl.withCString { fail(mixer_unit_set_custom_wgsl(unit.id, $0), "custom wgsl") }
        }
        fail(
            mixer_unit_auto(
                unit.id,
                preset.kind,
                unit.durationMs(for: preset),
                preset.swap ? 1 : 0,
                preset.keepPreview ? 1 : 0,
                preset.easing,
                preset.direction,
                preset.dipR,
                preset.dipG,
                preset.dipB,
                preset.dipA,
                EIVIZ_INCOMING_PREVIEW,
                preset.softness,
                preset.param
            ),
            reason
        )
    }

    func takeCut(unitId: UInt64? = nil) {
        let unit = unit(for: unitId)
        let preset = tbarPreset(for: unit)
        if isRemote {
            _ = mixer_remote_cut(remoteHandle, unit.id, preset.swap ? 1 : 0)
            return
        }
        fail(mixer_unit_cut(unit.id, preset.swap ? 1 : 0, EIVIZ_INCOMING_PREVIEW), "CUT")
        syncUnitBuses(unit.id)
    }

    func auto() {
        let preset = tbarPreset()
        if preset.kind == EIVIZ_TRANSITION_CUT || preset.durationValue <= 1 {
            cut()
            return
        }
        fireAuto(preset, unit: selectedUnit, reason: "AUTO")
        tbarLocked = false
        syncUnitBuses(selectedUnit.id)
    }

    func firePreset(_ preset: TransitionPreset, index: Int) {
        tbarPresetIndex = index
        firePreset(preset, unitId: selectedUnit.id)
        tbarLocked = false
    }

    func firePreset(_ preset: TransitionPreset, unitId: UInt64) {
        let unit = unit(for: unitId)
        if preset.kind == EIVIZ_TRANSITION_CUT || preset.durationValue <= 1 {
            if isRemote {
                _ = mixer_remote_cut(remoteHandle, unit.id, preset.swap ? 1 : 0)
            } else {
                fail(mixer_unit_cut(unit.id, preset.swap ? 1 : 0, EIVIZ_INCOMING_PREVIEW), "TAKE")
            }
        } else {
            fireAuto(preset, unit: unit, reason: "TAKE")
        }
        if !isRemote {
            syncUnitBuses(unit.id)
        }
    }

    func previewScene(_ scene: SceneEntry) {
        previewScene(scene, unitId: selectedUnit.id)
    }

    func previewScene(_ scene: SceneEntry, unitId: UInt64) {
        let unit = unit(for: unitId)
        if isRemote {
            _ = mixer_remote_preview(remoteHandle, unit.id, scene.gpuId)
            selectedSceneId = scene.id
            return
        }
        var state = currentState(unit.id)
        state.preview_source = scene.gpuId
        fail(mixer_unit_set_state(unit.id, &state), "Preview scene")
        applyBusSources(unitId: unit.id, preview: scene.gpuId, program: state.program_source)
        selectedSceneId = scene.id
    }

    func previewingSceneId(for unitId: UInt64) -> UInt64? {
        guard let gpuId = previewByUnit[unitId] else { return nil }
        return session.scenes.first { $0.gpuId == gpuId }?.id
    }

    func programmingSceneId(for unitId: UInt64) -> UInt64? {
        guard let gpuId = programByUnit[unitId] else { return nil }
        return session.scenes.first { $0.gpuId == gpuId }?.id
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
        setMix(value, unitId: selectedUnit.id)
    }

    func setMix(_ value: Float, unitId: UInt64) {
        if isRemote {
            _ = mixer_remote_set_mix(remoteHandle, unitId, value)
            return
        }
        var state = currentState(unitId)
        state.mix = value
        normalizePixelSortDefaults(unitId)
        var preset = tbarPreset(for: unit(for: unitId))
        TransitionCatalog.applyKindDefaults(&preset)
        state.transition_kind = preset.kind
        state.transition_easing = preset.easing
        state.transition_direction = preset.direction
        state.dip_r = preset.dipR
        state.dip_g = preset.dipG
        state.dip_b = preset.dipB
        state.dip_a = preset.dipA <= 0 ? 1 : preset.dipA
        state.softness = preset.softness
        state.param = preset.param
        let wgsl = resolvedCustomWgsl(preset)
        if wgsl.isEmpty {
            _ = mixer_unit_set_custom_wgsl(unitId, nil)
        } else {
            wgsl.withCString { _ = mixer_unit_set_custom_wgsl(unitId, $0) }
        }
        _ = mixer_unit_set_state(unitId, &state)
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
        if isRemote {
            if replacing == nil, input.kind == .still || input.kind == .video {
                guard let path = input.pathOrAddress else { return }
                let kind = input.kind == .still ? "still" : "video"
                let code = MixerFFI.withCString(path) { pathPtr in
                    MixerFFI.withCString(kind) { kindPtr in
                        MixerFFI.withCString(input.name) { namePtr in
                            mixer_remote_upload(remoteHandle, pathPtr, kindPtr, namePtr, input.videoLoop ? 1 : 0, remoteRevision)
                        }
                    }
                }
                if code != 0 {
                    presentInputError(L10n.t("msg.uploadFailed"))
                    return
                }
                pollRemote(force: true)
                return
            }
            if input.kind == .mix,
               input.mixSource != .sessionMultiview,
               let unit = session.units.first(where: { $0.id == input.mixTargetId }),
               mixUnitUses(unit, sourceId: replacing ?? input.id)
            {
                presentInputError(L10n.t("msg.mixCycle"), editing: replacing != nil)
                return
            }
            var entry = input
            if let id = replacing {
                entry.id = id
            } else if entry.id == 0 || session.inputs.contains(where: { $0.id == entry.id }) {
                entry.id = session.nextInputId
            }
            _ = mutateRemote(MixerRemote.upsertInput(entry))
            return
        }
        if input.kind == .mix,
           input.mixSource != .sessionMultiview,
           let unit = session.units.first(where: { $0.id == input.mixTargetId }),
           mixUnitUses(unit, sourceId: replacing ?? input.id)
        {
            presentInputError(L10n.t("msg.mixCycle"), editing: replacing != nil)
            return
        }
        var entry = input
        if let id = replacing, let index = session.inputs.firstIndex(where: { $0.id == id }) {
            if !session.inputs[index].isBuiltin {
                _ = mixer_destroy_source(id)
            }
            entry.id = id
            if entry.guid.isEmpty {
                entry.guid = session.inputs[index].guid
            }
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
        if isRemote {
            _ = mutateRemote(MixerRemote.deleteInput(id))
            return
        }
        closeInputPreview(id)
        closeAudioInput(id)
        videoRoles.removeValue(forKey: id)
        _ = mixer_destroy_source(id)
        session.inputs.remove(at: index)
        for unitIndex in session.units.indices {
            session.units[unitIndex].overlays.removeAll { $0.sourceKind == .input && $0.sceneGpuId == id }
        }
        selectedInputId = nil
        pushOverlays()
    }

    func addScene() {
        if isRemote {
            let scene = SceneEntry(id: session.nextSceneId, name: "Scene \(session.nextSceneId)")
            _ = mutateRemote(MixerRemote.upsertScene(scene))
            return
        }
        let scene = session.addScene(name: "Scene \(session.nextSceneId)", input: nil)
        pushScene(scene)
        previewScene(scene)
    }

    func removeScene() {
        guard let id = selectedSceneId,
              let scene = session.scenes.first(where: { $0.id == id })
        else { return }
        deleteScene(scene)
    }

    func deleteScene(_ scene: SceneEntry) {
        guard session.scenes.count > 1 else { return }
        if isRemote {
            _ = mutateRemote(MixerRemote.deleteScene(scene.id))
            return
        }
        closeInputPreview(scene.gpuId)
        _ = mixer_destroy_scene(scene.gpuId)
        session.scenes.removeAll { $0.id == scene.id }
        for unitIndex in session.units.indices {
            session.units[unitIndex].overlays.removeAll { $0.sourceKind == .scene && $0.sceneGpuId == scene.gpuId }
        }
        pushOverlays()
        if let next = session.scenes.first {
            previewScene(next)
        }
    }

    func cutScene(_ scene: SceneEntry) {
        previewScene(scene)
        cut()
    }

    func toggleSceneLoop(_ scene: SceneEntry) {
        guard let video = sceneVideo(scene),
              let index = session.inputs.firstIndex(where: { $0.id == video.id })
        else { return }
        session.inputs[index].videoLoop.toggle()
        if isRemote {
            _ = mixer_remote_video_loop(remoteHandle, video.id, session.inputs[index].videoLoop ? 1 : 0)
        } else {
            _ = mixer_video_set_loop(video.id, session.inputs[index].videoLoop ? 1 : 0)
        }
        objectWillChange.send()
    }

    func toggleScenePlay(_ scene: SceneEntry) {
        guard let video = sceneVideo(scene) else { return }
        if isRemote {
            _ = mixer_remote_video_play(remoteHandle, video.id, 1)
            objectWillChange.send()
            return
        }
        guard let info = copyVideoInfo(video.id) else { return }
        _ = mixer_video_set_playing(video.id, info.playing == 0 ? 1 : 0)
        objectWillChange.send()
    }

    func toggleSceneAudio(_ scene: SceneEntry) {
        let ids = sceneInputs(scene).map(\.id)
        guard !ids.isEmpty else { return }
        let mute = !sceneInputs(scene).allSatisfy(\.mute)
        for id in ids {
            guard let index = session.inputs.firstIndex(where: { $0.id == id }) else { continue }
            session.inputs[index].mute = mute
            let input = session.inputs[index]
            if isRemote {
                _ = mixer_remote_audio_set_input(remoteHandle, input.id, audioMask(input), max(0, input.gain), mute ? 1 : 0)
            } else {
                _ = mixer_audio_set_input(input.id, audioMask(input), max(0, input.gain), mute ? 1 : 0)
            }
        }
        objectWillChange.send()
    }

    func sceneVideo(_ scene: SceneEntry) -> InputEntry? {
        sceneInputs(scene).first { $0.kind == .video }
    }

    func sceneInputs(_ scene: SceneEntry) -> [InputEntry] {
        scene.layers.compactMap { layer in
            session.inputs.first { $0.id == layer.inputId }
        }
    }

    func scenePlaying(_ scene: SceneEntry) -> Bool {
        guard let video = sceneVideo(scene), let info = copyVideoInfo(video.id) else { return false }
        return info.playing != 0
    }

    func saveScene(_ scene: SceneEntry) {
        if let index = session.scenes.firstIndex(where: { $0.id == scene.id }) {
            session.scenes[index] = scene
        }
        pushScene(scene)
    }

    func addUnit() {
        var unit = MixingUnitEntry(id: session.nextUnitId, name: "Mixing Unit \(session.nextUnitId)")
        unit.width = session.settings.defaultWidth
        unit.height = session.settings.defaultHeight
        unit.fpsNum = session.settings.masterFpsNum
        unit.fpsDen = session.settings.masterFpsDen
        unit.transitions = [
            TransitionPreset(kind: EIVIZ_TRANSITION_CUT, durationValue: 1, swap: true),
            TransitionPreset(kind: EIVIZ_TRANSITION_FADE, durationValue: 30, swap: true)
        ]
        editingUnit = unit
        showMixingUnit = true
    }

    func commitUnit(_ unit: MixingUnitEntry) {
        if session.units.contains(where: { $0.id == unit.id }) {
            saveUnit(unit)
            return
        }
        var entry = unit
        entry.id = session.nextUnitId
        if entry.audioBusId == 0 {
            entry.audioBusId = 1
        }
        if entry.transitions.isEmpty {
            entry.transitions = [
                TransitionPreset(kind: EIVIZ_TRANSITION_CUT, durationValue: 1, swap: true),
                TransitionPreset(kind: EIVIZ_TRANSITION_FADE, durationValue: 30, swap: true)
            ]
        }
        if isRemote {
            _ = mutateRemote(MixerRemote.upsertUnit(entry))
            return
        }
        guard fail(mixer_create_unit(entry.id, entry.width, entry.height), "Create Mixing Unit") else { return }
        session.nextUnitId += 1
        fail(mixer_unit_configure(entry.id, entry.width, entry.height, entry.fpsNum, entry.fpsDen), "Configure Mixing Unit")
        fail(mixer_audio_set_unit_link(entry.id, entry.audioBusId, entry.audioLink.rawUInt), "Audio link")
        let preview = session.scenes.first?.gpuId ?? UInt64(EIVIZ_SRC_BARS)
        let program = session.scenes.count > 1 ? session.scenes[1].gpuId : preview
        applyBusSources(unitId: entry.id, preview: preview, program: program)
        pushState(unitId: entry.id, program: program, preview: preview, mix: 0, kind: EIVIZ_TRANSITION_FADE)
        session.units.append(entry)
        selectedUnitId = entry.id
        updateStatus()
    }

    func deleteUnit() {
        guard session.units.count > 1 else { return }
        let id = selectedUnitId
        if isRemote {
            _ = mutateRemote(MixerRemote.deleteUnit(id))
            return
        }
        closeSwitcher(id)
        _ = mixer_destroy_unit(id)
        session.units.removeAll { $0.id == id }
        selectedUnitId = session.units[0].id
    }

    func saveUnit(_ unit: MixingUnitEntry) {
        if let index = session.units.firstIndex(where: { $0.id == unit.id }) {
            session.units[index] = unit
        }
        if isRemote {
            _ = mutateRemote(MixerRemote.upsertUnit(unit))
            selectedUnitId = unit.id
            updateStatus()
            return
        }
        fail(mixer_unit_configure(unit.id, unit.width, unit.height, unit.fpsNum, unit.fpsDen), "Configure Mixing Unit")
        fail(mixer_audio_set_unit_link(unit.id, unit.audioBusId, unit.audioLink.rawUInt), "Audio link")
        selectedUnitId = unit.id
        if let window = switcherWindows[unit.id] {
            window.title = unit.name
            window.level = unit.alwaysOnTop ? .floating : .normal
        }
        updateStatus()
    }

    func setSwitcherAlwaysOnTop(_ unitId: UInt64, _ on: Bool) {
        guard let index = session.units.firstIndex(where: { $0.id == unitId }) else { return }
        session.units[index].alwaysOnTop = on
        switcherWindows[unitId]?.level = on ? .floating : .normal
    }

    func hideSceneOnSwitcher(_ unitId: UInt64, _ sceneId: UInt64) {
        guard let index = session.units.firstIndex(where: { $0.id == unitId }) else { return }
        var unit = session.units[index]
        if unit.switcherSceneFilter == .all {
            unit.switcherSceneFilter = .exclude
            unit.switcherSceneIds = []
        }
        if unit.switcherSceneFilter == .exclude, !unit.switcherSceneIds.contains(sceneId) {
            unit.switcherSceneIds.append(sceneId)
        }
        if unit.switcherSceneFilter == .include {
            unit.switcherSceneIds.removeAll { $0 == sceneId }
        }
        session.units[index] = unit
    }

    func setSwitcherSceneFilter(_ unitId: UInt64, _ filter: SwitcherSceneFilter, ids: [UInt64]) {
        guard let index = session.units.firstIndex(where: { $0.id == unitId }) else { return }
        session.units[index].switcherSceneFilter = filter
        session.units[index].switcherSceneIds = ids
    }

    func toggleSceneCollapsed(_ sceneId: UInt64) {
        guard let index = session.scenes.firstIndex(where: { $0.id == sceneId }) else { return }
        session.scenes[index].previewCollapsed.toggle()
    }

    func openOverlay(for unitId: UInt64) {
        selectedUnitId = unitId
        openOverlay()
    }

    func toggleOverlay(_ slot: OverlaySlot) {
        guard let index = session.units.firstIndex(where: { $0.id == selectedUnitId }),
              let slotIndex = session.units[index].overlays.firstIndex(where: { $0.id == slot.id })
        else { return }
        let enabled = !session.units[index].overlays[slotIndex].enabled
        setOverlayEnabled(slot.id, enabled: enabled)
    }

    func addOutput(_ output: OutputEntry) {
        if isRemote { return }
        var entry = output
        if entry.transport != .omt {
            entry.useGpu = false
        }
        if entry.sourceKind == .multiview, entry.sourceId != 0, entry.sourceId < EIVIZ_MULTIVIEW_BASE {
            entry.sourceId = EIVIZ_MULTIVIEW_BASE | entry.sourceId
        } else if entry.sourceKind == .scene, entry.sourceId != 0, entry.sourceId < EIVIZ_SCENE_BASE {
            entry.sourceId = EIVIZ_SCENE_BASE | entry.sourceId
        }
        if let index = session.outputs.firstIndex(where: { $0.id == entry.id }) {
            session.outputs[index] = entry
        }
        _ = mixer_output_remove(entry.id)
        guard entry.enabled else { return }
        let id = entry.id
        let transport = entry.transport.rawValueU32
        let name = entry.name
        let sourceKind = entry.sourceKind.rawValueU32
        let sourceId = entry.sourceId
        let unitId = entry.unitId
        let useGpu: UInt32 = entry.useGpu ? 1 : 0
        let audioBusId = entry.sourceKind == .multiview ? 0 : entry.audioBusId
        let skipIdle: UInt32 = entry.transport == .omt && entry.skipEncodeWhenNoReceivers ? 1 : 0
        let width = entry.width
        let height = entry.height
        let fpsNum = entry.fpsNum
        let fpsDen = entry.fpsDen
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            MixerFFI.withCString(name) { cName in
                let code = mixer_output_add(
                    id,
                    transport,
                    cName,
                    sourceKind,
                    sourceId,
                    unitId,
                    useGpu,
                    audioBusId,
                    skipIdle,
                    width,
                    height,
                    fpsNum,
                    fpsDen
                )
                if code != 0 {
                    DispatchQueue.main.async {
                        _ = self?.fail(code, "Add output")
                    }
                }
            }
        }
    }

}
