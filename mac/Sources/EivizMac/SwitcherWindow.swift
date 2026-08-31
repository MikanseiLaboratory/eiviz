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
            List(mixer.session.scenes) { scene in
                Text(scene.name)
                    .contentShape(Rectangle())
                    .onTapGesture { mixer.previewScene(scene, unitId: unitId) }
            }
            .scrollContentBackground(.hidden)
            .background(EivizTheme.list)
        }
        .frame(height: 160)
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
