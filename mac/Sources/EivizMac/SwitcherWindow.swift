import AppKit
import EivizMixer
import SwiftUI

struct SwitcherView: View {
    @EnvironmentObject private var mixer: MixerController
    let unitId: UInt64
    @State private var mix: Float = 0
    @State private var tbarLocked = false
    @State private var tbarLatching = false
    @State private var tbarDragging = false
    @State private var tbarPresetIndex = 0
    @State private var sceneTag: String? = nil
    @State private var showScenePicker = false

    private var unit: MixingUnitEntry {
        mixer.session.units.first { $0.id == unitId } ?? mixer.selectedUnit
    }

    private var previewTitle: String {
        if let id = mixer.previewingSceneId(for: unitId),
           let scene = mixer.session.scenes.first(where: { $0.id == id }) {
            return "PREVIEW — \(scene.name)"
        }
        return "PREVIEW"
    }

    private var displayedMix: Float {
        if tbarLocked || tbarDragging || tbarLatching {
            return mix
        }
        return mixer.mixByUnit[unitId] ?? mix
    }

    private var programTitle: String {
        if let id = mixer.programmingSceneId(for: unitId),
           let scene = mixer.session.scenes.first(where: { $0.id == id }) {
            return "PROGRAM — \(scene.name)"
        }
        return "PROGRAM"
    }

    var body: some View {
        VStack(spacing: 8) {
            HStack {
                Spacer()
                Toggle(L10n.t("settings.alwaysOnTop"), isOn: Binding(
                    get: { unit.alwaysOnTop },
                    set: { mixer.setSwitcherAlwaysOnTop(unitId, $0) }
                ))
                .toggleStyle(.checkbox)
            }
            VSplitView {
                HSplitView {
                    bus(title: previewTitle, color: mixer.session.settings.previewColor, kind: EIVIZ_OUTPUT_PREVIEW)
                    transitions
                    bus(title: programTitle, color: mixer.session.settings.programColor, kind: EIVIZ_OUTPUT_PROGRAM)
                }
                .frame(minHeight: 160)
                scenes
                    .frame(minHeight: 80)
                overlays
                    .frame(minHeight: 60)
            }
        }
        .padding(8)
        .background(EivizTheme.background)
        .foregroundStyle(EivizTheme.text)
        .sheet(isPresented: $showScenePicker) {
            SwitcherScenesSheet(unitId: unitId)
                .environmentObject(mixer)
        }
    }

    private var transitions: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("Transitions").fontWeight(.bold)
            ScrollView {
                VStack(spacing: 4) {
                    ForEach(Array(unit.transitions.enumerated()), id: \.element.id) { index, preset in
                        HStack {
                            Text("\(preset.label)  \(preset.durationLabel)")
                                .font(.system(size: 12, weight: .semibold))
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .padding(4)
                                .overlay(
                                    Rectangle().stroke(
                                        index == tbarPresetIndex ? mixer.session.settings.previewColor.color : EivizTheme.stroke,
                                        lineWidth: 1
                                    )
                                )
                                .onTapGesture { tbarPresetIndex = index }
                            Button("TAKE") {
                                tbarPresetIndex = index
                                mixer.firePreset(preset, unitId: unitId)
                                tbarLocked = false
                            }
                            .buttonStyle(MixerButtonStyle())
                        }
                    }
                }
            }
            .frame(minHeight: 80)
            .frame(maxHeight: .infinity)
            Slider(
                value: Binding(
                    get: { Double(displayedMix) },
                    set: { setMix(Float($0)) }
                ),
                in: 0 ... 1,
                onEditingChanged: { editing in
                    tbarDragging = editing
                    if !editing { finishTBar() }
                }
            )
            .tint(mixer.session.settings.previewColor.color)
            .frame(maxWidth: .infinity)
            .padding(.horizontal, 8)
            .padding(.top, 12)
            .padding(.bottom, 16)
        }
        .padding(.horizontal, 12)
        .frame(minWidth: 180, idealWidth: 260)
    }

    private var visibleScenes: [SceneEntry] {
        mixer.session.scenes.filter { scene in
            unit.showsOnSwitcher(scene)
                && (sceneTag == nil || scene.tags.contains(sceneTag!))
        }
    }

    private var scenes: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Text(L10n.t("chrome.scenes")).fontWeight(.bold)
                Spacer()
                Button(L10n.t("switcher.manageScenes")) { showScenePicker = true }
                    .buttonStyle(MixerButtonStyle())
            }
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 6) {
                    tagTab(L10n.t("tag.all"), selected: sceneTag == nil) { sceneTag = nil }
                    ForEach(mixer.session.sceneTags, id: \.self) { tag in
                        tagTab(tag, selected: sceneTag == tag) { sceneTag = tag }
                    }
                }
            }
            ScrollView {
                LazyVGrid(columns: [GridItem(.adaptive(minimum: 148), spacing: 8)], alignment: .leading, spacing: 8) {
                    ForEach(visibleScenes) { scene in
                        switcherSceneThumb(scene)
                    }
                }
                .padding(4)
            }
            .background(EivizTheme.list)
        }
    }

    private func tagTab(_ title: String, selected: Bool, action: @escaping () -> Void) -> some View {
        Button(title, action: action)
            .buttonStyle(.plain)
            .font(.system(size: 11, weight: selected ? .semibold : .regular))
            .foregroundStyle(selected ? Color.white : Color.secondary)
            .padding(.bottom, 2)
            .overlay(alignment: .bottom) {
                Rectangle()
                    .fill(selected ? mixer.session.settings.previewColor.color : Color.clear)
                    .frame(height: 2)
            }
    }

    private var overlays: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Text(L10n.t("chrome.overlay")).fontWeight(.bold)
                Spacer()
                Button(L10n.t("chrome.overlay")) { mixer.openOverlay(for: unitId) }
                    .buttonStyle(MixerButtonStyle())
            }
            ScrollView(.horizontal, showsIndicators: true) {
                HStack(alignment: .bottom, spacing: 12) {
                    ForEach(unit.overlays) { slot in
                        Toggle(isOn: Binding(
                            get: {
                                mixer.session.units.first { $0.id == unitId }?
                                    .overlays.first { $0.id == slot.id }?.enabled ?? slot.enabled
                            },
                            set: { mixer.setOverlayEnabled(slot.id, enabled: $0, unitId: unitId) }
                        )) {
                            Text(overlayName(slot))
                        }
                        .toggleStyle(.checkbox)
                    }
                }
            }
        }
    }

    private func overlayName(_ slot: OverlaySlot) -> String {
        if slot.sourceKind == .input {
            return mixer.session.inputs.first { $0.id == slot.sceneGpuId }?.name ?? "Input"
        }
        return mixer.session.scenes.first { $0.gpuId == slot.sceneGpuId }?.name ?? "Scene"
    }

    private func switcherSceneThumb(_ scene: SceneEntry) -> some View {
        let preview = isPreviewing(scene)
        let program = isProgramming(scene)
        return SwitcherSceneThumb(
            scene: scene,
            preview: preview,
            program: program,
            interval: mixer.session.settings.resolvedPresentInterval,
            previewColor: mixer.session.settings.previewColor.color,
            programColor: mixer.session.settings.programColor.color,
            inactiveColor: mixer.session.settings.inactiveColor.color,
            onPreview: { mixer.previewScene(scene, unitId: unitId) },
            onCollapse: { mixer.toggleSceneCollapsed(scene.id) },
            onHide: { mixer.hideSceneOnSwitcher(unitId, scene.id) },
            onEdit: { mixer.openSceneEditor(scene) }
        )
    }

    private func isPreviewing(_ scene: SceneEntry) -> Bool {
        mixer.previewingSceneId(for: unitId) == scene.id
    }

    private func isProgramming(_ scene: SceneEntry) -> Bool {
        mixer.programmingSceneId(for: unitId) == scene.id
    }

    private func bus(title: String, color: RgbColor, kind: UInt32) -> some View {
        VStack(spacing: 0) {
            Text(title)
                .font(.system(size: 13, weight: .bold))
                .foregroundStyle(color.headerForeground)
                .frame(maxWidth: .infinity)
                .padding(.vertical, 6)
                .background(color.color)
            MetalPreviewRepresentable(role: .unit(unitId: unitId, kind: kind))
                .frame(minWidth: 320, minHeight: 180)
        }
        .aspectRatio(
            CGFloat(unit.width) / max(1, CGFloat(unit.height)),
            contentMode: .fit
        )
        .background(Rectangle().stroke(color.color, lineWidth: 2))
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private func setMix(_ value: Float) {
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
            mixer.takeCut(unitId: unitId)
            return
        }
        mix = value
        mixer.setMix(value, unitId: unitId)
    }

    private func finishTBar() {
        guard tbarLocked else { return }
        tbarLatching = true
        mix = 0
        tbarLatching = false
        tbarLocked = false
    }
}

private struct SwitcherSceneThumb: View {
    let scene: SceneEntry
    let preview: Bool
    let program: Bool
    let interval: UInt32
    let previewColor: Color
    let programColor: Color
    let inactiveColor: Color
    let onPreview: () -> Void
    let onCollapse: () -> Void
    let onHide: () -> Void
    let onEdit: () -> Void

    @State private var appeared = false

    private var wanted: Bool { !scene.previewCollapsed && (preview || program || appeared) }

    var body: some View {
        VStack(spacing: 0) {
            Text(scene.name)
                .font(.system(size: 11, weight: .semibold))
                .lineLimit(1)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 6)
                .padding(.vertical, 3)
                .background(EivizTheme.chrome)
                .onTapGesture(count: 2, perform: onEdit)
                .onTapGesture(perform: onPreview)
                .overlay(RightClickCatcher(action: onCollapse))
            if !scene.previewCollapsed {
                ThumbRepresentable(
                    sourceId: scene.gpuId,
                    width: 142,
                    height: 80,
                    interval: interval,
                    wanted: wanted,
                    onClick: onPreview
                )
                .frame(width: 142, height: 80)
                .background(Color.black)
            }
        }
        .frame(width: 148)
        .background(rowFill)
        .overlay(Rectangle().stroke(rowStroke, lineWidth: 2))
        .contentShape(Rectangle())
        .onTapGesture(perform: onPreview)
        .contextMenu {
            Button(L10n.t("switcher.hideHere"), action: onHide)
        }
        .onAppear { appeared = true }
        .onDisappear { appeared = false }
    }

    private var rowFill: Color {
        if program { return programColor.opacity(0.28) }
        if preview { return previewColor.opacity(0.22) }
        return Color.clear
    }

    private var rowStroke: Color {
        if program { return programColor }
        if preview { return previewColor }
        return inactiveColor
    }
}

final class SwitcherHostWindow: NSWindow {}

private struct SwitcherScenesSheet: View {
    @EnvironmentObject private var mixer: MixerController
    @Environment(\.dismiss) private var dismiss
    let unitId: UInt64
    @State private var filter: SwitcherSceneFilter = .all
    @State private var selected: Set<UInt64> = []

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(L10n.t("switcher.manageScenes")).fontWeight(.bold)
            Picker("", selection: $filter) {
                Text(L10n.t("switcher.allScenes")).tag(SwitcherSceneFilter.all)
                Text(L10n.t("switcher.onlyThese")).tag(SwitcherSceneFilter.include)
                Text(L10n.t("switcher.hideThese")).tag(SwitcherSceneFilter.exclude)
            }
            .pickerStyle(.radioGroup)
            .labelsHidden()
            List(mixer.session.scenes) { scene in
                Toggle(isOn: Binding(
                    get: { selected.contains(scene.id) },
                    set: { on in
                        if on { selected.insert(scene.id) } else { selected.remove(scene.id) }
                    }
                )) {
                    Text(scene.name)
                }
                .disabled(filter == .all)
            }
            .frame(minHeight: 220)
            HStack {
                Spacer()
                Button(L10n.t("dialog.ok")) {
                    mixer.setSwitcherSceneFilter(
                        unitId,
                        filter,
                        ids: filter == .all ? [] : Array(selected)
                    )
                    dismiss()
                }
                Button(L10n.t("dialog.cancel")) { dismiss() }
            }
        }
        .padding(16)
        .frame(width: 360, height: 420)
        .background(EivizTheme.dialog)
        .foregroundStyle(EivizTheme.text)
        .onAppear {
            guard let unit = mixer.session.units.first(where: { $0.id == unitId }) else { return }
            filter = unit.switcherSceneFilter
            selected = Set(unit.switcherSceneIds)
        }
    }
}

