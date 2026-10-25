import SwiftUI

/// The playlist's clip table: every clip on one line, and every setting an operator changes
/// — angle, speed, transition, direction, timing, length, gain, pitch, audio and tags — a
/// cell of that line.
///
/// Each cell writes the field it owns through the handler it is given, so nothing here is
/// "select a clip, then use the controls somewhere else". The table took two hundred lines of
/// the deck's body; keeping it apart means the deck says what a clip edit means, not how a
/// column is laid out.
///
/// The columns are built inline rather than as `some View` properties: `TableColumn` is not a
/// `View`, and the table's column builder only accepts it written in place. Returning one from
/// a computed property type-erases it into `some View` and the builder rejects it.
struct ReplayClipTable: View {
    let clips: [ReplayClipInfo]
    let angles: [ReplayAngleInfo]
    let fps: (num: UInt32, den: UInt32)
    let handlers: Handlers

    /// Everything the table can ask the deck to do. Each one writes one field of one clip.
    struct Handlers {
        /// The clip's out point as the operator should read it: a timecode, plus a note when
        /// the frames it names have been overwritten by newer recording.
        var outText: (ReplayClipInfo) -> String
        var angle: (String, UInt32) -> Void
        var speed: (String, Int32) -> Void
        /// Kind, timing and length, whichever the operator changed; nil means "keep it".
        var transitionKind: (String, String) -> Void
        var transitionDirection: (String, String) -> Void
        var transitionTiming: (String, String) -> Void
        var transitionFrames: (String, UInt32) -> Void
        var gain: (String, Double) -> Void
        var preservePitch: (String, Bool) -> Void
        /// `FollowAngle`, `Silent`, or `Angle:<id>`.
        var audio: (String, String) -> Void
        var tags: (String, String) -> Void
        var cue: (ReplayClipInfo) -> Void
        var remove: (String) -> Void
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Table(clips) {
                TableColumn(L10n.t("replay.in")) { clip in
                    Text(ReplayTime.timecode(clip.inFrame, fps))
                        .font(.system(.caption, design: .monospaced))
                }
                .width(88)

                TableColumn(L10n.t("replay.out")) { clip in
                    Text(handlers.outText(clip))
                        .font(.system(.caption, design: .monospaced))
                }
                .width(88)

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
                .width(130)

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
                .width(84)

                // One column for the transition: kind, direction, timing and length belong
                // together, and a table column builder takes at most ten.
                TableColumn(L10n.t("replay.transition")) { clip in
                    HStack(spacing: 4) {
                        Picker("", selection: Binding(
                            get: { clip.transition.kind },
                            set: { handlers.transitionKind(clip.uid, $0) }
                        )) {
                            // The mixing unit's names, limited to the ones the replay path can
                            // draw: it composes two decoded frames, where the rest are warps.
                            Text(L10n.t("replay.transitionCut")).tag("Cut")
                            Text(L10n.t("replay.transitionDissolve")).tag("Dissolve")
                            Text(L10n.t("replay.transitionDip")).tag("Dip")
                            Text(L10n.t("replay.transitionWipe")).tag("Wipe")
                            Text(L10n.t("replay.transitionSlide")).tag("Slide")
                            Text(L10n.t("replay.transitionPush")).tag("Push")
                        }
                        .labelsHidden()
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
                        .help(L10n.t("replay.directionHelp"))
                        Picker("", selection: Binding(
                            get: { clip.transition.timing },
                            set: { handlers.transitionTiming(clip.uid, $0) }
                        )) {
                            Text(L10n.t("replay.timingOverwrap")).tag("Overwrap")
                            Text(L10n.t("replay.timingAdditive")).tag("Additive")
                        }
                        .labelsHidden()
                        .help(L10n.t("replay.timingHelp"))
                        TextField("", value: Binding(
                            get: { clip.transition.durationFrames },
                            set: { handlers.transitionFrames(clip.uid, $0) }
                        ), format: .number)
                        .frame(width: 48)
                    }
                }
                .width(380)

                TableColumn(L10n.t("replay.gain")) { clip in
                    TextField("", value: Binding(
                        get: { clip.gainDb },
                        set: { handlers.gain(clip.uid, $0) }
                    ), format: .number.precision(.fractionLength(1)))
                    .frame(width: 52)
                }
                .width(60)

                TableColumn(L10n.t("replay.preservePitch")) { clip in
                    Toggle("", isOn: Binding(
                        get: { clip.preservePitch },
                        set: { handlers.preservePitch(clip.uid, $0) }
                    ))
                    .labelsHidden()
                    .toggleStyle(.checkbox)
                }
                .width(56)

                TableColumn(L10n.t("replay.audio")) { clip in
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
                    .help(L10n.t("replay.clipAudioHelp"))
                }
                .width(150)

                TableColumn(L10n.t("replay.tags")) { clip in
                    TextField("", text: Binding(
                        get: { clip.tags.joined(separator: ", ") },
                        set: { handlers.tags(clip.uid, $0) }
                    ))
                    .frame(width: 120)
                }
                .width(130)

                TableColumn("") { clip in
                    HStack(spacing: 4) {
                        Button {
                            handlers.cue(clip)
                        } label: {
                            Image(systemName: "play.fill")
                        }
                        .help(L10n.t("replay.cueHint"))
                        Button {
                            handlers.remove(clip.uid)
                        } label: {
                            Image(systemName: "trash")
                        }
                        .help(L10n.t("replay.remove"))
                    }
                }
                .width(70)
            }
            .frame(minHeight: 140)
            if clips.isEmpty {
                Text(L10n.t("replay.noClips"))
                    .font(.caption)
                    .foregroundStyle(EivizTheme.dim)
            }
        }
    }
}
