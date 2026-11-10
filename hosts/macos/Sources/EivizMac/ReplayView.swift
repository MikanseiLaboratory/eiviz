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
    /// Show every angle as a live picture instead of the selected one, the way the
    /// Windows deck's Multiview does.
    @State private var multiview = false
    /// Monitor ids of the deck's output and live-input pictures.
    @State private var outputMonitor: UInt64 = 0
    @State private var liveMonitor: UInt64 = 0
    @State private var peakMins: [Float] = []
    @State private var peakMaxes: [Float] = []
    /// Buckets the mixer evicted, so the first bucket on screen is this many into
    /// the recording. The marks are placed with it.
    @State private var peakDropped: UInt32 = 0
    @State private var jogValue: Double = 0
    @State private var showExport = false
    @State private var markLastSeconds = "10"
    @State private var clipOrder = "Append"
    @State private var playingClipUid: String?
    /// The channel's playhead, as the mixer reports it. This is what the seek bar moves;
    /// the recording head is where the recorder is writing and must not drive the bar.
    @State private var playheadFrame: UInt64 = 0
    /// The out point of the clip the channel is playing, so "remaining in clip" does not
    /// depend on a Mark Out the operator may not have taken.
    @State private var channelOutFrame: UInt64 = 0
    /// While the operator drags the bar the engine has not moved yet, so the drag value
    /// owns the bar until the seek lands.
    @State private var scrubFrame: Double?
    @State private var sliderEditing = false

    private let tick = Timer.publish(every: 0.2, on: .main, in: .common).autoconnect()

    var body: some View {
        VStack(spacing: 8) {
            deckHeader
            deckPictures
            deckLists
            deckTransport
        }
        .padding(10)
        .frame(minWidth: 1280, minHeight: 720)
        .onAppear { load() }
        .onReceive(tick) { _ in refresh() }
        .onChange(of: channel) { _, _ in
            // A different channel has its own playhead; carrying the last one over made
            // the slider and the readouts describe a channel that is no longer selected.
            scrubFrame = nil
            playheadFrame = 0
            channelOutFrame = 0
            playingClipUid = nil
            selectedClip = nil
            transport = .atEdge
        }
    }

    // MARK: - Windows deck layout

    private var deckHeader: some View {
        HStack(spacing: 8) {
            Text(L10n.t("replay.channel"))
                .font(.system(size: 11))
                .foregroundStyle(Color(white: 0.54))
            Picker("", selection: $channel) {
                ForEach(channels) { item in Text(item.label).tag(item.index) }
            }
            .labelsHidden()
            .frame(width: 110, height: 30)
            Divider().frame(height: 24)
            Text(stateText)
                .font(.system(size: 14, weight: .bold))
                .foregroundStyle(isRecording ? Color.red : EivizTheme.text)
            Spacer(minLength: 8)
            Text(capacityLine)
                .font(.system(size: 11))
                .foregroundStyle(Color(white: 0.54))
                .lineLimit(1)
                .truncationMode(.tail)
                .frame(maxWidth: 200)
                .layoutPriority(0)
            Spacer(minLength: 8)
            HStack(spacing: 6) {
                Text(L10n.t("replay.recordTc"))
                    .font(.system(size: 11))
                    .foregroundStyle(Color(white: 0.54))
                Text(ReplayTime.timecode(recordingHead, fps))
                    .font(.system(size: 16, design: .monospaced))
                    .foregroundStyle(Color(red: 0.49, green: 0.82, blue: 0.29))
            }
            .padding(.horizontal, 6)
            .padding(.vertical, 2)
            .background(Color.black.opacity(0.22))
            .clipShape(RoundedRectangle(cornerRadius: 3))
            .layoutPriority(2)
            Text(String(format: "%.2f fps", ReplayTime.rate(fps)))
                .font(.system(size: 11))
                .foregroundStyle(Color(white: 0.54))
            Button(L10n.t("replay.capacityRun")) {}
                .disabled(true)
                .frame(minWidth: 130, minHeight: 30)
                .help(L10n.t("replay.capacityIdle"))
                .layoutPriority(2)
            Button(L10n.t("replay.openSetup")) { mixer.openReplaySettings() }
                .frame(minWidth: 130, minHeight: 30)
                .layoutPriority(2)
        }
    }

    /// The frame the recorder is on. The test store reports the recording store's head in
    /// last_frame and leaves recording_frame at zero, so the deck reads whichever is ahead.
    private var recordingHead: UInt64 {
        max(stats?.recording_frame ?? 0, stats?.last_frame ?? 0)
    }

    /// What the deck's monitors present at. Zero in the session means "follow the
    /// project default", exactly as the Windows settings window describes it.
    private var deckPresentInterval: UInt32 {
        let settings = mixer.session.settings
        let interval = settings.replayPresentInterval == 0
            ? settings.defaultPresentInterval
            : settings.replayPresentInterval
        return max(1, min(8, interval))
    }

    private var seekRange: ClosedRange<Double> {
        if let angle = activeAngle,
           let range = mixer.replayAngleRange(angle.id),
           range.last > range.first {
            let upper = max(range.last, playheadFrame, recordingHead)
            return Double(range.first)...Double(max(1, upper))
        }
        return 0...Double(max(1, stats?.last_frame ?? 1, playheadFrame))
    }

    private var displayPlayhead: UInt64 {
        scrubFrame.map { UInt64(max(0, $0.rounded())) } ?? playheadFrame
    }

    private var capacityLine: String {
        guard let snapshot = stats else { return "" }
        let free = ByteCountFormatter.string(
            fromByteCount: Int64(snapshot.free_bytes), countStyle: .file)
        if snapshot.remaining_secs == UInt32.max { return free }
        return "\(free) / \(ReplayTime.clock(snapshot.remaining_secs))"
    }

    private var deckPictures: some View {
        HStack(alignment: .top, spacing: 8) {
            VStack(alignment: .leading, spacing: 4) {
                HStack {
                    Text(L10n.t("replay.liveInput"))
                        .font(.system(size: 12, weight: .bold))
                        .foregroundStyle(Color(white: 0.78))
                    Spacer()
                    Button(L10n.t("replay.multiview")) { multiview.toggle() }
                        .controlSize(.small)
                        .tint(multiview ? .green : nil)
                }
                Group {
                    if multiview {
                        angleMultiview
                    } else {
                        livePicture
                    }
                }
                .frame(maxWidth: .infinity)
                Text(liveCaption)
                    .font(.system(size: 11))
                    .foregroundStyle(Color(white: 0.69))
                    .lineLimit(1)
                angleRow
            }
            .frame(maxWidth: .infinity)
            VStack(alignment: .leading, spacing: 4) {
                Text(L10n.t("replay.output"))
                    .font(.system(size: 12, weight: .bold))
                    .foregroundStyle(Color(white: 0.78))
                outputPicture
                Text(outputCaption)
                    .font(.system(size: 11))
                    .foregroundStyle(Color(white: 0.69))
                    .lineLimit(1)
            }
            .frame(maxWidth: .infinity)
        }
    }

    private var livePicture: some View {
        Group {
            if liveMonitor != 0, let angle = activeAngle {
                MetalPreviewRepresentable(
                    role: .monitor(monitorId: liveMonitor, sourceId: angle.liveSourceId),
                    presentInterval: deckPresentInterval
                )
                .aspectRatio(16.0 / 9.0, contentMode: .fit)
                .background(Color.black)
            } else {
                Color.black.aspectRatio(16.0 / 9.0, contentMode: .fit)
            }
        }
        .overlay(RoundedRectangle(cornerRadius: 4).stroke(EivizTheme.stroke))
    }

    private var outputPicture: some View {
        Group {
            if outputMonitor != 0, channelSource != 0 {
                MetalPreviewRepresentable(
                    role: .monitor(monitorId: outputMonitor, sourceId: channelSource),
                    presentInterval: deckPresentInterval
                )
                .aspectRatio(16.0 / 9.0, contentMode: .fit)
                .background(Color.black)
            } else {
                Color.black.aspectRatio(16.0 / 9.0, contentMode: .fit)
            }
        }
        .overlay(RoundedRectangle(cornerRadius: 4).stroke(EivizTheme.stroke))
    }

    private var angleRow: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 6) {
                ForEach(Array(angles.enumerated()), id: \.element.id) { index, angle in
                    let selected = (document?.activeAngleId ?? 0) == angle.id
                    Button {
                        selectAngle(angle.id)
                    } label: {
                        ZStack(alignment: .bottomLeading) {
                            if let monitor = angleMonitors[angle.id], monitor != 0 {
                                MetalPreviewRepresentable(
                                    role: .monitor(monitorId: monitor, sourceId: angle.liveSourceId),
                                    presentInterval: deckPresentInterval
                                )
                                .frame(width: 76, height: 40)
                                .clipped()
                                .allowsHitTesting(false)
                            } else {
                                Color.black
                            }
                            Text("\(index + 1)")
                                .font(.system(size: 11, weight: .bold))
                                .foregroundStyle(.white)
                                .padding(.horizontal, 3)
                                .background(Color.black.opacity(0.55))
                        }
                        .frame(width: 76, height: 40)
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .contentShape(Rectangle())
                    .overlay(
                        RoundedRectangle(cornerRadius: 4)
                            .stroke(selected ? Color.green : EivizTheme.stroke,
                                    lineWidth: selected ? 2 : 1)
                    )
                    .help("\(index + 1): \(angle.label)")
                }
            }
            .padding(.vertical, 2)
        }
        .frame(height: 48)
    }

    private var deckLists: some View {
        HStack(alignment: .top, spacing: 8) {
            eventList
                .frame(minWidth: 360, maxWidth: .infinity, alignment: .topLeading)
            playlist
                .frame(minWidth: 520, maxWidth: .infinity, alignment: .topLeading)
        }
        .frame(maxHeight: .infinity)
    }

    private var deckTransport: some View {
        VStack(spacing: 6) {
            HStack(spacing: 6) {
                Text(L10n.t("replay.defaultTransition"))
                    .font(.system(size: 11))
                    .foregroundStyle(Color(white: 0.54))
                Picker("", selection: $defaultKind) {
                    Text(L10n.t("replay.transitionCut")).tag("Cut")
                    Text(L10n.t("replay.transitionDissolve")).tag("Dissolve")
                }
                .labelsHidden()
                .frame(width: 104, height: 26)
                .onChange(of: defaultKind) { _, _ in applyDefaultTransition() }
                Picker("", selection: $defaultTiming) {
                    Text(L10n.t("replay.timingOverwrap")).tag("Overwrap")
                    Text(L10n.t("replay.timingAdditive")).tag("Additive")
                }
                .labelsHidden()
                .frame(width: 130, height: 26)
                .onChange(of: defaultTiming) { _, _ in applyDefaultTransition() }
                TextField("", text: $defaultFrames)
                    .frame(width: 48, height: 26)
                    .onSubmit { applyDefaultTransition() }
                Text(L10n.t("replay.transitionFrames"))
                    .font(.system(size: 11))
                    .foregroundStyle(Color(white: 0.54))
                Spacer()
                clipTagPanel
            }
            HStack(spacing: 8) {
                Slider(
                    value: Binding(
                        get: { scrubFrame ?? Double(playheadFrame) },
                        set: { scrubFrame = $0 }
                    ),
                    in: seekRange
                ) { editing in
                    sliderEditing = editing
                    guard !editing, let target = scrubFrame else { return }
                    let frame = UInt64(max(0, target.rounded()))
                    playheadFrame = frame
                    // Keep showing the drop point while the mixer catches up, then let
                    // the engine's position own the bar again.
                    seekWhole(to: frame)
                    DispatchQueue.main.asyncAfter(deadline: .now() + 0.8) {
                        if let current = scrubFrame, abs(current - target) < 0.5 {
                            scrubFrame = nil
                        }
                    }
                }
                .help(L10n.t("replay.seekWholeHint"))
                // Keyboard arrows change a Slider's value without starting an edit, so the
                // end-of-drag callback never runs. Commit those changes here.
                .onChange(of: scrubFrame) { _, value in
                    guard !sliderEditing, let value else { return }
                    let frame = UInt64(max(0, value.rounded()))
                    playheadFrame = frame
                    seekWhole(to: frame)
                    DispatchQueue.main.asyncAfter(deadline: .now() + 0.8) {
                        if let current = scrubFrame, abs(current - value) < 0.5 {
                            scrubFrame = nil
                        }
                    }
                }
                Button(L10n.t("replay.jumpLive")) { live() }
                    .frame(minWidth: 130, minHeight: 26)
                    .help(L10n.t("replay.jumpLiveHint"))
            }
            waveform
                .frame(height: 48)
                .accessibilityElement()
                .accessibilityLabel(L10n.t("replay.waveformSeek"))
            HStack(spacing: 12) {
                Spacer()
                transportReadout(
                    L10n.t("replay.playhead"),
                    ReplayTime.timecode(displayPlayhead, fps),
                    color: Color(red: 0.49, green: 0.82, blue: 0.29)
                )
                transportReadout(L10n.t("replay.remainingInClip"), remainingClipText)
                transportReadout(L10n.t("replay.remainingInPlaylist"), remainPlaylistText)
                transportReadout(
                    L10n.t("replay.in"),
                    inFrame.map { ReplayTime.timecode($0, fps) } ?? "--:--:--:--",
                    color: .orange
                )
                transportReadout(
                    L10n.t("replay.out"),
                    outFrame.map { ReplayTime.timecode($0, fps) } ?? "--:--:--:--",
                    color: .orange
                )
                transportReadout(L10n.t("replay.duration"), markDuration)
            }
            HStack(spacing: 8) {
                controlGroup {
                    Button {
                        record(true)
                    } label: {
                        HStack(spacing: 7) {
                            Circle()
                                .fill(isRecording ? Color.red : Color(red: 0.88, green: 0.32, blue: 0.32))
                                .frame(width: 11, height: 11)
                            Text(isRecording ? "REC" : L10n.t("replay.record"))
                        }
                    }
                    .disabled(isRecording)
                    .frame(minWidth: 88, minHeight: 44)
                    Button(L10n.t("replay.stop")) { record(false) }
                        .disabled(!isRecording)
                        .frame(minWidth: 66, minHeight: 44)
                    Button(L10n.t("replay.live")) { live() }
                        .frame(minWidth: 74, minHeight: 44)
                    Button(L10n.t(transport.playing ? "replay.pause" : "replay.play")) { togglePlay() }
                        .frame(minWidth: 84, minHeight: 44)
                    Button("◀") { step(-1) }
                        .frame(width: 44, height: 44)
                        .help(L10n.t("replay.stepBackHint"))
                    Button("▶") { step(1) }
                        .frame(width: 44, height: 44)
                        .help(L10n.t("replay.stepForwardHint"))
                }
                controlGroup {
                    Button(L10n.t("replay.markIn")) { markIn() }
                        .tint(inFrame != nil ? .orange : nil)
                        .frame(width: 96, height: 44)
                    Button(L10n.t("replay.markOut")) { markOut() }
                        .tint(outFrame != nil ? .orange : nil)
                        .frame(width: 96, height: 44)
                        .disabled(!markOpen)
                    Button(L10n.t("replay.cancelMark")) { cancelMark() }
                        .frame(width: 72, height: 44)
                    TextField("", text: $markLastSeconds)
                        .frame(width: 44, height: 44)
                    Text(L10n.t("replay.secondsShort"))
                        .font(.system(size: 11))
                        .foregroundStyle(Color(white: 0.54))
                    Button(L10n.t("replay.markLast")) { markLast() }
                        .frame(width: 100, height: 44)
                        .help(L10n.t("replay.markLastHint"))
                        .disabled(!isRecording)
                }
                Spacer(minLength: 8)
                controlGroup {
                    Button("25%") { setSpeed(250) }.frame(width: 54, height: 44)
                    Button("50%") { setSpeed(500) }.frame(width: 54, height: 44)
                    Button("75%") { setSpeed(750) }.frame(width: 54, height: 44)
                    Button("100%") { setSpeed(1000) }.frame(width: 56, height: 44)
                    Slider(value: $jogValue, in: -30...30, step: 1) { editing in
                        if !editing {
                            jog(Int(jogValue.rounded()))
                            jogValue = 0
                        }
                    }
                    .frame(width: 110)
                    Button(L10n.t("replay.export")) { showExport.toggle() }
                        .frame(minWidth: 110).frame(height: 44)
                        .popover(isPresented: $showExport) {
                            exportBar
                        }
                }
            }
            HStack(alignment: .center, spacing: 8) {
                Text(health)
                    .font(.system(size: 11))
                    .foregroundStyle(Color(white: 0.54))
                    .lineLimit(1)
                    .truncationMode(.tail)
                Spacer()
                if !status.isEmpty {
                    Text(status)
                        .font(.system(size: 11))
                        .foregroundStyle(Color(white: 0.54))
                        .lineLimit(1)
                }
            }
            Text(L10n.t("replay.shortcutHelp"))
                .font(.system(size: 11))
                .foregroundStyle(Color(white: 0.42))
                .lineLimit(1)
        }
    }

    /// The tags of the selected clip, on the right of the transition row where the Windows
    /// deck keeps them.
    private var clipTagPanel: some View {
        HStack(spacing: 4) {
            Text(L10n.t("replay.tags"))
                .font(.system(size: 11))
                .foregroundStyle(Color(white: 0.54))
            if let uid = selectedClip, let clip = clips.first(where: { $0.uid == uid }),
               !clip.tags.isEmpty {
                ForEach(clip.tags, id: \.self) { tag in
                    Text(tag)
                        .font(.system(size: 11))
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background(Color(white: 0.18))
                        .clipShape(Capsule())
                }
            } else {
                Text("—").font(.system(size: 11)).foregroundStyle(Color(white: 0.4))
            }
        }
    }

    private func controlGroup<Content: View>(@ViewBuilder _ content: () -> Content) -> some View {
        HStack(spacing: 4) { content() }
            .padding(.horizontal, 6)
            .padding(.vertical, 4)
            .background(RoundedRectangle(cornerRadius: 4).fill(Color(white: 0.1)))
    }

    private func transportReadout(
        _ title: String,
        _ value: String,
        color: Color = EivizTheme.text
    ) -> some View {
        VStack(alignment: .leading, spacing: 1) {
            Text(title)
                .font(.system(size: 10))
                .foregroundStyle(Color(white: 0.54))
            Text(value)
                .font(.system(size: 16, design: .monospaced))
                .foregroundStyle(value.hasPrefix("--") ? EivizTheme.dim : color)
        }
    }

    private var remainingClipText: String {
        guard let left = ReplayDeckMath.remainingClip(
            position: displayPlayhead,
            channelOut: channelOutFrame,
            markOut: outFrame)
        else { return "--:--:--:--" }
        return ReplayTime.duration(displayPlayhead, displayPlayhead + left, fps)
    }

    private var remainPlaylistText: String {
        let frame = displayPlayhead
        let left = ReplayDeckMath.remainingPlaylist(clips: clips, position: frame)
        return left > 0 ? ReplayTime.duration(frame, frame + left, fps) : "--:--:--:--"
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
            Text(stateText).fontWeight(.bold).foregroundStyle(isRecording ? Color.red : Color.primary)
            if !status.isEmpty {
                Text(status).font(.caption).foregroundStyle(EivizTheme.dim).lineLimit(1)
            }
            Spacer()
            Text(L10n.t("replay.recordTc")).font(.caption)
            Text(mixer.isRemote ? "--:--:--:--" : ReplayTime.timecode(recordingHead, fps))
                .font(.system(.title3, design: .monospaced))
                .frame(minWidth: 148, alignment: .trailing)
            // The deck is where an operator notices that storage or angles are
            // missing, so the way into the recording settings belongs here too.
            Button(L10n.t("replay.multiview")) { multiview.toggle() }
                .frame(height: 32)
                .tint(multiview ? .green : nil)
                .help(L10n.t("replay.multiview"))
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
                MetalPreviewRepresentable(
                    role: .monitor(monitorId: monitorId, sourceId: sourceId),
                    presentInterval: deckPresentInterval)
                    .aspectRatio(16.0 / 9.0, contentMode: .fit)
                    .frame(maxWidth: .infinity, maxHeight: 180)
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

    private var isRecording: Bool { stats?.state == 1 }

    private func markReadout(_ title: String, _ value: String, color: Color = EivizTheme.text) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(title).font(.caption).foregroundStyle(EivizTheme.dim)
            Text(value)
                .font(.system(.body, design: .monospaced))
                .foregroundStyle(value.hasPrefix("--") ? EivizTheme.dim : color)
        }
        .padding(.horizontal, 8)
        .frame(width: 130, height: 44, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 5).fill(EivizTheme.panel))
        .overlay(RoundedRectangle(cornerRadius: 5).stroke(EivizTheme.stroke))
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

    /// Every angle at once, live from its own monitor. The selected angle keeps the
    /// same border the numbered strip uses, so clicking a tile still means "use this
    /// one for the next mark".
    private var angleMultiview: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 8) {
                Text(L10n.t("replay.angles")).fontWeight(.bold)
                Button(L10n.t("replay.multiview")) { multiview = false }
                    .controlSize(.small)
                    .tint(.green)
                Spacer()
            }
            LazyVGrid(
                columns: [GridItem(.adaptive(minimum: 190), spacing: 6)],
                spacing: 6
            ) {
                ForEach(angles) { angle in
                    let selected = (document?.activeAngleId ?? 0) == angle.id
                    Button {
                        selectAngle(angle.id)
                    } label: {
                        VStack(spacing: 2) {
                            if let monitor = angleMonitors[angle.id], monitor != 0 {
                                MetalPreviewRepresentable(
                                    role: .monitor(monitorId: monitor, sourceId: angle.liveSourceId),
                                    presentInterval: deckPresentInterval
                                )
                                .aspectRatio(16.0 / 9.0, contentMode: .fit)
                                .frame(maxWidth: .infinity)
                                .background(Color.black)
                                .allowsHitTesting(false)
                            } else {
                                Color.black.aspectRatio(16.0 / 9.0, contentMode: .fit)
                            }
                            Text(angle.label)
                                .font(.caption2)
                                .lineLimit(1)
                                .frame(maxWidth: .infinity, alignment: .leading)
                        }
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .contentShape(Rectangle())
                    .overlay(
                        RoundedRectangle(cornerRadius: 6)
                            .stroke(selected ? Color.green : Color.gray.opacity(0.4),
                                    lineWidth: selected ? 3 : 1)
                    )
                }
            }
        }
        .frame(width: 430, alignment: .leading)
    }

    /// One numbered button per camera, the way a replay controller labels its cameras.
    /// The picture of the selected one is the monitor above, so the strip stays a row of
    /// numbers instead of a row of expensive previews.
    private var angleStrip: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 8) {
                Text(L10n.t("replay.angles")).fontWeight(.bold)
                if mixer.isRemote {
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
                                .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .frame(width: 48, height: 44)
                        .contentShape(Rectangle())
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
        .frame(width: 300, alignment: .leading)
    }

    /// Every event on one line, edited in that line: its name and its protection are
    /// cells, so nothing is "select one, then use the buttons above".
    private var eventList: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(L10n.t("replay.events")).fontWeight(.bold)
            ReplayEventTable(
                events: document?.events ?? [],
                fps: fps,
                rename: { renameEvent($0, $1) },
                protect: { setEventProtected($0, $1) },
                setRange: { id, start, end in setEventRange(id, start, end) },
                addToPlaylist: { addEventToPlaylist($0) },
                remove: { deleteEvent($0) })
            if (document?.events ?? []).isEmpty {
                Text(L10n.t("replay.noEvents"))
                    .font(.caption)
                    .foregroundStyle(EivizTheme.dim)
            }
        }
    }

    /// The playlist this channel plays, with every clip on one line and its settings
    /// edited in that line: angle, speed, transition, length, gain, pitch and audio are
    /// cells, so nothing is "select a clip, then use the controls below".
    private var playlist: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(alignment: .center, spacing: 4) {
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
                .frame(width: 160, height: 28)
                .help(L10n.t("replay.playlistPickHelp"))
                TextField("", text: Binding(
                    get: { playlists.first { $0.id == playlistId }?.name ?? "" },
                    set: { renamePlaylist(playlistId, $0) }
                ))
                .frame(width: 120, height: 28)
                .help(L10n.t("replay.playlistNameHelp"))
                Button(L10n.t("replay.playlistAdd")) { addPlaylist() }
                    .frame(minWidth: 40, minHeight: 28)
                    .help(L10n.t("replay.playlistAddHint"))
                Picker("", selection: $clipOrder) {
                    Text(L10n.t("replay.clipOrderAppend")).tag("Append")
                    Text(L10n.t("replay.clipOrderTimecode")).tag("Timecode")
                }
                .labelsHidden()
                .frame(width: 128, height: 28)
                .help(L10n.t("replay.clipOrderHint"))
                Button(L10n.t("replay.playlistRemove")) { deletePlaylist() }
                    .frame(minWidth: 40, minHeight: 28)
                    .disabled(playlists.count <= 1)
                    .help(L10n.t("replay.playlistRemove"))
                Spacer(minLength: 8)
            }
            HStack(alignment: .center, spacing: 4) {
                Spacer(minLength: 8)
                Button(L10n.t("replay.playlistPlay")) { playPlaylist() }
                    .frame(minWidth: 86, minHeight: 28)
                    .help(L10n.t("replay.playlistPlayHint"))
                Button(L10n.t("replay.pause")) { pausePlaylist() }
                    .frame(minWidth: 66, minHeight: 28)
                    .help(L10n.t("replay.pauseHint"))
                Button(L10n.t("replay.playlistStop")) { stopPlaylist() }
                    .frame(minWidth: 52, minHeight: 28)
                    .help(L10n.t("replay.playlistStopHint"))
                Toggle(L10n.t("replay.playlistLoop"), isOn: Binding(
                    get: { looping },
                    set: { next in
                        if mixer.replayLoop(channel: channel, on: next) {
                            looping = next
                        } else {
                            status = L10n.t("replay.commandRefused")
                        }
                    }))
                .toggleStyle(.button)
                .frame(minWidth: 60, minHeight: 28)
                Toggle(L10n.t("replay.playlistHold"), isOn: Binding(
                    get: { holdLast },
                    set: { next in
                        if mixer.replayHoldLast(channel: channel, on: next) {
                            holdLast = next
                        } else {
                            status = L10n.t("replay.commandRefused")
                        }
                    }))
                .toggleStyle(.button)
                .frame(minWidth: 72, minHeight: 28)
            }
            ReplayClipTableWindows(
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
                    remove: { removeClip($0) }),
                playingUid: playingClipUid,
                selection: $selectedClip)
        }
    }

    private var exportBar: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(L10n.t("replay.exportHint"))
                .font(.system(size: 11))
                .foregroundStyle(Color(white: 0.69))
                .fixedSize(horizontal: false, vertical: true)
            HStack(spacing: 8) {
                Text(L10n.t("replay.exportMode"))
                    .frame(width: 90, alignment: .leading)
                Picker("", selection: $exportMode) {
                    Text(L10n.t("replay.exportClip")).tag("Clip")
                    Text(L10n.t("replay.exportPlaylistTransitions")).tag("PlaylistWithTransitions")
                    Text(L10n.t("replay.exportPlaylistCut")).tag("Playlist")
                    Text(L10n.t("replay.exportAngle")).tag("Angle")
                    Text(L10n.t("replay.exportAllAngles")).tag("AllAngles")
                }
                .labelsHidden()
                .frame(width: 220, height: 32)
                Text(L10n.t("replay.exportCodec"))
                    .padding(.leading, 12)
                Picker("", selection: $exportCodec) {
                    Text("H.264").tag("H264")
                    Text("HEVC").tag("Hevc")
                }
                .labelsHidden()
                .frame(width: 120, height: 32)
                Spacer()
            }
            HStack(spacing: 8) {
                Text(L10n.t("replay.exportPath"))
                    .frame(width: 90, alignment: .leading)
                TextField("", text: $exportPath)
                    .frame(maxWidth: .infinity, minHeight: 32, maxHeight: 32)
                Button(L10n.t("replay.browse")) { chooseExportPath() }
                    .frame(minWidth: 90, minHeight: 32)
            }
            Text(exporter.status)
                .font(.system(size: 11))
                .foregroundStyle(Color(white: 0.69))
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 0)
            HStack(spacing: 10) {
                Spacer()
                Button(L10n.t("replay.exportStart")) { startExport() }
                    .disabled(exporter.busy)
                    .frame(minWidth: 130, minHeight: 36)
                Button(L10n.t("replay.close")) {
                    if exporter.busy { cancelExport() }
                    showExport = false
                }
                .frame(minWidth: 110, minHeight: 36)
            }
        }
        .padding(16)
        .frame(width: 720, height: 360)
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
        let typedPath = exportPath.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !typedPath.isEmpty else {
            exporter.report(L10n.t("replay.exportNeedPath"))
            return
        }
        let path = ReplayExportPlan.outputPath(typedPath)
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
            for angle in document.angles {
                let angleSegments = segmentsFor(angle, in: segments)
                // An angle with nothing recorded has no file to write. Queueing it asked
                // the mixer to export an empty segment list, which failed and stopped the
                // remaining (recorded) angles from being written.
                guard !angleSegments.isEmpty else { continue }
                let target = ReplayExportPlan.anglePath(path, angleId: angle.id)
                // A refusal is the mixer saying this file cannot be written at all, so the
                // rest of the angles would fail the same way: stop rather than queue them.
                guard exporter.start(
                    json: exportJson(
                        path: target, document: document, segments: angleSegments),
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
                    .tint(inFrame != nil ? .orange : nil)
                    .frame(width: 150, height: 44)
                Button(L10n.t("replay.markOut")) { markOut() }
                    .keyboardShortcut("o", modifiers: [])
                    .help(L10n.t("replay.markOutHint"))
                    .tint(outFrame != nil ? .orange : nil)
                    .frame(width: 150, height: 44)
                    .disabled(!markOpen)
                Button(L10n.t("replay.cancelMark")) { cancelMark() }
                    .frame(width: 110, height: 44)
                markReadout(L10n.t("replay.in"), inFrame.map { ReplayTime.timecode($0, fps) } ?? "--:--:--:--", color: .orange)
                markReadout(L10n.t("replay.out"), outFrame.map { ReplayTime.timecode($0, fps) } ?? "--:--:--:--", color: .orange)
                markReadout(L10n.t("replay.duration"), markDuration)
                Spacer()
            }
            HStack(alignment: .center, spacing: 8) {
                Button {
                    record(true)
                } label: {
                    HStack(spacing: 6) {
                        Circle()
                            .fill(isRecording ? Color.red : EivizTheme.dim)
                            .frame(width: 8, height: 8)
                        Text(isRecording ? "REC" : L10n.t("replay.record"))
                    }
                }
                .tint(isRecording ? .red : nil)
                .disabled(isRecording)
                .frame(width: 88, height: 36)
                Button(L10n.t("replay.stop")) { record(false) }
                    .disabled(!isRecording)
                    .frame(width: 84, height: 36)
                Button(L10n.t("replay.live")) { live() }
                    .keyboardShortcut("l", modifiers: [])
                    .frame(width: 84, height: 36)
                Button(L10n.t(transport.playing ? "replay.pause" : "replay.play")) { togglePlay() }
                    .keyboardShortcut(.space, modifiers: [])
                    .frame(width: 100, height: 36)
                Button("◀") { step(-1) }
                    .keyboardShortcut(.leftArrow, modifiers: [])
                    .help(L10n.t("replay.stepBackHint"))
                    .frame(width: 40, height: 36)
                Button("▶") { step(1) }
                    .keyboardShortcut(.rightArrow, modifiers: [])
                    .help(L10n.t("replay.stepForwardHint"))
                    .frame(width: 40, height: 36)
                Button("25%") { setSpeed(250) }.frame(width: 56, height: 36)
                Button("50%") { setSpeed(500) }.frame(width: 56, height: 36)
                Button("75%") { setSpeed(750) }.frame(width: 56, height: 36)
                Button("100%") { setSpeed(1000) }.frame(width: 60, height: 36)
                // Loops the playlist at its end, the way the deck's own button does.
                Toggle(L10n.t("replay.playlistLoop"), isOn: Binding(
                    get: { looping },
                    set: { next in
                        if mixer.replayLoop(channel: channel, on: next) {
                            looping = next
                        } else {
                            status = L10n.t("replay.commandRefused")
                        }
                    }))
                .toggleStyle(.button)
                .frame(width: 64, height: 36)
                .help(L10n.t("replay.playlistLoopHint"))
                // What happens when the list ends, next to looping because it is the same
                // question. Looping wins when both are on.
                Toggle(L10n.t("replay.playlistHold"), isOn: Binding(
                    get: { holdLast },
                    set: { next in
                        if mixer.replayHoldLast(channel: channel, on: next) {
                            holdLast = next
                        } else {
                            status = L10n.t("replay.commandRefused")
                        }
                    }))
                .toggleStyle(.button)
                .frame(width: 88, height: 36)
                .help(L10n.t("replay.playlistHoldHint"))
                Spacer(minLength: 8)
                Text(L10n.t("replay.jog"))
                Slider(value: $jogValue, in: -30...30, step: 1) { editing in
                    if !editing {
                        jog(Int(jogValue.rounded()))
                        jogValue = 0
                    }
                }
                .frame(width: 160, height: 36)
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
        } else {
            let shown = ReplayDeckMath.channelToShow(channels: channels, current: channel)
            if shown != channel {
                channel = shown
            }
        }
        // A selection that names something the document no longer has is not a selection.
        if let uid = selectedClip, !clips.contains(where: { $0.uid == uid }) {
            selectedClip = nil
        }
        if let id = selectedEvent,
           !(document?.events.contains(where: { $0.id == id }) ?? false) {
            selectedEvent = nil
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
                playingClipUid = clips.first {
                    ReplayDeckMath.clipUidHash($0.uid) == state.clip_uid_hash
                }?.uid
                // Only the engine's own answer may move the bar once a drag is over.
                if scrubFrame == nil {
                    playheadFrame = state.position_frame
                }
                channelOutFrame = state.out_frame
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
                (playheadFrame, .red, 2),
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
        let ratio = Double(point.x / size.width)
        let frame = ReplayDeckMath.waveformFrame(
            ratio: ratio,
            bucketRate: bucketRate,
            buckets: buckets,
            dropped: peakDropped,
            fps: framesPerSecond)
        let mark: WaveformMark? =
            modifiers.contains(.shift) ? .in : (modifiers.contains(.control) ? .out : nil)
        switch mark {
        case .in:
            inFrame = frame
            outFrame = nil
            markOpen = true
            seekFromWaveform(to: frame)
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
            seekFromWaveform(to: frame)
        }
    }

    /// Seeks the playhead without changing whether the channel is playing.
    ///
    /// Dragging the waveform or taking a mark moves the picture off the live edge, so the
    /// strip draws proxies from here on. It does not start or stop anything, though: an
    /// operator who was paused stays paused, and one who was playing keeps playing from the
    /// new frame. The transport was handed that play state, and the deck keeps it.
    /// Moves the whole-recording slider. The bar parks the channel on the chosen frame:
    /// a slider is a place to look at a picture, and a seek that kept playing made the
    /// thumb run away from the frame the operator just picked.
    private func seekWhole(to frame: UInt64) {
        // A channel that is following the recording edge has no current clip, and the
        // engine's live flag wins over a seek. Cue a scratch clip over the recorded range
        // first, which leaves the edge, then park on the chosen frame.
        cueScratchForSeek(target: frame)
        let action = ReplayTransportRules.seek()
        let accepted = mixer.replayTransport(
            channel: channel,
            playing: false,
            speedMilli: speedMilli,
            seekFrame: Int64(frame)
        )
        if accepted {
            transport = action.next
            playheadFrame = frame
            settleUntil = Date().addingTimeInterval(0.4)
        } else {
            let detail = MixerFFI.lastErrorText()
            status = detail.isEmpty ? L10n.t("replay.seekRefused") : detail
            scrubFrame = nil
        }
    }

    /// Leaves live follow for a seek without disturbing a clip that is already on air.
    private func cueScratchForSeek(target: UInt64) {
        if !transport.live, playingClipUid != nil { return }
        guard let angle = activeAngle,
              let range = mixer.replayAngleRange(angle.id),
              range.last > range.first
        else { return }
        let recorded = document?.raw["recordings"]?.arrayValue?.last?
            .objectValue?["uuid"]?.stringValue ?? ""
        let recording = recorded.isEmpty ? mixer.replayRecordingUuid() : recorded
        guard !recording.isEmpty else { return }
        let clip = ReplayClipInfo(
            uid: "seek-scrub",
            angleId: angle.id,
            recordingUuid: recording,
            inFrame: range.first,
            outFrame: max(range.last, target),
            speed: 1,
            gainDb: 0,
            transition: .cut,
            tags: [],
            preservePitch: false,
            audioKind: "FollowAngle",
            audioAngleId: angle.id
        )
        if mixer.replayCue(channel: channel, clip: clip) {
            transport = .held
            playingClipUid = nil
        }
    }

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

    /// Marks the last N seconds as one event without walking the playhead: the range is
    /// written straight into the session, the same range Mark In / Mark Out would leave.
    private func markLast() {
        guard isRecording else {
            status = L10n.t("replay.markLastNeedsRecording")
            return
        }
        guard let snapshot = stats, var current = document else {
            status = L10n.t("replay.markOutFailed")
            return
        }
        let seconds = max(1.0, Double(markLastSeconds) ?? 10)
        let span = UInt64(seconds * ReplayTime.rate(fps))
        let now = max(snapshot.recording_frame, snapshot.last_frame)
        let start = now > span ? now - span : 0
        let recording = mixer.replayRecordingUuid()
        guard !recording.isEmpty else {
            status = L10n.t("replay.markOutFailed")
            return
        }
        let name = L10n.format("replay.eventName", "\(current.events.count + 1)")
        let id = current.appendEvent(
            uuid: recording,
            inFrame: start,
            outFrame: now,
            name: name)
        // The session validates an event against its recording's range, and the range of a
        // recording still being written is the frame the recorder is on.
        current.setRecordingRange(uuid: recording, first: 0, last: now)
        addEventClipIfNeeded(id, to: &current)
        inFrame = start
        outFrame = now
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
            addEventClipIfNeeded(id, to: &current)
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
        addEventClipIfNeeded(id, to: &current)
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

    private func setEventRange(_ id: UInt64, _ inFrame: UInt64, _ outFrame: UInt64) {
        guard ReplayDeckMath.eventRangeValid(inFrame: inFrame, outFrame: outFrame) else {
            status = L10n.t("replay.markOutBeforeIn")
            return
        }
        guard var current = document else { return }
        current.setEventRange(id, inFrame: inFrame, outFrame: outFrame)
        save(current)
    }

    private func setEventProtected(_ id: UInt64, _ value: Bool) {
        guard var current = document else { return }
        current.setEventProtected(id, value)
        save(current)
    }

    private func deleteEvent(_ id: UInt64) {
        guard var current = document else { return }
        guard current.removeEventIfUnprotected(id) else {
            status = L10n.t("replay.eventProtected")
            return
        }
        // A clip keeps its own recording, range and angle, so it survives the event it
        // came from; only the reference to an event that is gone goes away.
        save(current, deletedEvents: [id])
    }

    private func removeClip(_ uid: String) {
        guard var current = document else { return }
        current.removeClip(playlistId: playlistId, uid: uid)
        if selectedClip == uid { selectedClip = nil }
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
            let alert = NSAlert()
            alert.messageText = L10n.format("replay.playlistDeleteClips", "\(clips)")
            alert.addButton(withTitle: L10n.t("dialog.ok"))
            alert.addButton(withTitle: L10n.t("dialog.cancel"))
            guard alert.runModal() == .alertFirstButtonReturn else { return }
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
        guard current.removeEventIfUnprotected(id) else {
            status = L10n.t("replay.eventProtected")
            return
        }
        // A clip keeps its own recording, range and angle, so it survives the event
        // it came from; only the reference to an event that is gone goes away.
        save(current, deletedEvents: [id])
        selectedEvent = nil
    }

    /// Every event path shares the auto-add rule, including the clip-order setting.
    private func addEventClipIfNeeded(_ eventId: UInt64, to current: inout ReplayDocument) {
        guard current.autoAddToPlaylist else { return }
        let angleId = current.activeAngle
        guard angleId != 0 else {
            status = L10n.t("replay.needAngle")
            return
        }
        _ = current.addEventClip(
            eventId: eventId,
            angleId: angleId,
            playlistId: current.channelPlaylist(channelIndex: channel) ?? 0,
            sortByTimecode: clipOrder == "Timecode")
    }

    private func addEventToPlaylist(_ event: ReplayEventInfo) {
        guard var current = document else { return }
        let angleId = current.activeAngle
        guard angleId != 0 else {
            status = L10n.t("replay.needAngle")
            return
        }
        // Add to the playlist the channel is on, not merely the first one.
        let target = current.ensurePlaylist(preferred: playlistId)
        _ = current.appendClip(playlistId: target, event: event, angleId: angleId)
        if clipOrder == "Timecode" {
            current.sortClips(playlistId: target)
        }
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
        scrubFrame = nil
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

    private func playPlaylist() {
        guard let first = clips.first else {
            status = L10n.t("replay.needClip")
            return
        }
        guard mixer.replayCue(channel: channel, clip: first) else {
            status = L10n.t("replay.cueRefused")
            return
        }
        if mixer.replayTransport(
            channel: channel, playing: true, speedMilli: speedMilli, seekFrame: -1) {
            transport = ReplayTransportState(live: false, playing: true)
            settleUntil = Date().addingTimeInterval(0.8)
        } else {
            status = L10n.t("replay.playRefused")
        }
    }

    private func pausePlaylist() {
        if transport.live {
            seekHolding(to: max(displayPlayhead, recordingHead), keepPlaying: false)
            return
        }
        if mixer.replayTransport(
            channel: channel, playing: false, speedMilli: 0, seekFrame: -1) {
            transport = .held
            settleUntil = Date().addingTimeInterval(0.4)
        } else {
            status = L10n.t("replay.commandRefused")
        }
    }

    private func stopPlaylist() {
        live()
    }

    private func togglePlay() {
        let action = ReplayTransportRules.toggle(transport)
        // Pausing a live channel has to leave the edge; a live channel with playing=false
        // still follows the write head, so the picture would keep moving.
        if !action.playing && transport.live {
            seekHolding(to: max(displayPlayhead, recordingHead), keepPlaying: false)
            return
        }
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

    /// Moves the channel by `delta` frames from where the playhead is now, stopping at
    /// the marks when both are set. A step is a look at the next frame, not a jump back
    /// to Mark In, and it must work before any mark has been taken.
    private func seekMarked(_ delta: Int64) {
        let target = ReplayDeckMath.stepTarget(
            current: displayPlayhead,
            delta: delta,
            markIn: inFrame,
            markOut: outFrame,
            range: UInt64(seekRange.lowerBound)...UInt64(seekRange.upperBound)
        )
        seekHolding(to: target, keepPlaying: false)
    }

    /// A waveform click or mark moves the picture without changing whether the channel
    /// is advancing. It still has to leave the live edge, or the next report puts the
    /// playhead back on the recording edge.
    private func seekFromWaveform(to frame: UInt64) {
        // A waveform click is a place to look at a sound or a picture before cutting;
        // parking there is what makes the mark that follows land on the frame clicked.
        seekHolding(to: frame, keepPlaying: false)
    }

    /// Leaves live follow when needed, seeks, and either holds or keeps playing.
    private func seekHolding(to frame: UInt64, keepPlaying: Bool) {
        let target = min(max(frame, UInt64(seekRange.lowerBound)),
                         UInt64(seekRange.upperBound))
        cueScratchForSeek(target: target)
        let action = ReplayTransportRules.leaveEdge(transport)
        let playing = keepPlaying && action.playing
        if mixer.replayTransport(
            channel: channel,
            playing: playing,
            speedMilli: speedMilli,
            seekFrame: Int64(target)
        ) {
            transport = playing
                ? ReplayTransportState(live: false, playing: true)
                : .held
            playheadFrame = target
            scrubFrame = nil
            settleUntil = Date().addingTimeInterval(0.4)
        } else {
            status = L10n.t("replay.seekRefused")
            scrubFrame = nil
        }
    }

    private func step(_ delta: Int64) {
        seekMarked(delta)
    }

    private func setSpeed(_ milli: Int32) {
        speedMilli = milli
        // A speed preset is a playback instruction. On a live channel there is no clip to
        // play, so leave the edge at the current frame and run that clip at the new rate.
        if transport.live, stats?.last_frame ?? 0 > 0 {
            // A speed preset on a live channel starts the recorded picture at that rate,
            // from the newest frame rather than from wherever a stale position said zero.
            seekHolding(to: max(displayPlayhead, recordingHead), keepPlaying: true)
            return
        }
        if !mixer.replayTransport(
            channel: channel, playing: transport.playing, speedMilli: speedMilli, seekFrame: -1) {
            status = L10n.t("replay.commandRefused")
        }
    }

    /// Live tiles stay the scene. Jog tiles decode the proxy, or say it is missing.
    @ViewBuilder
    private func angleTile(index: Int, angle: ReplayAngleInfo, selected: Bool) -> some View {
        if !mixer.isRemote, let monitor = angleMonitors[angle.id], monitor != 0 {
            // Every angle stays live, not only the one in the large monitor: the strip is
            // how the operator notices a camera that stopped before marking it.
            MetalPreviewRepresentable(
                role: .monitor(monitorId: monitor, sourceId: angle.liveSourceId),
                presentInterval: deckPresentInterval
            )
            .frame(width: 48, height: 44)
            .clipped()
            .allowsHitTesting(false)
        } else if transport.live || mixer.isRemote {
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

