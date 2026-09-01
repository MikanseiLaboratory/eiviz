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
                        Text(label(layer)).tag(Optional(layer.id))
                    }
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
                    Button("Z up") { shiftZ(1) }
                    Button("Z down") { shiftZ(-1) }
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
        return "\(layer.locked ? "🔒 " : "")\(layer.z): \(input?.name ?? "\(layer.inputId)")"
    }

    private func mutate(_ body: (inout SceneEntry) -> Void) {
        guard let i = sceneIndex else { return }
        var scene = mixer.session.scenes[i]
        body(&scene)
        scene.layers.sort { $0.z < $1.z }
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
            let a = scene.layers[index].z
            scene.layers[index].z = scene.layers[target].z
            scene.layers[target].z = a
        }
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
        return Grid(alignment: .leading) {
            GridRow {
                dragLabel("Pos X", index: index, axis: .x)
                pixelField(index, axis: .x).disabled(locked)
                dragLabel("Pos Y", index: index, axis: .y)
                pixelField(index, axis: .y).disabled(locked)
            }
            GridRow {
                dragLabel("Size X", index: index, axis: .w)
                pixelField(index, axis: .w).disabled(locked)
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
                Toggle("Link", isOn: boolBinding(index, \.sizeLinked)).disabled(locked)
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
            GridRow {
                Toggle("Lock", isOn: boolBinding(index, \.locked))
            }
            GridRow {
                Toggle("Audio Follow", isOn: boolBinding(index, \.audioFollow))
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
                }
                return
            }
            if name == "Full" {
                for i in scene.layers.indices where !scene.layers[i].locked {
                    scene.layers[i].x = 0
                    scene.layers[i].y = 0
                    scene.layers[i].width = 1
                    scene.layers[i].height = 1
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

struct OverlayView: View {
    @EnvironmentObject private var mixer: MixerController
    @Environment(\.dismiss) private var dismiss
    @State private var selected: UUID?
    @State private var previewMonitor: UInt64 = 0

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            VStack(alignment: .leading) {
                HStack {
                    Text("Overlays").fontWeight(.bold)
                    Spacer()
                    Button("+") { add() }
                }
                List(selection: $selected) {
                    ForEach(unit.overlays) { slot in
                        Text("\(slot.enabled ? "ON" : "off")  \(sceneName(slot))")
                            .tag(Optional(slot.id))
                    }
                }
                .scrollContentBackground(.hidden)
                .background(EivizTheme.list)
                Spacer()
                Button("Z up") { shift(1) }
                Button("Z down") { shift(-1) }
                Button("Delete") { delete() }
            }
            .frame(width: 260, maxHeight: .infinity, alignment: .top)
            .buttonStyle(MixerButtonStyle())

            VStack {
                Text("Layout (\(mixer.selectedUnit.width)x\(mixer.selectedUnit.height) Program)").fontWeight(.bold)
                WireCanvasView(
                    items: unit.overlays.map {
                        WireRect(id: $0.id, x: $0.x, y: $0.y, width: $0.width, height: $0.height, enabled: $0.enabled)
                    },
                    aspect: CGFloat(mixer.selectedUnit.width) / max(1, CGFloat(mixer.selectedUnit.height)),
                    selected: $selected,
                    onChange: applyWire
                )
                .clipped()
            }
            .clipped()

            VStack(alignment: .leading) {
                Text("Scene preview").fontWeight(.bold)
                MetalPreviewRepresentable(role: .monitor(
                    monitorId: previewMonitor,
                    sourceId: current?.sceneGpuId ?? mixer.session.scenes.first?.gpuId ?? 0
                ))
                .aspectRatio(
                    CGFloat(mixer.selectedUnit.width) / max(1, CGFloat(mixer.selectedUnit.height)),
                    contentMode: .fit
                )
                .frame(maxWidth: .infinity)
                .background(Color.black)
                if let slot = current {
                    Toggle("ON", isOn: Binding(
                        get: { slot.enabled },
                        set: { value in
                            mixer.setOverlayEnabled(slot.id, enabled: value)
                        }
                    ))
                    Picker("Scene", selection: Binding(
                        get: { slot.sceneGpuId },
                        set: { value in
                            mutate { $0.sceneGpuId = value }
                            mixer.pushOverlays()
                        }
                    )) {
                        ForEach(mixer.session.scenes) { scene in
                            Text(scene.name).tag(scene.gpuId)
                        }
                    }
                    Toggle("Audio Follow", isOn: Binding(
                        get: { slot.audioFollow },
                        set: { value in
                            mutate { $0.audioFollow = value }
                            mixer.pushOverlays()
                        }
                    ))
                    overlayFields
                }
                Spacer()
                HStack {
                    Spacer()
                    Button("Close") { dismiss() }
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
            previewMonitor = mixer.session.nextMonitorId
            mixer.session.nextMonitorId += 1
            selected = unit.overlays.first?.id
        }
    }

    private var unit: MixingUnitEntry { mixer.selectedUnit }
    private var current: OverlaySlot? { unit.overlays.first { $0.id == selected } }

    private func sceneName(_ slot: OverlaySlot) -> String {
        mixer.session.scenes.first { $0.gpuId == slot.sceneGpuId }?.name ?? "(none)"
    }

    private func mutate(_ body: (inout OverlaySlot) -> Void) {
        guard let ui = mixer.session.units.firstIndex(where: { $0.id == mixer.selectedUnitId }),
              let si = mixer.session.units[ui].overlays.firstIndex(where: { $0.id == selected })
        else { return }
        body(&mixer.session.units[ui].overlays[si])
    }

    private func add() {
        guard let ui = mixer.session.units.firstIndex(where: { $0.id == mixer.selectedUnitId }) else { return }
        guard mixer.session.units[ui].overlays.count < 8 else { return }
        var slot = OverlaySlot(sceneGpuId: mixer.session.scenes.first?.gpuId ?? 0)
        slot.z = Int32(mixer.session.units[ui].overlays.count)
        mixer.session.units[ui].overlays.append(slot)
        selected = slot.id
        mixer.pushOverlays()
    }

    private func delete() {
        guard let ui = mixer.session.units.firstIndex(where: { $0.id == mixer.selectedUnitId }) else { return }
        mixer.session.units[ui].overlays.removeAll { $0.id == selected }
        selected = mixer.session.units[ui].overlays.last?.id
        mixer.pushOverlays()
    }

    private func shift(_ delta: Int) {
        guard let ui = mixer.session.units.firstIndex(where: { $0.id == mixer.selectedUnitId }),
              let index = mixer.session.units[ui].overlays.firstIndex(where: { $0.id == selected })
        else { return }
        let target = index + delta
        guard mixer.session.units[ui].overlays.indices.contains(target) else { return }
        mixer.session.units[ui].overlays.swapAt(index, target)
        mixer.pushOverlays()
    }

    private func applyWire(id: UUID, x: Float, y: Float, w: Float, h: Float, ended: Bool) {
        selected = id
        guard let ui = mixer.session.units.firstIndex(where: { $0.id == mixer.selectedUnitId }),
              let si = mixer.session.units[ui].overlays.firstIndex(where: { $0.id == id })
        else { return }
        mixer.session.units[ui].overlays[si].x = x
        mixer.session.units[ui].overlays[si].y = y
        mixer.session.units[ui].overlays[si].width = w
        mixer.session.units[ui].overlays[si].height = h
        if ended { mixer.pushOverlays() }
    }

    private var overlayFields: some View {
        VStack(alignment: .leading, spacing: 6) {
            Grid {
                GridRow {
                    Text("X")
                    num(\.x)
                    Text("Y")
                    num(\.y)
                }
                GridRow {
                    Text("W")
                    num(\.width)
                    Text("H")
                    num(\.height)
                }
                GridRow {
                    Text("Op")
                    num(\.opacity)
                }
            }
            Picker("On/Off", selection: Binding(
                get: { current?.transitionKind ?? EIVIZ_TRANSITION_FADE },
                set: { value in
                    mutate { $0.transitionKind = value }
                    mixer.pushOverlays()
                }
            )) {
                Text("Cut").tag(EIVIZ_TRANSITION_CUT)
                Text("Fade").tag(EIVIZ_TRANSITION_FADE)
            }
            Text("Duration")
            HStack {
                mixerUintField(Binding(
                    get: { current?.durationValue ?? 15 },
                    set: { value in mutate { $0.durationValue = max(1, value) } }
                ))
                Picker("", selection: Binding(
                    get: { current?.durationUnit ?? 0 },
                    set: { value in
                        mutate { $0.durationUnit = value }
                        mixer.pushOverlays()
                    }
                )) {
                    Text("Frames").tag(EIVIZ_DURATION_FRAMES)
                    Text("Milliseconds").tag(EIVIZ_DURATION_MS)
                }
            }
        }
    }

    private func num(_ key: WritableKeyPath<OverlaySlot, Float>) -> some View {
        mixerFloatField(Binding(
            get: { current?[keyPath: key] ?? 0 },
            set: { value in mutate { $0[keyPath: key] = value } }
        ), onSubmit: { mixer.pushOverlays() })
        .frame(width: 72)
    }
}

private struct DragAdjustLabel: View {
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
