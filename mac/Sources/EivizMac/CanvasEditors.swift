import EivizMixer
import SwiftUI

struct SceneEditorView: View {
    @EnvironmentObject private var mixer: MixerController
    @Environment(\.dismiss) private var dismiss
    @State private var original: [SceneLayer] = []
    @State private var selectedLayer: UUID?
    @State private var name: String = ""
    @State private var presetName = ""
    @State private var copyFromId: UInt64 = 0
    @State private var lastGpuPush = Date.distantPast

    private var sceneIndex: Int? {
        mixer.session.scenes.firstIndex { $0.id == mixer.editingScene?.id }
    }

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            VStack(alignment: .leading) {
                Text("Layers").fontWeight(.bold)
                List(selection: $selectedLayer) {
                    ForEach(layers) { layer in
                        HStack {
                            Text(label(layer))
                                .foregroundStyle(layer.hidden ? EivizTheme.dim : EivizTheme.text)
                            Spacer()
                            Button(layer.hidden ? "–" : "👁") {
                                toggleLayer(layer.id, \.hidden)
                            }
                            .buttonStyle(.plain)
                            Button(layer.locked ? "🔒" : "🔓") {
                                toggleLayer(layer.id, \.locked)
                            }
                            .buttonStyle(.plain)
                        }
                        .tag(Optional(layer.id))
                    }
                    .onMove(perform: moveLayers)
                }
                .scrollContentBackground(.hidden)
                .background(EivizTheme.list)
                Text("Preset (all layers)").padding(.top, 8)
                Picker("", selection: $presetName) {
                    Text("Apply…").tag("")
                    ForEach(["Full", "Split H", "Split V", "Quad", "PiP TR", "PiP TL", "PiP BR", "PiP BL"], id: \.self) {
                        Text($0).tag($0)
                    }
                    ForEach(mixer.session.scenePresets) { preset in
                        Text(preset.name).tag(preset.name)
                    }
                }
                .onChange(of: presetName) { _, name in
                    guard !name.isEmpty else { return }
                    applyPreset(name)
                    presetName = ""
                }
                Picker("Copy from", selection: $copyFromId) {
                    Text("Copy from…").tag(UInt64(0))
                    ForEach(mixer.session.scenes.filter { $0.id != current?.id }) { scene in
                        Text(scene.name).tag(scene.id)
                    }
                }
                .onChange(of: copyFromId) { _, id in
                    guard id != 0 else { return }
                    copyFrom(id)
                    copyFromId = 0
                }
                Button("Save preset") { savePreset() }
                Picker("Input", selection: Binding(
                    get: { mixer.selectedInputId ?? EIVIZ_SRC_BARS },
                    set: { mixer.selectedInputId = $0 }
                )) {
                    ForEach(mixer.session.inputs) { input in
                        Text(input.name).tag(input.id)
                    }
                }
                Button("Add layer") { addLayer() }
                HStack {
                Button("Z up") { shiftZ(-1) }
                Button("Z down") { shiftZ(1) }
                    Button("Delete") { deleteLayer() }
                }
            }
            .frame(width: 240)
            .buttonStyle(MixerButtonStyle())

            VStack {
                Text("Wireframe (\(mixer.selectedUnit.width)x\(mixer.selectedUnit.height))").fontWeight(.bold)
                WireCanvasView(
                    items: layers.map {
                        WireRect(
                            id: $0.id,
                            x: $0.x,
                            y: $0.y,
                            width: $0.width,
                            height: $0.height,
                            enabled: !$0.hidden,
                            locked: $0.locked,
                            sizeLinked: $0.sizeLinked,
                            cropX: $0.cropX,
                            cropY: $0.cropY,
                            cropWidth: $0.cropWidth,
                            cropHeight: $0.cropHeight
                        )
                    },
                    aspect: projectAspect,
                    selected: $selectedLayer,
                    onChange: applyWire
                )
                .clipped()
            }
            .clipped()

            VStack(alignment: .leading) {
                Text("Live preview").fontWeight(.bold)
                if let scene = current {
                    MetalPreviewRepresentable(role: .monitor(monitorId: scene.monitorId, sourceId: scene.gpuId))
                        .aspectRatio(projectAspect, contentMode: .fit)
                        .frame(maxWidth: .infinity)
                        .background(Color.black)
                }
                Text("Name").padding(.top, 8)
                mixerTextField($name, placeholder: "Name")
                if let index = layers.firstIndex(where: { $0.id == selectedLayer }) {
                    layerFields(index)
                }
                Spacer()
                HStack {
                    Spacer()
                    Button("OK") {
                        if let i = sceneIndex {
                            mixer.session.scenes[i].name = name.trimmingCharacters(in: .whitespaces).isEmpty
                                ? mixer.session.scenes[i].name : name.trimmingCharacters(in: .whitespaces)
                            mixer.pushScene(mixer.session.scenes[i])
                        }
                        dismiss()
                    }
                    Button("Cancel") {
                        if let i = sceneIndex {
                            mixer.session.scenes[i].layers = original
                            mixer.pushScene(mixer.session.scenes[i])
                        }
                        dismiss()
                    }
                }
            }
            .frame(width: 300)
            .buttonStyle(MixerButtonStyle())
        }
        .padding(12)
        .frame(minWidth: 1280, minHeight: 720)
        .background(EivizTheme.dialog)
        .foregroundStyle(EivizTheme.text)
        .onAppear {
            original = current?.layers ?? []
            name = current?.name ?? ""
            selectedLayer = current?.layers.first?.id
            mutate { scene in
                scene.layers.sort { $0.z > $1.z }
            }
        }
    }

    private var current: SceneEntry? {
        guard let i = sceneIndex else { return mixer.editingScene }
        return mixer.session.scenes[i]
    }

    private var layers: [SceneLayer] { current?.layers ?? [] }
    private var projectAspect: CGFloat {
        CGFloat(mixer.selectedUnit.width) / max(1, CGFloat(mixer.selectedUnit.height))
    }
    private var projectW: Float { Float(mixer.selectedUnit.width) }
    private var projectH: Float { Float(mixer.selectedUnit.height) }

    private func label(_ layer: SceneLayer) -> String {
        let input = mixer.session.inputs.first { $0.id == layer.inputId }
        let order = (layers.firstIndex { $0.id == layer.id } ?? 0) + 1
        return "\(order). \(input?.name ?? "\(layer.inputId)")"
    }

    private func mutate(_ body: (inout SceneEntry) -> Void) {
        guard let i = sceneIndex else { return }
        var scene = mixer.session.scenes[i]
        body(&scene)
        for index in scene.layers.indices {
            scene.layers[index].z = Int32(scene.layers.count - 1 - index)
        }
        mixer.session.scenes[i] = scene
    }

    private func addLayer() {
        mutate { scene in
            let z = scene.layers.map(\.z).max().map { $0 + 1 } ?? 0
            let layer = SceneLayer(inputId: mixer.selectedInputId ?? EIVIZ_SRC_BARS, z: z)
            scene.layers.append(layer)
            selectedLayer = layer.id
        }
        push()
    }

    private func deleteLayer() {
        mutate { scene in
            scene.layers.removeAll { $0.id == selectedLayer }
            selectedLayer = scene.layers.last?.id
        }
        push()
    }

    private func shiftZ(_ delta: Int) {
        mutate { scene in
            guard let selected = selectedLayer,
                  let index = scene.layers.firstIndex(where: { $0.id == selected })
            else { return }
            let target = index + delta
            guard scene.layers.indices.contains(target) else { return }
            scene.layers.swapAt(index, target)
        }
        push()
    }

    private func moveLayers(from offsets: IndexSet, to dest: Int) {
        mutate { $0.layers.move(fromOffsets: offsets, toOffset: dest) }
        push()
    }

    private func applyWire(id: UUID, x: Float, y: Float, w: Float, h: Float, ended: Bool) {
        mutate { scene in
            if let i = scene.layers.firstIndex(where: { $0.id == id }), !scene.layers[i].locked {
                scene.layers[i].x = x
                scene.layers[i].y = y
                scene.layers[i].width = w
                scene.layers[i].height = h
            }
        }
        if ended || Date().timeIntervalSince(lastGpuPush) >= 0.05 {
            lastGpuPush = Date()
            push()
        }
    }

    private func push() {
        if let scene = current { mixer.pushScene(scene) }
    }

    private func layerFields(_ index: Int) -> some View {
        let locked = layer(index)?.locked == true
        return VStack(alignment: .leading, spacing: 8) {
        Grid(alignment: .leading) {
            GridRow {
                dragLabel("Pos X", index: index, axis: .x)
                pixelField(index, axis: .x).disabled(locked)
                dragLabel("Pos Y", index: index, axis: .y)
                pixelField(index, axis: .y).disabled(locked)
            }
            GridRow {
                dragLabel("Size X", index: index, axis: .w)
                pixelField(index, axis: .w).disabled(locked)
                Toggle("Link", isOn: boolBinding(index, \.sizeLinked)).disabled(locked)
                dragLabel("Size Y", index: index, axis: .h)
                pixelField(index, axis: .h).disabled(locked)
            }
            GridRow {
                dragLabel("Crop X", index: index, axis: .cx)
                pixelField(index, axis: .cx).disabled(locked)
                dragLabel("Crop Y", index: index, axis: .cy)
                pixelField(index, axis: .cy).disabled(locked)
            }
            GridRow {
                dragLabel("Crop W", index: index, axis: .cw)
                pixelField(index, axis: .cw).disabled(locked)
                dragLabel("Crop H", index: index, axis: .ch)
                pixelField(index, axis: .ch).disabled(locked)
            }
            GridRow {
                DragAdjustLabel(title: "Opacity") { delta, ended in
                    guard let i = sceneIndex, mixer.session.scenes[i].layers.indices.contains(index) else { return }
                    var layer = mixer.session.scenes[i].layers[index]
                    guard !layer.locked else { return }
                    layer.opacity = min(1, max(0, layer.opacity + delta / 80))
                    mixer.session.scenes[i].layers[index] = layer
                    if ended || Date().timeIntervalSince(lastGpuPush) >= 0.05 {
                        lastGpuPush = Date()
                        push()
                    }
                }
                field(index, get: \.opacity, set: { $0.opacity = min(1, max(0, $1)) }).disabled(locked)
            }
        }
            Picker("Input", selection: Binding(
                get: { layer(index)?.inputId ?? 0 },
                set: { value in
                    mutate { scene in
                        if scene.layers.indices.contains(index) {
                            scene.layers[index].inputId = value
                        }
                    }
                    push()
                }
            )) {
                ForEach(mixer.session.inputs) { input in
                    Text(input.name).tag(input.id)
                }
            }
            Toggle("Audio Follow", isOn: boolBinding(index, \.audioFollow))
            Button("Reset") {
                mutate { scene in
                    guard scene.layers.indices.contains(index), !scene.layers[index].locked else { return }
                    scene.layers[index].resetLayout()
                }
                push()
            }
        }
    }

    private enum PixelAxis { case x, y, w, h, cx, cy, cw, ch }

    private func dragLabel(_ title: String, index: Int, axis: PixelAxis) -> some View {
        DragAdjustLabel(title: title) { delta, ended in
            guard let i = sceneIndex, mixer.session.scenes[i].layers.indices.contains(index) else { return }
            var layer = mixer.session.scenes[i].layers[index]
            guard !layer.locked else { return }
            switch axis {
            case .x: layer.x = (layer.x * projectW + delta) / projectW
            case .y: layer.y = (layer.y * projectH + delta) / projectH
            case .w:
                let width = max(1, layer.width * projectW + delta) / projectW
                if layer.sizeLinked && layer.width > 0 {
                    layer.height = width * (layer.height / layer.width)
                }
                layer.width = width
            case .h:
                let height = max(1, layer.height * projectH + delta) / projectH
                if layer.sizeLinked && layer.height > 0 {
                    layer.width = height * (layer.width / layer.height)
                }
                layer.height = height
            case .cx: layer.cropX = (layer.cropX * projectW + delta) / projectW
            case .cy: layer.cropY = (layer.cropY * projectH + delta) / projectH
            case .cw: layer.cropWidth = (layer.cropWidth * projectW + delta) / projectW
            case .ch: layer.cropHeight = (layer.cropHeight * projectH + delta) / projectH
            }
            layer.clampCrop()
            mixer.session.scenes[i].layers[index] = layer
            if ended || Date().timeIntervalSince(lastGpuPush) >= 0.05 {
                lastGpuPush = Date()
                push()
            }
        }
    }

    private func layer(_ index: Int) -> SceneLayer? {
        guard let i = sceneIndex, mixer.session.scenes[i].layers.indices.contains(index) else { return nil }
        return mixer.session.scenes[i].layers[index]
    }

    private func pixelField(_ index: Int, axis: PixelAxis) -> some View {
        mixerFloatField(Binding(
            get: {
                guard let layer = layer(index) else { return 0 }
                switch axis {
                case .x: return layer.x * projectW
                case .y: return layer.y * projectH
                case .w: return layer.width * projectW
                case .h: return layer.height * projectH
                case .cx: return layer.cropX * projectW
                case .cy: return layer.cropY * projectH
                case .cw: return layer.cropWidth * projectW
                case .ch: return layer.cropHeight * projectH
                }
            },
            set: { value in
                guard let i = sceneIndex, mixer.session.scenes[i].layers.indices.contains(index) else { return }
                var layer = mixer.session.scenes[i].layers[index]
                switch axis {
                case .x: layer.x = value / projectW
                case .y: layer.y = value / projectH
                case .w:
                    let width = max(1, value) / projectW
                    if layer.sizeLinked && layer.width > 0 {
                        layer.height = width * (layer.height / layer.width)
                    }
                    layer.width = width
                case .h:
                    let height = max(1, value) / projectH
                    if layer.sizeLinked && layer.height > 0 {
                        layer.width = height * (layer.width / layer.height)
                    }
                    layer.height = height
                case .cx: layer.cropX = value / projectW
                case .cy: layer.cropY = value / projectH
                case .cw: layer.cropWidth = max(1, value) / projectW
                case .ch: layer.cropHeight = max(1, value) / projectH
                }
                layer.clampCrop()
                mixer.session.scenes[i].layers[index] = layer
            }
        ), onSubmit: { push() })
        .frame(width: 72)
    }

    private func toggleLayer(_ id: UUID, _ key: WritableKeyPath<SceneLayer, Bool>) {
        mutate { scene in
            if let i = scene.layers.firstIndex(where: { $0.id == id }) {
                scene.layers[i][keyPath: key].toggle()
            }
        }
        push()
    }

    private func boolBinding(_ index: Int, _ key: WritableKeyPath<SceneLayer, Bool>) -> Binding<Bool> {
        Binding(
            get: { layer(index)?[keyPath: key] ?? false },
            set: { value in
                guard let i = sceneIndex, mixer.session.scenes[i].layers.indices.contains(index) else { return }
                mixer.session.scenes[i].layers[index][keyPath: key] = value
                push()
            }
        )
    }

    private func field(
        _ index: Int,
        get: KeyPath<SceneLayer, Float>,
        set: @escaping (inout SceneLayer, Float) -> Void
    ) -> some View {
        mixerFloatField(Binding(
            get: { layer(index)?[keyPath: get] ?? 1 },
            set: { value in
                guard let i = sceneIndex, mixer.session.scenes[i].layers.indices.contains(index) else { return }
                set(&mixer.session.scenes[i].layers[index], value)
            }
        ), onSubmit: { push() })
        .frame(width: 72)
    }

    private func applyPreset(_ name: String) {
        mutate { scene in
            if let user = mixer.session.scenePresets.first(where: { $0.name == name }) {
                for i in 0..<min(scene.layers.count, user.layers.count) where !scene.layers[i].locked {
                    scene.layers[i].x = user.layers[i].x
                    scene.layers[i].y = user.layers[i].y
                    scene.layers[i].width = user.layers[i].width
                    scene.layers[i].height = user.layers[i].height
                    scene.layers[i].opacity = user.layers[i].opacity
                    scene.layers[i].z = user.layers[i].z
                    scene.layers[i].cropX = user.layers[i].cropX
                    scene.layers[i].cropY = user.layers[i].cropY
                    scene.layers[i].cropWidth = user.layers[i].cropWidth
                    scene.layers[i].cropHeight = user.layers[i].cropHeight
                    scene.layers[i].sizeLinked = user.layers[i].sizeLinked
                    scene.layers[i].audioFollow = user.layers[i].audioFollow
                    scene.layers[i].clampCrop()
                }
                scene.layers.sort { $0.z > $1.z }
                return
            }
            if name == "Full" {
                for i in scene.layers.indices where !scene.layers[i].locked {
                    scene.layers[i].x = 0
                    scene.layers[i].y = 0
                    scene.layers[i].width = 1
                    scene.layers[i].height = 1
                    scene.layers[i].resetLayoutExtras()
                }
                return
            }
            let boxes: [(Float, Float, Float, Float)] = switch name {
            case "Split H": [(0, 0, 0.5, 1), (0.5, 0, 0.5, 1)]
            case "Split V": [(0, 0, 1, 0.5), (0, 0.5, 1, 0.5)]
            case "Quad": [(0, 0, 0.5, 0.5), (0.5, 0, 0.5, 0.5), (0, 0.5, 0.5, 0.5), (0.5, 0.5, 0.5, 0.5)]
            case "PiP TR": [(0, 0, 1, 1), (0.62, 0.08, 0.32, 0.32)]
            case "PiP TL": [(0, 0, 1, 1), (0.06, 0.08, 0.32, 0.32)]
            case "PiP BR": [(0, 0, 1, 1), (0.62, 0.60, 0.32, 0.32)]
            case "PiP BL": [(0, 0, 1, 1), (0.06, 0.60, 0.32, 0.32)]
            default: []
            }
            let unlocked = scene.layers.indices.filter { !scene.layers[$0].locked }
            for (slot, i) in unlocked.prefix(boxes.count).enumerated() {
                let box = boxes[slot]
                scene.layers[i].x = box.0
                scene.layers[i].y = box.1
                scene.layers[i].width = box.2
                scene.layers[i].height = box.3
                scene.layers[i].resetLayoutExtras()
            }
        }
        push()
    }

    private func copyFrom(_ id: UInt64) {
        guard let source = mixer.session.scenes.first(where: { $0.id == id }) else { return }
        mutate { scene in
            scene.layers = source.layers.map { layer in
                var copy = layer
                copy.id = UUID()
                return copy
            }
            selectedLayer = scene.layers.first?.id
        }
        push()
    }

    private func savePreset() {
        guard let scene = current else { return }
        mixer.session.scenePresets.append(SceneLayoutPreset(
            name: "Preset \(mixer.session.scenePresets.count + 1)",
            layers: scene.layers
        ))
    }
}

struct DragAdjustLabel: View {
    let title: String
    let onDelta: (Float, Bool) -> Void
    @State private var lastY: CGFloat = 0

    var body: some View {
        Text(title)
            .gesture(
                DragGesture(minimumDistance: 2)
                    .onChanged { value in
                        let y = value.translation.height
                        let step = Float(lastY - y) / 2
                        lastY = y
                        if step != 0 {
                            onDelta(step, false)
                        }
                    }
                    .onEnded { _ in
                        lastY = 0
                        onDelta(0, true)
                    }
            )
    }
}
