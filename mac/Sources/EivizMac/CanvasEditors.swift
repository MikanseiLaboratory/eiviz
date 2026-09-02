import EivizMixer
import SwiftUI

struct SceneEditorView: View {
    @EnvironmentObject private var mixer: MixerController
    @Environment(\.dismiss) private var dismiss
    @State private var original: [SceneLayer] = []
    @State private var selectedLayer: UUID?
    @State private var name: String = ""
    @State private var copyFromId: UInt64 = 0
    @State private var lastGpuPush = Date.distantPast
    @State private var editorMonitor: UInt64 = 0

    private var sceneIndex: Int? {
        mixer.session.scenes.firstIndex { $0.id == mixer.editingScene?.id }
    }

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            VStack(alignment: .leading) {
                Text("Layers").fontWeight(.bold)
                List(selection: $selectedLayer) {
                    ForEach(layers) { layer in
                        HStack(spacing: 4) {
                            Button(layer.hidden ? "–" : "👁") {
                                toggleLayer(layer.id, \.hidden)
                            }
                            .buttonStyle(.plain)
                            Button(layer.audioFollow ? "🔊" : "🔇") {
                                toggleLayer(layer.id, \.audioFollow)
                            }
                            .buttonStyle(.plain)
                            Button(layer.locked ? "🔒" : "🔓") {
                                toggleLayer(layer.id, \.locked)
                            }
                            .buttonStyle(.plain)
                            Text(label(layer))
                                .lineLimit(1)
                                .truncationMode(.tail)
                                .foregroundStyle(layer.hidden ? EivizTheme.dim : EivizTheme.text)
                            Spacer(minLength: 0)
                        }
                        .tag(Optional(layer.id))
                    }
                    .onMove(perform: moveLayers)
                }
                .scrollContentBackground(.hidden)
                .background(EivizTheme.list)
                Text("Preset (all layers)").padding(.top, 8)
                ScrollView {
                    LazyVGrid(columns: [GridItem(.adaptive(minimum: 104), spacing: 8)], spacing: 8) {
                        ForEach(SceneLayoutPresets.builtIn, id: \.self) { name in
                            presetCard(name, SceneLayoutPresets.boxes(name))
                        }
                        ForEach(mixer.session.scenePresets) { preset in
                            presetCard(preset.name, preset.layers.map {
                                (CGFloat($0.x), CGFloat($0.y), CGFloat($0.width), CGFloat($0.height))
                            }, onDelete: { deletePreset(preset.id) })
                        }
                    }
                }
                .frame(maxHeight: 240)
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
            .frame(width: 300)
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
                if let scene = current, editorMonitor != 0 {
                    MetalPreviewRepresentable(role: .monitor(monitorId: editorMonitor, sourceId: scene.gpuId))
                        .id(editorMonitor)
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
            .frame(width: 380)
            .buttonStyle(MixerButtonStyle())
        }
        .padding(12)
        .frame(minWidth: 1400, minHeight: 720)
        .background(EivizTheme.dialog)
        .foregroundStyle(EivizTheme.text)
        .onAppear {
            if editorMonitor == 0 {
                editorMonitor = mixer.allocateMonitorId()
            }
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
        VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .top, spacing: 8) {
                layerMeter("Pos X", index: index, axis: .x, range: -projectW ... projectW * 2)
                layerMeter("Pos Y", index: index, axis: .y, range: -projectH ... projectH * 2)
            }
            HStack(alignment: .top, spacing: 8) {
                layerMeter("Size X", index: index, axis: .w, range: 1 ... projectW * 2)
                Toggle("Link", isOn: boolBinding(index, \.sizeLinked)).disabled(locked)
                layerMeter("Size Y", index: index, axis: .h, range: 1 ... projectH * 2)
            }
            VStack(alignment: .leading, spacing: 6) {
                Text("Crop").fontWeight(.bold)
                Text("px from each edge")
                    .font(.system(size: 11))
                    .opacity(0.7)
                HStack(alignment: .top, spacing: 8) {
                    layerMeter("Left", index: index, axis: .cx, range: 0 ... projectW)
                    layerMeter("Up", index: index, axis: .cy, range: 0 ... projectH)
                }
                HStack(alignment: .top, spacing: 8) {
                    layerMeter("Right", index: index, axis: .cw, range: 0 ... projectW)
                    layerMeter("Down", index: index, axis: .ch, range: 0 ... projectH)
                }
            }
            layerMeter("Opacity", index: index, axis: .op, range: 0 ... 1, pixelsPerUnit: 400)
        }
            Picker("Input", selection: Binding(
                get: { layer(index)?.inputId ?? 0 },
                set: { value in
                    guard locked == false else { return }
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
            .disabled(locked)
            Button("Reset") {
                mutate { scene in
                    guard scene.layers.indices.contains(index), !scene.layers[index].locked else { return }
                    scene.layers[index].resetLayout()
                }
                push()
            }
        }
    }

    private enum PixelAxis { case x, y, w, h, cx, cy, cw, ch, op }

    private func layerMeter(
        _ title: String,
        index: Int,
        axis: PixelAxis,
        range: ClosedRange<Float>,
        pixelsPerUnit: Float = 2
    ) -> some View {
        ExpandableMeter(
            title: title,
            value: pixelBinding(index, axis: axis),
            range: range,
            disabled: layer(index)?.locked == true,
            pixelsPerUnit: pixelsPerUnit
        ) { ended in
            if ended || Date().timeIntervalSince(lastGpuPush) >= 0.05 {
                lastGpuPush = Date()
                push()
            }
        }
    }

    private func pixelBinding(_ index: Int, axis: PixelAxis) -> Binding<Float> {
        Binding(
            get: {
                guard let layer = layer(index) else { return 0 }
                switch axis {
                case .x: return layer.x * projectW
                case .y: return layer.y * projectH
                case .w: return layer.width * projectW
                case .h: return layer.height * projectH
                case .cx: return layer.cropX * projectW
                case .cy: return layer.cropY * projectH
                case .cw: return (1 - layer.cropX - layer.cropWidth) * projectW
                case .ch: return (1 - layer.cropY - layer.cropHeight) * projectH
                case .op: return layer.opacity
                }
            },
            set: { value in
                applyPixel(index, axis: axis, value: value)
            }
        )
    }

    private func applyPixel(_ index: Int, axis: PixelAxis, value: Float) {
        guard let i = sceneIndex, mixer.session.scenes[i].layers.indices.contains(index) else { return }
        var layer = mixer.session.scenes[i].layers[index]
        guard !layer.locked else { return }
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
        case .cx: layer.setCropInset(value / projectW, edit: .left)
        case .cy: layer.setCropInset(value / projectH, edit: .up)
        case .cw: layer.setCropInset(value / projectW, edit: .right)
        case .ch: layer.setCropInset(value / projectH, edit: .down)
        case .op: layer.opacity = min(1, max(0, value))
        }
        mixer.session.scenes[i].layers[index] = layer
    }

    private func layer(_ index: Int) -> SceneLayer? {
        guard let i = sceneIndex, mixer.session.scenes[i].layers.indices.contains(index) else { return nil }
        return mixer.session.scenes[i].layers[index]
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

    private func presetCard(_ name: String, _ boxes: [(CGFloat, CGFloat, CGFloat, CGFloat)], onDelete: (() -> Void)? = nil) -> some View {
        VStack(spacing: 2) {
            Button {
                applyPreset(name)
            } label: {
                LayoutPresetMosaic(boxes: boxes)
                    .frame(height: 58)
                    .overlay(Rectangle().stroke(EivizTheme.stroke, lineWidth: 1))
            }
            .buttonStyle(.plain)
            HStack(spacing: 4) {
                Text(name)
                    .font(.system(size: 10))
                    .lineLimit(1)
                    .frame(maxWidth: .infinity, alignment: .leading)
                if let onDelete {
                    Button("×", action: onDelete)
                        .buttonStyle(.plain)
                        .font(.system(size: 12, weight: .bold))
                }
            }
        }
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
            let boxes = SceneLayoutPresets.boxes(name).map { (Float($0.0), Float($0.1), Float($0.2), Float($0.3)) }
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

    private func deletePreset(_ id: UUID) {
        mixer.session.scenePresets.removeAll { $0.id == id }
    }
}

