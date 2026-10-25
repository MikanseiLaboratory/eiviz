import AppKit
import EivizMixer
import SwiftUI
// `UTType.mpeg4Movie`, for the export save panel. AppKit does not re-export it.
import UniformTypeIdentifiers

/// Live replay operator surface. Mark In/Out and the transport sit at the bottom,
/// the angle multiview and the event/playlist lists fill the middle, and the
/// recording timecode stays visible.
struct ReplayView: View {
    @EnvironmentObject private var mixer: MixerController

    @State private var document: ReplayDocument?
    @State private var loadedJson = ""
    @State private var channel: UInt32 = 0
    /// Where the channel is: at the live edge or in recorded material, and whether it is
    @State private var transport = ReplayTransportState.atEdge
    /// When the mixer's own play state may be read back again. The deck trusts its own
    /// answer until then, because the mixer reports a start a frame or two late.
    @State private var settleUntil: Date?
    @State private var speedMilli: Int32 = 1000
    /// Whether the channel loops its playlist. The mixer owns it; this mirrors it.
    @State private var looping = false
    /// Whether the last clip keeps running past its out point. Also the mixer's; mirrored.
    @State private var holdLast = false
    @State private var inFrame: UInt64?
    /// Marked out point. Kept so the operator can read back what was marked.
    @State private var outFrame: UInt64?
    /// True between Mark In and Mark Out. The mark itself stays on screen after it
    /// is taken, so this, not `inFrame`, decides whether Mark Out may run.
    @State private var markOpen = false
    @State private var stats: EivizReplayStats?
    @State private var selectedEvent: UInt64?
    @State private var selectedClip: String?
    @State private var kind = "Cut"
    @State private var timing = "Overwrap"
    @State private var frames = "0"
    @State private var clipTags = ""
    @State private var preservePitch = false
    @State private var clipGain = "0"
    /// Default transition for clips added later.
    @State private var defaultKind = "Cut"
    @State private var defaultTiming = "Overwrap"
    @State private var defaultFrames = "0"
    @State private var clipAudio = "FollowAngle"
    /// Export: the same recordings, written to MP4 with the OS encoder.
    @State private var exportMode = "Clip"
    @State private var exportCodec = "H264"
    @State private var exportPath = ""
    /// The files an export writes and the progress it reports.
    @StateObject private var exporter = ReplayExportRunner()
    /// Recording health: where the frames went.
    @State private var health = ""
    /// True while a selection change fills the editors, so they do not write back.
    @State private var loadingClip = false
    @State private var status = ""
    /// Recorded waveform of the angle being watched.
    /// Monitor per angle, so each angle scene has somewhere to present.
    @State private var angleMonitors: [UInt32: UInt64] = [:]
    /// Monitor ids of the deck's output and live-input pictures.
    @State private var outputMonitor: UInt64 = 0
    @State private var liveMonitor: UInt64 = 0
    @State private var peakMins: [Float] = []
    @State private var peakMaxes: [Float] = []
    /// Buckets the mixer evicted, so the first bucket on screen is this many into
    /// the recording. The marks are placed with it.
    @State private var peakDropped: UInt32 = 0
    @State private var jogValue: Double = 0

    private let tick = Timer.publish(every: 0.2, on: .main, in: .common).autoconnect()

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            header
            deckMonitors
            HStack(alignment: .top, spacing: 10) {
                angleStrip
                VStack(spacing: 8) {
                    eventList
                    playlist
                }
            }
            transportBar
        }
        .padding(12)
        .frame(minWidth: 1100, minHeight: 720)
        .onAppear { load() }
        .onReceive(tick) { _ in refresh() }
    }

    private var fps: (num: UInt32, den: UInt32) {
        guard let document else { return (60_000, 1_001) }
        return mixer.replayFps(document)
    }

    private var channels: [ReplayChannelInfo] { document?.channels ?? [] }
    private var angles: [ReplayAngleInfo] { document?.angles ?? [] }
    /// The playlist the selected channel plays. A project can hold several, and the
    /// channel is what points at one.
    private var playlistId: UInt64 { document?.channelPlaylist(channelIndex: channel) ?? 0 }
    private var clips: [ReplayClipInfo] { document?.clips(playlistId: playlistId) ?? [] }
    private var playlists: [PlaylistInfo] { document?.playlists ?? [] }

    private var header: some View {
        HStack(alignment: .center, spacing: 8) {
            Text(L10n.t("replay.channel"))
            Picker("", selection: $channel) {
                ForEach(channels) { item in
                    Text(item.label).tag(item.index)
                }
            }
            .labelsHidden()
            .frame(width: 140, height: 32)
            Text(stateText).fontWeight(.bold)
            if !status.isEmpty {
                Text(status).font(.caption).foregroundStyle(EivizTheme.dim).lineLimit(1)
            }
            Spacer()
            Text(L10n.t("replay.recordTc")).font(.caption)
            Text(mixer.isRemote ? "--:--:--:--" : ReplayTime.timecode(stats?.recording_frame ?? 0, fps))
                .font(.system(.title3, design: .monospaced))
                .frame(minWidth: 148, alignment: .trailing)
            // The deck is where an operator notices that storage or angles are
            // missing, so the way into the recording settings belongs here too.
            Button(L10n.t("replay.openSetup")) { mixer.openReplaySettings() }
                .frame(height: 32)
                .help(L10n.t("replay.openSetupHint"))
        }
    }

    /// What the channel is sending out and the angle being recorded. A deck that
    /// only shows the recording is half a deck: the operator cuts from the live
    /// picture and watches what went out.
    private var deckMonitors: some View {
        HStack(alignment: .top, spacing: 8) {
            monitor(title: L10n.t("replay.output"),
                    caption: outputCaption,
                    monitorId: outputMonitor,
                    sourceId: channelSource)
            monitor(title: L10n.t("replay.liveInput"),
                    caption: liveCaption,
                    monitorId: liveMonitor,
                    sourceId: activeAngle?.liveSourceId ?? 0)
        }
    }

    private func monitor(title: String, caption: String, monitorId: UInt64, sourceId: UInt64) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(title).fontWeight(.bold)
            if monitorId != 0 && sourceId != 0 {
                MetalPreviewRepresentable(role: .monitor(monitorId: monitorId, sourceId: sourceId))
                    .aspectRatio(16.0 / 9.0, contentMode: .fit)
                    .frame(maxWidth: .infinity)
                    .background(Color.black)
            } else {
                Color.black.aspectRatio(16.0 / 9.0, contentMode: .fit)
            }
            Text(caption).font(.caption).foregroundStyle(EivizTheme.dim)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var outputCaption: String {
        guard let item = channels.first(where: { $0.index == channel }) else {
            return L10n.t("replay.noChannels")
        }
        return L10n.format(
            "replay.outputLabel",
            item.label,
            L10n.t(transport.playing ? "replay.playing" : "replay.stopped")
        )
    }

    private var liveCaption: String {
        guard let angle = activeAngle else { return L10n.t("replay.noActiveAngle") }
        return L10n.format("replay.inputLabel", angle.label)
    }

    /// Source the selected channel paints into, which is what "output" means.
    private var channelSource: UInt64 {
        guard let item = channels.first(where: { $0.index == channel }) else { return 0 }
        return item.sourceId
    }

    private var activeAngle: ReplayAngleInfo? {
        let active = document?.activeAngleId ?? 0
        return angles.first { $0.id == active } ?? angles.first
    }

    private var stateText: String {
        // A Remote drives Replay on the machine that owns the recording, so there is
        // no recording here to measure. Reporting "idle" would be a lie.
        if mixer.isRemote { return L10n.t("replay.stateRemote") }
        let base: String
        if let stats, stats.state != 0 {
            switch stats.state {
            case 1: base = recordingText(stats)
            case 2: base = L10n.t("replay.stateStopped")
            default: base = L10n.t("replay.stateIdle")
            }
        } else {
            base = L10n.t("replay.stateIdle")
        }
        let missing = document?.missingRecordingCount(skip: stats?.state == 1 ? mixer.replayRecordingUuid() : nil) ?? 0
        if missing > 0 {
            return base + "  " + L10n.format("replay.mediaMissing", "\(missing)")
        }
        return base
    }

    private func markReadout(_ title: String, _ value: String) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(title).font(.caption)
            Text(value).font(.system(.body, design: .monospaced))
        }
        .frame(width: 120, height: 44, alignment: .leading)
    }

    /// Length of the mark, or a placeholder while it is incomplete.
    private var markDuration: String {
        guard let start = inFrame, let end = outFrame, end > start else { return "--:--:--:--" }
        return ReplayTime.duration(start, end, fps)
    }

    /// What the recorder is doing, in the operator's words. A disk that ran out of
    /// room says so; a store that has not measured free space yet reports zero,
    /// which must not read as "full".
    private func recordingText(_ snapshot: EivizReplayStats) -> String {
        if snapshot.issues & ReplayIssues.storageFull != 0 {
            return L10n.t("replay.stateRecordingNoSpace")
        }
        if snapshot.remaining_secs == UInt32.max {
            return L10n.t("replay.stateRecordingUnknown")
        }
        return L10n.format("replay.stateRecording", ReplayTime.clock(snapshot.remaining_secs))
    }

    /// One numbered button per camera, the way a replay controller labels its cameras.
    /// The picture of the selected one is the monitor above, so the strip stays a row of
    /// numbers instead of a row of expensive previews.
    private var angleStrip: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 8) {
                Text(L10n.t("replay.angles")).fontWeight(.bold)
                if angles.isEmpty {
                    Text(L10n.t("replay.noAngles")).foregroundStyle(EivizTheme.dim)
                } else if mixer.isRemote {
                    // Angle previews are the recording machine's scenes, and a Remote has
                    // no local render of them. Saying where they are beats dead tiles.
                    Text(L10n.t("replay.remoteAngles")).foregroundStyle(EivizTheme.dim)
                } else {
                    ForEach(Array(angles.enumerated()), id: \.element.id) { index, angle in
                        let selected = (document?.activeAngleId ?? 0) == angle.id
                        Button {
                            selectAngle(angle.id)
                        } label: {
                            angleTile(index: index + 1, angle: angle, selected: selected)
                        }
                        .buttonStyle(.plain)
                        .frame(width: 48, height: 44)
                        .overlay(
                            RoundedRectangle(cornerRadius: 6)
                                .stroke(selected ? Color.green : Color.gray.opacity(0.4),
                                        lineWidth: selected ? 3 : 1)
                        )
                        .help("\(index + 1): \(angle.label)")
                    }
                }
                Spacer()
            }
            Text(L10n.t("replay.anglesHelpShort")).font(.caption).foregroundStyle(EivizTheme.dim)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    /// Every event on one line, edited in that line: its name and its protection are
    /// cells, so nothing is "select one, then use the buttons above".
    private var eventList: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Text(L10n.t("replay.events")).fontWeight(.bold)
                Spacer()
                Text(L10n.t("replay.eventsRowHelp")).font(.caption).foregroundStyle(EivizTheme.dim)
            }
            List {
                ForEach(document?.events ?? []) { event in
                    HStack(spacing: 6) {
                        Text(ReplayTime.timecode(event.inFrame, fps))
                            .font(.system(.caption, design: .monospaced))
                        Text(ReplayTime.timecode(event.outFrame, fps))
                            .font(.system(.caption, design: .monospaced))
                        Text(ReplayTime.duration(event.inFrame, event.outFrame, fps))
                            .font(.system(.caption, design: .monospaced))
                        TextField("", text: Binding(
                            get: { event.name },
                            set: { renameEvent(event.id, $0) }
                        ))
                        .frame(minWidth: 120)
                        Toggle(L10n.t("replay.protect"), isOn: Binding(
                            get: { event.protected },
                            set: { setEventProtected(event.id, $0) }
                        ))
                        .toggleStyle(.checkbox)
                        .help(L10n.t("replay.protectHint"))
                        Button {
                            addEventToPlaylist(event)
                        } label: {
                            Image(systemName: "play.fill")
                        }
                        .help(L10n.t("replay.cueHint"))
                        Button {
                            deleteEvent(event.id)
                        } label: {
                            Image(systemName: "trash")
                        }
                        .help(L10n.t("replay.delete"))
                    }
                }
            }
            .frame(minHeight: 120)
            if (document?.events ?? []).isEmpty {
                Text(L10n.t("replay.noEvents")).font(.caption).foregroundStyle(EivizTheme.dim)
            }
        }
    }

    /// The playlist this channel plays, with every clip on one line and its settings
    /// edited in that line: angle, speed, transition, length, gain, pitch and audio are
    /// cells, so nothing is "select a clip, then use the controls below".
    private var playlist: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(alignment: .center, spacing: 8) {
                Text(L10n.t("replay.playlist")).fontWeight(.bold)
                Picker("", selection: Binding(
                    get: { playlistId },
                    set: { selectPlaylist($0) }
                )) {
                    ForEach(playlists) { item in
                        Text(item.label).tag(item.id)
                    }
                }
                .labelsHidden()
                .frame(width: 190, height: 32)
                .help(L10n.t("replay.playlistPickHelp"))
                TextField("", text: Binding(
                    get: { playlists.first { $0.id == playlistId }?.name ?? "" },
                    set: { renamePlaylist(playlistId, $0) }
                ))
                .frame(width: 160, height: 32)
                .help(L10n.t("replay.playlistNameHelp"))
                Button(L10n.t("replay.playlistAdd")) { addPlaylist() }
                    .frame(width: 88, height: 32)
                    .help(L10n.t("replay.playlistAddHint"))
                Button(L10n.t("replay.playlistRemove")) { deletePlaylist() }
                    .frame(width: 72, height: 32)
                    .disabled(playlists.count <= 1)
                    .help(L10n.t("replay.playlistRemove"))
                Spacer()
                // Playlist default: what clips added from now on start with.
                Text(L10n.t("replay.defaultTransition")).font(.caption)
                Picker("", selection: $defaultKind) {
                    Text(L10n.t("replay.transitionCut")).tag("Cut")
                    Text(L10n.t("replay.transitionDissolve")).tag("Dissolve")
                }
                .labelsHidden()
                .frame(width: 110, height: 32)
                .onChange(of: defaultKind) { _, _ in applyDefaultTransition() }
                Picker("", selection: $defaultTiming) {
                    Text(L10n.t("replay.timingOverwrap")).tag("Overwrap")
                    Text(L10n.t("replay.timingAdditive")).tag("Additive")
                }
                .labelsHidden()
                .frame(width: 140, height: 32)
                .help(L10n.t("replay.timingHelp"))
                .onChange(of: defaultTiming) { _, _ in applyDefaultTransition() }
                TextField("", text: $defaultFrames)
                    .frame(width: 56, height: 32)
                    .onSubmit { applyDefaultTransition() }
            }
            ReplayClipTable(
                clips: clips,
                angles: angles,
                fps: fps,
                handlers: ReplayClipTable.Handlers(
                    outText: { clipOutText($0) },
                    angle: { setClipAngle($0, $1) },
                    speed: { setClipSpeed($0, $1) },
                    transitionKind: { setClipTransition($0, kind: $1, timing: nil, frames: nil) },
                    transitionDirection: { setClipTransition($0, direction: $1) },
                    transitionTiming: { setClipTransition($0, kind: nil, timing: $1, frames: nil) },
                    transitionFrames: { setClipTransition($0, kind: nil, timing: nil, frames: $1) },
                    gain: { setClipGain($0, $1) },
                    preservePitch: { setClipPitch($0, $1) },
                    audio: { setClipAudioChoice($0, $1) },
                    tags: { setClipTagsText($0, $1) },
                    cue: { cue($0) },
                    remove: { removeClip($0) }))
        }
    }

    private var exportBar: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(alignment: .center, spacing: 8) {
                Text(L10n.t("replay.export"))
                    .frame(width: 72, alignment: .leading)
                Picker("", selection: $exportCodec) {
                    Text("H.264").tag("H264")
                    Text("HEVC").tag("Hevc")
                }
                .labelsHidden()
                .frame(width: 100, height: 32)
                Picker("", selection: $exportMode) {
                    Text(L10n.t("replay.exportClip")).tag("Clip")
                    // Two playlist modes: the clips as they play, and the same clips
                    // joined by cuts. The first is what the operator watched.
                    Text(L10n.t("replay.exportPlaylistTransitions")).tag("PlaylistWithTransitions")
                    Text(L10n.t("replay.exportPlaylistCut")).tag("Playlist")
                    Text(L10n.t("replay.exportAngle")).tag("Angle")
                    Text(L10n.t("replay.exportAllAngles")).tag("AllAngles")
                }
                .labelsHidden()
                .frame(width: 220, height: 32)
                TextField(L10n.t("replay.exportPath"), text: $exportPath)
                    .frame(maxWidth: .infinity, minHeight: 32, maxHeight: 32)
                Button(L10n.t("replay.browse")) { chooseExportPath() }
                    .frame(width: 88, height: 32)
                Button(L10n.t("replay.exportStart")) { startExport() }
                    .disabled(exporter.busy)
                    .frame(width: 96, height: 32)
                Button(L10n.t("replay.exportCancel")) { cancelExport() }
                    .disabled(!exporter.busy)
                    .frame(width: 88, height: 32)
            }
            if !exporter.status.isEmpty {
                Text(exporter.status).font(.caption)
            }
            if !health.isEmpty {
                Text(health).font(.caption).foregroundStyle(EivizTheme.dim)
            }
        }
    }


    private func chooseExportPath() {
        let panel = NSSavePanel()
        panel.allowedContentTypes = [.mpeg4Movie]
        panel.nameFieldStringValue = "replay-\(ReplayView.stamp()).mp4"
        if panel.runModal() == .OK, let url = panel.url {
            exportPath = url.path
        }
    }

    private static func stamp() -> String {
        let formatter = DateFormatter()
        formatter.dateFormat = "yyyyMMdd-HHmmss"
        return formatter.string(from: Date())
    }

    private func cancelExport() {
        exporter.cancel(mixer: mixer)
    }

    private func startExport() {
        guard let document else { return }
        let path = exportPath.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !path.isEmpty else {
            exporter.report(L10n.t("replay.exportNeedPath"))
            return
        }
        if stats?.state == 1 {
            let alert = NSAlert()
            alert.messageText = L10n.t("replay.exportWhileRecording")
            alert.addButton(withTitle: "OK")
            alert.addButton(withTitle: "Cancel")
            if alert.runModal() != .alertFirstButtonReturn { return }
        }
        let segments = exportSegments(mode: exportMode, document: document)
        let exports = exportClips(mode: exportMode)
        guard !segments.isEmpty || !exports.isEmpty else {
            exporter.report(L10n.t("replay.exportNothing"))
            return
        }
        if exportMode == "AllAngles" {
            // One file per angle, so a multi-camera export stays usable.
            let base = (path as NSString).deletingPathExtension
            let ext = (path as NSString).pathExtension
            for angle in document.angles {
                let target = "\(base)-angle\(angle.id).\(ext)"
                // A refusal is the mixer saying this file cannot be written at all, so the
                // rest of the angles would fail the same way: stop rather than queue them.
                guard exporter.start(
                    json: exportJson(
                        path: target, document: document, segments: segmentsFor(angle, in: segments)),
                    label: (target as NSString).lastPathComponent,
                    mixer: mixer)
                else { break }
            }
        } else {
            exporter.start(
                json: exportJson(path: path, document: document, segments: segments, clips: exports),
                label: (path as NSString).lastPathComponent,
                mixer: mixer)
        }
    }

    /// The playlist clips a mode exports, with the transition out of each one.
    ///
    /// A single clip has nothing to join to, so its transition is not exported: there is
    /// no next clip for it to dissolve into.
    private func exportClips(mode: String) -> [(clip: ReplayClipInfo, transition: ReplayTransitionInfo?)] {
        ReplayExportPlan.clips(mode: mode, clips: clips, selectedClip: selectedClip)
    }

    /// Export progress, and the queue that follows it. The runner owns both.
    private func refreshExport() {
        exporter.poll(mixer: mixer)
    }

    /// Segments the chosen mode writes, in order.
    /// Whole recordings a mode exports. The planning itself lives in `ReplayExportPlan`, so
    /// the window only supplies what it knows.
    private func exportSegments(mode: String, document: ReplayDocument) -> [(UInt32, UInt64, UInt64)] {
        ReplayExportPlan.segments(
            mode: mode,
            clips: clips,
            angles: document.angles,
            active: document.activeAngleId,
            range: { mixer.replayAngleRange($0) })
    }

    private func segmentsFor(
        _ angle: ReplayAngleInfo,
        in segments: [(UInt32, UInt64, UInt64)]
    ) -> [(UInt32, UInt64, UInt64)] {
        segments.filter { $0.0 == angle.id }
    }

    private func exportJson(
        path: String,
        document: ReplayDocument,
        segments: [(UInt32, UInt64, UInt64)],
        clips: [(clip: ReplayClipInfo, transition: ReplayTransitionInfo?)] = []
    ) -> String {
        let format = document.raw["format"]?.objectValue ?? [:]
        let width = format["width"]?.uint32Value ?? 1920
        let height = format["height"]?.uint32Value ?? 1080
        let fps = document.fps
        // Matches the recording profiles: HQ carries the most detail.
        let bitrate: UInt32 = switch document.profile {
        case "Hq": 40_000_000
        case "Lq": 8_000_000
        default: 20_000_000
        }
        let list = segments
            .map { "{\"angleId\":\($0.0),\"fromFrame\":\($0.1),\"toFrame\":\($0.2)}" }
            .joined(separator: ",")
        let clipList = clips.map { ReplayExportJson.clip($0.clip, $0.transition) }
            .joined(separator: ",")
        return """
        {"path":\(ReplayJson.string(path)),"width":\(width),"height":\(height),\
        "fpsNum":\(fps.num),"fpsDen":\(fps.den),"bitrate":\(bitrate),\
        "sampleRate":48000,"channels":2,"withAudio":true,"codec":"\(exportCodec)",\
        "segments":[\(list)],"clips":[\(clipList)]}
        """
    }

    private var transportBar: some View {
        VStack(spacing: 8) {
            waveform
            exportBar
            HStack(alignment: .center, spacing: 8) {
                Button(L10n.t("replay.markIn")) { markIn() }
                    .keyboardShortcut("i", modifiers: [])
                    .help(L10n.t("replay.markInHint"))
                    .frame(width: 160, height: 44)
                Button(L10n.t("replay.markOut")) { markOut() }
                    .keyboardShortcut("o", modifiers: [])
                    .help(L10n.t("replay.markOutHint"))
                    .frame(width: 160, height: 44)
                    .disabled(!markOpen)
                Button(L10n.t("replay.cancelMark")) { cancelMark() }
                    .frame(width: 120, height: 44)
                Spacer()
                markReadout(L10n.t("replay.in"), inFrame.map { ReplayTime.timecode($0, fps) } ?? "--:--:--:--")
                markReadout(L10n.t("replay.out"), outFrame.map { ReplayTime.timecode($0, fps) } ?? "--:--:--:--")
                markReadout(L10n.t("replay.duration"), markDuration)
            }
            HStack(alignment: .center, spacing: 8) {
                Button(L10n.t("replay.record")) { record(true) }
                    .frame(width: 96, height: 36)
                Button(L10n.t("replay.stop")) { record(false) }
                    .frame(width: 96, height: 36)
                Button(L10n.t("replay.live")) { live() }
                    .keyboardShortcut("l", modifiers: [])
                    .frame(width: 96, height: 36)
                Button(L10n.t(transport.playing ? "replay.pause" : "replay.play")) { togglePlay() }
                    .keyboardShortcut(.space, modifiers: [])
                    .frame(width: 108, height: 36)
                Button("◀") { step(-1) }
                    .keyboardShortcut(.leftArrow, modifiers: [])
                    .help(L10n.t("replay.stepBackHint"))
                    .frame(width: 44, height: 36)
                Button("▶") { step(1) }
                    .keyboardShortcut(.rightArrow, modifiers: [])
                    .help(L10n.t("replay.stepForwardHint"))
                    .frame(width: 44, height: 36)
                Button("25%") { setSpeed(250) }.frame(width: 64, height: 36)
                Button("50%") { setSpeed(500) }.frame(width: 64, height: 36)
                Button("75%") { setSpeed(750) }.frame(width: 64, height: 36)
                Button("100%") { setSpeed(1000) }.frame(width: 64, height: 36)
                // Loops the playlist at its end, the way the deck's own button does.
                Toggle(L10n.t("replay.playlistLoop"), isOn: Binding(
                    get: { looping },
                    set: { next in
                        looping = next
                        _ = mixer.replayLoop(channel: channel, on: next)
                    }))
                .toggleStyle(.button)
                .frame(width: 72, height: 36)
                .help(L10n.t("replay.playlistLoopHint"))
                // What happens when the list ends, next to looping because it is the same
                // question. Looping wins when both are on.
                Toggle(L10n.t("replay.playlistHold"), isOn: Binding(
                    get: { holdLast },
                    set: { next in
                        holdLast = next
                        _ = mixer.replayHoldLast(channel: channel, on: next)
                    }))
                .toggleStyle(.button)
                .frame(width: 96, height: 36)
                .help(L10n.t("replay.playlistHoldHint"))
                Spacer(minLength: 8)
                Text(L10n.t("replay.jog"))
                Slider(value: $jogValue, in: -30...30, step: 1) { editing in
                    if !editing {
                        jog(Int(jogValue.rounded()))
                        jogValue = 0
                    }
                }
                .frame(width: 200, height: 36)
            }
        }
    }

    private func load() {
        document = mixer.replayDocument
        loadedJson = document?.json ?? ""
        if let first = channels.first {
            channel = first.index
        }
        if outputMonitor == 0 {
            outputMonitor = mixer.allocateMonitorId()
        }
        if liveMonitor == 0 {
            liveMonitor = mixer.allocateMonitorId()
        }
    }

    /// One monitor per angle, allocated when the angle set changes. The body only
    /// reads this, so no state changes during layout.
    private func ensureAngleMonitors() {
        guard let document else { return }
        var monitors: [UInt32: UInt64] = [:]
        for angle in document.angles {
            monitors[angle.id] = angleMonitors[angle.id] ?? mixer.allocateMonitorId()
        }
        if monitors != angleMonitors {
            angleMonitors = monitors
        }
    }

    /// Where the frames went: what the readback delivered, what the encoder
    /// accepted, and what the disk is doing. Without it an operator only sees that
    /// a recording is short, not which stage lost the picture.
    private func refreshHealth() {
        // The recording runs somewhere else, so its health is read there.
        if mixer.isRemote {
            health = L10n.t("replay.remoteMonitor")
            return
        }
        // Kept while a recording exists, including after it stopped, so the reason
        // it stopped stays readable.
        guard let snapshot = mixer.replayStatsWithAngles(), snapshot.stats.state != 0 else {
            health = ""
            return
        }
        let wanted = ReplayHealth.watchedAngle(
            active: document?.activeAngleId ?? 0,
            angles: document?.angles ?? [])
        let angle = snapshot.angles.first(where: { $0.angle_id == wanted })
        health = ReplayHealth.line(stats: snapshot.stats, angle: angle)
    }

    private func refresh() {
        stats = mixer.replayStats()
        refreshHealth()
        ensureAngleMonitors()
        // The core can change the document (remote edits, the vMix API), so adopt
        // anything we did not write ourselves.
        if let latest = mixer.replayDocument, latest.json != loadedJson {
            document = latest
            loadedJson = latest.json
        }
        if channels.isEmpty {
            load()
        }
        // The mixer owns the transport, so the deck reads the play state back instead of
        // trusting the button it last pressed. A Remote cannot read it and keeps its own.
        //
        // A channel the deck has just started is left alone for a moment: the mixer takes a
        // frame or two to report the change, and reading it too soon put the button back to
        // "Play" over a feed that was already running, so the next press paused it.
        if let settleUntil, Date() < settleUntil {
            // Inside the grace period after Live or Play: the deck's own answer stands.
        } else {
            settleUntil = nil
            if !mixer.isRemote,
               let state = MixerFFI.replayChannels().first(where: { $0.channel_index == channel }) {
                transport = ReplayTransportRules.adopt(transport, mixerPlaying: state.playing != 0)
                if state.speed_milli > 0 {
                    speedMilli = state.speed_milli
                }
            }
        }
        refreshExport()
        // The peaks live with the recording, so a Remote has no waveform to draw.
        if !mixer.isRemote, let document, let angleId = watchAngle(document) {
            let peaks = MixerFFI.replayPeaks(angleId: angleId)
            if peaks.mins.count != peakMins.count {
                peakMins = peaks.mins
                peakMaxes = peaks.maxes
            } else if !peaks.mins.isEmpty {
                peakMins = peaks.mins
                peakMaxes = peaks.maxes
            }
            peakDropped = peaks.dropped
        }
    }

    /// Angle whose waveform is on screen: the one the operator selected, else the
    /// first.
    private func watchAngle(_ document: ReplayDocument) -> UInt32? {
        let active = document.activeAngleId
        if active != 0 {
            return active
        }
        return document.angles.first?.id
    }

    /// Waveform strip. Each column is the min/max of the buckets it covers, which
    /// is how a quiet gap to cut on becomes visible. The marks and the record head
    /// are drawn on it so the operator can see where the cut lands.
    private var waveform: some View {
        Canvas { context, size in
            let buckets = peakMins.count
            guard buckets > 0, size.width > 1, peakMaxes.count == buckets else { return }
            let columns = Int(size.width)
            let middle = size.height / 2
            let scale = middle - 2
            for column in 0..<columns {
                let from = buckets * column / columns
                let to = max(from + 1, buckets * (column + 1) / columns)
                var low = Float.greatestFiniteMagnitude
                var high = -Float.greatestFiniteMagnitude
                for bucket in from..<min(to, buckets) {
                    low = min(low, peakMins[bucket])
                    high = max(high, peakMaxes[bucket])
                }
                guard low <= high else { continue }
                var path = Path()
                path.move(to: CGPoint(x: Double(column) + 0.5, y: middle - Double(high) * scale))
                path.addLine(to: CGPoint(x: Double(column) + 0.5, y: middle - Double(low) * scale))
                context.stroke(path, with: .color(.green), lineWidth: 1)
            }
            // Absolute bucket numbering, so a frame maps to the same column here as
            // it does in the mixer's own bucket list.
            let first = Int(peakDropped)
            let last = first + buckets
            let bucketRate = Double(MixerFFI.replayPeakRate())
            let framesPerSecond = max(1.0, ReplayTime.rate(fps))
            let marks: [(UInt64?, Color, CGFloat)] = [
                (inFrame, .yellow, 1),
                (outFrame, .yellow, 1),
                (stats?.recording_frame, .red, 2),
            ]
            for (frame, color, width) in marks {
                guard let frame, frame != 0, last > first else { continue }
                let bucket = Int(Double(frame) * bucketRate / framesPerSecond)
                guard bucket >= first, bucket <= last else { continue }
                let x = CGFloat(bucket - first) / CGFloat(last - first) * size.width
                var path = Path()
                path.move(to: CGPoint(x: x, y: 0))
                path.addLine(to: CGPoint(x: x, y: size.height))
                context.stroke(path, with: .color(color), lineWidth: width)
            }
        }
        .frame(height: 56)
        .background(Color.black.opacity(0.35))
        // Clicking the waveform moves the playhead; Shift-click sets Mark In there and
        // Control-click sets Mark Out. The waveform is where the quiet moment is visible,
        // so a cut is struck where the operator is looking instead of seeking, looking
        // away to the buttons, and pressing one while the playhead runs on. The modifier
        // is read from the live event because SwiftUI's tap gesture does not report it.
        .overlay(
            GeometryReader { proxy in
                Color.clear
                    .contentShape(Rectangle())
                    .gesture(
                        DragGesture(minimumDistance: 0)
                            .onEnded { drag in
                                seekOrMark(at: drag.location, size: proxy.size, modifiers: NSEvent.modifierFlags)
                            }
                    )
            }
        )
    }

    /// What a click on the waveform does.
    private enum WaveformMark {
        case `in`
        case out
    }

    /// Maps a point on the waveform to a recording frame, then acts on it.
    ///
    /// The waveform draws only the buckets the store still holds, and the store may have
    /// dropped some from the front, so column zero is not frame zero: the dropped count
    /// is the offset, exactly as the drawing uses.
    private func seekOrMark(at point: CGPoint, size: CGSize, modifiers: NSEvent.ModifierFlags) {
        guard size.width > 1, let document else { return }
        let buckets = peakMins.count
        guard buckets > 0 else { return }
        let bucketRate = Double(MixerFFI.replayPeakRate())
        let framesPerSecond = max(1.0, ReplayTime.rate(document.fps))
        guard bucketRate > 0 else { return }
        let ratio = min(max(point.x / size.width, 0), 1)
        let bucket = Double(peakDropped) + ratio * Double(buckets)
        let frame = UInt64(max(0, bucket * framesPerSecond / bucketRate).rounded())
        let mark: WaveformMark? =
            modifiers.contains(.shift) ? .in : (modifiers.contains(.control) ? .out : nil)
        switch mark {
        case .in:
            inFrame = frame
            outFrame = nil
            markOpen = true
            seekKeepingPlay(Int64(frame))
        case .out:
            guard markOpen, let inFrame, frame > inFrame else {
                status = L10n.t("replay.markOutBeforeIn")
                return
            }
            // The range is the one the operator selected, so the event is written from it
            // here rather than from the mixer's own live mark.
            outFrame = frame
            markOpen = false
            createEvent(inFrame: inFrame, outFrame: frame)
        case nil:
            seekKeepingPlay(Int64(frame))
        }
    }

    /// Seeks the playhead without changing whether the channel is playing.
    ///
    /// Dragging the waveform or taking a mark moves the picture off the live edge, so the
    /// strip draws proxies from here on. It does not start or stop anything, though: an
    /// operator who was paused stays paused, and one who was playing keeps playing from the
    /// new frame. The transport was handed that play state, and the deck keeps it.
    private func seekKeepingPlay(_ frame: Int64) {
        let action = ReplayTransportRules.leaveEdge(transport)
        if !mixer.replayTransport(
            channel: channel, playing: action.playing, speedMilli: speedMilli, seekFrame: frame)
        {
            status = L10n.t("replay.commandRefused")
        }
        transport = action.next
    }

    /// Writes the document back after an edit.
    ///
    /// A deletion is a decision, not an absence: the core cannot tell "the deck dropped this
    /// event" from "the deck never had it", so the caller says which ids it removed. Without
    /// that, saving after a deletion either resurrected the event or deleted one marked by
    /// somebody else in the meantime.
    private func save(
        _ updated: ReplayDocument,
        deletedEvents: [UInt64] = [],
        deletedClips: [String] = []
    ) {
        document = updated
        loadedJson = updated.json
        if !mixer.applyReplayDeck(
            updated,
            deletedEvents: deletedEvents,
            deletedClips: deletedClips)
        {
            // The core refused the edit, so it is not on air. Read back what it kept rather
            // than leaving the window showing something the mixer never took.
            status = L10n.t("replay.commandRefused")
            document = mixer.replayDocument
            loadedJson = document?.json ?? ""
        }
    }

    private func selectAngle(_ id: UInt32) {
        guard var current = document, current.activeAngleId != id else { return }
        current.setActiveAngle(id)
        save(current)
    }

    private func markIn() {
        // A failed mark leaves the old one alone instead of showing a nonsense
        // timecode.
        if let mark = mixer.replayMarkIn() {
            inFrame = mark
            // A new Mark In starts a new range: the old Mark Out no longer applies.
            outFrame = nil
            markOpen = true
            return
        }
        // A remote core keeps the mark, so this host only knows that one is open.
        // The frames come back with the session document, not from the mark.
        if mixer.isRemote {
            inFrame = nil
            outFrame = nil
            markOpen = true
            return
        }
        // Local mode returns the frame it marked, so nothing came back and there is
        // no recorded timeline yet: say so instead of opening a range that cannot be
        // closed.
        status = L10n.t("replay.markInFailed")
    }

    private func cancelMark() {
        inFrame = nil
        outFrame = nil
        markOpen = false
    }

    private func markOut() {
        guard var current = document, markOpen else { return }
        let name = L10n.format("replay.eventName", "\(current.events.count + 1)")
        switch mixer.replayMarkOut(name: name) {
        case .refused:
            // The mark was refused, so the range is left open for another try.
            status = L10n.t("replay.markOutFailed")
        case .remote:
            // The core took the mark and created the event; this host never sees
            // the frames, and the event arrives with the session document.
            inFrame = nil
            outFrame = nil
            markOpen = false
        case let .local(uuid, start, end):
            // Keep the range on screen after it is taken: the operator has to see
            // what was just cut, and the waveform and step buttons use the range.
            inFrame = start
            outFrame = end
            markOpen = false
            let id = current.appendEvent(
                uuid: uuid,
                inFrame: start,
                outFrame: end,
                name: name
            )
            if current.autoAddToPlaylist {
                let angleId = current.activeAngle
                if angleId == 0 {
                    status = L10n.t("replay.needAngle")
                } else if let event = current.events.first(where: { $0.id == id }) {
                    _ = current.appendClip(playlistId: current.ensurePlaylist(), event: event, angleId: angleId)
                }
            }
            save(current)
        }
    }

    /// Files an event for a range the operator chose on the waveform.
    ///
    /// The Mark Out button takes the mixer's own live edge. A range picked on the
    /// waveform is already on the recording, so it is written here instead of asking the
    /// core for a mark it would place somewhere else.
    private func createEvent(inFrame start: UInt64, outFrame end: UInt64) {
        guard var current = document else { return }
        let name = L10n.format("replay.eventName", "\(current.events.count + 1)")
        // The range belongs to the recording that is open. A Remote cannot read it, and
        // a range with no recording behind it cannot be played, so it is refused rather
        // than filed against nothing.
        let uuid = mixer.replayRecordingUuid()
        guard !uuid.isEmpty else {
            status = L10n.t("replay.markOutFailed")
            return
        }
        let id = current.appendEvent(uuid: uuid, inFrame: start, outFrame: end, name: name)
        if current.autoAddToPlaylist {
            let angleId = current.activeAngle
            if angleId == 0 {
                status = L10n.t("replay.needAngle")
            } else if let event = current.events.first(where: { $0.id == id }) {
                _ = current.appendClip(playlistId: current.ensurePlaylist(), event: event, angleId: angleId)
            }
        }
        save(current)
    }

    // MARK: - Row edits
    //
    // Each list edits the row it belongs to, so these take the id of the item rather
    // than reading a selection. Nothing here is "select first, then change".

    private func renameEvent(_ id: UInt64, _ name: String) {
        guard var current = document else { return }
        current.renameEvent(id, name: name)
        save(current)
    }

    private func setEventProtected(_ id: UInt64, _ value: Bool) {
        guard var current = document else { return }
        current.setEventProtected(id, value)
        save(current)
    }

    private func deleteEvent(_ id: UInt64) {
        guard var current = document else { return }
        current.removeEvent(id)
        // A clip keeps its own recording, range and angle, so it survives the event it
        // came from; only the reference to an event that is gone goes away.
        save(current, deletedEvents: [id])
    }

    private func removeClip(_ uid: String) {
        guard var current = document else { return }
        current.removeClip(playlistId: playlistId, uid: uid)
        // An animation step that cued this clip has nothing left to fire.
        mixer.clearReplayCues(clipUid: uid)
        save(current, deletedClips: [uid])
    }

    private func setClipAngle(_ uid: String, _ angleId: UInt32) {
        guard var current = document else { return }
        current.setClipAngle(playlistId: playlistId, uid: uid, angleId: angleId)
        save(current)
    }

    private func setClipSpeed(_ uid: String, _ milli: Int32) {
        guard var current = document else { return }
        current.setClipSpeed(playlistId: playlistId, uid: uid, milli: milli)
        save(current)
    }

    /// Writes one part of a clip's transition, keeping the parts the caller left out.
    /// A cut has no length, so choosing it clears the frames instead of leaving a stale
    /// duration behind.
    private func setClipTransition(
        _ uid: String,
        kind: String? = nil,
        direction: String? = nil,
        timing: String? = nil,
        frames: UInt32? = nil
    ) {
        guard var current = document,
              let clip = clips.first(where: { $0.uid == uid })
        else { return }
        let nextKind = kind ?? clip.transition.kind
        let nextTiming = timing ?? clip.transition.timing
        var nextFrames = frames ?? clip.transition.durationFrames
        if nextKind == "Cut" {
            nextFrames = 0
        }
        nextFrames = min(nextFrames, 600)
        current.setClipTransition(
            playlistId: playlistId,
            uid: uid,
            transition: ReplayTransitionInfo(
                kind: nextKind,
                direction: direction ?? clip.transition.direction,
                timing: nextTiming,
                durationFrames: nextFrames
            )
        )
        save(current)
    }

    private func setClipGain(_ uid: String, _ db: Double) {
        guard var current = document else { return }
        guard db.isFinite, db >= -60.0, db <= 12.0 else {
            status = L10n.t("replay.gainRange")
            return
        }
        current.setClipGain(playlistId: playlistId, uid: uid, db: db)
        save(current)
    }

    private func setClipPitch(_ uid: String, _ value: Bool) {
        guard var current = document else { return }
        current.setClipPreservePitch(playlistId: playlistId, uid: uid, value: value)
        save(current)
    }

    private func setClipAudioChoice(_ uid: String, _ value: String) {
        guard var current = document else { return }
        if value.hasPrefix("Angle:") {
            let id = UInt32(value.dropFirst("Angle:".count)) ?? 0
            current.setClipAudio(playlistId: playlistId, uid: uid, kind: "Angle", angleId: id)
        } else {
            current.setClipAudio(playlistId: playlistId, uid: uid, kind: value, angleId: 0)
        }
        save(current)
    }

    /// Writes a clip's tags from the comma separated text in its row.
    private func setClipTagsText(_ uid: String, _ text: String) {
        guard var current = document else { return }
        var tags: [String] = []
        for raw in text.split(separator: ",") {
            let tag = raw.trimmingCharacters(in: .whitespacesAndNewlines)
            if tag.isEmpty || tags.contains(tag) {
                continue
            }
            tags.append(tag)
        }
        current.setClipTags(playlistId: playlistId, uid: uid, tags: tags)
        save(current)
    }

    // MARK: - Playlist edits

    /// Points the channel at another playlist: that is what "which playlist" means.
    private func selectPlaylist(_ id: UInt64) {
        guard var current = document, id != 0 else { return }
        current.setChannelPlaylist(channelIndex: channel, playlistId: id)
        save(current)
    }

    private func renamePlaylist(_ id: UInt64, _ name: String) {
        guard var current = document else { return }
        guard current.renamePlaylist(id, name: name) else {
            return
        }
        save(current)
    }

    /// Adds a playlist and points this channel at it: creating one and then having to
    /// select it would be two steps for one intention.
    private func addPlaylist() {
        guard var current = document else { return }
        let id = current.addPlaylist(name: L10n.format("replay.playlistDefaultName", "\(playlists.count + 1)"))
        current.setChannelPlaylist(channelIndex: channel, playlistId: id)
        save(current)
    }

    private func deletePlaylist() {
        guard var current = document, playlists.count > 1 else {
            status = L10n.t("replay.playlistLast")
            return
        }
        let users = current.channelsUsingPlaylist(playlistId)
        if users > 1 {
            status = L10n.format("replay.playlistInUse", "\(users)")
            return
        }
        let clips = current.clips(playlistId: playlistId).count
        if clips > 0 {
            status = L10n.format("replay.playlistDeleteClips", "\(clips)")
        }
        _ = current.removePlaylist(playlistId)
        if let next = current.playlists.first {
            current.setChannelPlaylist(channelIndex: channel, playlistId: next.id)
        }
        save(current)
    }

    private func toggleProtect() {
        guard var current = document,
              let id = selectedEvent,
              let event = current.events.first(where: { $0.id == id })
        else {
            status = L10n.t("replay.needEvent")
            return
        }
        current.setEventProtected(id, !event.protected)
        save(current)
    }

    private func deleteEvent() {
        guard var current = document, let id = selectedEvent else {
            status = L10n.t("replay.needEvent")
            return
        }
        current.removeEvent(id)
        // A clip keeps its own recording, range and angle, so it survives the event
        // it came from; only the reference to an event that is gone goes away.
        save(current)
        selectedEvent = nil
    }

    private func addEventToPlaylist(_ event: ReplayEventInfo) {
        guard var current = document else { return }
        let angleId = current.activeAngle
        guard angleId != 0 else {
            status = L10n.t("replay.needAngle")
            return
        }
        _ = current.appendClip(playlistId: current.ensurePlaylist(), event: event, angleId: angleId)
        save(current)
    }

    private func cueSelectedClip() {
        guard let uid = selectedClip, let clip = clips.first(where: { $0.uid == uid }) else {
            // A silent no-op reads as a broken button, so say what is missing.
            status = L10n.t("replay.needClip")
            return
        }
        cue(clip)
    }

    private func cue(_ clip: ReplayClipInfo) {
        if !mixer.replayCue(channel: channel, clip: clip) {
            let detail = MixerFFI.lastErrorText()
            status = detail.isEmpty ? L10n.t("replay.cueRefused") : detail
        }
    }

    private func removeSelectedClip() {
        guard var current = document, let uid = selectedClip else { return }
        current.removeClip(playlistId: playlistId, uid: uid)
        // An animation step that cued this clip has nothing left to fire.
        mixer.clearReplayCues(clipUid: uid)
        save(current, deletedClips: [uid])
        selectedClip = nil
    }

    private func loadTransition() {
        guard let uid = selectedClip, let clip = clips.first(where: { $0.uid == uid }) else { return }
        kind = clip.transition.kind
        timing = clip.transition.timing
        frames = "\(clip.transition.durationFrames)"
        clipTags = clip.tags.joined(separator: ", ")
        loadingClip = true
        clipGain = String(format: "%.1f", clip.gainDb)
        loadDefaultTransition()
        preservePitch = clip.preservePitch
        clipAudio = clip.audioKind == "Angle"
            ? "Angle:\(clip.audioAngleId)"
            : clip.audioKind
        loadingClip = false
    }

    /// Writes the clip's audio source. A clip may deliberately be silent, or take
    /// another angle's sound.
    private func applyClipAudio(_ value: String) {
        guard !loadingClip, var current = document, let uid = selectedClip else { return }
        if value.hasPrefix("Angle:") {
            let id = UInt32(value.dropFirst("Angle:".count)) ?? 0
            current.setClipAudio(playlistId: playlistId, uid: uid, kind: "Angle", angleId: id)
        } else {
            current.setClipAudio(playlistId: playlistId, uid: uid, kind: value, angleId: 0)
        }
        save(current)
    }

    /// Writes the playlist default. New clips start on it, so an operator who
    /// sets "always dissolve" does not edit every clip by hand.
    private func applyDefaultTransition() {
        guard var current = document else { return }
        let frames = defaultKind == "Cut" ? 0 : UInt32(defaultFrames) ?? 0
        current.setDefaultTransition(
            playlistId: playlistId,
            transition: ReplayTransitionInfo(kind: defaultKind, timing: defaultTiming, durationFrames: frames)
        )
        save(current)
    }

    /// Shows the playlist default in the editors.
    private func loadDefaultTransition() {
        guard let current = document else { return }
        let fallback = current.defaultTransition(playlistId: playlistId)
        defaultKind = fallback.kind
        defaultTiming = fallback.timing
        defaultFrames = "\(fallback.durationFrames)"
    }

    /// Writes the clip gain. The mixer clamps it, so a nonsense value is refused
    /// here rather than silently corrected.
    private func applyClipGain() {
        guard !loadingClip, var current = document, let uid = selectedClip else { return }
        guard let db = Double(clipGain.trimmingCharacters(in: .whitespaces)), db.isFinite,
              db >= -60.0, db <= 12.0
        else {
            if let clip = clips.first(where: { $0.uid == uid }) {
                clipGain = String(format: "%.1f", clip.gainDb)
        loadDefaultTransition()
            }
            status = L10n.t("replay.gainRange")
            return
        }
        current.setClipGain(playlistId: playlistId, uid: uid, db: db)
        save(current)
    }

    /// Writes the pitch flag of the selected clip. Slow motion otherwise drops the
    /// pitch along with the speed.
    private func applyPreservePitch(_ value: Bool) {
        guard !loadingClip, var current = document, let uid = selectedClip else { return }
        current.setClipPreservePitch(playlistId: playlistId, uid: uid, value: value)
        save(current)
    }

    /// Writes the tag list back to the selected clip. Commas separate tags, blank
    /// entries are dropped, and duplicates collapse.
    private func applyTags() {
        guard var current = document, let uid = selectedClip else { return }
        var tags: [String] = []
        for raw in clipTags.split(separator: ",") {
            let tag = raw.trimmingCharacters(in: .whitespacesAndNewlines)
            if tag.isEmpty || tags.contains(tag) {
                continue
            }
            tags.append(tag)
        }
        current.setClipTags(playlistId: playlistId, uid: uid, tags: tags)
        save(current)
        clipTags = tags.joined(separator: ", ")
    }

    private func applyTransition() {
        guard var current = document, let uid = selectedClip else { return }
        var duration = UInt32(frames.trimmingCharacters(in: .whitespaces)) ?? 0
        duration = min(duration, 600)
        if kind == "Cut" {
            duration = 0
        }
        current.setClipTransition(
            playlistId: playlistId,
            uid: uid,
            transition: ReplayTransitionInfo(kind: kind, timing: timing, durationFrames: duration)
        )
        save(current)
    }

    private func record(_ on: Bool) {
        // A recorder without storage or angles writes nothing, so the reason is
        // named here instead of leaving the mixer's bare refusal on screen.
        if on, let current = document,
           current.storages.isEmpty || current.angles.isEmpty {
            status = L10n.t("replay.needSetup")
            return
        }
        if !mixer.replayEnableRecording(on) {
            status = L10n.t(on ? "replay.recordRefused" : "replay.commandRefused")
        }
    }

    private func live() {
        guard mixer.replayJumpToNow(channel: channel) else {
            status = L10n.t("replay.commandRefused")
            return
        }
        // Play from the edge, as the deck does: the transport takes the recording on from
        // now. Without this the live picture appears only once the poll notices the mixer
        // started the channel, and the button reads "Play" in the meantime, so a second
        // press pauses the feed the operator just asked for.
        //
        // The jump and the play are two mixer calls, and only the jump can fail: it is the
        // one that needs the recording. If it fails there is no edge to play from, so the
        // deck says why and leaves its state alone rather than claiming to be live.
        let action = ReplayTransportRules.jumpToLive()
        if mixer.replayTransport(
            channel: channel, playing: action.playing, speedMilli: speedMilli, seekFrame: action.seekFrame)
        {
            transport = action.next
            settleUntil = Date().addingTimeInterval(1.0)
        } else {
            status = L10n.t("replay.commandRefused")
        }
    }

    private func togglePlay() {
        let action = ReplayTransportRules.toggle(transport)
        if mixer.replayTransport(
            channel: channel, playing: action.playing, speedMilli: speedMilli, seekFrame: -1)
        {
            transport = action.next
            settleUntil = Date().addingTimeInterval(1.0)
        } else {
            // The channel never started, so the button must not say "Pause".
            status = L10n.t("replay.playRefused")
        }
    }

    /// Moves the channel by `delta` frames from Mark In, stopping at Mark Out when
    /// one was taken. The mixer clamps a seek to the clip on its own, so this only
    /// keeps the operator inside the mark they made.
    private func seekMarked(_ delta: Int64) {
        guard let start = inFrame, start != UInt64.max else {
            // Stepping needs a mark taken here; a remote mark has no frame to move from.
            status = L10n.t("replay.stepNeedsLocalMark")
            return
        }
        // Stepping leaves the live edge and holds where it lands.
        var target = max(0, Int64(start) + delta)
        if let end = outFrame, end > start {
            target = min(target, Int64(end))
        }
        let action = ReplayTransportRules.seek()
        if !mixer.replayTransport(
            channel: channel, playing: action.playing, speedMilli: speedMilli, seekFrame: target)
        {
            status = L10n.t("replay.commandRefused")
        }
        transport = action.next
    }

    private func step(_ delta: Int64) {
        seekMarked(delta)
    }

    private func setSpeed(_ milli: Int32) {
        speedMilli = milli
        if !mixer.replayTransport(channel: channel, playing: transport.playing, speedMilli: speedMilli, seekFrame: -1) {
            status = L10n.t("replay.commandRefused")
        }
    }

    /// Live tiles stay the scene. Jog tiles decode the proxy, or say it is missing.
    @ViewBuilder
    private func angleTile(index: Int, angle: ReplayAngleInfo, selected: Bool) -> some View {
        if transport.live || mixer.isRemote {
            Text("\(index)")
                .font(.title3)
                .fontWeight(selected ? .bold : .regular)
                .frame(width: 48, height: 44)
        } else if let image = proxyImage(angleId: angle.id) {
            Image(nsImage: image)
                .resizable()
                .frame(width: 48, height: 44)
        } else {
            Text(L10n.t("replay.proxyMissing"))
                .font(.caption2)
                .multilineTextAlignment(.center)
                .frame(width: 48, height: 44)
        }
    }

    private func proxyImage(angleId: UInt32) -> NSImage? {
        var bytes = [UInt8](repeating: 0, count: 480 * 270 * 2)
        var width: UInt32 = 0
        var height: UInt32 = 0
        // A store may pad its rows, so the source's own stride is read rather than assumed.
        var stride: UInt32 = 0
        let frame = stats?.recording_frame ?? 0
        let ok = bytes.withUnsafeMutableBufferPointer { ptr in
            mixer_replay_copy_proxy(
                angleId, frame, ptr.baseAddress, ptr.count, &width, &height, &stride)
        }
        guard ok == 1, width > 0, height > 0 else { return nil }
        guard let pixels = ReplayUyvy.toRgba(
                  bytes, width: Int(width), height: Int(height), stride: Int(stride)),
              let rep = NSBitmapImageRep(
            bitmapDataPlanes: nil,
            pixelsWide: Int(width),
            pixelsHigh: Int(height),
            bitsPerSample: 8,
            samplesPerPixel: 4,
            hasAlpha: true,
            isPlanar: false,
            colorSpaceName: .deviceRGB,
            bytesPerRow: Int(width) * 4,
            bitsPerPixel: 32
        ) else { return nil }
        pixels.withUnsafeBytes { raw in
            if let dest = rep.bitmapData {
                dest.update(from: raw.bindMemory(to: UInt8.self).baseAddress!, count: pixels.count)
            }
        }
        let image = NSImage(size: NSSize(width: Int(width), height: Int(height)))
        image.addRepresentation(rep)
        return image
    }

    private func clipOutText(_ clip: ReplayClipInfo) -> String {
        let time = ReplayTime.timecode(clip.outFrame, fps)
        guard clipExpired(clip) else { return time }
        return time + " " + L10n.t("replay.expired")
    }

    private func clipExpired(_ clip: ReplayClipInfo) -> Bool {
        let current = mixer.replayRecordingUuid()
        guard !current.isEmpty,
              clip.recordingUuid.caseInsensitiveCompare(current) == .orderedSame,
              let range = mixer.replayAngleRange(clip.angleId)
        else { return false }
        return clip.outFrame < range.first
    }

    private func jog(_ delta: Int) {
        guard delta != 0 else { return }
        seekMarked(Int64(delta))
    }

}

