import SwiftUI

/// Stream destinations beside the deck, like the Windows strip: a name, an
/// on/off switch and a lamp showing whether the destination is publishing.
///
/// The settings page only edits the document; starting and stopping a live
/// stream belongs here, so the operator does not have to open the settings
/// sheet during a show.
struct StreamStripsView: View {
    @EnvironmentObject private var mixer: MixerController
    @State private var onAirLock = OnAirLock.active

    private var streams: [OutputEntry] {
        mixer.session.outputs.filter { $0.transport == .rtmp }
    }

    var body: some View {
        HStack(alignment: .bottom, spacing: 12) {
            ForEach(streams) { output in
                HStack(spacing: 6) {
                    lamp(output)
                    Toggle(isOn: Binding(
                        get: { output.enabled },
                        set: { enabled in
                            mixer.setStreamEnabled(output, enabled: enabled)
                        }
                    )) {
                        Text(output.name.isEmpty ? L10n.t("chrome.stream") : output.name)
                    }
                    .toggleStyle(OnOffToggleStyle())
                    .disabled(onAirLock)
                    .help(L10n.t("chrome.stream"))
                }
            }
        }
        .onReceive(NotificationCenter.default.publisher(for: OnAirLock.changed)) { _ in
            onAirLock = OnAirLock.active
        }
    }

    private func lamp(_ output: OutputEntry) -> some View {
        let state = lampState(output)
        return Circle()
            .fill(state.color)
            .frame(width: 8, height: 8)
            .help(state.help)
    }

    private func lampState(_ output: OutputEntry) -> (color: Color, help: String) {
        guard output.enabled else {
            return (Color(white: 0.30), L10n.t("chrome.stream"))
        }
        guard let state = mixer.streamStats[output.id] else {
            return (amber, L10n.t("status.streamConnecting"))
        }
        if state.planLimited {
            return (red, state.message)
        }
        if state.connected {
            let encoder = mixer.session.streamEncoders.first { $0.id == output.streamEncoderId }
            let target = encoder.map { $0.fpsDen > 0 ? Double($0.fpsNum) / Double($0.fpsDen) : 0 } ?? 0
            let fps = Double(state.fpsMilli) / 1000
            if state.lagMs >= 400 || (target > 0 && fps * 100 < target * 85) {
                let slow = L10n.format(
                    "status.streamSlow",
                    String(format: "%.1f", fps),
                    String(format: "%.1f", target),
                    "\(state.lagMs)",
                    "\(state.dropped)"
                )
                // The encoder's own reason (for example VideoToolbox being held by
                // another input) is what tells the operator what to change.
                return (amber, state.message.isEmpty ? slow : "\(slow)\n\(state.message)")
            }
            let detail = "\(Int(Double(state.fpsMilli) / 1000)) fps / \(state.bitrate / 1000) kbps / frames \(state.frames)"
            return (green, state.message.isEmpty ? detail : "\(detail)\n\(state.message)")
        }
        // A destination takes a moment to connect; only call it a fault after
        // three seconds or when the core reports a reason.
        if !state.message.isEmpty {
            return (red, state.message)
        }
        return (amber, L10n.t("status.streamConnecting"))
    }

    private var green: Color { Color(red: 125 / 255, green: 200 / 255, blue: 125 / 255) }
    private var amber: Color { Color(red: 224 / 255, green: 195 / 255, blue: 106 / 255) }
    private var red: Color { Color(red: 224 / 255, green: 120 / 255, blue: 120 / 255) }
}
