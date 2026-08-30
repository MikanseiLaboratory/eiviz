import AppKit
import AVFoundation
import EivizMixer
import SwiftUI
import UniformTypeIdentifiers

struct SettingsView: View {
    @EnvironmentObject private var mixer: MixerController
    @Environment(\.dismiss) private var dismiss
    @State private var category = 0

    var body: some View {
        HStack(spacing: 0) {
            List(selection: $category) {
                Text("Display").tag(0)
                Text("Outputs").tag(1)
                Text("Multiview").tag(2)
                Text("Audio Auxiliary").tag(3)
                Text("About").tag(4)
            }
            .frame(width: 200)
            .listStyle(.sidebar)
            VStack(alignment: .leading, spacing: 12) {
                Group {
                    if category == 0 { display }
                    else if category == 1 { outputs }
                    else if category == 2 { multiview }
                    else if category == 3 { audio }
                    else { about }
                }
                Spacer()
                HStack {
                    Spacer()
                    Button("OK") {
                        mixer.pushAudio()
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
    }

    private var display: some View {
        VStack(alignment: .leading, spacing: 8) {
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

    private var outputs: some View {
        VStack(alignment: .leading) {
            HStack {
                Text("Outputs").fontWeight(.bold)
                Spacer()
                Button("+") {
                    let output = OutputEntry(id: mixer.session.nextOutputId, name: "eiviz-out-\(mixer.session.nextOutputId)")
                    mixer.session.nextOutputId += 1
                    mixer.session.outputs.append(output)
                    mixer.addOutput(output)
                }
            }
            Text("OMT and NDI® are sent from the mixer. NDI uses CPU encode.")
                .foregroundStyle(EivizTheme.dim)
            ForEach($mixer.session.outputs) { $output in
                HStack {
                    TextField("Name", text: $output.name)
                    Picker("", selection: $output.transport) {
                        Text("OMT").tag(OutputTransport.omt)
                        Text("NDI®").tag(OutputTransport.ndi)
                    }
                    Picker("", selection: $output.sourceKind) {
                        Text("MU Program").tag(OutputSourceKind.muProgram)
                        Text("MU Preview").tag(OutputSourceKind.muPreview)
                        Text("Scene").tag(OutputSourceKind.scene)
                    }
                    Toggle("GPU", isOn: $output.useGpu)
                    Button("Apply") { mixer.addOutput(output) }
                    Button("−") {
                        _ = mixer_output_remove(output.id)
                        mixer.session.outputs.removeAll { $0.id == output.id }
                    }
                }
            }
        }
    }

    private var multiview: some View {
        VStack(alignment: .leading) {
            HStack {
                Text("Multiviews").fontWeight(.bold)
                Spacer()
                Button("+") {
                    let layout = MultiviewLayout(
                        id: mixer.session.nextMultiviewId,
                        name: "Multiview \(mixer.session.nextMultiviewId)",
                        monitorId: mixer.session.nextMonitorId
                    )
                    mixer.session.nextMultiviewId += 1
                    mixer.session.nextMonitorId += 1
                    mixer.session.multiviews.append(layout)
                }
            }
            ForEach(mixer.session.multiviews) { layout in
                Text(layout.name)
            }
            Text("Each Multiview window is a mosaic: Preview and Program on top, eight Input or Scene windows below.")
                .foregroundStyle(EivizTheme.dim)
        }
    }

    private var audio: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Internal mix is 48 kHz stereo. Master and Headphone cannot be removed.")
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
                TextField("Name", text: bus.name)
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
                    Text("WASAPI").tag(AudioDeviceKind.wasapi)
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
                    TextField("0", value: bus.mapLeft, format: .number).frame(width: 48)
                    Text("R ch")
                    TextField("1", value: bus.mapRight, format: .number).frame(width: 48)
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
                || (kind == .wasapi && $0.kind == AudioDeviceKind.coreAudio.rawUInt)
        }
    }

    private var about: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("eiviz").font(.title)
            Text("macOS host and Metal mixer. NDI® via grafton-ndi.")
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
    @State private var uvcList: [AVCaptureDevice] = []
    @State private var selectedUvc: String = ""
    @State private var r: Double = 220
    @State private var g: Double = 32
    @State private var b: Double = 32
    @State private var bars = false
    @State private var scroll = false
    @State private var useGpu = true
    @State private var buffer: UInt32 = 1
    @State private var quality: UInt32 = 0
    @State private var ndiLow = false

    var body: some View {
        HStack(spacing: 0) {
            List(["Colours", "Still", "Video", "OMT", "NDI®", "Video Capture"], id: \.self, selection: $category) { item in
                Text(item).tag(item)
            }
            .frame(width: 200)
            VStack(alignment: .leading, spacing: 12) {
                Text("Name")
                TextField("Name", text: $name)
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
        .onAppear {
            name = editing?.name ?? ""
            stillPath = editing?.pathOrAddress ?? ""
            refreshOmt()
            refreshNdi()
            refreshUvc()
        }
    }

    @ViewBuilder
    private var form: some View {
        switch category {
        case "Colours":
            Toggle("SMPTE colour bars", isOn: $bars)
            if !bars {
                colorSlider("R", $r)
                colorSlider("G", $g)
                colorSlider("B", $b)
                Rectangle().fill(Color(red: r / 255, green: g / 255, blue: b / 255)).frame(height: 48)
                Toggle("Scroll", isOn: $scroll)
            }
        case "Still":
            pathRow($stillPath) { pick(["public.image"], $stillPath) }
        case "Video":
            pathRow($videoPath) { pick(["public.movie"], $videoPath) }
        case "OMT":
            TextField("OMT source address", text: $omtAddress)
            Button("Refresh discovery") { refreshOmt() }
            List(omtList, id: \.self, selection: $omtAddress) { Text($0) }
                .frame(height: 120)
            Toggle("GPU decode", isOn: $useGpu)
        case "NDI®":
            TextField("NDI® source", text: $ndiAddress)
            Button("Refresh discovery") { refreshNdi() }
            List(ndiList, id: \.self, selection: $ndiAddress) { Text($0) }
                .frame(height: 120)
            Toggle("Lowest bandwidth", isOn: $ndiLow)
            Text("NDI is received on the CPU and uploaded for compose.")
                .foregroundStyle(EivizTheme.dim)
        default:
            Button("Refresh devices") { refreshUvc() }
            List(uvcList, id: \.uniqueID, selection: $selectedUvc) { device in
                Text(device.localizedName).tag(device.uniqueID)
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
            TextField("Path", text: text)
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
        if mixer.errorText.contains("NDI") == false, ndiList.isEmpty {
            // discovery can be empty until sources appear
        }
    }

    private func refreshUvc() {
        let session = AVCaptureDevice.DiscoverySession(
            deviceTypes: [.builtInWideAngleCamera, .external],
            mediaType: .video,
            position: .unspecified
        )
        uvcList = session.devices
    }

    private func commit() {
        var input = InputEntry(id: mixer.session.nextInputId, name: name.isEmpty ? category : name, kind: .still)
        switch category {
        case "Colours":
            input.kind = bars ? .bars : .color
            input.colorR = Float(r / 255)
            input.colorG = Float(g / 255)
            input.colorB = Float(b / 255)
            input.scroll = scroll
        case "Still":
            input.kind = .still
            input.pathOrAddress = stillPath
        case "Video":
            input.kind = .video
            input.pathOrAddress = videoPath
        case "OMT":
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
            input.kind = .ndi
            input.pathOrAddress = ndiAddress
            input.ndiBandwidth = ndiLow ? .lowest : .highest
            input.frameBufferFrames = buffer
        default:
            input.kind = .uvc
            input.pathOrAddress = selectedUvc
        }
        mixer.addInput(input)
    }
}

struct MixingUnitView: View {
    @EnvironmentObject private var mixer: MixerController
    @Environment(\.dismiss) private var dismiss
    @State var unit: MixingUnitEntry

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            labeled("Name") { TextField("Name", text: $unit.name) }
            labeled("Width") { TextField("Width", value: $unit.width, format: .number) }
            labeled("Height") { TextField("Height", value: $unit.height, format: .number) }
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
        .frame(width: 420, height: 280)
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
    @State private var layout: MultiviewLayout?

    var body: some View {
        VStack(alignment: .leading) {
            HStack {
                Text("Multiview").fontWeight(.bold)
                Spacer()
                Button("Close") { dismiss() }
            }
            if mixer.session.multiviews.isEmpty {
                Text("Add a Multiview in Settings.")
                    .foregroundStyle(EivizTheme.dim)
            }
            ForEach(mixer.session.multiviews) { item in
                VStack(alignment: .leading) {
                    Text(item.name).fontWeight(.bold)
                    MetalPreviewRepresentable(role: .monitor(monitorId: item.monitorId, sourceId: item.gpuId))
                        .frame(minHeight: 280)
                        .background(Color.black)
                }
            }
        }
        .padding(12)
        .frame(minWidth: 960, minHeight: 540)
        .background(EivizTheme.dialog)
        .foregroundStyle(EivizTheme.text)
        .onAppear { pushLayouts() }
    }

    private func pushLayouts() {
        for layout in mixer.session.multiviews {
            var layers: [EivizOverlayDesc] = []
            func layer(_ source: UInt64, _ x: Float, _ y: Float, _ w: Float, _ h: Float, _ z: Int32) {
                var desc = MixerFFI.emptyOverlay()
                desc.source_id = source
                desc.rect = EivizRect(x: x, y: y, width: w, height: h)
                desc.opacity = 1
                desc.z = z
                layers.append(desc)
            }
            layer(EIVIZ_MU_SOURCE_FLAG | EIVIZ_MU_BUS_PREVIEW | (layout.previewUnitId & EIVIZ_MU_ID_MASK), 0, 0, 0.5, 0.5, 0)
            layer(EIVIZ_MU_SOURCE_FLAG | (layout.programUnitId & EIVIZ_MU_ID_MASK), 0.5, 0, 0.5, 0.5, 1)
            for i in 0..<8 {
                let col = Float(i % 4)
                let row = Float(i / 4)
                layer(0, col / 4, 0.5 + row / 4, 0.25, 0.25, Int32(2 + i))
            }
            layers.withUnsafeMutableBufferPointer { ptr in
                _ = mixer_define_scene(layout.gpuId, mixer.selectedUnit.width, mixer.selectedUnit.height, UInt32(ptr.count), ptr.baseAddress)
            }
            _ = mixer_bind_multiview(layout.gpuId, layout.previewUnitId, layout.programUnitId)
        }
    }
}

struct ResourcesView: View {
    @EnvironmentObject private var mixer: MixerController
    @Environment(\.dismiss) private var dismiss
    @State private var rows: [EivizSourceUsage] = []

    var body: some View {
        VStack(alignment: .leading) {
            Text("Resources").fontWeight(.bold)
            Text(mixer.resourceText)
            Button("Refresh") { load() }
            List(rows, id: \.source_id) { row in
                Text(String(format: "id %llu  %ux%u  RAM %llu  VRAM %llu", row.source_id, row.width, row.height, row.ram_bytes, row.vram_bytes))
            }
            HStack {
                Spacer()
                Button("Close") { dismiss() }
            }
        }
        .padding(12)
        .frame(width: 640, height: 420)
        .background(EivizTheme.dialog)
        .onAppear(perform: load)
    }

    private func load() {
        var buffer = [EivizSourceUsage](repeating: MixerFFI.zeroed(), count: 64)
        let n = buffer.withUnsafeMutableBufferPointer { ptr in
            mixer_copy_source_usage(ptr.baseAddress, UInt32(ptr.count))
        }
        if n > 0 {
            rows = Array(buffer.prefix(Int(n)))
        }
    }
}
