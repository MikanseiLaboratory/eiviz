import AppKit
import EivizMixer
import SwiftUI

struct SwitcherView: View {
    @EnvironmentObject private var mixer: MixerController
    let unitId: UInt64
    @State private var mix: Float = 0
    @State private var tbarLocked = false
    @State private var tbarLatching = false
    @State private var tbarPresetIndex = 0

    private var unit: MixingUnitEntry {
        mixer.session.units.first { $0.id == unitId } ?? mixer.selectedUnit
    }

    var body: some View {
        VStack(spacing: 8) {
            HStack(spacing: 8) {
                bus(title: "PREVIEW", color: EivizTheme.preview, kind: EIVIZ_OUTPUT_PREVIEW)
                transitions
                bus(title: "PROGRAM", color: EivizTheme.program, kind: EIVIZ_OUTPUT_PROGRAM)
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
                            Text("\(preset.label)  \(preset.durationFrames)f")
                                .font(.system(size: 12, weight: .semibold))
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .padding(4)
                                .overlay(
                                    Rectangle().stroke(
                                        index == tbarPresetIndex ? EivizTheme.preview : EivizTheme.stroke,
                                        lineWidth: 1
                                    )
                                )
                                .onTapGesture { tbarPresetIndex = index }
                            Button("TAKE") {
                                tbarPresetIndex = index
                                mixer.firePreset(preset, unitId: unitId)
                                mix = 0
                                tbarLocked = false
                            }
                            .buttonStyle(MixerButtonStyle())
                        }
                    }
                }
            }
            Slider(
                value: Binding(
                    get: { Double(mix) },
                    set: { setMix(Float($0)) }
                ),
                in: 0 ... 1,
                onEditingChanged: { editing in
                    if !editing { finishTBar() }
                }
            )
            .tint(EivizTheme.preview)
            .frame(width: 132)
            .frame(maxWidth: .infinity)
        }
        .frame(width: 168)
    }

    private var scenes: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("Scenes").fontWeight(.bold)
            ScrollView {
                VStack(spacing: 2) {
                    ForEach(mixer.session.scenes) { scene in
                        sceneRow(scene)
                    }
                }
                .padding(4)
            }
            .background(EivizTheme.list)
        }
        .frame(height: 160)
    }

    private func sceneRow(_ scene: SceneEntry) -> some View {
        let preview = isPreviewing(scene)
        let program = isProgramming(scene)
        return HStack(spacing: 6) {
            Text(scene.name)
                .frame(maxWidth: .infinity, alignment: .leading)
            if program {
                tally("PGM", EivizTheme.program)
            }
            if preview {
                tally("PRV", EivizTheme.preview)
            }
        }
        .padding(.vertical, 3)
        .padding(.horizontal, 6)
        .background(rowFill(preview: preview, program: program))
        .overlay(
            Rectangle().stroke(rowStroke(preview: preview, program: program), lineWidth: 1)
        )
        .contentShape(Rectangle())
        .onTapGesture {
            mixer.previewScene(scene, unitId: unitId)
        }
    }

    private func tally(_ title: String, _ color: Color) -> some View {
        Text(title)
            .font(.system(size: 10, weight: .bold))
            .foregroundStyle(title == "PRV" ? Color(red: 0.07, green: 0.07, blue: 0.07) : Color.white)
            .padding(.horizontal, 5)
            .padding(.vertical, 1)
            .background(color)
    }

    private func rowFill(preview: Bool, program: Bool) -> Color {
        if program { return EivizTheme.program.opacity(0.28) }
        if preview { return EivizTheme.preview.opacity(0.22) }
        return Color.clear
    }

    private func rowStroke(preview: Bool, program: Bool) -> Color {
        if preview { return EivizTheme.preview }
        if program { return EivizTheme.program }
        return Color.clear
    }

    private func isPreviewing(_ scene: SceneEntry) -> Bool {
        mixer.previewingSceneId(for: unitId) == scene.id
    }

    private func isProgramming(_ scene: SceneEntry) -> Bool {
        mixer.programmingSceneId(for: unitId) == scene.id
    }

    private func bus(title: String, color: Color, kind: UInt32) -> some View {
        VStack(spacing: 0) {
            Text(title)
                .font(.system(size: 13, weight: .bold))
                .foregroundStyle(title == "PREVIEW" ? Color(red: 0.07, green: 0.07, blue: 0.07) : Color.white)
                .frame(maxWidth: .infinity)
                .padding(.vertical, 6)
                .background(color)
            MetalPreviewRepresentable(role: .unit(unitId: unitId, kind: kind))
                .frame(minWidth: 320, minHeight: 180)
                .background(Color.black)
        }
        .aspectRatio(16.0 / 9.0, contentMode: .fit)
        .clipped()
        .overlay(Rectangle().stroke(color, lineWidth: 2))
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

final class SwitcherHostWindow: NSWindow {}
