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
            }
            .frame(width: 200)
            .listStyle(.sidebar)
            VStack(alignment: .leading, spacing: 12) {
                Group {
                    if category == 0 { display }
                    else if category == 1 { performance }
                    else if category == 2 { outputs }
                    else if category == 3 { multiview }
                    else { audio }
                }
                Spacer()
                HStack {
                    Spacer()
                    Button("OK") {
                        if !copyUmaInfo().available {
                            mixer.session.settings.rebarOptimization = false
                        }
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
            if info.available {
                Text("Unified Memory")
                Text(info.uma ? "Enabled" : "Not available")
                Toggle("Use Unified Memory optimization", isOn: Binding(
                    get: { mixer.session.settings.rebarOptimizationEnabled },
                    set: { mixer.session.settings.rebarOptimization = $0 }
                ))
            }
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

}

struct PreferencesView: View {
    @Environment(\.dismiss) private var dismiss
    @ObservedObject private var prefs = AppPrefs.shared
    @State private var originalLanguage = AppPrefs.shared.language
    @State private var originalTheme = AppPrefs.shared.theme
    @State private var reverting = false

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(L10n.t("prefs.language"))
            Picker("", selection: $prefs.language) {
                Text(L10n.t("prefs.english")).tag(AppLanguage.en)
                Text(L10n.t("prefs.japanese")).tag(AppLanguage.ja)
            }
            .frame(width: 220)
            Text(L10n.t("prefs.theme"))
            Picker("", selection: $prefs.theme) {
                Text(L10n.t("prefs.themeDark")).tag(AppThemeMode.dark)
                Text(L10n.t("prefs.themeLight")).tag(AppThemeMode.light)
                Text(L10n.t("prefs.themeSystem")).tag(AppThemeMode.system)
            }
            .frame(width: 220)
            Text("eiviz").font(.title)
            Text("Version \(HostVersion.display)")
            Text(L10n.t("about.blurb")).fixedSize(horizontal: false, vertical: true)
            Text(L10n.t("about.author"))
            Link("https://github.com/MikanseiLaboratory/eiviz", destination: URL(string: "https://github.com/MikanseiLaboratory/eiviz")!)
            Link("https://mikanseilaboratory.github.io/", destination: URL(string: "https://mikanseilaboratory.github.io/")!)
            Text(L10n.t("about.openSource")).fontWeight(.bold)
            Text(L10n.t("about.license"))
                .foregroundStyle(EivizTheme.dim)
                .fixedSize(horizontal: false, vertical: true)
            HStack {
                Spacer()
                Button(L10n.t("dialog.ok")) {
                    prefs.save()
                    prefs.localeRevision += 1
                    dismiss()
                }
                Button(L10n.t("dialog.cancel")) {
                    reverting = true
                    prefs.language = originalLanguage
                    prefs.theme = originalTheme
                    prefs.save()
                    prefs.localeRevision += 1
                    dismiss()
                }
            }
        }
        .padding(20)
        .frame(minWidth: 520, minHeight: 420)
        .background(EivizTheme.dialog)
        .foregroundStyle(EivizTheme.text)
        .onAppear {
            originalLanguage = prefs.language
            originalTheme = prefs.theme
        }
        .onChange(of: prefs.language) { _, _ in
            if !reverting {
                prefs.save()
                prefs.localeRevision += 1
            }
        }
        .onChange(of: prefs.theme) { _, _ in
            if !reverting { prefs.save() }
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
    @State private var uvcModes: [CaptureMode] = []
    @State private var selectedMode: CaptureMode?
    @State private var r: Double = 220
    @State private var g: Double = 32
    @State private var b: Double = 32
    @State private var bars = false
    @State private var scroll = false
    @State private var toneHz: Float = 1000
    @State private var useGpu = true
    @State private var buffer: UInt32 = 1
    @State private var mediaBuffer: UInt32 = 3
    @State private var videoPreloadRam = false
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
                if let editing {
                    Text("ID \(editing.id)   GUID \(editing.guid)")
                        .font(.system(size: 11))
                        .foregroundStyle(EivizTheme.dim)
                }
                form
                Spacer()
                HStack {
                    Spacer()
                    Button("OK") {
                        if commit() { dismiss() }
                    }
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
            recentList(AppPrefs.shared.recentStills) { stillPath = $0 }
        case "Video":
            pathRow($videoPath) { pick(["public.movie"], $videoPath) }
            recentList(AppPrefs.shared.recentVideos) { videoPath = $0 }
            Toggle("Loop", isOn: $videoLoop)
            frameBufferPicker($mediaBuffer)
            Toggle("Preload into RAM", isOn: $videoPreloadRam)
            Text("Frame buffer (1–8) absorbs decode jitter. Preload decodes the clip into system RAM (not VRAM). If it will not fit, an error is shown and the file streams.")
                .foregroundStyle(EivizTheme.dim)
                .fixedSize(horizontal: false, vertical: true)
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
            frameBufferPicker($buffer)
        case "NDI®":
            mixerTextField($ndiAddress, placeholder: "NDI® source")
            Button("Refresh discovery") { refreshNdi() }
            List(ndiList, id: \.self, selection: $ndiAddress) { Text($0) }
                .frame(height: 120)
            Toggle("Lowest bandwidth", isOn: $ndiLow)
            frameBufferPicker($buffer)
            Text("NDI is received on the CPU and uploaded for compose.")
                .foregroundStyle(EivizTheme.dim)
        default:
            Button("Refresh devices") { refreshUvc() }
            List(uvcList, id: \.id, selection: $selectedUvc) { device in
                Text(device.name).tag(device.id)
            }
            .onChange(of: selectedUvc) { _, _ in refreshUvcModes() }
            Text("Mode")
            Picker("", selection: $selectedMode) {
                Text("Select mode").tag(Optional<CaptureMode>.none)
                ForEach(uvcModes) { mode in
                    Text(mode.label).tag(Optional(mode))
                }
            }
            frameBufferPicker($mediaBuffer)
            Text("Frame buffer (1–8) holds decoded camera frames, same as NDI/OMT.")
                .foregroundStyle(EivizTheme.dim)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    private func frameBufferPicker(_ selection: Binding<UInt32>) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("Frame buffer (frames)")
            Picker("", selection: selection) {
                ForEach([UInt32(1), 2, 3, 4, 6, 8], id: \.self) { Text("\($0)").tag($0) }
            }
            .frame(width: 220)
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

    @ViewBuilder
    private func recentList(_ paths: [String], choose: @escaping (String) -> Void) -> some View {
        if !paths.isEmpty {
            Text(L10n.t("chrome.openRecent"))
            List(paths, id: \.self) { path in
                Button(URL(fileURLWithPath: path).lastPathComponent) { choose(path) }
            }
            .frame(height: 120)
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
        refreshUvcModes()
    }

    private func refreshUvcModes() {
        guard !selectedUvc.isEmpty else {
            uvcModes = []
            selectedMode = nil
            return
        }
        uvcModes = MixerFFI.videoCaptureModes(deviceId: selectedUvc)
        if let current = selectedMode, uvcModes.contains(current) {
            return
        }
        selectedMode = uvcModes.first
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
        if editing.kind == .uvc, editing.captureWidth > 0, editing.captureHeight > 0 {
            selectedMode = CaptureMode(
                width: editing.captureWidth,
                height: editing.captureHeight,
                fpsNum: editing.captureFpsNum,
                fpsDen: max(1, editing.captureFpsDen)
            )
        }
        refreshUvcModes()
        r = Double(editing.colorR) * 255
        g = Double(editing.colorG) * 255
        b = Double(editing.colorB) * 255
        bars = editing.kind == .bars
        scroll = editing.scroll
        toneHz = editing.toneHz
        useGpu = editing.useGpu
        buffer = editing.frameBufferFrames
        mediaBuffer = max(1, min(8, editing.frameBufferFrames == 0 ? 3 : editing.frameBufferFrames))
        videoPreloadRam = editing.videoPreloadRam
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

    private func commit() -> Bool {
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
            guard !stillPath.isEmpty else { return false }
            guard FileManager.default.fileExists(atPath: stillPath) else {
                mixer.presentInputError(L10n.missingFile("Still load"), editing: editing != nil)
                return false
            }
            input.kind = .still
            input.pathOrAddress = stillPath
            AppPrefs.shared.rememberStill(stillPath)
        case "Video":
            guard !videoPath.isEmpty else { return false }
            guard FileManager.default.fileExists(atPath: videoPath) else {
                mixer.presentInputError(L10n.missingFile("Video start"), editing: editing != nil)
                return false
            }
            input.kind = .video
            input.pathOrAddress = videoPath
            AppPrefs.shared.rememberVideo(videoPath)
            input.videoLoop = videoLoop
            input.videoPlayWhen = videoPlayWhen
            input.videoRestartWhen = videoRestartWhen
            input.videoPauseWhen = videoPauseWhen
            input.frameBufferFrames = max(1, min(8, mediaBuffer))
            input.videoPreloadRam = videoPreloadRam
        case "OMT":
            guard !omtAddress.isEmpty else { return false }
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
            guard !ndiAddress.isEmpty else { return false }
            input.kind = .ndi
            input.pathOrAddress = ndiAddress
            input.ndiBandwidth = ndiLow ? .lowest : .highest
            input.frameBufferFrames = buffer
            input.useGpu = false
        default:
            guard !selectedUvc.isEmpty, let mode = selectedMode else { return false }
            input.kind = .uvc
            input.pathOrAddress = selectedUvc
            input.captureWidth = mode.width
            input.captureHeight = mode.height
            input.captureFpsNum = mode.fpsNum
            input.captureFpsDen = mode.fpsDen
            input.frameBufferFrames = max(1, min(8, mediaBuffer))
        }
        if let editing {
            input.guid = editing.guid
            input.id = editing.id
        }
        mixer.upsertInput(input, replacing: editing?.id)
        return true
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
    var layoutId: UInt64?

    private var resolvedId: UInt64? { layoutId ?? mixer.openMultiview?.id }

    private var layout: MultiviewLayout? {
        if let id = resolvedId,
           let live = mixer.session.multiviews.first(where: { $0.id == id })
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
                if let index = mixer.session.multiviews.firstIndex(where: { $0.id == layout?.id }) {
                    Toggle(L10n.t("settings.alwaysOnTop"), isOn: Binding(
                        get: { mixer.session.multiviews[index].alwaysOnTop },
                        set: {
                            mixer.session.multiviews[index].alwaysOnTop = $0
                            mixer.applyMultiviewWindowLevel(mixer.session.multiviews[index])
                        }
                    ))
                    Picker(L10n.t("settings.labelPosition"), selection: Binding(
                        get: { mixer.session.multiviews[index].labelAnchor ?? mixer.session.settings.multiviewLabelAnchor },
                        set: { mixer.session.multiviews[index].labelAnchor = $0; mixer.applyBusColors() }
                    )) {
                        Text(L10n.t("mv.bottom")).tag(MvLabelAnchor.bottom)
                        Text(L10n.t("mv.top")).tag(MvLabelAnchor.top)
                    }
                    .frame(width: 120)
                    TextField("", value: Binding(
                        get: { mixer.session.multiviews[index].resolvedLabelSize(mixer.session.settings) },
                        set: {
                            mixer.session.multiviews[index].labelSize = min(200, max(1, $0))
                            mixer.applyBusColors()
                        }
                    ), format: .number)
                    .frame(width: 48)
                    Picker("", selection: Binding(
                        get: { mixer.session.multiviews[index].resolvedLabelUnit(mixer.session.settings) },
                        set: {
                            mixer.session.multiviews[index].labelUnit = $0
                            mixer.applyBusColors()
                        }
                    )) {
                        Text("px").tag(MvLabelUnit.px)
                        Text("%").tag(MvLabelUnit.percent)
                    }
                    .frame(width: 56)
                }
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
    @State private var template: MultiviewTemplate = .previewProgram8
    @State private var tiles: [MvSlot] = Array(repeating: MvSlot(), count: 10)
    @State private var selectedTile = 0

    private var layoutId: UInt64? { mixer.openMultiview?.id }
    private var layoutIndex: Int? {
        guard let id = layoutId else { return nil }
        return mixer.session.multiviews.firstIndex(where: { $0.id == id })
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Numbers match Window 1…N. Click a pane to assign that window.")
                .foregroundStyle(EivizTheme.dim)
            if let index = layoutIndex {
                HStack(spacing: 8) {
                    Text(L10n.t("mv.labelPosition"))
                    Picker("", selection: Binding(
                        get: { mixer.session.multiviews[index].labelAnchor ?? mixer.session.settings.multiviewLabelAnchor },
                        set: { mixer.session.multiviews[index].labelAnchor = $0; mixer.applyBusColors() }
                    )) {
                        Text(L10n.t("mv.bottom")).tag(MvLabelAnchor.bottom)
                        Text(L10n.t("mv.top")).tag(MvLabelAnchor.top)
                    }
                    .frame(width: 88)
                    Text(L10n.t("settings.mvLabelSize"))
                    TextField("", value: Binding(
                        get: { mixer.session.multiviews[index].resolvedLabelSize(mixer.session.settings) },
                        set: {
                            mixer.session.multiviews[index].labelSize = min(200, max(1, $0))
                            mixer.applyBusColors()
                        }
                    ), format: .number)
                    .frame(width: 48)
                    Picker("", selection: Binding(
                        get: { mixer.session.multiviews[index].resolvedLabelUnit(mixer.session.settings) },
                        set: {
                            mixer.session.multiviews[index].labelUnit = $0
                            mixer.applyBusColors()
                        }
                    )) {
                        Text("px").tag(MvLabelUnit.px)
                        Text("%").tag(MvLabelUnit.percent)
                    }
                    .frame(width: 56)
                }
            }
            HStack(alignment: .top, spacing: 16) {
                VStack(alignment: .leading, spacing: 6) {
                    Text("Template").fontWeight(.bold)
                    MosaicThumb(template: template, selected: true, selectedPane: selectedTile) { pane in
                        selectedTile = pane
                    }
                    .frame(width: 320, height: 180)
                }
                if tiles.indices.contains(selectedTile) {
                    tileRow(selectedTile)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
            ScrollView {
                VStack(alignment: .leading, spacing: 6) {
                    ForEach(MultiviewTemplate.groups, id: \.title) { group in
                        Text(group.title)
                            .foregroundStyle(EivizTheme.dim)
                        LazyVGrid(columns: [GridItem(.adaptive(minimum: 112), spacing: 6)], alignment: .leading, spacing: 6) {
                            ForEach(group.items) { item in
                                Button {
                                    template = item
                                } label: {
                                    VStack(spacing: 2) {
                                        MosaicThumb(
                                            template: item,
                                            selected: template == item,
                                            selectedPane: template == item ? selectedTile : -1
                                        )
                                        Text(item.title)
                                            .font(.system(size: 10))
                                    }
                                }
                                .buttonStyle(.plain)
                            }
                        }
                    }
                }
            }
            HStack {
                Spacer()
                Button("OK") { commit(); dismiss() }
                Button("Cancel") { dismiss() }
            }
        }
        .padding(12)
        .frame(width: 820, height: 520)
        .background(EivizTheme.dialog)
        .foregroundStyle(EivizTheme.text)
        .onAppear {
            if let id = layoutId, let layout = mixer.session.multiviews.first(where: { $0.id == id }) {
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
        if selectedTile >= tiles.count {
            selectedTile = max(0, tiles.count - 1)
        }
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
                set: {
                    tiles[index].kind = $0
                    tiles[index].sourceId = 0
                    if $0 == .muPreview || $0 == .muProgram {
                        tiles[index].sourceId = mixer.session.units.first?.id ?? 0
                    }
                }
            )) {
                Text("None").tag(MvSlotKind.none)
                Text("Input").tag(MvSlotKind.input)
                Text("Scene").tag(MvSlotKind.scene)
                Text("MU PRV").tag(MvSlotKind.muPreview)
                Text("MU PGM").tag(MvSlotKind.muProgram)
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
            } else if tiles[index].kind == .muPreview || tiles[index].kind == .muProgram {
                Picker("", selection: Binding(
                    get: { tiles[index].sourceId },
                    set: { tiles[index].sourceId = $0 }
                )) {
                    ForEach(mixer.session.units) { unit in
                        Text(unit.name).tag(unit.id)
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
        mixer.session.multiviews[index].tiles = tiles
        mixer.session.multiviews[index].ensureTiles()
        if let preview = tiles.first(where: { $0.kind == .muPreview }) {
            mixer.session.multiviews[index].previewUnitId = preview.sourceId
        }
        if let program = tiles.first(where: { $0.kind == .muProgram }) {
            mixer.session.multiviews[index].programUnitId = program.sourceId
        }
        mixer.openMultiview = mixer.session.multiviews[index]
        mixer.pushMultiview(mixer.session.multiviews[index])
    }
}

private struct MosaicThumb: View {
    var template: MultiviewTemplate
    var selected: Bool
    var selectedPane: Int = -1
    var onPick: ((Int) -> Void)?

    var body: some View {
        GeometryReader { geo in
            let w = geo.size.width
            let h = geo.size.height
            ZStack(alignment: .topLeading) {
                Rectangle().fill(Color.black)
                ForEach(Array(template.panes.enumerated()), id: \.offset) { index, pane in
                    let pw = max(1, CGFloat(pane.width) * w - 1)
                    let ph = max(1, CGFloat(pane.height) * h - 1)
                    ZStack {
                        Rectangle()
                            .fill(Color(white: index == selectedPane ? 0.43 : 0.29))
                            .overlay(Rectangle().stroke(Color.black, lineWidth: 1))
                        Text("\(index + 1)")
                            .font(.system(size: min(18, max(7, min(pw, ph) * 0.42)), weight: .bold))
                            .foregroundStyle(Color.white)
                    }
                    .frame(width: pw, height: ph)
                    .offset(x: CGFloat(pane.x) * w, y: CGFloat(pane.y) * h)
                    .onTapGesture {
                        onPick?(index)
                    }
                }
            }
        }
        .aspectRatio(16 / 9, contentMode: .fit)
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
        var stats = MixerFFI.zeroed() as EivizMixerStats
        _ = mixer_copy_stats(&stats)
        var totalRam = stats.ram_bytes
        var totalVram = stats.vram_bytes
        if totalRam == 0 && totalVram == 0 {
            for usage in usages.values {
                totalRam += usage.ram_bytes
                totalVram += usage.vram_bytes
            }
        }
        if totalRam == 0 { totalRam = 1 }
        if totalVram == 0 { totalVram = 1 }
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
        let extra = stats.compose_vram_bytes > 0 || stats.delay_vram_bytes > 0
            ? "    Compose \(formatBytes(stats.compose_vram_bytes))    Delay \(formatBytes(stats.delay_vram_bytes))"
            : ""
        summary = "Inputs \(mixer.session.inputs.count)    RAM \(formatBytes(totalRam == 1 ? 0 : totalRam))    VRAM \(formatBytes(totalVram == 1 ? 0 : totalVram))\(extra)    Render \(String(format: "%.1f", stats.render_ms)) / \(String(format: "%.1f", stats.frame_budget_ms)) ms"
    }
}

struct LogsView: View {
    @Environment(\.dismiss) private var dismiss
    @State private var text = ""
    @State private var showHost = true
    @State private var showMixer = true
    @State private var follow = true
    @State private var paused = false
    @State private var hostOffset: UInt64 = .max
    @State private var mixerOffset: UInt64 = .max

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text(L10n.t("logs.title")).fontWeight(.bold)
                Spacer()
                Toggle(L10n.t("logs.host"), isOn: $showHost)
                Toggle(L10n.t("logs.mixer"), isOn: $showMixer)
                Toggle(L10n.t("logs.autoscroll"), isOn: $follow)
                Toggle(L10n.t("logs.pause"), isOn: $paused)
                Button(L10n.t("logs.clear")) { text = "" }
                Button(L10n.t("logs.folder")) {
                    NSWorkspace.shared.open(HostLog.directory)
                }
            }
            .toggleStyle(.checkbox)
            Text(HostLog.directory.path)
                .font(.system(size: 11))
                .foregroundStyle(EivizTheme.hud)
                .lineLimit(1)
                .truncationMode(.middle)
            ScrollViewReader { proxy in
                ScrollView {
                    Text(text.isEmpty ? " " : text)
                        .font(.system(size: 12, design: .monospaced))
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .textSelection(.enabled)
                        .id("end")
                }
                .onChange(of: text) { _, _ in
                    if follow {
                        proxy.scrollTo("end", anchor: .bottom)
                    }
                }
            }
            HStack {
                Spacer()
                Button("Close") { dismiss() }
            }
        }
        .padding(12)
        .frame(width: 920, height: 560)
        .background(EivizTheme.dialog)
        .foregroundStyle(EivizTheme.text)
        .task {
            while !Task.isCancelled {
                if !paused {
                    pump()
                }
                try? await Task.sleep(nanoseconds: 200_000_000)
            }
        }
    }

    private func pump() {
        if showHost {
            append(source: "host", url: HostLog.directory.appendingPathComponent("eiviz-host.log"), offset: &hostOffset)
        } else {
            _ = readNew(url: HostLog.directory.appendingPathComponent("eiviz-host.log"), offset: &hostOffset)
        }
        if showMixer {
            append(source: "mixer", url: HostLog.directory.appendingPathComponent("eiviz-mixer.log"), offset: &mixerOffset)
        } else {
            _ = readNew(url: HostLog.directory.appendingPathComponent("eiviz-mixer.log"), offset: &mixerOffset)
        }
    }

    private func append(source: String, url: URL, offset: inout UInt64) {
        let lines = readNew(url: url, offset: &offset)
        guard !lines.isEmpty else { return }
        var next = text
        for line in lines where !line.isEmpty {
            next += "\(source) \(line)\n"
        }
        if next.count > 400_000 {
            next = String(next.suffix(300_000))
        }
        text = next
    }

    private func readNew(url: URL, offset: inout UInt64) -> [String] {
        guard let handle = try? FileHandle(forReadingFrom: url) else { return [] }
        defer { try? handle.close() }
        let size = (try? handle.seekToEnd()) ?? 0
        if offset == .max {
            offset = size > 64 * 1024 ? size - 64 * 1024 : 0
        } else if size < offset {
            offset = 0
        }
        guard size > offset else { return [] }
        do {
            try handle.seek(toOffset: offset)
            guard let data = try handle.readToEnd(), !data.isEmpty else { return [] }
            offset = size
            var text = String(data: data, encoding: .utf8) ?? ""
            text = text.replacingOccurrences(of: "\r\n", with: "\n")
            var lines = text.split(separator: "\n", omittingEmptySubsequences: false).map(String.init)
            if offset > data.count && !text.hasPrefix("\n") && lines.count > 1 {
                lines.removeFirst()
            }
            if !text.hasSuffix("\n"), let last = lines.last, last.isEmpty == false {
                // keep last partial until the next read by rolling offset back
                if let lastData = last.data(using: .utf8) {
                    offset -= UInt64(lastData.count)
                }
                lines.removeLast()
            }
            return lines
        } catch {
            return []
        }
    }
}

struct CustomWgslEditor: View {
    @State private var text: String
    @State private var status = ""
    @State private var valid = false
    let onSave: (String) -> Void
    let onCancel: () -> Void

    init(text: String, onSave: @escaping (String) -> Void, onCancel: @escaping () -> Void) {
        _text = State(initialValue: text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? Self.template : text)
        self.onSave = onSave
        self.onCancel = onCancel
    }

    static let template = """
    fn user_transition(uv: vec2<f32>, t: f32) -> vec4<f32> {
        let a = textureSample(pgm_tex, src_samp, uv);
        let b = textureSample(pvw_tex, src_samp, uv);
        let prev = textureSample(prev_tex, src_samp, uv);
        let w = 1.0 - smoothstep(t - 0.04, t + 0.04, uv.x);
        return mix(mix(a, prev, 0.15 * t), b, w);
    }
    """

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Custom WGSL").fontWeight(.bold)
            Text("fn user_transition(uv, t). Optional fn user_compute(id, dim) writes aux via user_store. Bound: pgm/pvw/prev/flow/bloom/aux/aux2, src_samp, src_samp_n, params. Compute uses textureSampleLevel.")
                .font(.system(size: 11))
                .foregroundStyle(EivizTheme.dim)
            TextEditor(text: $text)
                .font(.system(.body, design: .monospaced))
                .scrollContentBackground(.hidden)
                .foregroundStyle(EivizTheme.text)
                .padding(6)
                .background(Color.black.opacity(0.45))
                .frame(minWidth: 640, minHeight: 360)
            Text(status)
                .font(.system(size: 11))
                .foregroundStyle(valid ? Color.green : Color.red)
                .fixedSize(horizontal: false, vertical: true)
            HStack {
                Button("Load file…") { loadFile() }
                Button("Validate") { _ = validate() }
                Spacer()
                Button("Cancel") { onCancel() }
                Button("Save") {
                    if validate() { onSave(text) }
                }
            }
            .buttonStyle(MixerButtonStyle())
        }
        .padding(16)
        .background(EivizTheme.dialog)
        .foregroundStyle(EivizTheme.text)
        .onAppear { _ = validate() }
    }

    @discardableResult
    private func validate() -> Bool {
        let code = text.withCString { mixer_validate_custom_wgsl($0) }
        if code == EIVIZ_OK {
            status = "Valid WGSL. user_transition will be used."
            valid = true
            return true
        }
        let error = MixerFFI.lastErrorText()
        status = error.isEmpty ? "Invalid WGSL." : error
        valid = false
        return false
    }

    private func loadFile() {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = false
        if let wgsl = UTType(filenameExtension: "wgsl") {
            panel.allowedContentTypes = [wgsl, .plainText]
        } else {
            panel.allowedContentTypes = [.plainText]
        }
        guard panel.runModal() == .OK, let url = panel.url else { return }
        if let loaded = try? String(contentsOf: url, encoding: .utf8) {
            text = loaded
            _ = validate()
        }
    }
}
