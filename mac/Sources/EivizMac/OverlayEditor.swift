import EivizMixer
import SwiftUI

struct OverlayView: View {
    @EnvironmentObject private var mixer: MixerController
    @Environment(\.dismiss) private var dismiss
    @State private var selected: UUID?
    @State private var previewMonitor: UInt64 = 0
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
        .frame(minWidth: 1280, minHeight: 720)
        .background(EivizTheme.dialog)
        .foregroundStyle(EivizTheme.text)
        .onAppear {
            previewMonitor = mixer.session.nextMonitorId
            mixer.session.nextMonitorId += 1
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
                    enabled: pair.element.enabled,
                    locked: pair.element.locked,
                    onToggleEnabled: {
                        mixer.setOverlayEnabled(pair.element.id, enabled: !pair.element.enabled)
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
            if let slot = current {
                overlaySourcePickers(slot)
                overlayFollowAndReset(slot)
                overlayTransform
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

    private var overlayLivePreview: some View {
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

    private func overlayFollowAndReset(_ slot: OverlaySlot) -> some View {
        Group {
            Toggle("Audio Follow", isOn: Binding(
                get: { slot.audioFollow },
                set: { value in
                    mutate { $0.audioFollow = value }
                    mixer.pushOverlays()
                }
            ))
            Button("Reset") {
                guard slot.locked == false else { return }
                mutate { $0.resetLayout() }
                mixer.pushOverlays()
            }
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
        let size = overlayCanvasSize
        return Grid(alignment: .leading) {
            overlayPosRow(pw: size.0, ph: size.1)
            overlaySizeRow(pw: size.0, ph: size.1)
            overlayCropOriginRow(pw: size.0, ph: size.1)
            overlayCropSizeRow(pw: size.0, ph: size.1)
            overlayOpacityRow
        }
    }

    private func overlayPosRow(pw: Float, ph: Float) -> some View {
        GridRow {
            overlayDrag("Pos X") { $0.x = ($0.x * pw + $1) / pw }
            overlayPixel(\.x, scale: pw)
            overlayDrag("Pos Y") { $0.y = ($0.y * ph + $1) / ph }
            overlayPixel(\.y, scale: ph)
        }
    }

    private func overlaySizeRow(pw: Float, ph: Float) -> some View {
        GridRow {
            overlayDrag("Size X") { slot, delta in
                let width = max(1, slot.width * pw + delta) / pw
                if slot.sizeLinked && slot.width > 0 {
                    slot.height = width * (slot.height / slot.width)
                }
                slot.width = width
            }
            overlayPixel(\.width, scale: pw, linked: .width)
            Toggle("Link", isOn: Binding(
                get: { current?.sizeLinked ?? true },
                set: { value in mutate { $0.sizeLinked = value } }
            ))
            overlayDrag("Size Y") { slot, delta in
                let height = max(1, slot.height * ph + delta) / ph
                if slot.sizeLinked && slot.height > 0 {
                    slot.width = height * (slot.width / slot.height)
                }
                slot.height = height
            }
            overlayPixel(\.height, scale: ph, linked: .height)
        }
    }

    private func overlayCropOriginRow(pw: Float, ph: Float) -> some View {
        GridRow {
            overlayDrag("Crop X") { $0.cropX = ($0.cropX * pw + $1) / pw; $0.clampCrop() }
            overlayPixel(\.cropX, scale: pw, crop: true)
            overlayDrag("Crop Y") { $0.cropY = ($0.cropY * ph + $1) / ph; $0.clampCrop() }
            overlayPixel(\.cropY, scale: ph, crop: true)
        }
    }

    private func overlayCropSizeRow(pw: Float, ph: Float) -> some View {
        GridRow {
            overlayDrag("Crop W") { $0.cropWidth = ($0.cropWidth * pw + $1) / pw; $0.clampCrop() }
            overlayPixel(\.cropWidth, scale: pw, crop: true)
            overlayDrag("Crop H") { $0.cropHeight = ($0.cropHeight * ph + $1) / ph; $0.clampCrop() }
            overlayPixel(\.cropHeight, scale: ph, crop: true)
        }
    }

    private var overlayOpacityRow: some View {
        GridRow {
            DragAdjustLabel(title: "Opacity") { delta, ended in
                mutate { $0.opacity = min(1, max(0, $0.opacity + delta / 80)) }
                if ended || Date().timeIntervalSince(lastGpuPush) >= 0.05 {
                    lastGpuPush = Date()
                    mixer.pushOverlays()
                }
            }
            mixerFloatField(Binding(
                get: { current?.opacity ?? 1 },
                set: { value in mutate { $0.opacity = min(1, max(0, value)) } }
            ), onSubmit: { mixer.pushOverlays() })
            .frame(width: 72)
        }
    }

    private func overlayDrag(_ title: String, _ apply: @escaping (inout OverlaySlot, Float) -> Void) -> some View {
        DragAdjustLabel(title: title) { delta, ended in
            guard current?.locked != true else { return }
            mutate { apply(&$0, delta) }
            if ended || Date().timeIntervalSince(lastGpuPush) >= 0.05 {
                lastGpuPush = Date()
                mixer.pushOverlays()
            }
        }
    }

    private enum OverlayLink { case width, height }

    private func overlayPixel(
        _ key: WritableKeyPath<OverlaySlot, Float>,
        scale: Float,
        linked: OverlayLink? = nil,
        crop: Bool = false
    ) -> some View {
        mixerFloatField(Binding(
            get: { (current?[keyPath: key] ?? 0) * scale },
            set: { value in
                guard current?.locked != true else { return }
                mutate { slot in
                    if linked == .width, slot.sizeLinked, slot.width > 0 {
                        let width = max(1, value) / scale
                        slot.height = width * (slot.height / slot.width)
                        slot.width = width
                    } else if linked == .height, slot.sizeLinked, slot.height > 0 {
                        let height = max(1, value) / scale
                        slot.width = height * (slot.width / slot.height)
                        slot.height = height
                    } else {
                        slot[keyPath: key] = value / scale
                    }
                    if crop { slot.clampCrop() }
                }
            }
        ), onSubmit: { mixer.pushOverlays() })
        .frame(width: 72)
        .disabled(current?.locked == true)
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
    let locked: Bool
    let onToggleEnabled: () -> Void
    let onToggleLock: () -> Void

    var body: some View {
        HStack {
            Text(title)
                .foregroundStyle(enabled ? EivizTheme.text : EivizTheme.dim)
            Spacer()
            Button(enabled ? "ON" : "OFF", action: onToggleEnabled)
                .buttonStyle(.plain)
            Button(locked ? "🔒" : "🔓", action: onToggleLock)
                .buttonStyle(.plain)
        }
    }
}
