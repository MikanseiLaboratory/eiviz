import SwiftUI

/// The playlist table with the Windows deck's columns: marker, IN, OUT, duration, angle,
/// speed, transition, audio and row actions.
struct ReplayClipTableWindows: View {
    let clips: [ReplayClipInfo]
    let angles: [ReplayAngleInfo]
    let fps: (num: UInt32, den: UInt32)
    let handlers: ReplayClipTable.Handlers
    @Binding var selection: String?

    @State private var tagUid: String?
    @State private var tagText = ""

    var body: some View {
        Table(clips, selection: $selection) {
            TableColumn("") { _ in
                Text("")
                    .frame(width: 22)
            }
            .width(22)

            TableColumn(L10n.t("replay.in")) { clip in
                Text(ReplayTime.timecode(clip.inFrame, fps))
                    .font(.system(.caption, design: .monospaced))
            }
            .width(92)

            TableColumn(L10n.t("replay.out")) { clip in
                Text(handlers.outText(clip))
                    .font(.system(.caption, design: .monospaced))
            }
            .width(92)

            TableColumn(L10n.t("replay.duration")) { clip in
                Text(ReplayTime.duration(clip.inFrame, clip.outFrame, fps))
                    .font(.system(.caption, design: .monospaced))
            }
            .width(74)

            TableColumn(L10n.t("replay.angle")) { clip in
                Picker("", selection: Binding(
                    get: { clip.angleId },
                    set: { handlers.angle(clip.uid, $0) }
                )) {
                    ForEach(angles) { angle in
                        Text(angle.label).tag(angle.id)
                    }
                }
                .labelsHidden()
            }
            .width(118)

            TableColumn(L10n.t("replay.speed")) { clip in
                Picker("", selection: Binding(
                    get: { Int32((clip.speed * 1000).rounded()) },
                    set: { handlers.speed(clip.uid, $0) }
                )) {
                    Text("25%").tag(Int32(250))
                    Text("50%").tag(Int32(500))
                    Text("75%").tag(Int32(750))
                    Text("100%").tag(Int32(1000))
                    Text("150%").tag(Int32(1500))
                    Text("200%").tag(Int32(2000))
                }
                .labelsHidden()
            }
            .width(72)

            TableColumn(L10n.t("replay.transition")) { clip in
                HStack(spacing: 4) {
                    Picker("", selection: Binding(
                        get: { clip.transition.kind },
                        set: { handlers.transitionKind(clip.uid, $0) }
                    )) {
                        Text(L10n.t("replay.transitionCut")).tag("Cut")
                        Text(L10n.t("replay.transitionDissolve")).tag("Dissolve")
                        Text(L10n.t("replay.transitionDip")).tag("Dip")
                        Text(L10n.t("replay.transitionWipe")).tag("Wipe")
                        Text(L10n.t("replay.transitionSlide")).tag("Slide")
                        Text(L10n.t("replay.transitionPush")).tag("Push")
                    }
                    .labelsHidden()
                    .frame(width: 100)
                    Picker("", selection: Binding(
                        get: { clip.transition.direction },
                        set: { handlers.transitionDirection(clip.uid, $0) }
                    )) {
                        Text(L10n.t("replay.directionLeft")).tag("Left")
                        Text(L10n.t("replay.directionRight")).tag("Right")
                        Text(L10n.t("replay.directionUp")).tag("Up")
                        Text(L10n.t("replay.directionDown")).tag("Down")
                    }
                    .labelsHidden()
                    .frame(width: 74)
                    Picker("", selection: Binding(
                        get: { clip.transition.timing },
                        set: { handlers.transitionTiming(clip.uid, $0) }
                    )) {
                        Text(L10n.t("replay.timingOverwrap")).tag("Overwrap")
                        Text(L10n.t("replay.timingAdditive")).tag("Additive")
                    }
                    .labelsHidden()
                    .frame(width: 86)
                    TextField("", value: Binding(
                        get: { clip.transition.durationFrames },
                        set: { handlers.transitionFrames(clip.uid, $0) }
                    ), format: .number)
                    .frame(width: 42)
                    Text(L10n.t("replay.framesShort"))
                        .font(.system(size: 11))
                        .foregroundStyle(Color(white: 0.54))
                }
            }
            .width(330)

            TableColumn(L10n.t("replay.audioShort")) { clip in
                Picker("", selection: Binding(
                    get: {
                        clip.audioKind == "Angle" ? "Angle:\(clip.audioAngleId)" : clip.audioKind
                    },
                    set: { handlers.audio(clip.uid, $0) }
                )) {
                    Text(L10n.t("replay.audioFollow")).tag("FollowAngle")
                    Text(L10n.t("replay.audioSilent")).tag("Silent")
                    ForEach(angles) { angle in
                        Text(angle.label).tag("Angle:\(angle.id)")
                    }
                }
                .labelsHidden()
            }
            .width(70)

            TableColumn("") { clip in
                HStack(spacing: 2) {
                    Button {
                        handlers.cue(clip)
                    } label: {
                        Image(systemName: "play.fill")
                    }
                    .help(L10n.t("replay.cueHint"))
                    Button {
                        tagUid = clip.uid
                        tagText = clip.tags.joined(separator: ", ")
                    } label: {
                        Image(systemName: "tag")
                    }
                    .help(L10n.t("tag.add"))
                    Button {
                        handlers.remove(clip.uid)
                    } label: {
                        Image(systemName: "trash")
                    }
                    .help(L10n.t("replay.remove"))
                }
            }
            .width(86)
        }
        .tableStyle(.inset(alternatesRowBackgrounds: false))
        .popover(item: Binding(
            get: { tagUid.map { TagTarget(uid: $0) } },
            set: { tagUid = $0?.uid }
        )) { target in
            VStack(alignment: .leading, spacing: 6) {
                Text(L10n.t("tag.add")).fontWeight(.bold)
                TextField("", text: $tagText)
                    .frame(width: 220)
                HStack {
                    Spacer()
                    Button(L10n.t("dialog.cancel")) { tagUid = nil }
                    Button(L10n.t("dialog.ok")) {
                        handlers.tags(target.uid, tagText)
                        tagUid = nil
                    }
                }
            }
            .padding(12)
        }
    }

    private struct TagTarget: Identifiable {
        let uid: String
        var id: String { uid }
    }
}
