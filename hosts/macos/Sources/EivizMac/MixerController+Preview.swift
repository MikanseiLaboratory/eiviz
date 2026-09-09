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
    func openMultiviewSettings() {
        settingsCategory = 3
        showSettings = true
    }

    func openNewMultiview() {
        if isRemote {
            let unitId = session.settings.defaultMultiviewUnitId == 0
                ? selectedUnitId
                : session.settings.defaultMultiviewUnitId
            var layout = MultiviewLayout(
                id: session.nextMultiviewId,
                name: "Multiview \(session.nextMultiviewId)",
                previewUnitId: unitId,
                programUnitId: unitId,
                labelAnchor: session.settings.multiviewLabelAnchor,
                labelSize: session.settings.multiviewLabelSize,
                labelUnit: session.settings.multiviewLabelUnit
            )
            layout.ensureTiles()
            layout.seedDefaultBuses(unitId)
            _ = mutateRemote(MixerRemote.upsertMultiview(layout))
            return
        }
        guard FlipBudget.tryOpen(1) else { return }
        let unitId = session.settings.defaultMultiviewUnitId == 0
            ? selectedUnitId
            : session.settings.defaultMultiviewUnitId
        let layout = session.addMultiview(unitId: unitId)
        pushMultiview(layout)
        openMultiviewWindow(layout)
    }

    func openMultiviewWindow(_ layout: MultiviewLayout) {
        if isRemote {
            openMultiviewSettings()
            return
        }
        pushMultiview(layout)
        openMultiview = layout
        if let existing = multiviewWindows[layout.id] {
            existing.makeKeyAndOrderFront(nil)
            existing.level = layout.alwaysOnTop ? .floating : .normal
            return
        }
        guard FlipBudget.tryOpen(1) else { return }
        let host = NSHostingController(rootView: MultiviewView(layoutId: layout.id).environmentObject(self).environment(\.mixerSurfaceEpoch, surfaceEpoch))
        let window = SwitcherHostWindow(
            contentRect: NSRect(x: 0, y: 0, width: 1280, height: 792),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        window.title = layout.name
        window.identifier = NSUserInterfaceItemIdentifier("multiview-\(layout.id)")
        window.contentViewController = host
        window.isReleasedWhenClosed = false
        window.appearance = NSApp.appearance
        window.backgroundColor = EivizTheme.nsBackground
        window.level = layout.alwaysOnTop ? .floating : .normal
        window.center()
        multiviewCloser.onClose = { [weak self] closedId in
            Task { @MainActor in
                self?.multiviewWindows.removeValue(forKey: closedId)
            }
        }
        window.delegate = multiviewCloser
        window.makeKeyAndOrderFront(nil)
        multiviewWindows[layout.id] = window
        showMultiview = false
    }

    func applyMultiviewWindowLevel(_ layout: MultiviewLayout) {
        multiviewWindows[layout.id]?.level = layout.alwaysOnTop ? .floating : .normal
    }

    func openSwitcher(_ unitId: UInt64? = nil) {
        let unit = unit(for: unitId)
        if let existing = switcherWindows[unit.id] {
            existing.makeKeyAndOrderFront(nil)
            existing.level = unit.alwaysOnTop ? .floating : .normal
            return
        }
        guard FlipBudget.tryOpen(2) else { return }
        let host = NSHostingController(rootView: SwitcherView(unitId: unit.id).environmentObject(self).environment(\.mixerSurfaceEpoch, surfaceEpoch))
        let window = SwitcherHostWindow(
            contentRect: NSRect(x: 0, y: 0, width: 1280, height: 640),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        window.title = unit.name
        window.identifier = NSUserInterfaceItemIdentifier("switcher-\(unit.id)")
        window.contentViewController = host
        window.isReleasedWhenClosed = false
        window.appearance = NSApp.appearance
        window.backgroundColor = EivizTheme.nsBackground
        window.tabbingMode = .disallowed
        window.level = unit.alwaysOnTop ? .floating : .normal
        window.center()
        switcherCloser.onClose = { [weak self] closedId in
            Task { @MainActor in
                self?.switcherWindows.removeValue(forKey: closedId)
            }
        }
        window.delegate = switcherCloser
        switcherWindows[unit.id] = window
        window.makeKeyAndOrderFront(nil)
    }

    func closeSwitcher(_ unitId: UInt64) {
        let window = switcherWindows.removeValue(forKey: unitId)
        window?.delegate = nil
        window?.close()
    }

    func openSceneEditor(_ scene: SceneEntry?) {
        if showSceneEditor {
            editingScene = scene
            return
        }
        if !isRemote, !FlipBudget.tryOpen(1) { return }
        editingScene = scene
        showSceneEditor = true
    }

    func openOverlay() {
        if showOverlay {
            return
        }
        if !isRemote, !FlipBudget.tryOpen(1) { return }
        showOverlay = true
    }

    func closeAllSwitchers() {
        for id in Array(switcherWindows.keys) {
            closeSwitcher(id)
        }
    }

    func deleteMultiview(_ id: UInt64) {
        if isRemote {
            _ = mutateRemote(MixerRemote.deleteMultiview(id))
            if openMultiview?.id == id {
                showMultiview = false
                openMultiview = nil
            }
            return
        }
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
        if isRemote {
            _ = mutateRemote(MixerRemote.upsertMultiview(layout))
            return
        }
        guard let index = session.multiviews.firstIndex(where: { $0.id == layout.id }) else { return }
        var item = layout
        item.ensureTiles()
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
        for (z, pane) in item.template.panes.enumerated() {
            let source = z < item.tiles.count
                ? item.tiles[z].kind.encoded(item.tiles[z].sourceId)
                : 0
            layer(source, pane.x, pane.y, pane.width, pane.height, Int32(z))
        }
        let names = slotNames(item)
        var owned = names.map { $0.isEmpty ? nil : strdup($0) }
        defer {
            for pointer in owned {
                if let pointer {
                    free(pointer)
                }
            }
        }
        for i in layers.indices where i < owned.count {
            if let pointer = owned[i] {
                layers[i].label = UnsafePointer(pointer)
            }
        }
        layers.withUnsafeMutableBufferPointer { ptr in
            fail(
                mixer_define_scene(item.gpuId, selectedUnit.width, selectedUnit.height, UInt32(ptr.count), ptr.baseAddress),
                "Define Multiview"
            )
        }
        let previewUnit = item.tiles.first(where: { $0.kind == .muPreview })?.sourceId ?? item.previewUnitId
        let programUnit = item.tiles.first(where: { $0.kind == .muProgram })?.sourceId ?? item.programUnitId
        fail(mixer_bind_multiview(item.gpuId, previewUnit == 0 ? 1 : previewUnit, programUnit == 0 ? 1 : programUnit), "Bind Multiview")
        let interval = item.presentInterval == 0 ? session.settings.defaultPresentInterval : item.presentInterval
        _ = mixer_set_monitor_present_interval(item.monitorId, max(1, interval))
    }

    func applyBusColors() {
        let preview = session.settings.previewColor
        let program = session.settings.programColor
        let inactive = session.settings.inactiveColor
        _ = mixer_set_bus_colors(
            preview.r, preview.g, preview.b,
            program.r, program.g, program.b,
            inactive.r, inactive.g, inactive.b
        )
        let fallback = min(200, max(1, session.settings.multiviewLabelSize))
        session.settings.multiviewLabelSize = fallback
        _ = mixer_set_mv_label(
            0,
            fallback,
            session.settings.multiviewLabelUnit == .percent ? 1 : 0,
            session.settings.multiviewLabelAnchor == .top ? 1 : 0
        )
        for layout in session.multiviews {
            let size = layout.resolvedLabelSize(session.settings)
            let percent = layout.resolvedLabelUnit(session.settings) == .percent ? UInt32(1) : 0
            let top = layout.resolvedLabelAnchor(session.settings) == .top ? UInt32(1) : 0
            _ = mixer_set_mv_label(layout.gpuId, size, percent, top)
        }
    }

    private func slotNames(_ layout: MultiviewLayout) -> [String] {
        layout.template.panes.indices.map { index in
            index < layout.tiles.count ? tileLabel(layout.tiles[index]) : ""
        }
    }

    private func tileLabel(_ tile: MvSlot) -> String {
        if !tile.labelFollow {
            return tile.label
        }
        switch tile.kind {
        case .input:
            return session.inputs.first(where: { $0.id == tile.sourceId })?.name ?? ""
        case .scene:
            return session.scenes.first(where: { $0.gpuId == tile.sourceId })?.name ?? ""
        case .muPreview:
            let name = session.units.first(where: { $0.id == tile.sourceId })?.name ?? String(tile.sourceId)
            return "PRV  \(name)"
        case .muProgram:
            let name = session.units.first(where: { $0.id == tile.sourceId })?.name ?? String(tile.sourceId)
            return "PGM  \(name)"
        default:
            return ""
        }
    }

    func setOverlayEnabled(_ id: UUID, enabled: Bool, unitId: UInt64? = nil) {
        let targetId = unitId ?? selectedUnitId
        guard let unitIndex = session.units.firstIndex(where: { $0.id == targetId }),
              let slotIndex = session.units[unitIndex].overlays.firstIndex(where: { $0.id == id })
        else { return }
        let slot = session.units[unitIndex].overlays[slotIndex]
        let unit = session.units[unitIndex]
        var desc = MixerFFI.emptyOverlay()
        desc.source_id = slot.sceneGpuId
        desc.rect = EivizRect(x: slot.x, y: slot.y, width: slot.width, height: slot.height)
        desc.crop = EivizRect(x: slot.cropX, y: slot.cropY, width: slot.cropWidth, height: slot.cropHeight)
        desc.opacity = slot.opacity
        desc.z = slot.z
        desc.audio_follow = slot.audioFollow ? 1 : 0
        desc.hidden = slot.hidden ? 1 : 0
        let ms = slot.durationUnit == EIVIZ_DURATION_MS
            ? max(1, slot.durationValue)
            : unit.durationMs(slot.durationValue)
        if slot.transitionKind == EIVIZ_TRANSITION_CUT || ms <= 1 {
            session.units[unitIndex].overlays[slotIndex].enabled = enabled
            overlayOn[id] = enabled
            if isRemote {
                _ = mixer_remote_overlay_auto(remoteHandle, unit.id, UInt32(slotIndex), 1, enabled ? 1 : 0)
            } else {
                pushOverlays(unitId: unit.id)
            }
            return
        }
        if enabled {
            session.units[unitIndex].overlays[slotIndex].enabled = true
            overlayOn[id] = true
            if isRemote {
                _ = mixer_remote_overlay_auto(remoteHandle, unit.id, UInt32(slotIndex), ms, 1)
                return
            }
            pushOverlays(unitId: unit.id)
            fail(mixer_unit_overlay_auto(unit.id, 1, ms, &desc), "overlay auto")
            return
        }
        session.units[unitIndex].overlays[slotIndex].enabled = false
        overlayOn[id] = false
        if isRemote {
            _ = mixer_remote_overlay_auto(remoteHandle, unit.id, UInt32(slotIndex), ms, 0)
            return
        }
        pushOverlays(forceEnabled: id, unitId: unit.id)
        fail(mixer_unit_overlay_auto(unit.id, 0, ms, &desc), "overlay auto")
        DispatchQueue.main.asyncAfter(deadline: .now() + .milliseconds(Int(ms))) { [weak self] in
            guard let self,
                  let ui = self.session.units.firstIndex(where: { $0.id == unit.id }),
                  let si = self.session.units[ui].overlays.firstIndex(where: { $0.id == id })
            else { return }
            if self.session.units[ui].overlays[si].enabled {
                return
            }
            self.overlayOn[id] = false
            self.pushOverlays(unitId: unit.id)
        }
    }

    func snapshotProgram() {
        if isRemote { return }
        saveSnapshot(
            sourceId: selectedUnitId,
            kind: EIVIZ_OUTPUT_PROGRAM,
            name: selectedUnit.name
        )
    }

    func snapshotScene(_ scene: SceneEntry) {
        if isRemote { return }
        saveSnapshot(sourceId: scene.gpuId, kind: 0, name: scene.name)
    }

    func snapshotInput(_ input: InputEntry) {
        if isRemote { return }
        saveSnapshot(sourceId: input.id, kind: EIVIZ_OUTPUT_SOURCE, name: input.name)
    }

    func snapshotSelectedInput() {
        if isRemote { return }
        guard let id = selectedInputId,
              let input = session.inputs.first(where: { $0.id == id })
        else {
            presentError(L10n.t("msg.selectInputScreenshot"), title: L10n.t("chrome.screenshot"))
            return
        }
        snapshotInput(input)
    }

    private func saveSnapshot(sourceId: UInt64, kind: UInt32, name: String) {
        let panel = NSSavePanel()
        panel.allowedContentTypes = [.png, .jpeg]
        panel.allowsOtherFileTypes = false
        panel.nameFieldStringValue = snapshotFileName(name)
        guard panel.runModal() == .OK, let url = panel.url else { return }
        MixerFFI.withCString(url.path) { path in
            _ = fail(mixer_snapshot(sourceId, kind, path), "Screenshot")
        }
    }

    private func snapshotFileName(_ name: String) -> String {
        let cleaned = name.replacingOccurrences(
            of: "[/\\\\?%*|\"<>:]",
            with: "_",
            options: .regularExpression
        )
        return (cleaned.isEmpty ? "eiviz" : cleaned) + ".png"
    }

}
