import EivizMixer
import SwiftUI

struct OverlayView: View {
    @EnvironmentObject private var mixer: MixerController
    @Environment(\.dismiss) private var dismiss
    @State private var selected: UUID?
    @State private var lastGpuPush = Date.distantPast
    @State private var addKind: OverlaySourceKind = .scene
    @State private var addSourceId: UInt64 = 0

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            overlaySidebar
            overlayCanvas
            overlayInspector
        }
        .padding(12)
        .frame(minWidth: 1400, minHeight: 720)
        .background(EivizTheme.dialog)
        .foregroundStyle(EivizTheme.text)
        .onAppear {
            selected = unit.overlays.first?.id
            addSourceId = mixer.session.scenes.first?.gpuId ?? 0
            reindexOverlays()
        }
        .onChange(of: addKind) { _, kind in
            addSourceId = kind == .input
                ? mixer.session.inputs.first?.id ?? 0
                : mixer.session.scenes.first?.gpuId ?? 0
        }
    }

    private var overlaySidebar: some View {
        VStack(alignment: .leading) {
            HStack {
                Text("Overlays").fontWeight(.bold)
                Spacer()
                Button("+") { add() }
            }
            overlayList
            addSourcePickers
            Spacer()
            Button("Z up") { shift(-1) }
            Button("Z down") { shift(1) }
            Button("Delete") { delete() }
        }
        .frame(width: 260)
        .frame(maxHeight: .infinity, alignment: .top)
        .buttonStyle(MixerButtonStyle())
    }

    private var overlayList: some View {
        List(selection: $selected) {
            ForEach(Array(unit.overlays.enumerated()), id: \.element.id) { pair in
                OverlayListRow(
                    title: rowTitle(pair.offset, pair.element),
                    enabled: mixer.overlayOn[pair.element.id] ?? pair.element.enabled,
                    audioFollow: pair.element.audioFollow,
                    locked: pair.element.locked,
                    onToggleAudio: {
                        toggleOverlay(pair.element.id, \.audioFollow)
                    },
                    onToggleLock: {
                        toggleOverlay(pair.element.id, \.locked)
                    }
                )
                .tag(Optional(pair.element.id))
            }
            .onMove(perform: moveOverlays)
        }
        .scrollContentBackground(.hidden)
        .background(EivizTheme.list)
    }

    private var addSourcePickers: some View {
        Group {
            Picker("", selection: $addKind) {
                Text("Scene").tag(OverlaySourceKind.scene)
                Text("Input").tag(OverlaySourceKind.input)
            }
            Picker("", selection: $addSourceId) {
                if addKind == .scene {
                    ForEach(mixer.session.scenes) { scene in
                        Text(scene.name).tag(scene.gpuId)
                    }
                } else {
                    ForEach(mixer.session.inputs) { input in
                        Text(input.name).tag(input.id)
                    }
                }
            }
        }
    }

    private var overlayCanvas: some View {
        VStack {
            Text("Wireframe (\(mixer.selectedUnit.width)x\(mixer.selectedUnit.height))").fontWeight(.bold)
            WireCanvasView(
                items: overlayWireItems,
                aspect: CGFloat(mixer.selectedUnit.width) / max(1, CGFloat(mixer.selectedUnit.height)),
                selected: $selected,
                onChange: applyWire
            )
            .clipped()
        }
        .clipped()
    }

    private var overlayWireItems: [WireRect] {
        unit.overlays.map {
            WireRect(
                id: $0.id,
                x: $0.x,
                y: $0.y,
                width: $0.width,
                height: $0.height,
                enabled: $0.enabled,
                locked: $0.locked,
                sizeLinked: $0.sizeLinked,
                cropX: $0.cropX,
                cropY: $0.cropY,
                cropWidth: $0.cropWidth,
                cropHeight: $0.cropHeight
            )
        }
    }

    private var overlayInspector: some View {
        VStack(alignment: .leading) {
            Text("Live preview").fontWeight(.bold)
            overlayLivePreview
            ScrollView {
                VStack(alignment: .leading, spacing: 8) {
                    if let slot = current {
                        overlaySourcePickers(slot)
                            .disabled(slot.locked)
                        overlayReset(slot)
                        overlayTransform
                        overlayFields
                    }
                }
            }
            HStack {
                Spacer()
                Button("Close") { dismiss() }
            }
        }
        .frame(width: 380)
        .buttonStyle(MixerButtonStyle())
    }

    private var overlayLivePreview: some View {
        MetalPreviewRepresentable(role: .unit(unitId: unit.id, kind: EIVIZ_OUTPUT_PROGRAM))
            .aspectRatio(
                CGFloat(mixer.selectedUnit.width) / max(1, CGFloat(mixer.selectedUnit.height)),
                contentMode: .fit
            )
            .frame(maxWidth: .infinity, minHeight: 160)
            .background(Color.black)
    }

    @ViewBuilder
    private func overlaySourcePickers(_ slot: OverlaySlot) -> some View {
        Picker("Source", selection: Binding(
            get: { slot.sourceKind },
            set: { value in
                mutate { current in
                    current.sourceKind = value
                    current.sceneGpuId = value == .input
                        ? mixer.session.inputs.first?.id ?? 0
                        : mixer.session.scenes.first?.gpuId ?? 0
                }
                mixer.pushOverlays()
            }
        )) {
            Text("Scene").tag(OverlaySourceKind.scene)
            Text("Input").tag(OverlaySourceKind.input)
        }
        Picker("", selection: Binding(
            get: { slot.sceneGpuId },
            set: { value in
                mutate { $0.sceneGpuId = value }
                mixer.pushOverlays()
            }
        )) {
            if slot.sourceKind == .input {
                ForEach(mixer.session.inputs) { input in
                    Text(input.name).tag(input.id)
                }
            } else {
                ForEach(mixer.session.scenes) { scene in
                    Text(scene.name).tag(scene.gpuId)
                }
            }
        }
    }

    private func overlayReset(_ slot: OverlaySlot) -> some View {
        Button("Reset") {
            guard slot.locked == false else { return }
            mutate { $0.resetLayout() }
            mixer.pushOverlays()
        }
    }

    private var unit: MixingUnitEntry { mixer.selectedUnit }
    private var current: OverlaySlot? { unit.overlays.first { $0.id == selected } }

    private func rowTitle(_ index: Int, _ slot: OverlaySlot) -> String {
        "\(index + 1). \(sourceName(slot))"
    }

    private func sourceName(_ slot: OverlaySlot) -> String {
        if slot.sourceKind == .input {
            return mixer.session.inputs.first { $0.id == slot.sceneGpuId }?.name ?? "Input"
        }
        return mixer.session.scenes.first { $0.gpuId == slot.sceneGpuId }?.name ?? "Scene"
    }

    private func toggleOverlay(_ id: UUID, _ key: WritableKeyPath<OverlaySlot, Bool>) {
        guard let ui = mixer.session.units.firstIndex(where: { $0.id == mixer.selectedUnitId }),
              let si = mixer.session.units[ui].overlays.firstIndex(where: { $0.id == id })
        else { return }
        mixer.session.units[ui].overlays[si][keyPath: key].toggle()
        selected = id
        mixer.pushOverlays()
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
        let source = addSourceId != 0
            ? addSourceId
            : (addKind == .input ? mixer.session.inputs.first?.id : mixer.session.scenes.first?.gpuId) ?? 0
        let slot = OverlaySlot(sourceKind: addKind, sceneGpuId: source)
        mixer.session.units[ui].overlays.insert(slot, at: 0)
        selected = slot.id
        reindexOverlays()
        mixer.pushOverlays()
    }

    private func delete() {
        guard let ui = mixer.session.units.firstIndex(where: { $0.id == mixer.selectedUnitId }) else { return }
        mixer.session.units[ui].overlays.removeAll { $0.id == selected }
        selected = mixer.session.units[ui].overlays.first?.id
        reindexOverlays()
        mixer.pushOverlays()
    }

    private func shift(_ delta: Int) {
        guard let ui = mixer.session.units.firstIndex(where: { $0.id == mixer.selectedUnitId }),
              let index = mixer.session.units[ui].overlays.firstIndex(where: { $0.id == selected })
        else { return }
        let target = index + delta
        guard mixer.session.units[ui].overlays.indices.contains(target) else { return }
        mixer.session.units[ui].overlays.swapAt(index, target)
        reindexOverlays()
        mixer.pushOverlays()
    }

    private func moveOverlays(from offsets: IndexSet, to dest: Int) {
        guard let ui = mixer.session.units.firstIndex(where: { $0.id == mixer.selectedUnitId }) else { return }
        mixer.session.units[ui].overlays.move(fromOffsets: offsets, toOffset: dest)
        reindexOverlays()
        mixer.pushOverlays()
    }

    private func reindexOverlays() {
        guard let ui = mixer.session.units.firstIndex(where: { $0.id == mixer.selectedUnitId }) else { return }
        for index in mixer.session.units[ui].overlays.indices {
            mixer.session.units[ui].overlays[index].z = Int32(mixer.session.units[ui].overlays.count - 1 - index)
        }
    }

    private func applyWire(id: UUID, x: Float, y: Float, w: Float, h: Float, ended: Bool) {
        selected = id
        guard let ui = mixer.session.units.firstIndex(where: { $0.id == mixer.selectedUnitId }),
              let si = mixer.session.units[ui].overlays.firstIndex(where: { $0.id == id })
        else { return }
        guard !mixer.session.units[ui].overlays[si].locked else { return }
        mixer.session.units[ui].overlays[si].x = x
        mixer.session.units[ui].overlays[si].y = y
        mixer.session.units[ui].overlays[si].width = w
        mixer.session.units[ui].overlays[si].height = h
        if ended || Date().timeIntervalSince(lastGpuPush) >= 0.05 {
            lastGpuPush = Date()
            mixer.pushOverlays()
        }
    }

    private var overlayCanvasSize: (Float, Float) {
        (Float(mixer.selectedUnit.width), Float(mixer.selectedUnit.height))
    }

    private var overlayTransform: some View {
        let pw = overlayCanvasSize.0
        let ph = overlayCanvasSize.1
        let locked = current?.locked == true
        return VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .top, spacing: 8) {
                overlayMeter("Pos X", range: -pw ... pw * 2) { $0.x * pw } set: { $0.x = $1 / pw }
                overlayMeter("Pos Y", range: -ph ... ph * 2) { $0.y * ph } set: { $0.y = $1 / ph }
            }
            HStack(alignment: .top, spacing: 8) {
                overlayMeter("Size X", range: 1 ... pw * 2) { $0.width * pw } set: { slot, value in
                    let width = max(1, value) / pw
                    if slot.sizeLinked && slot.width > 0 {
                        slot.height = width * (slot.height / slot.width)
                    }
                    slot.width = width
                }
                Toggle("Link", isOn: Binding(
                    get: { current?.sizeLinked ?? true },
                    set: { value in mutate { $0.sizeLinked = value } }
                ))
                .disabled(locked)
                overlayMeter("Size Y", range: 1 ... ph * 2) { $0.height * ph } set: { slot, value in
                    let height = max(1, value) / ph
                    if slot.sizeLinked && slot.height > 0 {
                        slot.width = height * (slot.width / slot.height)
                    }
                    slot.height = height
                }
            }
            VStack(alignment: .leading, spacing: 6) {
                Text("Crop").fontWeight(.bold)
                Text("px from each edge")
                    .font(.system(size: 11))
                    .opacity(0.7)
                HStack(alignment: .top, spacing: 8) {
                    overlayMeter("Left", range: 0 ... pw) { $0.cropX * pw } set: { $0.setCropInset($1 / pw, edit: .left) }
                    overlayMeter("Up", range: 0 ... ph) { $0.cropY * ph } set: { $0.setCropInset($1 / ph, edit: .up) }
                }
                HStack(alignment: .top, spacing: 8) {
                    overlayMeter("Right", range: 0 ... pw) { (1 - $0.cropX - $0.cropWidth) * pw } set: { $0.setCropInset($1 / pw, edit: .right) }
                    overlayMeter("Down", range: 0 ... ph) { (1 - $0.cropY - $0.cropHeight) * ph } set: { $0.setCropInset($1 / ph, edit: .down) }
                }
            }
            overlayMeter("Opacity", range: 0 ... 1, pixelsPerUnit: 400) { $0.opacity } set: { $0.opacity = min(1, max(0, $1)) }
        }
    }

    private func overlayMeter(
        _ title: String,
        range: ClosedRange<Float>,
        pixelsPerUnit: Float = 2,
        get: @escaping (OverlaySlot) -> Float,
        set: @escaping (inout OverlaySlot, Float) -> Void
    ) -> some View {
        ExpandableMeter(
            title: title,
            value: Binding(
                get: { current.map(get) ?? 0 },
                set: { value in
                    guard current?.locked != true else { return }
                    mutate { set(&$0, value) }
                }
            ),
            range: range,
            disabled: current?.locked == true,
            pixelsPerUnit: pixelsPerUnit
        ) { ended in
            if ended || Date().timeIntervalSince(lastGpuPush) >= 0.05 {
                lastGpuPush = Date()
                mixer.pushOverlays()
            }
        }
    }

    private var overlayFields: some View {
        VStack(alignment: .leading, spacing: 6) {
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
}

private struct OverlayListRow: View {
    let title: String
    let enabled: Bool
    let audioFollow: Bool
    let locked: Bool
    let onToggleAudio: () -> Void
    let onToggleLock: () -> Void

    var body: some View {
        HStack(spacing: 4) {
            Button(audioFollow ? "🔊" : "🔇", action: onToggleAudio)
                .buttonStyle(.plain)
            Button(locked ? "🔒" : "🔓", action: onToggleLock)
                .buttonStyle(.plain)
            Text(title)
                .lineLimit(1)
                .truncationMode(.tail)
                .foregroundStyle(enabled ? EivizTheme.text : EivizTheme.dim)
            Spacer(minLength: 0)
        }
    }
}
