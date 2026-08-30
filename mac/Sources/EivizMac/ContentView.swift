import EivizMixer
import SwiftUI

struct ContentView: View {
    @EnvironmentObject private var mixer: MixerSession

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 12) {
                bus(title: "PREVIEW", color: Color(red: 232 / 255, green: 119 / 255, blue: 34 / 255), kind: EIVIZ_OUTPUT_PREVIEW)
                controls
                bus(title: "PROGRAM", color: Color(red: 46 / 255, green: 125 / 255, blue: 50 / 255), kind: EIVIZ_OUTPUT_PROGRAM)
            }
            .padding(12)
            HStack {
                Text(mixer.status)
                    .foregroundStyle(Color(red: 124 / 255, green: 252 / 255, blue: 124 / 255))
                Spacer()
                if !mixer.errorText.isEmpty {
                    Text(mixer.errorText)
                        .foregroundStyle(Color(red: 1, green: 183 / 255, blue: 77 / 255))
                }
            }
            .font(.system(size: 12))
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .background(Color(red: 17 / 255, green: 17 / 255, blue: 17 / 255))
        }
        .background(Color(red: 26 / 255, green: 26 / 255, blue: 26 / 255))
    }

    private var controls: some View {
        VStack(spacing: 12) {
            Text("Transitions")
                .font(.headline)
                .foregroundStyle(Color(white: 0.93))
            Button("CUT") { mixer.cut() }
                .buttonStyle(MixerButtonStyle())
            Button("AUTO") { mixer.auto() }
                .buttonStyle(MixerButtonStyle())
            Slider(
                value: Binding(
                    get: { Double(mixer.mix) },
                    set: { mixer.setMix(Float($0)) }
                ),
                in: 0 ... 1,
                onEditingChanged: { editing in
                    if !editing {
                        mixer.finishTBar()
                    }
                }
            )
            .tint(Color(red: 232 / 255, green: 119 / 255, blue: 34 / 255))
            VStack(alignment: .leading, spacing: 8) {
                Text("Scenes")
                    .font(.headline)
                    .foregroundStyle(Color(white: 0.93))
                Button("Scene 1  Bars") { mixer.preview(MixerSession.sceneBars) }
                    .buttonStyle(MixerButtonStyle())
                Button("Scene 2  Color") { mixer.preview(MixerSession.sceneColor) }
                    .buttonStyle(MixerButtonStyle())
            }
            Spacer()
        }
        .frame(width: 168)
        .padding(.vertical, 8)
    }

    private func bus(title: String, color: Color, kind: UInt32) -> some View {
        VStack(spacing: 0) {
            Text(title)
                .font(.system(size: 13, weight: .bold))
                .foregroundStyle(title == "PREVIEW" ? Color(red: 0.07, green: 0.07, blue: 0.07) : Color.white)
                .frame(maxWidth: .infinity)
                .padding(.vertical, 6)
                .background(color)
            MetalPreviewRepresentable(kind: kind)
                .frame(minWidth: 320, minHeight: 180)
                .background(Color.black)
        }
        .overlay(
            Rectangle()
                .stroke(color, lineWidth: 2)
        )
    }
}

private struct MixerButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .foregroundStyle(Color(white: 0.93))
            .frame(maxWidth: .infinity)
            .padding(.vertical, 8)
            .background(Color(red: 58 / 255, green: 58 / 255, blue: 58 / 255))
            .overlay(
                Rectangle()
                    .stroke(Color(white: 0.33), lineWidth: 1)
            )
            .opacity(configuration.isPressed ? 0.7 : 1)
    }
}
