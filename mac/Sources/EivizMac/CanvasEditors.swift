import EivizMixer
import SwiftUI

struct SceneEditorView: View {
    @EnvironmentObject private var mixer: MixerController
    @Environment(\.dismiss) private var dismiss
    @State private var original: [SceneLayer] = []
    @State private var selectedLayer: UUID?
    @State private var name: String = ""

    private var sceneIndex: Int? {
        mixer.session.scenes.firstIndex { $0.id == mixer.editingScene?.id }
    }

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            VStack(alignment: .leading) {
                Text("Layers").fontWeight(.bold)
                List(layers, selection: $selectedLayer) { layer in
                    Text(label(layer)).tag(layer.id)
                }
                .scrollContentBackground(.hidden)
                .background(EivizTheme.list)
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
                Text("Wireframe (16:9)").fontWeight(.bold)
                WireCanvasView(
                    items: layers.map {
                        WireRect(id: $0.id, x: $0.x, y: $0.y, width: $0.width, height: $0.height)
                    },
                    selected: $selectedLayer,
                    onChange: applyWire
                )
            }

            VStack(alignment: .leading) {
                Text("Live preview").fontWeight(.bold)
                if let scene = current {
                    MetalPreviewRepresentable(role: .monitor(monitorId: scene.monitorId, sourceId: scene.gpuId))
                        .frame(height: 180)
                        .background(Color.black)
                }
                Text("Name").padding(.top, 8)
                TextField("Name", text: $name)
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

    private func label(_ layer: SceneLayer) -> String {
        let input = mixer.session.inputs.first { $0.id == layer.inputId }
        return "\(layer.z): \(input?.name ?? "\(layer.inputId)")"
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
            if let i = scene.layers.firstIndex(where: { $0.id == id }) {
                scene.layers[i].x = x
                scene.layers[i].y = y
                scene.layers[i].width = w
                scene.layers[i].height = h
            }
        }
        if ended { push() }
    }

    private func push() {
        if let scene = current { mixer.pushScene(scene) }
    }

    private func layerFields(_ index: Int) -> some View {
        Grid(alignment: .leading) {
            GridRow {
                Text("X")
                field(index, get: \.x, set: { $0.x = $1 })
                Text("Y")
                field(index, get: \.y, set: { $0.y = $1 })
            }
            GridRow {
                Text("W")
                field(index, get: \.width, set: { $0.width = max(0.01, $1) })
                Text("H")
                field(index, get: \.height, set: { $0.height = max(0.01, $1) })
            }
            GridRow {
                Text("Op")
                field(index, get: \.opacity, set: { $0.opacity = min(1, max(0, $1)) })
                if let i = sceneIndex, mixer.session.scenes[i].layers.indices.contains(index) {
                    Toggle("Audio Follow", isOn: Binding(
                        get: { mixer.session.scenes[i].layers[index].audioFollow },
                        set: { value in
                            mixer.session.scenes[i].layers[index].audioFollow = value
                            push()
                        }
                    ))
                }
            }
        }
    }

    private func field(
        _ index: Int,
        get: KeyPath<SceneLayer, Float>,
        set: @escaping (inout SceneLayer, Float) -> Void
    ) -> some View {
        TextField("", text: Binding(
            get: {
                guard let i = sceneIndex, mixer.session.scenes[i].layers.indices.contains(index) else { return "" }
                return String(format: "%.4g", mixer.session.scenes[i].layers[index][keyPath: get])
            },
            set: { text in
                guard let value = Float(text), let i = sceneIndex,
                      mixer.session.scenes[i].layers.indices.contains(index)
                else { return }
                set(&mixer.session.scenes[i].layers[index], value)
            }
        ))
        .onSubmit { push() }
        .frame(width: 72)
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
                List(unit.overlays, selection: $selected) { slot in
                    Text("\(slot.enabled ? "ON" : "off")  \(sceneName(slot))")
                        .tag(slot.id)
                }
                .scrollContentBackground(.hidden)
                .background(EivizTheme.list)
                Button("Z up") { shift(1) }
                Button("Z down") { shift(-1) }
                Button("Delete") { delete() }
            }
            .frame(width: 260)
            .buttonStyle(MixerButtonStyle())

            VStack {
                Text("Layout (16:9 Program)").fontWeight(.bold)
                WireCanvasView(
                    items: unit.overlays.map {
                        WireRect(id: $0.id, x: $0.x, y: $0.y, width: $0.width, height: $0.height, enabled: $0.enabled)
                    },
                    selected: $selected,
                    onChange: applyWire
                )
            }

            VStack(alignment: .leading) {
                Text("Scene preview").fontWeight(.bold)
                MetalPreviewRepresentable(role: .monitor(
                    monitorId: previewMonitor,
                    sourceId: current?.sceneGpuId ?? mixer.session.scenes.first?.gpuId ?? 0
                ))
                .frame(height: 180)
                .background(Color.black)
                if let slot = current {
                    Toggle("ON", isOn: Binding(
                        get: { slot.enabled },
                        set: { value in
                            mutate { $0.enabled = value }
                            mixer.pushOverlays()
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
    }

    private func num(_ key: WritableKeyPath<OverlaySlot, Float>) -> some View {
        TextField("", text: Binding(
            get: {
                guard let slot = current else { return "" }
                return String(format: "%.4g", slot[keyPath: key])
            },
            set: { text in
                guard let value = Float(text) else { return }
                mutate { $0[keyPath: key] = value }
            }
        ))
        .onSubmit { mixer.pushOverlays() }
        .frame(width: 72)
    }
}
