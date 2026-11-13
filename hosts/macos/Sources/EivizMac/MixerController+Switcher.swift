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

    /// Cut/Fade changes the unit's program/preview scene but the control document
    /// keeps the buses it had. Send the unit so eivizctl/remote and the saved
    /// document see the same picture the operator just took.
    private func persistLiveBuses(_ unitId: UInt64) {
        guard !isRemote, let unit = session.units.first(where: { $0.id == unitId }) else { return }
        _ = mutateLocal(MixerRemote.upsertUnit(unit))
    }

    private func fireAuto(_ preset: TransitionPreset, unit: MixingUnitEntry, reason: String) {
        var curve = EivizCurve(
            kind: preset.easing,
            x1: preset.bezier?.x1 ?? 0,
            y1: preset.bezier?.y1 ?? 0,
            x2: preset.bezier?.x2 ?? 1,
            y2: preset.bezier?.y2 ?? 1
        )
        if isRemote {
            let ms = preset.durationUnit == EIVIZ_DURATION_MS
                ? max(1, preset.durationValue)
                : session.settings.wallMs(max(1, preset.durationValue))
            _ = mixer_remote_auto(
                remoteHandle,
                unit.id,
                preset.kind,
                ms,
                preset.swap ? 1 : 0,
                preset.keepPreview ? 1 : 0,
                preset.easing,
                preset.direction,
                preset.dipR,
                preset.dipG,
                preset.dipB,
                preset.dipA <= 0 ? 1 : preset.dipA,
                preset.softness,
                preset.param,
                curve.x1,
                curve.y1,
                curve.x2,
                curve.y2
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
                session.settings.frames(for: preset.durationValue, unit: preset.durationUnit),
                preset.swap ? 1 : 0,
                preset.keepPreview ? 1 : 0,
                &curve,
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
        persistLiveBuses(unit.id)
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
        persistLiveBuses(selectedUnit.id)
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
            persistLiveBuses(unit.id)
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
            // The core creates the replay document and the channel input, with the
            // reserved channel id.
            if input.kind == .replay, replacing == nil {
                _ = mutateRemote(MixerRemote.addReplayChannel())
                return
            }
            if replacing == nil, input.kind == .still || input.kind == .video {
                guard let path = input.pathOrAddress else { return }
                guard !remoteUploading else { return }
                remoteUploading = true
                let kind = input.kind == .still ? "still" : "video"
                let name = input.name
                let loop: UInt32 = input.videoLoop ? 1 : 0
                let handle = remoteHandle
                let revision = remoteRevision
                // Uploads can take tens of seconds; keep them off the main actor.
                Task { [weak self] in
                    let code = await Task.detached(priority: .userInitiated) {
                        MixerFFI.withCString(path) { pathPtr in
                            MixerFFI.withCString(kind) { kindPtr in
                                MixerFFI.withCString(name) { namePtr in
                                    mixer_remote_upload(handle, pathPtr, kindPtr, namePtr, loop, revision)
                                }
                            }
                        }
                    }.value
                    guard let self else { return }
                    self.remoteUploading = false
                    if code != 0 {
                        self.presentInputError(L10n.t("msg.uploadFailed"))
                        return
                    }
                    self.pollRemote(force: true)
                }
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
        var isNew = false
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
            isNew = true
            // A Replay channel owns a reserved id and brings the replay document
            // with it; it never takes a normal input id.
            if entry.kind == .replay {
                guard assignReplayChannel(&entry) else {
                    presentInputError(L10n.t("replay.channelsFull"))
                    return
                }
            } else if entry.id == 0 || session.inputs.contains(where: { $0.id == entry.id }) {
                entry.id = session.nextInputId
                session.nextInputId += 1
            }
            session.inputs.append(entry)
            pendingInputAdds.insert(entry.id)
        }
        attach(entry)
        selectedInputId = entry.id
        // The mixer learns a Replay channel and its document from the session, so
        // adding the input publishes straight away instead of at the first edit.
        if entry.kind == .replay {
            publishSession()
        } else if isNew {
            // The host just started the source (capture device, decoder, generator).
            // Registering the input updates the document without reconciling it,
            // because a reconcile would start the same source again and restart UVC.
            if MixerFFI.registerInput(entry) == EIVIZ_OK {
                pendingInputAdds.remove(entry.id)
            } else if mutateLocal(MixerRemote.upsertInput(entry)) {
                // The document was not ready for a direct registration; fall back to
                // the reconciling path (it restarts the source, but stays correct).
                pendingInputAdds.remove(entry.id)
            }
        } else if mutateLocal(MixerRemote.upsertInput(entry)) {
            // An edit changed the source config, so it has to be applied again.
            pendingInputAdds.remove(entry.id)
        }
        objectWillChange.send()
    }

    func deleteSelectedInput() {
        guard let id = selectedInputId,
              let index = session.inputs.firstIndex(where: { $0.id == id }),
              !session.inputs[index].isBuiltin
        else { return }
        if isRemote {
            if session.inputs[index].kind == .replay, isLastReplayChannel(inputId: id) {
                // Removing the last channel removes the whole replay document.
                _ = mutateRemote(MixerRemote.deleteReplay())
            } else {
                _ = mutateRemote(MixerRemote.deleteInput(id))
            }
            return
        }
        let kind = session.inputs[index].kind
        let lastReplay = kind == .replay && isLastReplayChannel(inputId: id)
        let wasPending = pendingInputAdds.remove(id) != nil
        closeInputPreview(id)
        closeAudioInput(id)
        videoRoles.removeValue(forKey: id)
        _ = mixer_destroy_source(id)
        removeReplayChannel(inputId: id)
        session.inputs.remove(at: index)
        session.overlays.removeAll { $0.sourceKind == .input && $0.sceneGpuId == id }
        for unitIndex in session.units.indices {
            session.units[unitIndex].overlaysOnAir.removeAll { oid in
                !session.overlays.contains { $0.id == oid }
            }
        }
        selectedInputId = nil
        pushOverlays()
        // Same rule as adding: the document has to learn about the removal, or a
        // client reading it still sees an input the mixer no longer owns. An input
        // the document never accepted (still pending) has nothing to remove, and the
        // last Replay channel has to go through DeleteReplay or the core refuses.
        if !wasPending {
            if kind == .replay {
                _ = mutateLocal(lastReplay ? MixerRemote.deleteReplay() : MixerRemote.deleteInput(id))
            } else {
                _ = mutateLocal(MixerRemote.deleteInput(id))
            }
        }
    }

    func moveInputs(from offsets: IndexSet, to destination: Int) {
        guard inputFilter == .all else { return }
        let previous = session.inputs
        var next = previous
        next.move(fromOffsets: offsets, toOffset: destination)
        session.inputs = next
        let ids = next.map(\.id)
        let applied = isRemote
            ? mutateRemote(MixerRemote.setInputOrder(ids))
            : mutateLocal(MixerRemote.setInputOrder(ids))
        if !applied {
            session.inputs = previous
        }
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
        _ = mutateLocal(MixerRemote.upsertScene(scene))
    }

    func removeScene() {
        guard let id = selectedSceneId,
              let scene = session.scenes.first(where: { $0.id == id })
        else { return }
        deleteScene(scene)
    }

    func deleteScene(_ scene: SceneEntry) {
        guard session.scenes.count > 1 else { return }
        // An angle records a scene's composited output, so the scene is not the
        // operator's to delete while Replay depends on it.
        if let angle = replayAngle(recording: scene.id) {
            presentError(
                L10n.format("replay.sceneRecorded", angle.label),
                title: L10n.t("replay.windowTitle")
            )
            return
        }
        if isRemote {
            _ = mutateRemote(MixerRemote.deleteScene(scene.id))
            return
        }
        closeInputPreview(scene.gpuId)
        _ = mixer_destroy_scene(scene.gpuId)
        session.scenes.removeAll { $0.id == scene.id }
        session.overlays.removeAll { $0.sourceKind == .scene && $0.sceneGpuId == scene.gpuId }
        for unitIndex in session.units.indices {
            session.units[unitIndex].overlaysOnAir.removeAll { oid in
                !session.overlays.contains { $0.id == oid }
            }
        }
        pushOverlays()
        if let next = session.scenes.first {
            previewScene(next)
        }
        _ = mutateLocal(MixerRemote.deleteScene(scene.id))
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
        scheduleInputPersist(session.inputs[index])
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
            pushInputAudio(input)
            scheduleInputPersist(input)
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
        if isRemote {
            _ = mutateRemote(MixerRemote.upsertUnit(entry))
            return
        }
        guard fail(mixer_create_unit(entry.id, entry.width, entry.height), "Create Mixing Unit") else { return }
        session.nextUnitId += 1
        fail(mixer_unit_configure(entry.id, entry.width, entry.height, entry.fpsNum, entry.fpsDen), "Configure Mixing Unit")
        MixerFFI.withCString(entry.audio.deviceId) { device in
            fail(
                mixer_audio_unit_bus_set(
                    entry.id,
                    entry.audio.deviceKind.rawUInt,
                    device,
                    entry.audio.mapLeft,
                    entry.audio.mapRight
                ),
                "MU Bus"
            )
        }
        fail(mixer_audio_set_unit_link(entry.id, entry.audioLink.rawUInt), "Audio link")
        let preview = session.scenes.first?.gpuId ?? UInt64(EIVIZ_SRC_BARS)
        let program = session.scenes.count > 1 ? session.scenes[1].gpuId : preview
        applyBusSources(unitId: entry.id, preview: preview, program: program)
        pushState(unitId: entry.id, program: program, preview: preview, mix: 0, kind: EIVIZ_TRANSITION_FADE)
        session.units.append(entry)
        selectedUnitId = entry.id
        updateStatus()
        _ = mutateLocal(MixerRemote.upsertUnit(entry))
    }

    func deleteUnit() {
        guard session.units.count > 1 else { return }
        let id = selectedUnitId
        // An angle may record a unit's program mix, which makes the unit the wrong
        // thing to delete.
        if let angle = replayAngle(listening: id) {
            presentError(
                L10n.format("replay.unitRecorded", angle.label),
                title: L10n.t("replay.windowTitle")
            )
            return
        }
        if isRemote {
            _ = mutateRemote(MixerRemote.deleteUnit(id))
            return
        }
        closeSwitcher(id)
        _ = mixer_destroy_unit(id)
        session.units.removeAll { $0.id == id }
        selectedUnitId = session.units[0].id
        _ = mutateLocal(MixerRemote.deleteUnit(id))
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
        MixerFFI.withCString(unit.audio.deviceId) { device in
            fail(
                mixer_audio_unit_bus_set(
                    unit.id,
                    unit.audio.deviceKind.rawUInt,
                    device,
                    unit.audio.mapLeft,
                    unit.audio.mapRight
                ),
                "MU Bus"
            )
        }
        fail(mixer_audio_set_unit_link(unit.id, unit.audioLink.rawUInt), "Audio link")
        _ = mixer_audio_set_bus_gain(unit.id, max(0, unit.audio.gain), unit.audio.mute ? 1 : 0)
        selectedUnitId = unit.id
        if let window = switcherWindows[unit.id] {
            window.title = unit.name
            window.level = unit.alwaysOnTop ? .floating : .normal
        }
        updateStatus()
        _ = mutateLocal(MixerRemote.upsertUnit(unit))
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
        guard let index = session.units.firstIndex(where: { $0.id == selectedUnitId }) else { return }
        let enabled = !session.units[index].overlaysOnAir.contains(slot.id)
        setOverlayEnabled(slot.id, enabled: enabled)
    }

    /// Removes an output row and stops whatever it started.
    ///
    /// RTMP is not an Output in the mixer: a row is an encoder plus a destination and
    /// `mixer_output_remove` knows neither. Deleting the row alone left the encoder
    /// publishing to the URL with no UI left to stop it.
    func removeOutput(_ output: OutputEntry) {
        if isRemote { return }
        if output.transport == .rtmp, output.streamEncoderId != 0 {
            _ = MixerFFI.streamEncoderRemove(output.streamEncoderId)
            session.streamEncoders.removeAll { $0.id == output.streamEncoderId }
        }
        if output.transport == .rtmp, !output.rtmpSecretRef.isEmpty {
            // The key belongs to the row: leaving it in the Keychain after the row is
            // gone keeps a live credential around with nothing that uses it.
            KeychainStore.save(account: "rtmp/" + output.rtmpSecretRef, token: "")
        }
        _ = mixer_output_remove(output.id)
        session.outputs.removeAll { $0.id == output.id }
        if output.transport != .rtmp {
            _ = mutateLocal(MixerRemote.deleteOutput(output.id))
        }
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
        if entry.transport == .rtmp, entry.rtmpSecretRef.isEmpty {
            entry.rtmpSecretRef = "stream-\(entry.id)"
        }
        // The encoder this row is already running (the row is bound to the session,
        // so `streamEncoderId` is the one to stop), but stop it only once the new
        // request is known to be startable: a missing key used to tear down a live
        // stream and then refuse to start its replacement.
        let previousEncoder = session.outputs.first { $0.id == entry.id }?.streamEncoderId ?? 0
        var rtmpKey = ""
        if entry.transport == .rtmp, entry.enabled {
            guard !entry.rtmpSecretRef.isEmpty else {
                presentInputError(L10n.t("settings.streamKeyMissing"), editing: false)
                return
            }
            rtmpKey = KeychainStore.load(account: "rtmp/" + entry.rtmpSecretRef)
            guard !rtmpKey.isEmpty else {
                presentInputError(L10n.t("settings.streamKeyMissing"), editing: false)
                return
            }
        }
        if let index = session.outputs.firstIndex(where: { $0.id == entry.id }) {
            session.outputs[index] = entry
        } else {
            session.outputs.append(entry)
        }
        if entry.transport == .rtmp {
            if previousEncoder != 0 {
                _ = MixerFFI.streamEncoderRemove(previousEncoder)
            }
            guard entry.enabled else { return }
            let key = rtmpKey
            var encoder = session.streamEncoders.first { $0.id == entry.streamEncoderId }
                ?? StreamEncoderEntry(id: entry.streamEncoderId == 0 ? session.nextStreamEncoderId : entry.streamEncoderId)
            if encoder.id == session.nextStreamEncoderId {
                session.nextStreamEncoderId = max(session.nextStreamEncoderId + 1, encoder.id + 1)
            }
            encoder.sourceKind = entry.sourceKind
            encoder.sourceId = entry.sourceId
            encoder.unitId = entry.unitId
            encoder.audioUnitId = entry.audioUnitId
            encoder.width = entry.width == 0 ? 1920 : entry.width
            encoder.height = entry.height == 0 ? 1080 : entry.height
            encoder.fpsNum = entry.fpsNum == 0 ? session.settings.masterFpsNum : entry.fpsNum
            encoder.fpsDen = entry.fpsDen == 0 ? session.settings.masterFpsDen : entry.fpsDen
            encoder.name = entry.name.isEmpty ? "Stream" : entry.name
            let sourceId = entry.sourceId
            entry.streamEncoderId = encoder.id
            if let index = session.streamEncoders.firstIndex(where: { $0.id == encoder.id }) {
                session.streamEncoders[index] = encoder
            } else {
                session.streamEncoders.append(encoder)
            }
            if let index = session.outputs.firstIndex(where: { $0.id == entry.id }) {
                session.outputs[index] = entry
            }
            let code = MixerFFI.streamEncoderAdd(encoder, sourceKind: entry.sourceKind.rawValueU32, sourceId: sourceId)
            guard code == 0 else {
                _ = fail(code, "Start stream encoder")
                return
            }
            MixerFFI.withCString(entry.rtmpSecretRef) { name in
                MixerFFI.withCString(key) { value in
                    _ = mixer_secret_set(name, value)
                }
            }
            let destination = MixerFFI.streamDestinationAdd(
                encoderId: encoder.id,
                destinationId: entry.id,
                url: entry.rtmpUrl,
                secretRef: entry.rtmpSecretRef
            )
            _ = fail(destination, "Start RTMP destination")
            return
        }
        _ = mixer_output_remove(entry.id)
        guard entry.enabled else {
            // Keep the disabled row in the document so another client sees the same
            // list; its outputs are what the core reconciles against.
            _ = mutateLocal(MixerRemote.upsertOutput(entry))
            return
        }
        let config = outputConfigJSON(entry)
        let id = entry.id
        let transport = entry.transport.rawValueU32
        let name = entry.name
        let sourceKind = entry.sourceKind.rawValueU32
        let sourceId = entry.sourceId
        let unitId = entry.unitId
        let useGpu: UInt32 = entry.useGpu ? 1 : 0
        let audioUnitId = entry.sourceKind == .multiview ? 0 : entry.audioUnitId
        let skipIdle: UInt32 = entry.transport == .omt && entry.skipEncodeWhenNoReceivers ? 1 : 0
        let width = entry.width
        let height = entry.height
        let fpsNum = entry.fpsNum
        let fpsDen = entry.fpsDen
        let settledEntry = entry
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            MixerFFI.withCString(name) { cName in
                MixerFFI.withCString(config) { cConfig in
                let code = mixer_output_add_ex(
                    id,
                    transport,
                    cName,
                    sourceKind,
                    sourceId,
                    unitId,
                    useGpu,
                    audioUnitId,
                    skipIdle,
                    width,
                    height,
                    fpsNum,
                    fpsDen,
                    cConfig
                )
                DispatchQueue.main.async {
                    if code != 0 {
                        _ = self?.fail(code, "Add output")
                    } else {
                        _ = self?.mutateLocal(MixerRemote.upsertOutput(settledEntry))
                    }
                }
                }
            }
        }
    }

    private func outputConfigJSON(_ entry: OutputEntry) -> String {
        func esc(_ value: String) -> String {
            value
                .replacingOccurrences(of: "\\", with: "\\\\")
                .replacingOccurrences(of: "\"", with: "\\\"")
        }
        return """
        {"device":"\(esc(entry.decklinkDevice))","mode":"\(esc(entry.decklinkMode))","externalKey":\(entry.decklinkExternalKey),"url":"\(esc(entry.rtmpUrl))","secretRef":"\(esc(entry.rtmpSecretRef))"}
        """
    }

}
