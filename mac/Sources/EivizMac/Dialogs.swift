import AppKit
import EivizMixer
import SwiftUI
import UniformTypeIdentifiers

struct SettingsView: View {
    @EnvironmentObject private var mixer: MixerController
    @Environment(\.dismiss) private var dismiss
    @State private var category = 0
    @State private var selectedMultiviewId: UInt64?

    var body: some View {
        HStack(spacing: 0) {
            List(selection: $category) {
                Text("Display").tag(0)
                Text("Performance").tag(1)
                Text("Outputs").tag(2)
                Text("Multiview").tag(3)
                Text("Audio Auxiliary").tag(4)
                Text("About").tag(5)
            }
            .frame(width: 200)
            .listStyle(.sidebar)
            VStack(alignment: .leading, spacing: 12) {
                Group {
                    if category == 0 { display }
                    else if category == 1 { performance }
                    else if category == 2 { outputs }
                    else if category == 3 { multiview }
                    else if category == 4 { audio }
                    else { about }
                }
                Spacer()
                HStack {
                    Spacer()
                    Button("OK") {
                        mixer.pushAudio()
                        mixer.applyBusColors()
                        _ = mixer_set_rebar_optimization(mixer.session.settings.rebarOptimizationEnabled ? 1 : 0)
                        _ = mixer_set_ndi_gpu_upload(mixer.session.settings.ndiGpuUploadEnabled ? 1 : 0)
                        for layout in mixer.session.multiviews {
                            mixer.pushMultiview(layout)
                        }
                        dismiss()
                    }
                    Button("Cancel") { dismiss() }
                }
            }
            .padding(20)
            .frame(minWidth: 520, minHeight: 420)
        }
        .background(EivizTheme.dialog)
        .foregroundStyle(EivizTheme.text)
        .sheet(isPresented: $mixer.showMultiviewSlots) { MultiviewSlotsView() }
    }

    private var display: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Preview color")
            ColorPicker("", selection: Binding(
                get: { mixer.session.settings.previewColor.color },
                set: { mixer.session.settings.previewColor = RgbColor($0) }
            ), supportsOpacity: false)
            .labelsHidden()
            .frame(width: 220, alignment: .leading)
            Text("Program color")
            ColorPicker("", selection: Binding(
                get: { mixer.session.settings.programColor.color },
                set: { mixer.session.settings.programColor = RgbColor($0) }
            ), supportsOpacity: false)
            .labelsHidden()
            .frame(width: 220, alignment: .leading)
            Text("Inactive color")
            ColorPicker("", selection: Binding(
                get: { mixer.session.settings.inactiveColor.color },
                set: { mixer.session.settings.inactiveColor = RgbColor($0) }
            ), supportsOpacity: false)
            .labelsHidden()
            .frame(width: 220, alignment: .leading)
            Text("Master Frame Rate")
            Picker("", selection: Binding(
                get: { "\(mixer.session.settings.masterFpsNum)/\(mixer.session.settings.masterFpsDen)" },
                set: { value in
                    let parts = value.split(separator: "/").compactMap { UInt32($0) }
                    if parts.count == 2 {
                        mixer.session.settings.masterFpsNum = parts[0]
                        mixer.session.settings.masterFpsDen = parts[1]
                    }
                }
            )) {
                Text("NTSC 59.94p").tag("60000/1001")
                Text("50p").tag("50/1")
                Text("30p").tag("30/1")
                Text("24p").tag("24/1")
                Text("60p").tag("60/1")
            }
            .frame(width: 220)
            Text("Frame buffer (frames)")
            Picker("", selection: $mixer.session.settings.frameBufferFrames) {
                ForEach([UInt32(1), 2, 3, 4, 6, 8], id: \.self) { Text("\($0)").tag($0) }
            }
            .frame(width: 220)
            Text("Master frame rate clocks the render thread. Restart the mixer after changing it.")
                .foregroundStyle(EivizTheme.dim)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    private var performance: some View {
        VStack(alignment: .leading, spacing: 8) {
            let info = copyUmaInfo()
            Text("Graphics Adapter").fontWeight(.bold)
            Text(info.name)
            Text("Unified Memory")
            Text(info.uma ? "Enabled" : "Not available")
            Toggle("Use Unified Memory optimization", isOn: Binding(
                get: { mixer.session.settings.rebarOptimizationEnabled },
                set: { mixer.session.settings.rebarOptimization = $0 }
            ))
            .disabled(!info.available)
            Toggle("Upload NDI on the ingest thread", isOn: Binding(
                get: { mixer.session.settings.ndiGpuUploadEnabled },
                set: { mixer.session.settings.ndiGpuUpload = $0 }
            ))
            Text("NDI upload on the ingest thread writes each frame to the GPU before the mixer samples it. Turn it off to go back to CPU frames on the render thread. On Apple Silicon, Unified Memory writes live CPU inputs into MTLStorageModeShared textures and samples them directly. Turn that off to use the default Metal upload path.")
                .foregroundStyle(EivizTheme.dim)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    private func copyUmaInfo() -> (name: String, uma: Bool, available: Bool) {
        var info = EivizMixerRebarInfo()
        guard mixer_copy_rebar_info(&info) == 0 else {
            return ("Mixer is not running.", false, false)
        }
        var name = "—"
        withUnsafeBytes(of: info.adapter) { raw in
            if let base = raw.baseAddress?.assumingMemoryBound(to: CChar.self), base.pointee != 0 {
                name = String(cString: base)
            }
        }
        return (name, info.uma != 0, info.available != 0)
    }

    private var outputs: some View {
        VStack(alignment: .leading) {
            HStack {
                Text("Outputs").fontWeight(.bold)
                Spacer()
                Button("+") {
                    let output = OutputEntry(
                        id: mixer.session.nextOutputId,
                        name: "eiviz-out-\(mixer.session.nextOutputId)",
                        unitId: mixer.selectedUnitId
                    )
                    mixer.session.nextOutputId += 1
                    mixer.session.outputs.append(output)
                    mixer.addOutput(output)
                }
            }
            Text("OMT and NDI® are sent from the mixer. NDI uses CPU encode.")
                .foregroundStyle(EivizTheme.dim)
            ForEach($mixer.session.outputs) { $output in
                outputRow($output)
            }
        }
    }

    private func outputRow(_ output: Binding<OutputEntry>) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                mixerTextField(output.name, placeholder: "Name")
                Picker("", selection: Binding(
                    get: { output.wrappedValue.transport },
                    set: { value in
                        output.wrappedValue.transport = value
                        if value != .omt {
                            output.wrappedValue.useGpu = false
                        }
                    }
                )) {
                    Text("OMT").tag(OutputTransport.omt)
                    Text("NDI®").tag(OutputTransport.ndi)
                }
                if output.wrappedValue.transport == .omt {
                    Toggle("GPU", isOn: output.useGpu)
                }
                Toggle("Enabled", isOn: Binding(
                    get: { output.wrappedValue.enabled },
                    set: { value in
                        output.wrappedValue.enabled = value
                        mixer.addOutput(output.wrappedValue)
                    }
                ))
                Button("Apply") { mixer.addOutput(output.wrappedValue) }
                Button("−") {
                    _ = mixer_output_remove(output.wrappedValue.id)
                    mixer.session.outputs.removeAll { $0.id == output.wrappedValue.id }
                }
            }
            HStack {
                Picker("", selection: output.sourceKind) {
                    Text("Input").tag(OutputSourceKind.input)
                    Text("Scene").tag(OutputSourceKind.scene)
                    Text("MU PRV").tag(OutputSourceKind.muPreview)
                    Text("MU PGM").tag(OutputSourceKind.muProgram)
                    Text("Multiview").tag(OutputSourceKind.multiview)
                }
                outputSourcePick(output)
            }
        }
        .padding(6)
        .overlay(Rectangle().stroke(EivizTheme.stroke, lineWidth: 1))
    }

    @ViewBuilder
    private func outputSourcePick(_ output: Binding<OutputEntry>) -> some View {
        switch output.wrappedValue.sourceKind {
        case .scene:
            Picker("", selection: output.sourceId) {
                ForEach(mixer.session.scenes) { scene in
                    Text(scene.name).tag(scene.gpuId)
                }
            }
        case .input:
            Picker("", selection: output.sourceId) {
                ForEach(mixer.session.inputs) { input in
                    Text(input.name).tag(input.id)
                }
            }
        case .multiview:
            Picker("", selection: output.sourceId) {
                ForEach(mixer.session.multiviews) { layout in
                    Text(layout.name).tag(layout.gpuId)
                }
            }
        case .muPreview, .muProgram:
            Picker("", selection: output.unitId) {
                ForEach(mixer.session.units) { unit in
                    Text(unit.name).tag(unit.id)
                }
            }
        }
    }

    private var multiview: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text("Multiviews").fontWeight(.bold)
                Spacer()
                Button("Open") { openSelectedMultiview() }
                Button("Layout…") { editSelectedLayout() }
                Button("Delete") {
                    if let id = selectedMultiviewId {
                        mixer.deleteMultiview(id)
                    }
                }
                Button("+") {
                    mixer.openNewMultiview()
                    dismiss()
                }
            }
            .buttonStyle(MixerButtonStyle())
            List(mixer.session.multiviews, selection: $selectedMultiviewId) { layout in
                Text(layout.name).tag(Optional(layout.id))
            }
            .frame(minHeight: 120)
            Text("Default Mixing Unit for new Multiview windows")
            Picker("", selection: $mixer.session.settings.defaultMultiviewUnitId) {
                ForEach(mixer.session.units) { unit in
                    Text(unit.name).tag(unit.id)
                }
            }
            .frame(width: 280)
            Text("Project default preview refresh interval")
            Picker("", selection: $mixer.session.settings.defaultPresentInterval) {
                Text("Every frame").tag(UInt32(1))
                Text("Every 2 frames").tag(UInt32(2))
                Text("Every 3 frames").tag(UInt32(3))
                Text("Every 4 frames").tag(UInt32(4))
                Text("Every 6 frames").tag(UInt32(6))
                Text("Every 8 frames").tag(UInt32(8))
            }
            .frame(width: 280)
            Text("Each Multiview window picks its own mosaic template (Preview + Program plus matching tiles, or a full 2×2 / 3×3 / 4×4).")
                .foregroundStyle(EivizTheme.dim)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    private func openSelectedMultiview() {
        if let id = selectedMultiviewId,
           let layout = mixer.session.multiviews.first(where: { $0.id == id })
        {
            mixer.openMultiviewWindow(layout)
        } else {
            mixer.openNewMultiview()
            selectedMultiviewId = mixer.openMultiview?.id
        }
        dismiss()
    }

    private func editSelectedLayout() {
        guard let id = selectedMultiviewId,
              let layout = mixer.session.multiviews.first(where: { $0.id == id })
        else { return }
        mixer.openMultiview = layout
        mixer.showMultiviewSlots = true
    }

    private var audio: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Internal mix is 48 kHz stereo. Enabled keeps the bus in the mix with no hardware device. Core Audio sends that mix to a device. Master and Headphone cannot be removed.")
                .foregroundStyle(EivizTheme.dim)
                .fixedSize(horizontal: false, vertical: true)
            Toggle("Headphone copies Master", isOn: $mixer.session.headphoneCopyMaster)
            HStack {
                Spacer()
                Button("+") { addAuxBus() }
            }
            ForEach($mixer.session.buses) { $bus in
                busRow($bus)
            }
        }
    }

    private func addAuxBus() {
        let aux = mixer.session.buses.filter { $0.role == .aux }.count
        guard aux < 8 else { return }
        var bit: UInt32 = 2
        while mixer.session.buses.contains(where: { $0.bit == bit }) && bit < 31 {
            bit += 1
        }
        mixer.session.buses.append(AudioBusEntry(
            id: mixer.session.nextBusId,
            name: nextAuxName(),
            role: .aux,
            deviceKind: .none,
            bit: bit
        ))
        mixer.session.nextBusId += 1
    }

    private func nextAuxName() -> String {
        for letter in "ABCDEFGH" {
            let name = "Bus \(letter)"
            if mixer.session.buses.allSatisfy({ $0.name != name }) {
                return name
            }
        }
        return "Bus \(mixer.session.nextBusId)"
    }

    private func busRow(_ bus: Binding<AudioBusEntry>) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                mixerTextField(bus.name, placeholder: "Name")
                    .disabled(bus.wrappedValue.role != .aux)
                if bus.wrappedValue.role == .aux {
                    Button("−") {
                        mixer.session.buses.removeAll { $0.id == bus.wrappedValue.id }
                    }
                }
            }
            HStack {
                Picker("", selection: bus.deviceKind) {
                    Text("Enabled").tag(AudioDeviceKind.none)
                    Text("Core Audio").tag(AudioDeviceKind.coreAudio)
                }
                .frame(width: 140)
                if bus.wrappedValue.deviceKind != .none {
                    Picker("", selection: bus.deviceId) {
                        Text("Default").tag("")
                        ForEach(devices(for: bus.wrappedValue.deviceKind)) { device in
                            Text("\(device.name)  (\(device.channels)ch)").tag(device.id)
                        }
                    }
                }
            }
            if bus.wrappedValue.deviceKind != .none {
                HStack {
                    Text("L ch")
                    mixerInt32Field(bus.mapLeft).frame(width: 48)
                    Text("R ch")
                    mixerInt32Field(bus.mapRight).frame(width: 48)
                }
            }
            HStack {
                Slider(value: bus.gain, in: 0 ... 2)
                Toggle("Mute", isOn: bus.mute)
            }
        }
        .padding(8)
        .overlay(Rectangle().stroke(EivizTheme.stroke, lineWidth: 1))
    }

    private func devices(for kind: AudioDeviceKind) -> [AudioDevice] {
        MixerFFI.audioDevices().filter {
            $0.kind == kind.rawUInt
                || (kind == .coreAudio && $0.kind == AudioDeviceKind.wasapi.rawUInt)
        }
    }

    private var about: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("eiviz").font(.title)
            Text("Version \(HostVersion.display)")
            Text("eiviz is an experimental software switcher developed and maintained by Mikansei Laboratory.")
                .fixedSize(horizontal: false, vertical: true)
            Text("Shugo Kawamura")
            Link("https://github.com/MikanseiLaboratory/eiviz", destination: URL(string: "https://github.com/MikanseiLaboratory/eiviz")!)
            Link("https://mikanseilaboratory.github.io/", destination: URL(string: "https://mikanseilaboratory.github.io/")!)
            Text("Open source").fontWeight(.bold)
            Text("eiviz original source is PolyForm Shield License 1.0.0. Third-party crates stay MIT / Apache-2.0 / Zlib. NDI® is a trademark of Vizrt NDI AB.")
                .foregroundStyle(EivizTheme.dim)
                .fixedSize(horizontal: false, vertical: true)
        }
    }
}

struct AddInputView: View {
    @EnvironmentObject private var mixer: MixerController
    @Environment(\.dismiss) private var dismiss
    let editing: InputEntry?
    @State private var category = "Still"
    @State private var name = ""
    @State private var stillPath = ""
    @State private var videoPath = ""
    @State private var omtAddress = ""
    @State private var ndiAddress = ""
    @State private var omtList: [String] = []
    @State private var ndiList: [String] = []
    @State private var uvcList: [VideoCaptureDevice] = []
    @State private var selectedUvc: String = ""
    @State private var r: Double = 220
    @State private var g: Double = 32
    @State private var b: Double = 32
    @State private var bars = false
    @State private var scroll = false
    @State private var toneHz: Float = 1000
    @State private var useGpu = true
    @State private var buffer: UInt32 = 1
    @State private var quality: UInt32 = 0
    @State private var ndiLow = false
    @State private var videoLoop = true
    @State private var videoPlayWhen: VideoPlayWhen = .never
    @State private var videoRestartWhen: VideoTriggerWhen = .never
    @State private var videoPauseWhen: VideoTriggerWhen = .never

    var body: some View {
        HStack(spacing: 0) {
            List(["Colours", "Still", "Video", "OMT", "NDI®", "Video Capture"], id: \.self, selection: $category) { item in
                Text(item).tag(item)
            }
            .frame(width: 200)
            VStack(alignment: .leading, spacing: 12) {
                Text("Name")
                mixerTextField($name, placeholder: defaultName())
                form
                Spacer()
                HStack {
                    Spacer()
                    Button("OK") { commit(); dismiss() }
                    Button("Cancel") { dismiss() }
                }
            }
            .padding(16)
            .frame(minWidth: 520, minHeight: 480)
        }
        .background(EivizTheme.dialog)
        .foregroundStyle(EivizTheme.text)
        .onAppear(perform: loadEditing)
    }

    @ViewBuilder
    private var form: some View {
        switch category {
        case "Colours":
            Toggle("SMPTE HD colour bars", isOn: $bars)
            if !bars {
                colorSlider("R", $r)
                colorSlider("G", $g)
                colorSlider("B", $b)
                Rectangle().fill(Color(red: r / 255, green: g / 255, blue: b / 255)).frame(height: 48)
            }
            Toggle("Scroll", isOn: $scroll)
            Picker("Test tone", selection: $toneHz) {
                Text("Mute").tag(Float(0))
                Text("440 Hz").tag(Float(440))
                Text("1 kHz").tag(Float(1000))
                Text("2 kHz").tag(Float(2000))
            }
            Text("Scroll shifts SMPTE HD bars (or a white ident on solid colours). Tone is -20 dBFS.")
                .foregroundStyle(EivizTheme.dim)
                .fixedSize(horizontal: false, vertical: true)
        case "Still":
            pathRow($stillPath) { pick(["public.image"], $stillPath) }
        case "Video":
            pathRow($videoPath) { pick(["public.movie"], $videoPath) }
            Toggle("Loop", isOn: $videoLoop)
            Picker("Play when", selection: $videoPlayWhen) {
                Text("Never (manual)").tag(VideoPlayWhen.never)
                Text("Active (Program)").tag(VideoPlayWhen.onActive)
                Text("On Preview").tag(VideoPlayWhen.onPreview)
                Text("Always").tag(VideoPlayWhen.always)
            }
            Picker("Restart when", selection: $videoRestartWhen) {
                Text("Never").tag(VideoTriggerWhen.never)
                Text("Active (Program)").tag(VideoTriggerWhen.onActive)
                Text("Taken off Active").tag(VideoTriggerWhen.onDeactivated)
                Text("On Preview").tag(VideoTriggerWhen.onPreview)
            }
            Picker("Pause when", selection: $videoPauseWhen) {
                Text("Never").tag(VideoTriggerWhen.never)
                Text("Active (Program)").tag(VideoTriggerWhen.onActive)
                Text("Taken off Active").tag(VideoTriggerWhen.onDeactivated)
                Text("On Preview").tag(VideoTriggerWhen.onPreview)
            }
        case "OMT":
            mixerTextField($omtAddress, placeholder: "OMT source address")
            Button("Refresh discovery") { refreshOmt() }
            List(omtList, id: \.self, selection: $omtAddress) { Text($0) }
                .frame(height: 120)
            Toggle("GPU decode", isOn: $useGpu)
        case "NDI®":
            mixerTextField($ndiAddress, placeholder: "NDI® source")
            Button("Refresh discovery") { refreshNdi() }
            List(ndiList, id: \.self, selection: $ndiAddress) { Text($0) }
                .frame(height: 120)
            Toggle("Lowest bandwidth", isOn: $ndiLow)
            Text("NDI is received on the CPU and uploaded for compose.")
                .foregroundStyle(EivizTheme.dim)
        default:
            Button("Refresh devices") { refreshUvc() }
            List(uvcList, id: \.id, selection: $selectedUvc) { device in
                Text(device.name).tag(device.id)
            }
        }
    }

    private func colorSlider(_ label: String, _ value: Binding<Double>) -> some View {
        HStack {
            Text(label).frame(width: 16)
            Slider(value: value, in: 0 ... 255)
        }
    }

    private func pathRow(_ text: Binding<String>, browse: @escaping () -> Void) -> some View {
        HStack {
            mixerTextField(text, placeholder: "Path")
            Button("Browse", action: browse)
        }
    }

    private func pick(_ types: [String], _ dest: Binding<String>) {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = types.compactMap { UTType($0) }
        panel.allowsMultipleSelection = false
        if panel.runModal() == .OK, let url = panel.url {
            dest.wrappedValue = url.path
            if name.isEmpty { name = url.deletingPathExtension().lastPathComponent }
        }
    }

    private func refreshOmt() {
        omtList = MixerFFI.discover { mixer_omt_discover($0, $1) }
    }

    private func refreshNdi() {
        ndiList = MixerFFI.discover { mixer_ndi_discover($0, $1) }
    }

    private func refreshUvc() {
        uvcList = MixerFFI.videoCaptures()
    }

    private func loadEditing() {
        refreshOmt()
        refreshNdi()
        refreshUvc()
        guard let editing else { return }
        name = editing.name
        category = editing.kind.category
        stillPath = editing.kind == .still ? (editing.pathOrAddress ?? "") : stillPath
        videoPath = editing.kind == .video ? (editing.pathOrAddress ?? "") : videoPath
        omtAddress = editing.kind == .omt ? (editing.pathOrAddress ?? "") : omtAddress
        ndiAddress = editing.kind == .ndi ? (editing.pathOrAddress ?? "") : ndiAddress
        selectedUvc = editing.kind == .uvc ? (editing.pathOrAddress ?? "") : selectedUvc
        r = Double(editing.colorR) * 255
        g = Double(editing.colorG) * 255
        b = Double(editing.colorB) * 255
        bars = editing.kind == .bars
        scroll = editing.scroll
        toneHz = editing.toneHz
        useGpu = editing.useGpu
        buffer = editing.frameBufferFrames
        ndiLow = editing.ndiBandwidth == .lowest
        quality = editing.omtQuality.rawUInt
        videoLoop = editing.videoLoop
        videoPlayWhen = editing.videoPlayWhen
        videoRestartWhen = editing.videoRestartWhen
        videoPauseWhen = editing.videoPauseWhen
    }

    private func defaultName() -> String {
        switch category {
        case "Colours":
            if bars {
                return scroll ? "SMPTE HD Bars (scroll)" : "SMPTE HD Bars"
            }
            return String(format: "Colour %02X%02X%02X", Int(r), Int(g), Int(b))
        case "Still":
            return URL(fileURLWithPath: stillPath).lastPathComponent
        case "Video":
            return URL(fileURLWithPath: videoPath).lastPathComponent
        case "OMT":
            return omtAddress
        case "NDI®":
            return ndiAddress
        default:
            return uvcList.first { $0.id == selectedUvc }?.name ?? "Video Capture"
        }
    }

    private func commit() {
        let trimmed = name.trimmingCharacters(in: .whitespaces)
        var input = InputEntry(id: editing?.id ?? 0, name: trimmed.isEmpty ? defaultName() : trimmed, kind: .still)
        switch category {
        case "Colours":
            input.kind = bars ? .bars : .color
            input.colorR = Float(r / 255)
            input.colorG = Float(g / 255)
            input.colorB = Float(b / 255)
            input.scroll = scroll
            input.toneHz = toneHz
            input.toneLevelDbfs = toneHz > 0 ? -20 : 0
        case "Still":
            guard !stillPath.isEmpty else { return }
            input.kind = .still
            input.pathOrAddress = stillPath
        case "Video":
            guard !videoPath.isEmpty else { return }
            input.kind = .video
            input.pathOrAddress = videoPath
            input.videoLoop = videoLoop
            input.videoPlayWhen = videoPlayWhen
            input.videoRestartWhen = videoRestartWhen
            input.videoPauseWhen = videoPauseWhen
        case "OMT":
            guard !omtAddress.isEmpty else { return }
            input.kind = .omt
            input.pathOrAddress = omtAddress
            input.useGpu = useGpu
            input.frameBufferFrames = buffer
            input.omtQuality = switch quality {
            case 1: .low
            case 50: .medium
            case 100: .high
            default: .default
            }
        case "NDI®":
            guard !ndiAddress.isEmpty else { return }
            input.kind = .ndi
            input.pathOrAddress = ndiAddress
            input.ndiBandwidth = ndiLow ? .lowest : .highest
            input.frameBufferFrames = buffer
            input.useGpu = false
        default:
            guard !selectedUvc.isEmpty else { return }
            input.kind = .uvc
            input.pathOrAddress = selectedUvc
        }
        mixer.upsertInput(input, replacing: editing?.id)
    }
}

struct MixingUnitView: View {
    @EnvironmentObject private var mixer: MixerController
    @Environment(\.dismiss) private var dismiss
    @State var unit: MixingUnitEntry

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            labeled("Name") {
                mixerTextField($unit.name, placeholder: "Name")
            }
            labeled("Width") { mixerUintField($unit.width) }
            labeled("Height") { mixerUintField($unit.height) }
            labeled("Frame rate") {
                Picker("", selection: Binding(
                    get: { "\(unit.fpsNum)/\(unit.fpsDen)" },
                    set: { value in
                        let parts = value.split(separator: "/").compactMap { UInt32($0) }
                        if parts.count == 2 {
                            unit.fpsNum = parts[0]
                            unit.fpsDen = parts[1]
                        }
                    }
                )) {
                    Text("59.94p").tag("60000/1001")
                    Text("50p").tag("50/1")
                    Text("30p").tag("30/1")
                    Text("24p").tag("24/1")
                    Text("60p").tag("60/1")
                }
            }
            labeled("Audio") {
                Picker("", selection: $unit.audioBusId) {
                    ForEach(mixer.session.buses) { bus in
                        Text(bus.name).tag(bus.id)
                    }
                }
            }
            Picker("Link", selection: $unit.audioLink) {
                Text("Follow").tag(AudioLinkMode.follow)
                Text("Independent").tag(AudioLinkMode.independent)
            }
            Text("Audio bus is which mix this Mixing Unit feeds. Follow: the bus mix follows Preview/Program and the T-bar.")
                .foregroundStyle(EivizTheme.dim)
            HStack {
                Spacer()
                Button("OK") {
                    mixer.saveUnit(unit)
                    dismiss()
                }
                Button("Cancel") { dismiss() }
            }
        }
        .padding(16)
        .frame(width: 420, height: 360)
        .background(EivizTheme.dialog)
        .foregroundStyle(EivizTheme.text)
    }

    private func labeled<V: View>(_ title: String, @ViewBuilder content: () -> V) -> some View {
        HStack {
            Text(title).frame(width: 80, alignment: .leading)
            content()
        }
    }
}

struct MultiviewView: View {
    @EnvironmentObject private var mixer: MixerController
    @Environment(\.dismiss) private var dismiss

    private var layout: MultiviewLayout? {
        if let open = mixer.openMultiview,
           let live = mixer.session.multiviews.first(where: { $0.id == open.id })
        {
            return live
        }
        return mixer.session.multiviews.last
    }

    var body: some View {
        VStack(alignment: .leading) {
            HStack {
                Text(layout?.name ?? "Multiview").fontWeight(.bold)
                Spacer()
                Button("Layout…") {
                    mixer.showMultiviewSlots = true
                }
                Button("Close") { dismiss() }
            }
            .buttonStyle(MixerButtonStyle())
            if let layout {
                MetalPreviewRepresentable(role: .monitor(monitorId: layout.monitorId, sourceId: layout.gpuId))
                    .frame(minHeight: 280)
                    .background(Color.black)
            } else {
                Text("Add a Multiview in Settings.")
                    .foregroundStyle(EivizTheme.dim)
            }
        }
        .padding(12)
        .frame(minWidth: 960, minHeight: 540)
        .background(EivizTheme.dialog)
        .foregroundStyle(EivizTheme.text)
        .onAppear {
            if let layout {
                mixer.pushMultiview(layout)
            }
        }
    }
}

struct MultiviewSlotsView: View {
    @EnvironmentObject private var mixer: MixerController
    @Environment(\.dismiss) private var dismiss
    @State private var previewUnit: UInt64 = 1
    @State private var programUnit: UInt64 = 1
    @State private var previewFollow = true
    @State private var previewLabel = ""
    @State private var programFollow = true
    @State private var programLabel = ""
    @State private var template: MultiviewTemplate = .previewProgram8
    @State private var tiles: [MvSlot] = Array(repeating: MvSlot(), count: 8)

    private var layoutId: UInt64? { mixer.openMultiview?.id }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Click a mosaic. Every pane matches the canvas aspect and fills the frame with no gaps, crop, or zoom.")
                .foregroundStyle(EivizTheme.dim)
                .fixedSize(horizontal: false, vertical: true)
            VStack(alignment: .leading, spacing: 8) {
                Text("Template").fontWeight(.bold)
                ForEach(MultiviewTemplate.groups, id: \.title) { group in
                    Text(group.title)
                        .foregroundStyle(EivizTheme.dim)
                    LazyVGrid(columns: [GridItem(.adaptive(minimum: 148), spacing: 8)], alignment: .leading, spacing: 8) {
                        ForEach(group.items) { item in
                            Button {
                                template = item
                            } label: {
                                VStack(spacing: 4) {
                                    MosaicThumb(template: item, selected: template == item)
                                    Text(item.title)
                                        .font(.system(size: 11))
                                }
                            }
                            .buttonStyle(.plain)
                        }
                    }
                }
            }
            .padding(8)
            .overlay(Rectangle().stroke(EivizTheme.stroke, lineWidth: 1))
            unitRow(
                template.busTitles.preview,
                $previewUnit,
                follow: $previewFollow,
                custom: $previewLabel
            )
            unitRow(
                template.busTitles.program,
                $programUnit,
                follow: $programFollow,
                custom: $programLabel
            )
            ScrollView {
                VStack(alignment: .leading, spacing: 8) {
                    ForEach(tiles.indices, id: \.self) { index in
                        tileRow(index)
                    }
                }
            }
            HStack {
                Spacer()
                Button("OK") { commit(); dismiss() }
                Button("Cancel") { dismiss() }
            }
        }
        .padding(16)
        .frame(width: 760, height: 720)
        .background(EivizTheme.dialog)
        .foregroundStyle(EivizTheme.text)
        .onAppear {
            if let id = layoutId, let layout = mixer.session.multiviews.first(where: { $0.id == id }) {
                previewUnit = layout.previewUnitId
                programUnit = layout.programUnitId
                previewFollow = layout.previewLabelFollow
                previewLabel = layout.previewLabel
                programFollow = layout.programLabelFollow
                programLabel = layout.programLabel
                template = layout.template
                tiles = layout.tiles
                resizeTiles()
            }
        }
        .onChange(of: template) { _, _ in
            resizeTiles()
        }
    }

    private func resizeTiles() {
        let want = template.tileCount
        if tiles.count < want {
            tiles.append(contentsOf: Array(repeating: MvSlot(), count: want - tiles.count))
        }
        if tiles.count > want {
            tiles.removeLast(tiles.count - want)
        }
    }

    private func unitRow(_ title: String, _ unit: Binding<UInt64>, follow: Binding<Bool>, custom: Binding<String>) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(title).fontWeight(.bold)
            Picker("", selection: unit) {
                ForEach(mixer.session.units) { item in
                    Text(item.name).tag(item.id)
                }
            }
            labelEditor(follow: follow, custom: custom)
        }
        .padding(8)
        .overlay(Rectangle().stroke(EivizTheme.stroke, lineWidth: 1))
    }

    private func labelEditor(follow: Binding<Bool>, custom: Binding<String>) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Picker("", selection: follow) {
                Text("Follow").tag(true)
                Text("Custom").tag(false)
            }
            .pickerStyle(.segmented)
            mixerTextField(custom)
                .disabled(follow.wrappedValue)
                .opacity(follow.wrappedValue ? 0.45 : 1)
        }
    }

    private func tileRow(_ index: Int) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Window \(index + 1)").fontWeight(.bold)
            Picker("", selection: Binding(
                get: { tiles[index].kind },
                set: { tiles[index].kind = $0; tiles[index].sourceId = 0 }
            )) {
                Text("None").tag(MvSlotKind.none)
                Text("Input").tag(MvSlotKind.input)
                Text("Scene").tag(MvSlotKind.scene)
            }
            .pickerStyle(.segmented)
            if tiles[index].kind == .input {
                Picker("", selection: Binding(
                    get: { tiles[index].sourceId },
                    set: { tiles[index].sourceId = $0 }
                )) {
                    ForEach(mixer.session.inputs) { input in
                        Text(input.name).tag(input.id)
                    }
                }
            } else if tiles[index].kind == .scene {
                Picker("", selection: Binding(
                    get: { tiles[index].sourceId },
                    set: { tiles[index].sourceId = $0 }
                )) {
                    ForEach(mixer.session.scenes) { scene in
                        Text(scene.name).tag(scene.gpuId)
                    }
                }
            }
            labelEditor(
                follow: Binding(
                    get: { tiles[index].labelFollow },
                    set: { tiles[index].labelFollow = $0 }
                ),
                custom: Binding(
                    get: { tiles[index].label },
                    set: { tiles[index].label = $0 }
                )
            )
        }
        .padding(8)
        .overlay(Rectangle().stroke(EivizTheme.stroke, lineWidth: 1))
    }

    private func commit() {
        guard let id = layoutId,
              let index = mixer.session.multiviews.firstIndex(where: { $0.id == id })
        else { return }
        mixer.session.multiviews[index].template = template
        mixer.session.multiviews[index].previewUnitId = previewUnit
        mixer.session.multiviews[index].programUnitId = programUnit
        mixer.session.multiviews[index].previewLabelFollow = previewFollow
        mixer.session.multiviews[index].previewLabel = previewLabel
        mixer.session.multiviews[index].programLabelFollow = programFollow
        mixer.session.multiviews[index].programLabel = programLabel
        mixer.session.multiviews[index].tiles = tiles
        mixer.session.multiviews[index].ensureTiles()
        mixer.openMultiview = mixer.session.multiviews[index]
        mixer.pushMultiview(mixer.session.multiviews[index])
    }
}

private struct MosaicThumb: View {
    var template: MultiviewTemplate
    var selected: Bool

    var body: some View {
        GeometryReader { geo in
            let w = geo.size.width
            let h = geo.size.height
            ZStack(alignment: .topLeading) {
                Rectangle().fill(Color.black)
                ForEach(Array(template.panes.enumerated()), id: \.offset) { _, pane in
                    Rectangle()
                        .fill(Color(white: 0.29))
                        .overlay(Rectangle().stroke(Color.black, lineWidth: 1))
                        .frame(width: max(1, CGFloat(pane.width) * w - 1), height: max(1, CGFloat(pane.height) * h - 1))
                        .offset(x: CGFloat(pane.x) * w, y: CGFloat(pane.y) * h)
                    if pane.kind != .tile {
                        Text(pane.kind == .preview ? "PRV" : "PGM")
                            .font(.system(size: 8))
                            .foregroundStyle(EivizTheme.text.opacity(0.8))
                            .offset(x: CGFloat(pane.x) * w + 3, y: CGFloat(pane.y) * h + 2)
                    }
                }
            }
        }
        .aspectRatio(16 / 9, contentMode: .fit)
        .frame(height: 83)
        .overlay(Rectangle().stroke(selected ? Color.white : EivizTheme.stroke, lineWidth: selected ? 2 : 1))
    }
}

private struct ResourceRow: Identifiable {
    var id: UInt64
    var name: String
    var kind: String
    var size: String
    var cpu: String
    var gpu: String
    var ram: String
    var vram: String
}

struct ResourcesView: View {
    @EnvironmentObject private var mixer: MixerController
    @Environment(\.dismiss) private var dismiss
    @State private var rows: [ResourceRow] = []
    @State private var summary = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Resources").fontWeight(.bold)
            Text(summary.isEmpty ? mixer.resourceText : summary)
                .foregroundStyle(EivizTheme.hud)
            Grid(alignment: .leading, horizontalSpacing: 12, verticalSpacing: 4) {
                GridRow {
                    Text("Input").fontWeight(.bold)
                    Text("Kind").fontWeight(.bold)
                    Text("Size").fontWeight(.bold)
                    Text("CPU").fontWeight(.bold)
                    Text("GPU").fontWeight(.bold)
                    Text("RAM").fontWeight(.bold)
                    Text("VRAM").fontWeight(.bold)
                }
                ForEach(rows) { row in
                    GridRow {
                        Text(row.name)
                        Text(row.kind)
                        Text(row.size)
                        Text(row.cpu)
                        Text(row.gpu)
                        Text(row.ram)
                        Text(row.vram)
                    }
                }
            }
            .font(.system(size: 12, design: .monospaced))
            Spacer()
            HStack {
                Spacer()
                Button("Close") { dismiss() }
            }
        }
        .padding(12)
        .frame(width: 820, height: 480)
        .background(EivizTheme.dialog)
        .foregroundStyle(EivizTheme.text)
        .task {
            while !Task.isCancelled {
                load()
                try? await Task.sleep(nanoseconds: 400_000_000)
            }
        }
    }

    private func load() {
        var buffer = [EivizSourceUsage](repeating: MixerFFI.zeroed(), count: 64)
        let n = buffer.withUnsafeMutableBufferPointer { ptr in
            mixer_copy_source_usage(ptr.baseAddress, UInt32(ptr.count))
        }
        var usages: [UInt64: EivizSourceUsage] = [:]
        if n > 0 {
            for usage in buffer.prefix(Int(n)) {
                usages[usage.source_id] = usage
            }
        }
        var totalRam: UInt64 = 0
        var totalVram: UInt64 = 0
        for usage in usages.values {
            totalRam += usage.ram_bytes
            totalVram += usage.vram_bytes
        }
        if totalRam == 0 { totalRam = 1 }
        if totalVram == 0 { totalVram = 1 }
        var stats = EivizMixerStats(render_ms: 0, frame_budget_ms: 0)
        _ = mixer_copy_stats(&stats)
        let gpuLoad = stats.frame_budget_ms > 0.1
            ? min(100, stats.render_ms / stats.frame_budget_ms * 100)
            : 0
        rows = mixer.session.inputs.map { input in
            let usage = usages[input.id]
            let ram = usage?.ram_bytes ?? 0
            let vram = usage?.vram_bytes ?? 0
            let width = usage?.width ?? 0
            let height = usage?.height ?? 0
            let live = input.kind == .omt || input.kind == .ndi || input.kind == .uvc || input.kind == .video
            return ResourceRow(
                id: input.id,
                name: input.name,
                kind: input.kind.rawValue,
                size: width == 0 ? "—" : "\(width)x\(height)",
                cpu: live ? "live" : "—",
                gpu: vram == 0 ? "—" : String(format: "%.0f%%", Double(vram) / Double(totalVram) * Double(gpuLoad)),
                ram: formatBytes(ram),
                vram: formatBytes(vram)
            )
        }
        summary = "Inputs \(mixer.session.inputs.count)    RAM \(formatBytes(totalRam == 1 ? 0 : totalRam))    VRAM \(formatBytes(totalVram == 1 ? 0 : totalVram))    Render \(String(format: "%.1f", stats.render_ms)) / \(String(format: "%.1f", stats.frame_budget_ms)) ms"
    }
}
