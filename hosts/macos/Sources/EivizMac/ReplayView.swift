import AppKit
import EivizMixer
import SwiftUI
import UniformTypeIdentifiers

/// Live replay operator surface. Mark In/Out and the transport sit at the bottom,
/// the angle multiview and the event/playlist lists fill the middle, and the
/// recording timecode stays visible.
struct ReplayView: View {
    @EnvironmentObject private var mixer: MixerController

    @State private var document: ReplayDocument?
    @State private var loadedJson = ""
    @State private var channel: UInt32 = 0
    @State private var playing = false
    @State private var speedMilli: Int32 = 1000
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
    @State private var exportPath = ""
    @State private var exportStatus = ""
    /// Recording health: where the frames went.
    @State private var health = ""
    @State private var exportQueue: [ReplayExportJob] = []
    @State private var exporting = false
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
            transport
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
    private var playlistId: UInt64 { document?.firstPlaylistId ?? 0 }
    private var clips: [ReplayClipInfo] { document?.clips(playlistId: playlistId) ?? [] }

    private var header: some View {
        HStack {
            Text(L10n.t("replay.channel"))
            Picker("", selection: $channel) {
                ForEach(channels) { item in
                    Text(item.label).tag(item.index)
                }
            }
            .labelsHidden()
            .frame(width: 140)
            Text(stateText).fontWeight(.bold)
            Spacer()
            Text(L10n.t("replay.recordTc")).font(.caption)
            Text(mixer.isRemote ? "--:--:--:--" : ReplayTime.timecode(stats?.recording_frame ?? 0, fps))
                .font(.system(.title3, design: .monospaced))
            // The deck is where an operator notices that storage or angles are
            // missing, so the way into the recording settings belongs here too.
            Button(L10n.t("replay.openSetup")) { mixer.openReplaySettings() }
                .help(L10n.t("replay.openSetupHint"))
            if !status.isEmpty {
                Text(status).font(.caption).foregroundStyle(EivizTheme.dim)
            }
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
            L10n.t(playing ? "replay.playing" : "replay.stopped")
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
        guard let stats, stats.state != 0 else { return L10n.t("replay.stateIdle") }
        switch stats.state {
        case 1:
            return recordingText(stats)
        case 2:
            return L10n.t("replay.stateStopped")
        default:
            return L10n.t("replay.stateIdle")
        }
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

    private var angleStrip: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(L10n.t("replay.angles")).fontWeight(.bold)
            if angles.isEmpty {
                Text(L10n.t("replay.noAngles"))
                    .foregroundStyle(EivizTheme.dim)
                    .frame(width: 280, alignment: .leading)
            } else if mixer.isRemote {
                // Angle previews are the recording machine's scenes, and a Remote
                // has no local render of them. Saying where they are beats dead tiles.
                Text(L10n.t("replay.remoteAngles"))
                    .foregroundStyle(EivizTheme.dim)
                    .frame(width: 280, alignment: .leading)
            } else {
                ScrollView {
                    LazyVGrid(columns: [GridItem(.adaptive(minimum: 260), spacing: 8)], spacing: 8) {
                        ForEach(angles) { angle in
                            Button {
                                selectAngle(angle.id)
                            } label: {
                                VStack(spacing: 2) {
                                    // Each angle is its own scene, shown through a
                                    // monitor. The id is prepared outside the body
                                    // because the body must not mutate state.
                                    if let monitor = angleMonitors[angle.id], monitor != 0 {
                                        MetalPreviewRepresentable(
                                            role: .monitor(monitorId: monitor, sourceId: angle.liveSourceId)
                                        )
                                        .aspectRatio(16.0 / 9.0, contentMode: .fit)
                                        .frame(maxWidth: .infinity)
                                        .background(Color.black)
                                    } else {
                                        Color.black
                                            .aspectRatio(16.0 / 9.0, contentMode: .fit)
                                    }
                                    Text(angle.label)
                                        .font(.caption)
                                        .fontWeight(document?.activeAngleId == angle.id ? .bold : .regular)
                                }
                                .padding(4)
                                .overlay(
                                    RoundedRectangle(cornerRadius: 6)
                                        .stroke(
                                            document?.activeAngleId == angle.id ? Color.green : Color.gray.opacity(0.4),
                                            lineWidth: document?.activeAngleId == angle.id ? 3 : 1
                                        )
                                )
                            }
                            .buttonStyle(.plain)
                        }
                    }
                }
                .frame(minWidth: 300)
            }
            Spacer()
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var eventList: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Text(L10n.t("replay.events")).fontWeight(.bold)
                Spacer()
                Button(L10n.t("replay.protect")) { toggleProtect() }
                Button(L10n.t("replay.delete")) { deleteEvent() }
            }
            List(selection: $selectedEvent) {
                ForEach(document?.events ?? []) { event in
                    HStack {
                        Text(ReplayTime.timecode(event.inFrame, fps)).font(.system(.caption, design: .monospaced))
                        Text(ReplayTime.timecode(event.outFrame, fps)).font(.system(.caption, design: .monospaced))
                        Text(ReplayTime.duration(event.inFrame, event.outFrame, fps))
                            .font(.system(.caption, design: .monospaced))
                        Text(event.protected ? "🔒 \(event.name)" : event.name)
                        Spacer()
                    }
                    .tag(event.id)
                    .contentShape(Rectangle())
                    .onTapGesture(count: 2) { addEventToPlaylist(event) }
                }
            }
            .frame(minHeight: 120)
        }
    }

    private var playlist: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Text(L10n.t("replay.playlist")).fontWeight(.bold)
                Spacer()
                Button(L10n.t("replay.cue")) { cueSelectedClip() }
                Button(L10n.t("replay.remove")) { removeSelectedClip() }
            }
            List(selection: $selectedClip) {
                ForEach(clips) { clip in
                    HStack {
                        Text(ReplayTime.timecode(clip.inFrame, fps)).font(.system(.caption, design: .monospaced))
                        Text(ReplayTime.timecode(clip.outFrame, fps)).font(.system(.caption, design: .monospaced))
                        Text(document?.angleLabel(clip.angleId) ?? "-")
                        Text("\(Int(clip.speed * 100))%")
                        Text(clip.transition.label)
                        Spacer()
                    }
                    .tag(clip.uid)
                    .contentShape(Rectangle())
                    .onTapGesture(count: 2) { cue(clip) }
                }
            }
            .frame(minHeight: 120)
            HStack {
                Text(L10n.t("replay.transition"))
                Picker("", selection: $kind) {
                    Text(L10n.t("replay.transitionCut")).tag("Cut")
                    Text(L10n.t("replay.transitionDissolve")).tag("Dissolve")
                }
                .labelsHidden()
                .frame(width: 120)
                .onChange(of: kind) { _, _ in applyTransition() }
                Picker("", selection: $timing) {
                    Text(L10n.t("replay.timingOverwrap")).tag("Overwrap")
                    Text(L10n.t("replay.timingAdditive")).tag("Additive")
                }
                .labelsHidden()
                .frame(width: 160)
                .help(L10n.t("replay.timingHelp"))
                .onChange(of: timing) { _, _ in applyTransition() }
                TextField("", text: $frames)
                    .frame(width: 60)
                    .onSubmit { applyTransition() }
                Text(L10n.t("replay.transitionFrames"))
                Text(L10n.t("replay.tags")).padding(.leading, 12)
                TextField("", text: $clipTags)
                    .frame(width: 200)
                    .onSubmit { applyTags() }
                Text(L10n.t("replay.gain")).padding(.leading, 12)
                TextField("", text: $clipGain)
                    .frame(width: 50)
                    .onSubmit { applyClipGain() }
                Toggle(L10n.t("replay.preservePitch"), isOn: $preservePitch)
                    .padding(.leading, 12)
                    .onChange(of: preservePitch) { _, value in applyPreservePitch(value) }
                Picker("", selection: $clipAudio) {
                    Text(L10n.t("replay.audioFollow")).tag("FollowAngle")
                    Text(L10n.t("replay.audioSilent")).tag("Silent")
                    ForEach(document?.angles ?? []) { angle in
                        Text(angle.label).tag("Angle:\(angle.id)")
                    }
                }
                .labelsHidden()
                .frame(width: 170)
                .onChange(of: clipAudio) { _, value in applyClipAudio(value) }
            }
            .onChange(of: selectedClip) { _, _ in loadTransition() }
            // Playlist default: what clips added from now on start with.
            HStack {
                Text(L10n.t("replay.defaultTransition")).font(.caption)
                Picker("", selection: $defaultKind) {
                    Text(L10n.t("replay.transitionCut")).tag("Cut")
                    Text(L10n.t("replay.transitionDissolve")).tag("Dissolve")
                }
                .labelsHidden()
                .frame(width: 100)
                .onChange(of: defaultKind) { _, _ in applyDefaultTransition() }
                Picker("", selection: $defaultTiming) {
                    Text(L10n.t("replay.timingOverwrap")).tag("Overwrap")
                    Text(L10n.t("replay.timingAdditive")).tag("Additive")
                }
                .labelsHidden()
                .frame(width: 100)
                .help(L10n.t("replay.timingHelp"))
                .onChange(of: defaultTiming) { _, _ in applyDefaultTransition() }
                TextField("", text: $defaultFrames)
                    .frame(width: 50)
                    .onSubmit { applyDefaultTransition() }
                Text(L10n.t("replay.transitionFrames"))
                Text(L10n.t("replay.defaultTransitionHelp")).font(.caption)
            }
        }
    }

    private var exportBar: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Text(L10n.t("replay.export"))
                Picker("", selection: $exportMode) {
                    Text(L10n.t("replay.exportClip")).tag("Clip")
                    Text(L10n.t("replay.exportPlaylist")).tag("Playlist")
                    Text(L10n.t("replay.exportAngle")).tag("Angle")
                    Text(L10n.t("replay.exportAllAngles")).tag("AllAngles")
                }
                .labelsHidden()
                .frame(width: 170)
                TextField(L10n.t("replay.exportPath"), text: $exportPath)
                    .frame(width: 250)
                Button(L10n.t("replay.browse")) { chooseExportPath() }
                Button(L10n.t("replay.exportStart")) { startExport() }
                    .disabled(exporting)
                Button(L10n.t("replay.exportCancel")) { cancelExport() }
                    .disabled(!exporting)
            }
            if !exportStatus.isEmpty {
                Text(exportStatus).font(.caption)
            }
            if !health.isEmpty {
                Text(health).font(.caption).foregroundStyle(EivizTheme.dim)
            }
        }
    }

    /// One file to write. Several angles become several files, written in turn
    /// because the store runs one export at a time.
    private struct ReplayExportJob {
        let json: String
        let label: String
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
        mixer.cancelReplayExport()
        exportQueue = []
        exporting = false
    }

    private func startExport() {
        guard let document else { return }
        let path = exportPath.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !path.isEmpty else {
            exportStatus = L10n.t("replay.exportNeedPath")
            return
        }
        let segments = exportSegments(mode: exportMode, document: document)
        guard !segments.isEmpty else {
            exportStatus = L10n.t("replay.exportNothing")
            return
        }
        exportQueue = []
        if exportMode == "AllAngles" {
            // One file per angle, so a multi-camera export stays usable.
            let base = (path as NSString).deletingPathExtension
            let ext = (path as NSString).pathExtension
            for angle in exportAngles(document) {
                let target = "\(base)-angle\(angle.id).\(ext)"
                exportQueue.append(
                    ReplayExportJob(
                        json: exportJson(path: target, document: document, segments: segmentsFor(angle, in: segments)),
                        label: (target as NSString).lastPathComponent
                    )
                )
            }
        } else {
            exportQueue = [
                ReplayExportJob(
                    json: exportJson(path: path, document: document, segments: segments),
                    label: (path as NSString).lastPathComponent
                )
            ]
        }
        startNextExport()
    }

    private func startNextExport() {
        guard !exportQueue.isEmpty else {
            exporting = false
            return
        }
        let job = exportQueue.removeFirst()
        guard mixer.startReplayExport(job.json) else {
            exporting = false
            exportQueue = []
            exportStatus = L10n.t("replay.exportFailed")
            return
        }
        exporting = true
        exportStatus = L10n.format("replay.exportStarted", job.label)
    }

    /// Export progress, and the queue that follows it.
    private func refreshExport() {
        guard exporting, let progress = mixer.replayExportProgress() else { return }
        switch progress.state {
        case .running:
            exportStatus = L10n.format(
                "replay.exportRunning",
                "\(progress.done)",
                "\(progress.total)",
                "\(progress.skipped)"
            )
        case .done:
            if exportQueue.isEmpty {
                exporting = false
                exportStatus = L10n.format(
                    "replay.exportDone",
                    "\(progress.done)",
                    "\(progress.skipped)"
                )
            } else {
                startNextExport()
            }
        case .failed:
            exporting = false
            exportQueue = []
            exportStatus = L10n.format("replay.exportError", progress.message)
        case .cancelled:
            exporting = false
            exportQueue = []
            exportStatus = L10n.t("replay.exportCancelled")
        case .idle:
            break
        }
    }

    /// Segments the chosen mode writes, in order.
    private func exportSegments(mode: String, document: ReplayDocument) -> [(UInt32, UInt64, UInt64)] {
        var result: [(UInt32, UInt64, UInt64)] = []
        switch mode {
        case "Clip":
            if let uid = selectedClip, let clip = clips.first(where: { $0.uid == uid }) {
                result.append((clip.angleId, clip.inFrame, clip.outFrame))
            }
        case "Playlist":
            for clip in clips {
                result.append((clip.angleId, clip.inFrame, clip.outFrame))
            }
        case "Angle", "AllAngles":
            for angle in exportAngles(document) {
                if let range = mixer.replayAngleRange(angle.id) {
                    result.append((angle.id, range.first, range.last))
                }
                if mode == "Angle" {
                    break
                }
            }
        default:
            break
        }
        return result
    }

    private func segmentsFor(
        _ angle: ReplayAngleInfo,
        in segments: [(UInt32, UInt64, UInt64)]
    ) -> [(UInt32, UInt64, UInt64)] {
        segments.filter { $0.0 == angle.id }
    }

    /// Angles in the order an export should walk them: the one on air first.
    private func exportAngles(_ document: ReplayDocument) -> [ReplayAngleInfo] {
        let active = document.activeAngleId
        let angles = document.angles
        guard active != 0 else { return angles }
        return angles.sorted { ($0.id == active ? 0 : 1) < ($1.id == active ? 0 : 1) }
    }

    private func exportJson(
        path: String,
        document: ReplayDocument,
        segments: [(UInt32, UInt64, UInt64)]
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
        return """
        {"path":\(ReplayJson.string(path)),"width":\(width),"height":\(height),\
        "fpsNum":\(fps.num),"fpsDen":\(fps.den),"bitrate":\(bitrate),\
        "sampleRate":48000,"channels":2,"withAudio":true,"segments":[\(list)]}
        """
    }

    private var transport: some View {
        VStack(spacing: 8) {
            waveform
            exportBar
            HStack {
                Button(L10n.t("replay.markIn")) { markIn() }
                    .keyboardShortcut("i", modifiers: [])
                    .help(L10n.t("replay.markInHint"))
                    .frame(width: 160, height: 48)
                Button(L10n.t("replay.markOut")) { markOut() }
                    .keyboardShortcut("o", modifiers: [])
                    .help(L10n.t("replay.markOutHint"))
                    .frame(width: 160, height: 48)
                    .disabled(!markOpen)
                Button(L10n.t("replay.cancelMark")) { cancelMark() }
                    .frame(width: 120, height: 48)
                Spacer()
                VStack(alignment: .leading) {
                    Text(L10n.t("replay.in")).font(.caption)
                    Text(inFrame.map { ReplayTime.timecode($0, fps) } ?? "--:--:--:--")
                        .font(.system(.body, design: .monospaced))
                }
                VStack(alignment: .leading) {
                    Text(L10n.t("replay.out")).font(.caption)
                    Text(outFrame.map { ReplayTime.timecode($0, fps) } ?? "--:--:--:--")
                        .font(.system(.body, design: .monospaced))
                }
                VStack(alignment: .leading) {
                    Text(L10n.t("replay.duration")).font(.caption)
                    // The length of the mark, which is what the operator cut.
                    Text(markDuration)
                        .font(.system(.body, design: .monospaced))
                }
            }
            HStack {
                Button(L10n.t("replay.record")) { record(true) }
                Button(L10n.t("replay.stop")) { record(false) }
                Button(L10n.t("replay.live")) { live() }
                    .keyboardShortcut("l", modifiers: [])
                Button(L10n.t(playing ? "replay.pause" : "replay.play")) { togglePlay() }
                    .keyboardShortcut(.space, modifiers: [])
                Button("◀◀") { step(-1) }
                    .keyboardShortcut(.leftArrow, modifiers: [])
                    .help(L10n.t("replay.stepBackHint"))
                Button("▶▶") { step(1) }
                    .keyboardShortcut(.rightArrow, modifiers: [])
                    .help(L10n.t("replay.stepForwardHint"))
                Button("25%") { setSpeed(250) }
                Button("50%") { setSpeed(500) }
                Button("100%") { setSpeed(1000) }
                Spacer()
                Text(L10n.t("replay.jog"))
                Slider(value: $jogValue, in: -30...30, step: 1) { editing in
                    if !editing {
                        jog(Int(jogValue.rounded()))
                        jogValue = 0
                    }
                }
                .frame(width: 200)
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
        let active = document?.activeAngleId ?? 0
        let wanted = active != 0 ? active : (document?.angles.first?.id ?? 0)
        let issues = ReplayIssues.text(snapshot.stats.issues)
        guard let angle = snapshot.angles.first(where: { $0.angle_id == wanted }) else {
            // Without a selected angle there is no delivery figure, but why the
            // recorder is unhappy is still worth reading.
            health = issues
            return
        }
        var line = L10n.format(
            "replay.healthDelivery",
            "\(angle.frames_written)",
            "\(snapshot.stats.frames_offered)",
            "\(snapshot.stats.frames_pushed)",
            "\(angle.dropped_readback)"
        ) + "  " + L10n.format(
            "replay.healthDisk",
            "\(angle.dropped_encode)",
            "\(angle.dropped_write)",
            "\(angle.segments)",
            "\(angle.encode_queue_len)",
            "\(angle.encode_queue_cap)",
            "\(angle.write_latency_p95_ms)"
        )
        // The numbers say how much was lost; the issues say why.
        if !issues.isEmpty {
            line += "  " + issues
        }
        if snapshot.stats.frames_dropped_recent > 0 {
            line += "  " + L10n.format("replay.dropped", "\(snapshot.stats.frames_dropped_recent)")
        }
        health = line
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
    }

    private func save(_ updated: ReplayDocument) {
        document = updated
        loadedJson = updated.json
        mixer.applyReplay(updated)
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
        current.clearSourceEvent(id)
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
            status = L10n.t("replay.cueRefused")
        }
    }

    private func removeSelectedClip() {
        guard var current = document, let uid = selectedClip else { return }
        current.removeClip(playlistId: playlistId, uid: uid)
        // An animation step that cued this clip has nothing left to fire.
        mixer.clearReplayCues(clipUid: uid)
        save(current)
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
        if !mixer.replayJumpToNow(channel: channel) {
            status = L10n.t("replay.commandRefused")
        }
        playing = false
    }

    private func togglePlay() {
        let next = !playing
        if mixer.replayTransport(channel: channel, playing: next, speedMilli: speedMilli, seekFrame: -1) {
            playing = next
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
        var target = max(0, Int64(start) + delta)
        if let end = outFrame, end > start {
            target = min(target, Int64(end))
        }
        if !mixer.replayTransport(channel: channel, playing: false, speedMilli: speedMilli, seekFrame: target) {
            status = L10n.t("replay.commandRefused")
        }
        playing = false
    }

    private func step(_ delta: Int64) {
        seekMarked(delta)
    }

    private func setSpeed(_ milli: Int32) {
        speedMilli = milli
        if !mixer.replayTransport(channel: channel, playing: playing, speedMilli: speedMilli, seekFrame: -1) {
            status = L10n.t("replay.commandRefused")
        }
    }

    private func jog(_ delta: Int) {
        guard delta != 0 else { return }
        seekMarked(Int64(delta))
    }
}

/// Project-scoped replay configuration. It only exists while the project has a
/// Replay input, so it is a dedicated window rather than a settings tab.
struct ReplaySettingsView: View {
    @EnvironmentObject private var mixer: MixerController

    @State private var document: ReplayDocument?
    @State private var storages: [ReplayDocument.StorageSpec] = []
    @State private var profile = "Sq"
    /// Segment length in frames, as text so the operator can type it.
    @State private var segmentFrames = "120"
    @State private var retention = "StopWhenFull"
    @State private var queue = "6"
    @State private var buffer = "512"
    @State private var reserve = "2048"
    @State private var overflow = "DropFrames"
    @State private var autoAdd = false
    @State private var angles: [ReplayAngleInfo] = []
    @State private var status = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(planText).font(.callout)
            if !status.isEmpty {
                // Apply and every refusal report here, where it cannot be missed.
                Text(status).fontWeight(.bold).foregroundStyle(EivizTheme.dim)
            }
            GroupBox(L10n.t("replay.storage")) {
                VStack(alignment: .leading, spacing: 6) {
                    Text(L10n.t("replay.storageHelp"))
                        .font(.caption)
                        .foregroundStyle(EivizTheme.dim)
                    // Each drive is edited in its own row: the path is the row's text field,
                    // and the angles that record there are shown in the same row.
                    List {
                        ForEach($storages) { $storage in
                            HStack(spacing: 6) {
                                TextField(L10n.t("settings.replayPath"), text: $storage.path)
                                    .frame(minWidth: 240)
                                    .help(L10n.t("replay.pickStorageHint"))
                                Text(storage.angleIds.map(String.init).joined(separator: ", "))
                                    .foregroundStyle(EivizTheme.dim)
                                Spacer()
                                Button(L10n.t("replay.browse")) { browseInto(storage.id) }
                                Button(L10n.t("replay.removeStorage")) { removeStorageRow(storage.id) }
                            }
                        }
                    }
                    .frame(minHeight: 90)
                    Button(L10n.t("replay.addStorageRow")) { browseInto(nil) }
                    HStack {
                        Picker(L10n.t("settings.replayProfile"), selection: $profile) {
                            Text("HQ").tag("Hq")
                            Text("SQ").tag("Sq")
                            Text("LQ").tag("Lq")
                        }
                        Picker(L10n.t("replay.retention"), selection: $retention) {
                            Text(L10n.t("replay.retentionStop")).tag("StopWhenFull")
                            Text(L10n.t("replay.retentionOverwrite")).tag("Overwrite")
                            Text(L10n.t("replay.retentionProtect")).tag("OverwriteUnprotected")
                        }
                    }
                    HStack {
                        TextField(L10n.t("settings.replayQueue"), text: $queue).frame(width: 80)
                        TextField(L10n.t("settings.replayBuffer"), text: $buffer).frame(width: 90)
                        TextField(L10n.t("replay.reserve"), text: $reserve).frame(width: 100)
                        Picker(L10n.t("replay.overflow"), selection: $overflow) {
                            Text(L10n.t("replay.overflowDrop")).tag("DropFrames")
                            Text(L10n.t("replay.overflowStop")).tag("StopRecording")
                        }
                    }
                    HStack(alignment: .top) {
                        // One format for every angle, taken from the first Mixing Unit.
                        // It is shown rather than edited here.
                        VStack(alignment: .leading, spacing: 2) {
                            Text(L10n.t("replay.formatFromUnit")).font(.caption)
                            Text(formatText).font(.system(.body, design: .monospaced))
                        }
                        VStack(alignment: .leading, spacing: 2) {
                            Text(L10n.t("replay.segmentFrames")).font(.caption)
                            TextField("120", text: $segmentFrames)
                                .frame(width: 90)
                                .help(L10n.t("replay.segmentFramesRange"))
                        }
                    }
                    Text(L10n.t("replay.retentionHelp")).font(.caption)
                }
            }
            GroupBox(L10n.t("replay.angles")) {
                VStack(alignment: .leading, spacing: 6) {
                    Text(L10n.t("replay.anglesHelp")).font(.caption)
                    // Every angle is edited in its own row: name, what it records, how, its
                    // audio and its drive are cells, not a separate panel.
                    List {
                        ForEach($angles) { $angle in
                            HStack(spacing: 6) {
                                TextField(L10n.t("replay.name"), text: $angle.name)
                                    .frame(width: 130)
                                Picker("", selection: $angle.source) {
                                    Text(L10n.t("replay.sourceScene")).tag("Scene")
                                    Text(L10n.t("replay.sourceInput")).tag("Input")
                                }
                                .labelsHidden()
                                .frame(width: 90)
                                .onChange(of: angle.source) { _, _ in resetAngleSource(angle.id) }
                                Picker("", selection: sourceBinding($angle)) {
                                    ForEach(sources(for: angle), id: \.id) { source in
                                        Text(source.name).tag(source.id)
                                    }
                                }
                                .labelsHidden()
                                .frame(width: 170)
                                .onChange(of: angle.sceneId) { _, _ in normalizeAngleMode(angle.id) }
                                .onChange(of: angle.inputId) { _, _ in normalizeAngleMode(angle.id) }
                                Picker("", selection: $angle.recordMode) {
                                    Text(L10n.t("replay.recordComposite")).tag("Composite")
                                    if angleIsOmt(angle) {
                                        Text(L10n.t("replay.recordPassthrough")).tag("Passthrough")
                                    }
                                }
                                .labelsHidden()
                                .frame(width: 120)
                                .help(L10n.t("replay.passthroughHelp"))
                                Picker("", selection: $angle.audioUnitId) {
                                    Text(L10n.t("replay.audioSilent")).tag(UInt64(0))
                                    ForEach(mixer.session.units) { unit in
                                        Text(unit.name).tag(unit.id)
                                    }
                                }
                                .labelsHidden()
                                .frame(width: 140)
                                .help(L10n.t("replay.audioHelp"))
                                Picker("", selection: driveBinding(angle.id)) {
                                    Text(L10n.t("replay.driveUnassigned")).tag(-1)
                                    ForEach(Array(storages.enumerated()), id: \.offset) { index, storage in
                                        Text("\(index + 1): \(storage.path)").tag(index)
                                    }
                                }
                                .labelsHidden()
                                .frame(width: 150)
                                .help(L10n.t("replay.storageHelp"))
                                Button(L10n.t("replay.removeAngle")) { removeAngle(angle.id) }
                            }
                        }
                    }
                    .frame(minHeight: 150)
                    HStack {
                        Button(L10n.t("replay.addAngleRow")) { addAngle() }
                        Text(L10n.t("replay.angleRowHelp")).font(.caption)
                    }
                }
            }
            Toggle(L10n.t("replay.autoAdd"), isOn: $autoAdd)
            Text(L10n.t("replay.autoAddHelp")).font(.caption)
            if let active = document?.activeAngleId, active != 0 {
                Text(L10n.format("replay.activeAngle", document?.angleLabel(active) ?? ""))
            } else {
                Text(L10n.t("replay.noActiveAngle"))
            }
            if !status.isEmpty {
                Text(status).font(.caption)
            }
            Spacer()
            HStack {
                Spacer()
                // Cancelling discards the edits: nothing was published, so the
                // window just goes away.
                Button(L10n.t("dialog.cancel")) { mixer.closeReplaySettings() }
                Button(L10n.t("replay.apply")) { apply() }
                    .keyboardShortcut(.defaultAction)
            }
        }
        .padding(14)
        .frame(minWidth: 620, minHeight: 520)
        .onAppear { load() }
    }

    private var planText: String {        guard let limits = mixer.replayLimits() else { return "" }
        if limits.angles == 0 || limits.channels == 0 {
            return L10n.t("replay.planDenied")
        }
        return L10n.format("replay.planLimits", "\(limits.angles)", "\(limits.channels)")
    }

    /// The format Replay records in: the first Mixing Unit's picture and rate. It is
    /// shown rather than edited, because every angle shares one format.
    private var formatText: String {
        guard let unit = mixer.session.units.first else {
            return L10n.t("replay.needMixingUnit")
        }
        return "\(unit.width) x \(unit.height)   \(unit.fpsNum)/\(unit.fpsDen)"
    }

    private func sceneName(_ sceneId: UInt64) -> String {
        mixer.session.scenes.first { $0.id == sceneId }?.name
            ?? L10n.format("replay.sceneMissing", "\(sceneId)")
    }

    private func load() {
        guard let current = mixer.replayDocument else {
            status = L10n.t("replay.missing")
            return
        }
        document = current
        storages = current.storages
        profile = current.profile
        segmentFrames = "\(current.segmentFrames)"
        retention = current.retention
        let buffering = current.buffering
        queue = "\(buffering.queue)"
        buffer = "\(buffering.buffer)"
        reserve = "\(buffering.reserve)"
        overflow = buffering.overflow
        autoAdd = current.autoAddToPlaylist
        angles = current.angles
    }

    private func inputKind(_ inputId: UInt64) -> InputKind? {
        mixer.session.inputs.first { $0.id == inputId }?.kind
    }

    private func inputName(_ inputId: UInt64) -> String {
        mixer.session.inputs.first { $0.id == inputId }?.name
            ?? L10n.format("replay.inputMissing", "\(inputId)")
    }

    /// One entry of the add row's source list.
    private struct SourceOption: Identifiable {
        let id: UInt64
        let name: String
    }

    /// What a row can record: scenes, or the inputs that have a picture. A Replay
    /// channel is not a camera, and an audio input has nothing to record.
    private func sources(for angle: ReplayAngleInfo) -> [SourceOption] {
        if angle.source == "Input" {
            return mixer.session.inputs
                .filter { $0.kind != .replay && $0.kind != .audio }
                .map { SourceOption(id: $0.id, name: $0.name) }
        }
        return mixer.session.scenes.map { SourceOption(id: $0.id, name: $0.name) }
    }

    /// The scene or input a row points at, depending on its kind. The row keeps one id
    /// per kind so switching back and forth does not lose the other choice.
    private func sourceBinding(_ angle: Binding<ReplayAngleInfo>) -> Binding<UInt64> {
        Binding(
            get: {
                angle.wrappedValue.source == "Input"
                    ? angle.wrappedValue.inputId
                    : angle.wrappedValue.sceneId
            },
            set: { value in
                if angle.wrappedValue.source == "Input" {
                    angle.wrappedValue.inputId = value
                } else {
                    angle.wrappedValue.sceneId = value
                }
            }
        )
    }

    /// Which drive a row records to. The document keeps the assignment on the drives, so
    /// this reads and writes there rather than on the angle.
    private func driveBinding(_ angleId: UInt32) -> Binding<Int> {
        Binding(
            get: { driveOf(angleId) },
            set: { index in assignDrive(angleId, index) }
        )
    }

    private func angleIsOmt(_ angle: ReplayAngleInfo) -> Bool {
        angle.source == "Input" && inputKind(angle.inputId) == .omt
    }

    /// A new kind starts from the first source of that kind.
    private func resetAngleSource(_ angleId: UInt32) {
        guard let index = angles.firstIndex(where: { $0.id == angleId }) else { return }
        var angle = angles[index]
        let first = sources(for: angle).first?.id ?? 0
        if angle.source == "Input" {
            angle.inputId = first
        } else {
            angle.sceneId = first
        }
        angle.recordMode = "Composite"
        angles[index] = angle
    }

    /// Keeps the mode honest: passthrough exists only for an OMT input.
    private func normalizeAngleMode(_ angleId: UInt32) {
        guard let index = angles.firstIndex(where: { $0.id == angleId }) else { return }
        if angles[index].recordMode == "Passthrough" && !angleIsOmt(angles[index]) {
            angles[index].recordMode = "Composite"
        }
    }

    /// Moves an angle to a drive. The assignment lives on the drives, so it is rewritten
    /// there: an angle belongs to exactly one.
    private func assignDrive(_ angleId: UInt32, _ index: Int) {
        for slot in storages.indices {
            storages[slot].angleIds.removeAll { $0 == angleId }
        }
        if index >= 0 && index < storages.count {
            storages[index].angleIds.append(angleId)
        }
    }

    private func driveIndex(_ storage: ReplayDocument.StorageSpec) -> Int {
        storages.firstIndex(of: storage) ?? 0
    }

    /// Drive an angle records to. Angles that name no drive report zero, which the
    /// list shows as such: Apply refuses a setup with an unassigned angle.
    private func driveOf(_ angleId: UInt32) -> Int {
        storages.firstIndex { $0.angleIds.contains(angleId) } ?? -1
    }

    /// Asks for a folder the way every other path on this host is chosen. Picking one
    /// fills the row it was asked from, or adds a row when none was named.
    private func browseInto(_ storageId: String?) {
        let panel = NSOpenPanel()
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.allowsMultipleSelection = false
        panel.canCreateDirectories = true
        panel.prompt = L10n.t("replay.pickStorage")
        panel.message = L10n.t("replay.pickStorageHint")
        guard panel.runModal() == .OK, let url = panel.url else { return }
        let trimmed = url.path.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        guard !storages.contains(where: { $0.id != storageId && $0.path.caseInsensitiveCompare(trimmed) == .orderedSame }) else {
            status = L10n.t("replay.duplicateStorage")
            return
        }
        if let storageId, let index = storages.firstIndex(where: { $0.id == storageId }) {
            storages[index].path = trimmed
        } else {
            storages.append(ReplayDocument.StorageSpec(path: trimmed, angleIds: []))
            // The first drive takes the angles that have none: a setup with one drive
            // should not need a second step per angle.
            if storages.count == 1 {
                storages[0].angleIds = angles.map(\.id).sorted()
            }
        }
        status = L10n.format("replay.storageAdded", trimmed)
    }

    /// Removes a drive by its row, moving any angle that recorded there to the first one.
    private func removeStorageRow(_ storageId: String) {
        guard storages.count > 1, let index = storages.firstIndex(where: { $0.id == storageId }) else {
            status = L10n.t("replay.needStorable")
            return
        }
        let removed = storages.remove(at: index)
        for angle in removed.angleIds {
            assignDrive(angle, 0)
        }
    }

    /// Audio source label for the angle list.
    private func audioLabel(_ unitId: UInt64) -> String {
        if unitId == 0 {
            return L10n.t("replay.audioSilent")
        }
        return mixer.session.units.first { $0.id == unitId }?.name
            ?? L10n.format("replay.audioUnit", "\(unitId)")
    }

    /// Adds a row for a new angle. It starts on the next unused scene, so the common case
    /// needs no edits at all, and the row is there to be changed in place if it is not the
    /// one they wanted.
    private func addAngle() {
        guard !storages.isEmpty else {
            status = L10n.t("replay.needDriveForAngle")
            return
        }
        if let limits = mixer.replayLimits(), limits.angles != .max, angles.count >= Int(limits.angles) {
            status = L10n.format("replay.planLimits", "\(limits.angles)", "\(limits.channels)")
            return
        }
        let used = Set(angles.filter { $0.source == "Scene" }.map(\.sceneId))
        let scene = mixer.session.scenes.first { !used.contains($0.id) }
        let id = (angles.map(\.id).max() ?? 0) + 1
        angles.append(
            ReplayAngleInfo(
                id: id,
                name: scene?.name ?? "",
                source: "Scene",
                sceneId: scene?.id ?? 0,
                inputId: 0,
                recordMode: "Composite",
                audioUnitId: 0
            )
        )
        // A single drive takes the new angle: a setup with one drive should not need a
        // second step per angle.
        if storages.count == 1 {
            storages[0].angleIds.append(id)
        }
        status = L10n.format("replay.angleAdded", scene?.name ?? "")
    }

    /// Removes one angle by id, from its own row's button.
    private func removeAngle(_ angleId: UInt32) {
        guard let angle = angles.first(where: { $0.id == angleId }) else { return }
        // A clip that plays this angle, or takes its sound from it, would be left
        // pointing at nothing, so the angle waits until those clips are gone.
        let users = clipCount(using: angle.id)
        if users > 0 {
            status = L10n.format("replay.angleInUse", "\(users)")
            return
        }
        angles.removeAll { $0.id == angle.id }
        // A drive that still listed it would fail validation on Apply.
        for index in storages.indices {
            storages[index].angleIds.removeAll { $0 == angle.id }
        }
    }

    /// How many clips play this angle, or take their sound from it.
    private func clipCount(using angleId: UInt32) -> Int {
        guard let document else { return 0 }
        var count = 0
        for playlistId in document.playlistIds {
            for clip in document.clips(playlistId: playlistId) {
                let picture = clip.angleId == angleId
                let audio = clip.audioKind == "Angle" && clip.audioAngleId == angleId
                if picture || audio {
                    count += 1
                }
            }
        }
        return count
    }

    private func apply() {
        guard var current = document else { return }
        guard let queueValue = UInt32(queue), (2...60).contains(queueValue),
              let bufferValue = UInt32(buffer), (64...8192).contains(bufferValue),
              let reserveValue = UInt32(reserve)
        else {
            status = L10n.t("replay.badBuffering")
            return
        }
        // The segment length is the operator's setting: the recorder no longer
        // substitutes one when it is missing or out of range.
        guard let segmentValue = UInt32(segmentFrames), (30...1800).contains(segmentValue) else {
            status = L10n.t("replay.segmentFramesRange")
            return
        }
        guard let unit = mixer.session.units.first else {
            // Every angle records one format, and the format comes from the Mixing
            // Unit. Without one there is nothing to record in.
            status = L10n.t("replay.needMixingUnit")
            return
        }
        let usable = storages.filter { !$0.path.trimmingCharacters(in: .whitespaces).isEmpty }
        if !usable.isEmpty && angles.isEmpty {
            status = L10n.t("replay.needStorable")
            return
        }
        // Only angles that still exist are assigned, so a removed one cannot be
        // left behind on a drive and fail validation.
        let known = Set(angles.map { $0.id })
        for index in storages.indices {
            storages[index].angleIds.removeAll { !known.contains($0) }
        }
        // One drive takes every angle: the operator added the drive for this
        // recording, and a recording never guesses which disk to write to.
        if storages.count == 1 {
            storages[0].angleIds = angles.map(\.id).sorted()
        }
        current.setStorages(storages)
        var format = current.raw["format"]?.objectValue ?? [:]
        // The whole format travels: a document that keeps a zero resolution or a zero
        // segment length can never record, and the mixer would only be able to say so
        // much later.
        format["width"] = .number(Double(unit.width))
        format["height"] = .number(Double(unit.height))
        format["fpsNum"] = .number(Double(unit.fpsNum))
        format["fpsDen"] = .number(Double(unit.fpsDen))
        format["profile"] = .string(profile)
        format["segmentFrames"] = .number(Double(segmentValue))
        current.raw["format"] = .object(format)
        current.raw["buffering"] = .object([
            "encodeQueueFrames": .number(Double(queueValue)),
            "writeBufferMb": .number(Double(bufferValue)),
            "overflowPolicy": .string(overflow),
            "reserveMb": .number(Double(reserveValue)),
        ])
        current.setRetention(retention)
        current.setAngles(angles)
        current.setAutoAddToPlaylist(autoAdd)
        document = current
        // Applying either works or says why. A window that closes without a word
        // leaves the operator unable to tell the two apart.
        if mixer.applyReplay(current) {
            AppKitDialog.toast(L10n.t("replay.applied"))
            mixer.closeReplaySettings()
        } else {
            status = L10n.t("replay.publishRefused")
        }
    }
}
