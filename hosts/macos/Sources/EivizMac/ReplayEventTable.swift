import SwiftUI

/// The event list, laid out like the Windows deck: IN, OUT, duration, name, actions.
struct ReplayEventTable: View {
    let events: [ReplayEventInfo]
    let fps: (num: UInt32, den: UInt32)
    let rename: (UInt64, String) -> Void
    let protect: (UInt64, Bool) -> Void
    let setRange: (UInt64, UInt64, UInt64) -> Void
    let addToPlaylist: (ReplayEventInfo) -> Void
    let remove: (UInt64) -> Void

    var body: some View {
        Table(events) {
            TableColumn(L10n.t("replay.in")) { event in
                TimecodeField(frames: event.inFrame, fps: fps) { value in
                    setRange(event.id, value, event.outFrame)
                }
            }
            .width(96)

            TableColumn(L10n.t("replay.out")) { event in
                TimecodeField(frames: event.outFrame, fps: fps) { value in
                    setRange(event.id, event.inFrame, value)
                }
            }
            .width(96)

            TableColumn(L10n.t("replay.duration")) { event in
                Text(ReplayTime.duration(event.inFrame, event.outFrame, fps))
                    .font(.system(.caption, design: .monospaced))
            }
            .width(76)

            TableColumn(L10n.t("replay.name")) { event in
                TextField("", text: Binding(
                    get: { event.name },
                    set: { rename(event.id, $0) }
                ))
                .textFieldStyle(.roundedBorder)
            }
            .width(min: 140, ideal: 200)

            TableColumn("") { event in
                HStack(spacing: 4) {
                    Toggle("", isOn: Binding(
                        get: { event.protected },
                        set: { protect(event.id, $0) }
                    ))
                    .labelsHidden()
                    .toggleStyle(.checkbox)
                    .help(L10n.t("replay.protectHint"))
                    Button {
                        addToPlaylist(event)
                    } label: {
                        Image(systemName: "text.badge.plus")
                    }
                    .help(L10n.t("replay.cueHint"))
                    Button {
                        remove(event.id)
                    } label: {
                        Image(systemName: "trash")
                    }
                    .help(L10n.t("replay.delete"))
                }
            }
            .width(110)
        }
        .tableStyle(.inset(alternatesRowBackgrounds: false))
    }
}


/// An inline timecode cell. The text is local while it is being typed so the table does
/// not rewrite every keystroke; Enter commits the parsed frame.
private struct TimecodeField: View {
    let frames: UInt64
    let fps: (num: UInt32, den: UInt32)
    let commit: (UInt64) -> Void

    @State private var text = ""

    var body: some View {
        TextField("", text: $text)
            .font(.system(.caption, design: .monospaced))
            .textFieldStyle(.roundedBorder)
            .onAppear { text = ReplayTime.timecode(frames, fps) }
            .onChange(of: frames) { _, next in
                text = ReplayTime.timecode(next, fps)
            }
            .onSubmit {
                if let value = Self.parse(text, fps: fps) {
                    commit(value)
                } else {
                    text = ReplayTime.timecode(frames, fps)
                }
            }
    }

    private static func parse(_ text: String, fps: (num: UInt32, den: UInt32)) -> UInt64? {
        let parts = text.split(separator: ":")
        guard parts.count == 4,
              let hours = UInt64(parts[0]),
              let minutes = UInt64(parts[1]),
              let seconds = UInt64(parts[2]),
              let frames = UInt64(parts[3])
        else { return nil }
        let rate = UInt64(ReplayTime.rate(fps).rounded())
        guard rate > 0, frames < rate else { return nil }
        return ((hours * 3600 + minutes * 60 + seconds) * rate) + frames
    }
}
