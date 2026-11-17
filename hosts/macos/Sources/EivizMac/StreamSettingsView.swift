import EivizMixer
import SwiftUI

/// The Stream page of the settings window.
///
/// Mirrors the Windows `SettingsWindow` "Stream" category: encode profiles on
/// top and destinations below, in the same field order, with the same hints and
/// the same per-setting load gauges. The Windows page edits a live session; this
/// host applies the document when the settings sheet closes, so the fields bind
/// to the session exactly like the rest of the macOS settings.
struct StreamSettingsPane: View {
    @EnvironmentObject private var mixer: MixerController
    @State private var caps = StreamCaps()
    @State private var onAirLock = OnAirLock.active

    private var rtmpOutputs: [OutputEntry] {
        mixer.session.outputs.filter { $0.transport == .rtmp }
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 8) {
                sectionHeader(
                    title: L10n.t("settings.encode"),
                    addHelp: L10n.t("settings.addEncode"),
                    add: { mixer.addStreamEncoder() }
                )
                .disabled(!caps.linked || onAirLock)
                Text(L10n.t("settings.encodeHelp"))
                    .font(.caption)
                    .foregroundStyle(EivizTheme.dim)
                    .fixedSize(horizontal: false, vertical: true)

                if mixer.session.streamEncoders.isEmpty {
                    Text(L10n.t("settings.encodingEmpty"))
                        .font(.caption)
                        .foregroundStyle(EivizTheme.dim)
                }
                ForEach($mixer.session.streamEncoders) { $encoder in
                    EncodeProfileCard(
                        encoder: $encoder,
                        caps: caps,
                        locked: onAirLock
                    ) {
                        mixer.removeStreamEncoder(encoder.id)
                    }
                }

                sectionHeader(
                    title: L10n.t("settings.destinations"),
                    addHelp: L10n.t("settings.addStream"),
                    add: { mixer.addRtmpDestination() }
                )
                .disabled(onAirLock)
                Text(L10n.t("settings.streamHelp"))
                    .font(.caption)
                    .foregroundStyle(EivizTheme.dim)
                    .fixedSize(horizontal: false, vertical: true)

                if rtmpOutputs.isEmpty {
                    Text(L10n.t("settings.destinationsEmpty"))
                        .font(.caption)
                        .foregroundStyle(EivizTheme.dim)
                }
                ForEach($mixer.session.outputs) { $output in
                    if output.transport == .rtmp {
                        StreamDestinationCard(output: $output, locked: onAirLock)
                    }
                }

                Spacer(minLength: 0)
            }
        }
        .onAppear { caps = StreamCaps.read() }
        .onReceive(NotificationCenter.default.publisher(for: OnAirLock.changed)) { _ in
            onAirLock = OnAirLock.active
        }
    }

    private func sectionHeader(title: String, addHelp: String, add: @escaping () -> Void) -> some View {
        HStack {
            Text(title).fontWeight(.bold)
            Spacer()
            Button(action: add) {
                Image(systemName: "plus")
                    .frame(width: 20, height: 18)
            }
            .buttonStyle(.borderless)
            .help(addHelp)
        }
    }
}

/// One encode profile, laid out like the Windows `BuildEncodeRow` grid.
private struct EncodeProfileCard: View {
    @Binding var encoder: StreamEncoderEntry
    let caps: StreamCaps
    let locked: Bool
    let onRemove: () -> Void
    @EnvironmentObject private var mixer: MixerController

    private var meters: EncodeMeters { EncodeMeters(encoder) }

    /// What the selected encoder actually honours. VideoToolbox accepts only the
    /// automatic thread count and refuses presets it cannot express; Media
    /// Foundation has no switches for complexity, entropy or the four toggles.
    private var support: EncoderSupport { EncoderSupport.of(encoder.encoder) }

    var body: some View {
        Grid(alignment: .leading, horizontalSpacing: 8, verticalSpacing: 8) {
            GridRow {
                TextField(L10n.t("settings.encodeName"), text: $encoder.name)
                    .textFieldStyle(.roundedBorder)
                    .disabled(locked)
                    .gridCellColumns(3)
                Button(action: onRemove) {
                    Image(systemName: "minus")
                }
                .disabled(locked)
                .help(L10n.t("settings.removeEncode"))
            }

            GridRow {
                metered(L10n.t("settings.encodeLoad"), meters.total, L10n.t("settings.encodeLoadHint"), count: 10) {
                    Color.clear.frame(height: 0)
                }
                .gridCellColumns(3)
            }

            GridRow {
                metered(L10n.t("settings.videoBitrate"), meters.video, L10n.t("settings.videoBitrateHint")) {
                    bitrateField($encoder.videoBitrate, minimum: 100)
                }
                metered(L10n.t("settings.audioBitrate"), meters.audio, L10n.t("settings.audioBitrateHint")) {
                    bitrateField($encoder.audioBitrate, minimum: 32)
                }
                metered(L10n.t("settings.keyframeInterval"), meters.keyint, L10n.t("settings.keyframeIntervalHint")) {
                    DraggableUIntField(value: $encoder.keyint, minimum: 1)
                        .disabled(locked)
                }
            }

            GridRow {
                metered(L10n.t("settings.encodeSize"), meters.size, L10n.t("settings.encodeSizeHint")) {
                    sizePicker
                }
                metered(L10n.t("settings.encodeFps"), meters.fps, L10n.t("settings.encodeFpsHint")) {
                    fpsPicker
                }
            }

            GridRow {
                metered(L10n.t("settings.threads"), meters.threads, L10n.t("settings.threadsHint"), fixedByEncoder: !support.threads) {
                    threadsPicker
                }
                metered(L10n.t("settings.sendBuffer"), meters.buffer, L10n.t("settings.sendBufferHint")) {
                    DraggableUIntField(
                        value: Binding(
                            get: { encoder.bufferMs },
                            set: { encoder.bufferMs = min($0, 10_000) }
                        ),
                        step: 10
                    )
                    .disabled(locked)
                }
            }

            GridRow {
                metered(L10n.t("settings.encoder"), meters.encoder, encoderHint) {
                    encoderPicker
                }
                metered(L10n.t("settings.complexity"), meters.complexity, L10n.t("settings.complexityHint"), fixedByEncoder: !support.complexity) {
                    complexityPicker
                }
                metered(L10n.t("settings.refFrames"), meters.refs, L10n.t("settings.refFramesHint"), fixedByEncoder: !support.refFrames) {
                    refFramesPicker
                }
            }

            GridRow {
                metered(L10n.t("settings.audioDelay"), meters.delay, L10n.t("settings.audioDelayHint")) {
                    DraggableUIntField(
                        value: Binding(
                            get: { encoder.audioDelayMs },
                            set: { encoder.audioDelayMs = min($0, 1_000) }
                        ),
                        step: 10
                    )
                    .disabled(locked)
                }
                metered(L10n.t("settings.entropy"), meters.entropy, L10n.t("settings.entropyHint"), fixedByEncoder: !support.entropy) {
                    entropyPicker
                }
            }

            GridRow {
                VStack(alignment: .leading, spacing: 6) {
                    meteredToggle(L10n.t("settings.deblock"), meters.deblock, L10n.t("settings.deblockHint"), $encoder.deblock, enabled: support.deblock)
                    meteredToggle(L10n.t("settings.backgroundDetect"), meters.background, L10n.t("settings.backgroundDetectHint"), $encoder.backgroundDetection, enabled: support.background)
                    meteredToggle(L10n.t("settings.adaptiveQuant"), meters.quant, L10n.t("settings.adaptiveQuantHint"), $encoder.adaptiveQuant, enabled: support.adaptiveQuant)
                    meteredToggle(L10n.t("settings.sceneChange"), meters.scene, L10n.t("settings.sceneChangeHint"), $encoder.sceneChange, enabled: support.sceneChange)
                    // macOS streams video only until AudioToolbox AAC is linked, so this
                    // switch is not in the Windows page.
                    Toggle(L10n.t("settings.videoOnly"), isOn: $encoder.videoOnly)
                        .toggleStyle(.checkbox)
                        .disabled(locked)
                }
                .gridCellColumns(3)
            }
        }
        .padding(8)
        .background(
            Rectangle()
                .stroke(Color(white: 0.27), lineWidth: 1)
        )
        .onAppear { applyEncoderSupport() }
        .onChange(of: encoder.encoder) { _, _ in applyEncoderSupport() }
    }

    /// Locks the values the selected encoder cannot express. VideoToolbox only
    /// takes the automatic thread count and a preset with deblocking and
    /// background detection on, adaptive quantization and scene change off; it
    /// refuses anything else at start, so the card shows the accepted shape.
    private func applyEncoderSupport() {
        let support = support
        if let threads = support.threadsValue { encoder.encoderThreads = threads }
        if let value = support.deblockValue { encoder.deblock = value }
        if let value = support.backgroundValue { encoder.backgroundDetection = value }
        if let value = support.adaptiveQuantValue { encoder.adaptiveQuant = value }
        if let value = support.sceneChangeValue { encoder.sceneChange = value }
    }

    private var encoderHint: String {
        encoder.encoder == "MediaFoundation"
            ? L10n.t("settings.encoderHintMediaFoundation")
            : L10n.t("settings.encoderHintOpenH264")
    }

    // MARK: - Controls

    /// Bitrates are stored in bits per second; the page edits them in kbps like Windows.
    private func bitrateField(_ value: Binding<UInt32>, minimum: UInt32) -> some View {
        TextField(
            "",
            value: Binding(
                get: { Int(value.wrappedValue == 0 ? 0 : value.wrappedValue / 1000) },
                set: { kbps in
                    guard kbps > 0 else { return }
                    value.wrappedValue = UInt32(kbps) * 1000
                }
            ),
            format: .number
        )
        .textFieldStyle(.roundedBorder)
        .frame(maxWidth: 120)
        .disabled(locked)
    }

    private var sizeTag: String {
        encoder.width == 0 || encoder.height == 0 ? "0x0" : "\(encoder.width)x\(encoder.height)"
    }

    private var sizeOptions: [(tag: String, label: String)] {
        var options: [(String, String)] = [
            ("0x0", L10n.t("settings.followSessionSettings")),
            ("1920x1080", "1920x1080"),
            ("1280x720", "1280x720"),
            ("3840x2160", "3840x2160"),
        ]
        if !options.contains(where: { $0.0 == sizeTag }) {
            options.append((sizeTag, sizeTag))
        }
        return options
    }

    private var sizePicker: some View {
        Picker("", selection: Binding(
            get: { sizeTag },
            set: { tag in
                guard !locked else { return }
                let parts = tag.split(separator: "x").compactMap { UInt32($0) }
                let width = tag == "0x0" ? 0 : (parts.count == 2 ? parts[0] : encoder.width)
                let height = tag == "0x0" ? 0 : (parts.count == 2 ? parts[1] : encoder.height)
                // The mixer refuses an over-plan encode; roll the picker back and say why.
                guard caps.allowsSize(width, height) else {
                    mixer.presentError(L10n.t("msg.planEncodeSize"), title: L10n.t("settings.encode"))
                    return
                }
                encoder.width = width
                encoder.height = height
            }
        )) {
            ForEach(sizeOptions, id: \.tag) { option in
                Text(option.label).tag(option.tag)
            }
        }
        .labelsHidden()
        .frame(maxWidth: 160)
        .disabled(locked)
    }

    private var fpsTag: String {
        encoder.fpsNum == 0 || encoder.fpsDen == 0 ? "0/0" : "\(encoder.fpsNum)/\(encoder.fpsDen)"
    }

    private var fpsOptions: [(tag: String, label: String)] {
        var options: [(String, String)] = [
            ("0/0", L10n.t("settings.followSessionSettings")),
            ("24000/1001", "23.976p"),
            ("24/1", "24p"),
            ("25/1", "25p"),
            ("30000/1001", "29.97p"),
            ("30/1", "30p"),
            ("50/1", "50p"),
            ("60000/1001", "NTSC 59.94p"),
            ("60/1", "60p"),
        ]
        if !options.contains(where: { $0.0 == fpsTag }) {
            options.append((fpsTag, fpsTag))
        }
        return options
    }

    private var fpsPicker: some View {
        Picker("", selection: Binding(
            get: { fpsTag },
            set: { tag in
                guard !locked else { return }
                let parts = tag.split(separator: "/").compactMap { UInt32($0) }
                let num = tag == "0/0" ? 0 : (parts.count == 2 ? parts[0] : encoder.fpsNum)
                let den = tag == "0/0" ? 0 : (parts.count == 2 ? parts[1] : encoder.fpsDen)
                guard caps.allowsFps(num, den) else {
                    mixer.presentError(L10n.t("msg.planEncodeFps"), title: L10n.t("settings.encode"))
                    return
                }
                encoder.fpsNum = num
                encoder.fpsDen = den
            }
        )) {
            ForEach(fpsOptions, id: \.tag) { option in
                Text(option.label).tag(option.tag)
            }
        }
        .labelsHidden()
        .frame(maxWidth: 160)
        .disabled(locked)
    }

    private var threadsPicker: some View {
        Picker("", selection: Binding(
            get: { encoder.encoderThreads <= 4 ? encoder.encoderThreads : 0 },
            set: { encoder.encoderThreads = $0 }
        )) {
            Text(L10n.t("settings.threadsAuto")).tag(UInt32(0))
            ForEach(1 ... 4, id: \.self) { count in
                Text("\(count)").tag(UInt32(count))
            }
        }
        .labelsHidden()
        .frame(maxWidth: 120)
        .disabled(locked || !support.threads)
    }

    /// Encoder kinds follow the mixer's mask: bit 1 OpenH264, 2 VideoToolbox,
    /// 4 MediaFoundation. Without a mask the page keeps OpenH264 and a saved
    /// value, so an old document does not silently change encoder.
    private var encoderOptions: [String] {
        var listed = ["OpenH264"]
        if caps.encoderMask & 2 != 0 { listed.append("VideoToolbox") }
        if caps.encoderMask & 4 != 0 { listed.append("MediaFoundation") }
        let saved = encoder.encoder
        if !saved.isEmpty && !listed.contains(saved) { listed.append(saved) }
        return listed
    }

    private var encoderPicker: some View {
        Picker("", selection: $encoder.encoder) {
            ForEach(encoderOptions, id: \.self) { kind in
                Text(kind).tag(kind)
            }
        }
        .labelsHidden()
        .frame(maxWidth: 160)
        .disabled(locked)
    }

    private var complexityPicker: some View {
        Picker("", selection: Binding(
            get: { min(encoder.complexity, 2) },
            set: { encoder.complexity = $0 }
        )) {
            Text(L10n.t("settings.complexityLow")).tag(UInt32(0))
            Text(L10n.t("settings.complexityMedium")).tag(UInt32(1))
            Text(L10n.t("settings.complexityHigh")).tag(UInt32(2))
        }
        .labelsHidden()
        .frame(maxWidth: 120)
        .disabled(locked || !support.complexity)
    }

    private var refFramesPicker: some View {
        Picker("", selection: Binding(
            get: { encoder.refFrames >= 1 && encoder.refFrames <= 4 ? encoder.refFrames : 1 },
            set: { encoder.refFrames = $0 }
        )) {
            ForEach(1 ... 4, id: \.self) { count in
                Text("\(count)").tag(UInt32(count))
            }
        }
        .labelsHidden()
        .frame(maxWidth: 80)
        .disabled(locked || !support.refFrames)
    }

    private var entropyPicker: some View {
        Picker("", selection: Binding(
            get: { encoder.cabac ? "1" : "0" },
            set: { tag in
                guard !locked else { return }
                encoder.cabac = tag == "1"
            }
        )) {
            Text("CAVLC").tag("0")
            Text("CABAC").tag("1")
        }
        .labelsHidden()
        .frame(maxWidth: 120)
        .disabled(locked || !support.entropy)
    }

    // MARK: - Layout helpers

    private func metered<Control: View>(
        _ label: String,
        _ level: Int,
        _ hint: String,
        count: Int = 5,
        fixedByEncoder: Bool = false,
        @ViewBuilder control: () -> Control
    ) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack(spacing: 6) {
                Text(label).font(.caption)
                Spacer(minLength: 4)
                ImpactGauge(level: level, count: count)
            }
            control()
            Text(fixedByEncoder ? "\(hint)\n\(L10n.t("settings.encoderLockedHint"))" : hint)
                .font(.caption2)
                .foregroundStyle(Color(white: 0.5))
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    private func meteredToggle(
        _ label: String,
        _ level: Int,
        _ hint: String,
        _ value: Binding<Bool>,
        enabled: Bool
    ) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack(spacing: 6) {
                Toggle(label, isOn: value)
                    .toggleStyle(.checkbox)
                    .disabled(locked || !enabled)
                Spacer(minLength: 4)
                ImpactGauge(level: level)
            }
            Text(enabled ? hint : "\(hint)\n\(L10n.t("settings.encoderLockedHint"))")
                .font(.caption2)
                .foregroundStyle(Color(white: 0.5))
                .fixedSize(horizontal: false, vertical: true)
        }
    }
}

/// One RTMP destination, laid out like the Windows `BuildStreamRow`.
private struct StreamDestinationCard: View {
    @Binding var output: OutputEntry
    let locked: Bool
    @EnvironmentObject private var mixer: MixerController

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 8) {
                TextField(L10n.t("settings.streamName"), text: $output.name)
                    .textFieldStyle(.roundedBorder)
                    .disabled(locked)
                Button {
                    mixer.removeOutput(output)
                } label: {
                    Image(systemName: "minus")
                }
                .disabled(locked)
                .help(L10n.t("settings.removeStream"))
            }

            labeled(L10n.t("settings.useEncode")) {
                Picker("", selection: $output.streamEncoderId) {
                    if mixer.session.streamEncoders.isEmpty {
                        Text(L10n.t("settings.encodingEmptyShort")).tag(UInt64(0))
                    }
                    ForEach(mixer.session.streamEncoders) { encoder in
                        Text(encoder.name.isEmpty ? L10n.t("settings.encodeName") : encoder.name)
                            .tag(encoder.id)
                    }
                    let current = output.streamEncoderId
                    if current != 0
                        && !mixer.session.streamEncoders.contains(where: { $0.id == current }) {
                        Text(L10n.t("settings.encodeMissing")).tag(current)
                    }
                }
                .labelsHidden()
                .frame(maxWidth: 220)
                .disabled(locked)
            }

            labeled(L10n.t("settings.streamServer")) {
                TextField(L10n.t("settings.rtmpUrl"), text: $output.rtmpUrl)
                    .textFieldStyle(.roundedBorder)
                    .disabled(locked)
            }

            labeled(L10n.t("settings.streamKey")) {
                RtmpKeyEditor(output: $output)
                    .disabled(locked)
            }

            HStack(spacing: 12) {
                Button(L10n.t("settings.apply")) {
                    mixer.addOutput(output)
                }
                .disabled(locked)

                Toggle(L10n.t("settings.enabled"), isOn: Binding(
                    get: { output.enabled },
                    set: { value in
                        output.enabled = value
                        mixer.addOutput(output)
                    }
                ))
                .toggleStyle(.checkbox)
                .disabled(locked)

                // macOS picks a source per destination; the Windows page keeps the
                // same source fields on the profile, so this stays here without
                // changing the profile fields above.
                Picker("", selection: Binding(
                    get: { output.sourceKind },
                    set: { kind in
                        let wasMultiview = output.sourceKind == .multiview
                        output.sourceKind = kind
                        if kind == .multiview {
                            output.audioUnitId = 0
                        } else if wasMultiview {
                            output.audioUnitId = output.unitId == 0 ? 1 : output.unitId
                        }
                        switch kind {
                        case .multiview:
                            if mixer.session.multiviews.first(where: { $0.gpuId == output.sourceId }) == nil {
                                output.sourceId = mixer.session.multiviews.first?.gpuId ?? 0
                            }
                        case .scene:
                            if mixer.session.scenes.first(where: { $0.gpuId == output.sourceId }) == nil {
                                output.sourceId = mixer.session.scenes.first?.gpuId ?? 0
                            }
                        case .input:
                            if mixer.session.inputs.first(where: { $0.id == output.sourceId }) == nil {
                                output.sourceId = mixer.session.inputs.first?.id ?? 0
                            }
                        case .muPreview, .muProgram:
                            break
                        }
                        mixer.addOutput(output)
                    }
                )) {
                    Text("MU PGM").tag(OutputSourceKind.muProgram)
                    Text("MU PRV").tag(OutputSourceKind.muPreview)
                    Text("Scene").tag(OutputSourceKind.scene)
                    Text("Input").tag(OutputSourceKind.input)
                    Text("Multiview").tag(OutputSourceKind.multiview)
                }
                .labelsHidden()
                .frame(maxWidth: 140)
                .disabled(locked)
            }
        }
        .padding(8)
        .background(
            Rectangle()
                .stroke(Color(white: 0.27), lineWidth: 1)
        )
    }

    private func labeled<Content: View>(_ label: String, @ViewBuilder content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(label).font(.caption)
            content()
        }
    }
}

/// The Windows `ImpactGauge`: five pips, green to amber to red.
struct ImpactGauge: View {
    var level: Int
    var count: Int = 5

    private var fill: Color {
        let ratio = count == 0 ? 0 : Double(level) / Double(count)
        if ratio >= 0.75 { return Color(red: 224 / 255, green: 120 / 255, blue: 120 / 255) }
        if ratio >= 0.45 { return Color(red: 224 / 255, green: 195 / 255, blue: 106 / 255) }
        return Color(red: 125 / 255, green: 200 / 255, blue: 125 / 255)
    }

    var body: some View {
        HStack(spacing: 2) {
            ForEach(0 ..< count, id: \.self) { index in
                RoundedRectangle(cornerRadius: 1)
                    .fill(index < level ? fill : Color(white: 0.20))
                    .frame(width: count > 5 ? 7 : 10, height: 8)
            }
        }
        .accessibilityHidden(true)
    }
}

/// Per-setting load gauges, ported from the Windows `PaintEncodeLoad`.
private struct EncodeMeters {
    let total: Int
    let video: Int
    let audio: Int
    let keyint: Int
    let size: Int
    let fps: Int
    let threads: Int
    let buffer: Int
    let encoder: Int
    let complexity: Int
    let refs: Int
    let delay: Int
    let entropy: Int
    let deblock: Int
    let background: Int
    let quant: Int
    let scene: Int

    init(_ entry: StreamEncoderEntry) {
        let videoKbps = entry.videoBitrate == 0 ? 6000 : entry.videoBitrate / 1000
        video = videoKbps <= 2500 ? 1 : videoKbps <= 4500 ? 2 : videoKbps <= 8000 ? 3 : videoKbps <= 12000 ? 4 : 5
        audio = 1
        let key = entry.keyint == 0 ? 120 : entry.keyint
        keyint = key < 30 ? 5 : key < 60 ? 4 : key < 90 ? 3 : key < 180 ? 2 : 1
        let height = entry.height == 0 ? 1080 : entry.height
        size = height >= 2160 ? 5 : height >= 1440 ? 4 : height >= 1080 ? 3 : height >= 720 ? 2 : 1
        let rate = entry.fpsDen == 0 || entry.fpsNum == 0
            ? 60
            : Double(entry.fpsNum) / Double(entry.fpsDen)
        fps = rate >= 59 ? 5 : rate >= 50 ? 4 : rate >= 30 ? 2 : 1
        threads = entry.encoderThreads == 1 ? 5 : entry.encoderThreads == 2 ? 4 : entry.encoderThreads == 3 ? 3 : 1
        buffer = 0
        encoder = 4
        complexity = entry.complexity == 0 ? 1 : entry.complexity == 1 ? 3 : 5
        let frames = entry.refFrames >= 1 && entry.refFrames <= 4 ? Int(entry.refFrames) : 1
        refs = frames
        delay = 0
        entropy = entry.cabac ? 4 : 1
        deblock = entry.deblock ? 2 : 1
        background = entry.backgroundDetection ? 1 : 3
        quant = entry.adaptiveQuant ? 3 : 1
        scene = entry.sceneChange ? 3 : 1
        total = EncodeMeters.loadPips(entry)
    }

    /// The Windows `EncodeLoadPips` score, clamped to ten pips.
    private static func loadPips(_ encoder: StreamEncoderEntry) -> Int {
        var score: Int = encoder.complexity == 0 ? 8 : encoder.complexity == 1 ? 18 : 30
        let frames = encoder.refFrames >= 1 && encoder.refFrames <= 4 ? Int(encoder.refFrames) : 1
        score += frames * 4
        if encoder.cabac { score += 10 }
        if encoder.deblock { score += 6 }
        score += encoder.backgroundDetection ? 2 : 6
        if encoder.adaptiveQuant { score += 6 }
        if encoder.sceneChange { score += 6 }
        score += encoder.encoderThreads == 1 ? 16 : encoder.encoderThreads == 2 ? 8 : encoder.encoderThreads == 3 ? 4 : 0
        let pixels = encoder.width == 0 || encoder.height == 0
            ? 1920 * 1080
            : Int(encoder.width) * Int(encoder.height)
        score += pixels > 1920 * 1080 ? 28 : pixels >= 1920 * 1080 ? 12 : 6
        let fpsMilli = encoder.fpsDen == 0 || encoder.fpsNum == 0
            ? 60_000
            : Int(UInt64(encoder.fpsNum) * 1000 / UInt64(encoder.fpsDen))
        score += fpsMilli >= 50_000 ? 10 : 4
        let kbps = encoder.videoBitrate == 0 ? 6000 : Int(encoder.videoBitrate / 1000)
        score += min(kbps / 2000, 8)
        if encoder.keyint > 0 && encoder.keyint < 60 { score += 4 }
        score += 8
        return max(1, min(10, Int((Double(score - 45) * 10 / 90).rounded())))
    }
}

/// Plan limits the Stream page enforces while the operator edits a profile.
struct StreamCaps {
    var linked = false
    var maxWidth: UInt32 = 0
    var maxHeight: UInt32 = 0
    var maxFpsNum: UInt32 = 0
    var maxFpsDen: UInt32 = 0
    var encoderMask: UInt32 = 0

    static func read() -> StreamCaps {
        var caps = EivizCapabilities()
        guard mixer_capabilities(&caps) == 0 else { return StreamCaps() }
        return StreamCaps(
            linked: caps.rtmp_linked != 0 && caps.rtmp_max_width != 0,
            maxWidth: caps.rtmp_max_width,
            maxHeight: caps.rtmp_max_height,
            maxFpsNum: caps.rtmp_max_fps_num,
            maxFpsDen: caps.rtmp_max_fps_den,
            encoderMask: caps.stream_encoder_mask
        )
    }

    func allowsSize(_ width: UInt32, _ height: UInt32) -> Bool {
        if width == 0 || height == 0 { return true }
        return maxWidth != 0 && maxHeight != 0 && width <= maxWidth && height <= maxHeight
    }

    func allowsFps(_ num: UInt32, _ den: UInt32) -> Bool {
        if num == 0 || den == 0 { return true }
        guard maxFpsNum != 0, maxFpsDen != 0 else { return false }
        return UInt64(num) * UInt64(maxFpsDen) <= UInt64(maxFpsNum) * UInt64(den)
    }
}

/// Which encode-profile controls the selected encoder honours.
struct EncoderSupport {
    var threads = true
    var complexity = true
    var refFrames = true
    var entropy = true
    var deblock = true
    var background = true
    var adaptiveQuant = true
    var sceneChange = true
    /// Values the encoder requires; nil keeps whatever the operator chose.
    var threadsValue: UInt32?
    var deblockValue: Bool?
    var backgroundValue: Bool?
    var adaptiveQuantValue: Bool?
    var sceneChangeValue: Bool?

    static func of(_ encoder: String) -> EncoderSupport {
        switch encoder {
        case "VideoToolbox":
            // `vt_preset_supported` in the module: automatic threads and the
            // OpenH264-shaped preset only. It refuses a pinned count and any
            // other combination at stream start.
            return EncoderSupport(
                threads: false, complexity: true, refFrames: true, entropy: true,
                deblock: false, background: false, adaptiveQuant: false, sceneChange: false,
                threadsValue: 0, deblockValue: true, backgroundValue: true,
                adaptiveQuantValue: false, sceneChangeValue: false
            )
        case "MediaFoundation":
            // The MFT has no switches for these; it honours threads and reference
            // frames only.
            return EncoderSupport(
                threads: true, complexity: false, refFrames: true, entropy: false,
                deblock: false, background: false, adaptiveQuant: false, sceneChange: false,
                threadsValue: nil, deblockValue: nil, backgroundValue: nil,
                adaptiveQuantValue: nil, sceneChangeValue: nil
            )
        default:
            // OpenH264 implements every switch.
            return EncoderSupport(
                threads: true, complexity: true, refFrames: true, entropy: true,
                deblock: true, background: true, adaptiveQuant: true, sceneChange: true,
                threadsValue: nil, deblockValue: nil, backgroundValue: nil,
                adaptiveQuantValue: nil, sceneChangeValue: nil
            )
        }
    }
}
