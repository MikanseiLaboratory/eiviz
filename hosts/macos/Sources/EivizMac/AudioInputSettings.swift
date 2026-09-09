import AppKit
import SwiftUI

enum AudioMeter {
    static func post(left: Float, right: Float, gain: Float, mute: Bool) -> (Float, Float) {
        if mute {
            return (0, 0)
        }
        let scale = max(0, gain)
        return (left * scale, right * scale)
    }

    static func height(_ linear: Float, maxHeight: CGFloat = 80) -> CGFloat {
        if linear <= 1e-5 {
            return 4
        }
        let db = 20 * log10(linear)
        return 4 + CGFloat(max(0, min(1, (db + 60) / 60))) * maxHeight
    }

    static func slider(fromGain gain: Float) -> Double {
        if gain <= 1e-6 {
            return 0
        }
        let db = 20 * log10(gain)
        return min(1, max(0, Double((db + 60) / 72)))
    }

    static func gain(fromSlider slider: Double) -> Float {
        if slider <= 0.001 {
            return 0
        }
        return pow(10, Float(slider * 72 - 60) / 20)
    }
}

struct AudioInputSettingsView: View {
    let inputId: UInt64
    @ObservedObject var mixer: MixerController

    var body: some View {
        if let input = mixer.session.inputs.first(where: { $0.id == inputId }) {
            let pre = mixer.peaks[input.id] ?? (0, 0)
            let post = AudioMeter.post(left: pre.0, right: pre.1, gain: input.gain, mute: input.mute)
            VStack(alignment: .leading, spacing: 12) {
                Text(input.listLabel(in: mixer.session, localFiles: !mixer.isRemote))
                    .font(.headline)
                HStack(alignment: .bottom, spacing: 20) {
                    meterColumn(title: L10n.t("audio.pre"), left: pre.0, right: pre.1)
                    meterColumn(title: L10n.t("audio.post"), left: post.0, right: post.1)
                    VStack(alignment: .leading, spacing: 8) {
                        Slider(
                            value: Binding(
                                get: { AudioMeter.slider(fromGain: input.gain) },
                                set: { mixer.applyInputAudio(id: input.id, mask: mixer.audioMask(input), gain: AudioMeter.gain(fromSlider: $0), mute: input.mute) }
                            )
                        )
                        .frame(width: 140)
                        Text(gainLabel(input.gain))
                            .font(.system(size: 11))
                            .foregroundStyle(.secondary)
                        Toggle(isOn: Binding(
                            get: { input.mute },
                            set: { mixer.applyInputAudio(id: input.id, mask: mixer.audioMask(input), gain: input.gain, mute: $0) }
                        )) {
                            Text("Mute")
                        }
                        .toggleStyle(.checkbox)
                    }
                }
                if input.kind != .mix {
                    HStack(spacing: 6) {
                        ForEach(mixer.session.buses) { bus in
                            let bit = UInt32(1) << bus.bit
                            Toggle(isOn: Binding(
                                get: { (mixer.audioMask(input) & bit) != 0 },
                                set: { on in
                                    var mask = mixer.audioMask(input)
                                    if on {
                                        mask |= bit
                                    } else {
                                        mask &= ~bit
                                    }
                                    mixer.applyInputAudio(id: input.id, mask: mask == 0 ? 1 : mask, gain: input.gain, mute: input.mute)
                                }
                            )) {
                                Text(busChip(bus))
                            }
                            .toggleStyle(.checkbox)
                        }
                    }
                }
            }
            .padding(16)
            .frame(minWidth: 360, minHeight: 200)
            .background(EivizTheme.panel)
            .foregroundStyle(EivizTheme.text)
        } else {
            Text(L10n.t("audio.settings"))
                .padding(16)
        }
    }

    private func meterColumn(title: String, left: Float, right: Float) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(title).font(.system(size: 11, weight: .semibold))
            HStack(alignment: .bottom, spacing: 3) {
                Rectangle().fill(EivizTheme.status).frame(width: 10, height: AudioMeter.height(left))
                Rectangle().fill(EivizTheme.status).frame(width: 10, height: AudioMeter.height(right))
            }
            .frame(height: 84, alignment: .bottom)
        }
    }

    private func gainLabel(_ gain: Float) -> String {
        if gain <= 1e-5 {
            return "−∞ dB"
        }
        return String(format: "%.0f dB", 20 * log10(gain))
    }

    private func busChip(_ bus: AudioBusEntry) -> String {
        if bus.role == .master {
            return "M"
        }
        if bus.role == .headphone {
            return "H"
        }
        if bus.name.hasPrefix("Bus "), bus.name.count > 4 {
            return String(bus.name.suffix(1))
        }
        return bus.name.isEmpty ? "?" : String(bus.name.prefix(1))
    }
}
