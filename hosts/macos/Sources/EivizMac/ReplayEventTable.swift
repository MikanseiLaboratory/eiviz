import SwiftUI

/// The event list, laid out like the Windows deck: IN, OUT, duration, name, actions.
struct ReplayEventTable: View {
    let events: [ReplayEventInfo]
    let fps: (num: UInt32, den: UInt32)
    let rename: (UInt64, String) -> Void
    let protect: (UInt64, Bool) -> Void
    let addToPlaylist: (ReplayEventInfo) -> Void
    let remove: (UInt64) -> Void

    var body: some View {
        Table(events) {
            TableColumn(L10n.t("replay.in")) { event in
                Text(ReplayTime.timecode(event.inFrame, fps))
                    .font(.system(.caption, design: .monospaced))
            }
            .width(96)

            TableColumn(L10n.t("replay.out")) { event in
                Text(ReplayTime.timecode(event.outFrame, fps))
                    .font(.system(.caption, design: .monospaced))
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
