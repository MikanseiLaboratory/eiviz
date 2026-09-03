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
            HStack(spacing: 16) {
                bus(title: previewTitle, color: mixer.session.settings.previewColor, kind: EIVIZ_OUTPUT_PREVIEW)
                transitions
                bus(title: programTitle, color: mixer.session.settings.programColor, kind: EIVIZ_OUTPUT_PROGRAM)
            }
            .frame(maxHeight: .infinity)
            scenes
        }
        .padding(8)
        .background(EivizTheme.background)
        .foregroundStyle(EivizTheme.text)
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
        .frame(width: 260)
    }

    private var scenes: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("Scenes").fontWeight(.bold)
            ScrollView(.horizontal, showsIndicators: true) {
                HStack(alignment: .top, spacing: 8) {
                    ForEach(mixer.session.scenes) { scene in
                        switcherSceneThumb(scene)
                    }
                }
                .padding(4)
            }
            .background(EivizTheme.list)
        }
        .frame(height: 200)
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
            onPreview: { mixer.previewScene(scene, unitId: unitId) }
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

    @State private var appeared = false

    private var wanted: Bool { preview || program || appeared }

    var body: some View {
        VStack(spacing: 4) {
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
            Text(scene.name)
                .font(.system(size: 11))
                .lineLimit(1)
                .frame(width: 142)
        }
        .padding(4)
        .background(rowFill)
        .overlay(Rectangle().stroke(rowStroke, lineWidth: 2))
        .contentShape(Rectangle())
        .onTapGesture(perform: onPreview)
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
