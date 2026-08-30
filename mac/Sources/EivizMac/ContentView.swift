import EivizMixer
import SwiftUI

struct ContentView: View {
    @EnvironmentObject private var mixer: MixerController

    var body: some View {
        VStack(spacing: 0) {
            topBar
            VStack(spacing: 8) {
                HStack(spacing: 8) {
                    bus(title: "PREVIEW", color: EivizTheme.preview, kind: EIVIZ_OUTPUT_PREVIEW)
                    transitions
                    bus(title: "PROGRAM", color: EivizTheme.program, kind: EIVIZ_OUTPUT_PROGRAM)
                }
                .frame(maxHeight: .infinity)
                lower
            }
            .padding(8)
            audioBar
            statusBar
        }
        .background(EivizTheme.background)
        .foregroundStyle(EivizTheme.text)
        .sheet(isPresented: $mixer.showSettings) { SettingsView() }
        .sheet(isPresented: $mixer.showAddInput, onDismiss: { mixer.editingInput = nil }) {
            AddInputView(editing: mixer.editingInput)
        }
        .sheet(isPresented: $mixer.showMixingUnit) {
            MixingUnitView(unit: mixer.editingUnit ?? mixer.selectedUnit)
        }
        .sheet(isPresented: $mixer.showSceneEditor) { SceneEditorView() }
        .sheet(isPresented: $mixer.showOverlay) { OverlayView() }
        .sheet(isPresented: $mixer.showMultiview) { MultiviewView() }
        .sheet(isPresented: $mixer.showMultiviewSlots) { MultiviewSlotsView() }
        .sheet(isPresented: $mixer.showResources) { ResourcesView() }
    }

    private var topBar: some View {
        HStack {
            Text("Mixing Unit")
            Picker("", selection: $mixer.selectedUnitId) {
                ForEach(mixer.session.units) { unit in
                    Text(unit.displayName).tag(unit.id)
                }
            }
            .frame(width: 280)
            Button("Add") { mixer.addUnit() }
            Button("Edit") {
                mixer.editingUnit = mixer.selectedUnit
                mixer.showMixingUnit = true
            }
            Button("Delete") { mixer.deleteUnit() }
            Spacer()
            Button("Save") { mixer.saveSession() }
            Button("Load") { mixer.loadSession() }
            Button("Overlay") { mixer.showOverlay = true }
            Button("Multiview") { mixer.openNewMultiview() }
            Button("Resources") { mixer.showResources = true }
            Button("Settings") { mixer.showSettings = true }
        }
        .buttonStyle(MixerButtonStyle())
        .padding(.horizontal, 8)
        .padding(.vertical, 6)
        .background(EivizTheme.chrome)
    }

    private var transitions: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Text("Transitions").fontWeight(.bold)
                Spacer()
                Button("+") {
                    var unit = mixer.selectedUnit
                    unit.transitions.append(TransitionPreset())
                    mixer.saveUnit(unit)
                    mixer.tbarPresetIndex = unit.transitions.count - 1
                    if let id = unit.transitions.last?.id {
                        mixer.expandedTransitions.insert(id)
                    }
                }
                .buttonStyle(MixerButtonStyle())
            }
            ScrollView {
                VStack(spacing: 4) {
                    ForEach(Array(mixer.selectedUnit.transitions.enumerated()), id: \.element.id) { index, preset in
                        transitionRow(index: index, preset: preset)
                    }
                }
            }
            Slider(
                value: Binding(
                    get: { Double(mixer.mix) },
                    set: { mixer.setMix(Float($0)) }
                ),
                in: 0 ... 1,
                onEditingChanged: { editing in
                    if !editing { mixer.finishTBar() }
                }
            )
            .tint(EivizTheme.preview)
            .frame(width: 132)
            .frame(maxWidth: .infinity)
        }
        .frame(width: 168)
    }

    private func transitionRow(index: Int, preset: TransitionPreset) -> some View {
        let selected = index == mixer.tbarPresetIndex
        return HStack(alignment: .top, spacing: 4) {
            DisclosureGroup(
                isExpanded: Binding(
                    get: { mixer.expandedTransitions.contains(preset.id) },
                    set: { open in
                        if open {
                            mixer.expandedTransitions.insert(preset.id)
                        } else {
                            mixer.expandedTransitions.remove(preset.id)
                        }
                    }
                )
            ) {
                VStack(alignment: .leading, spacing: 4) {
                    Picker("", selection: Binding(
                        get: { mixer.selectedUnit.transitions[safe: index]?.kind ?? preset.kind },
                        set: { value in
                            var unit = mixer.selectedUnit
                            if index < unit.transitions.count {
                                unit.transitions[index].kind = value
                                mixer.saveUnit(unit)
                            }
                        }
                    )) {
                        Text("Cut").tag(EIVIZ_TRANSITION_CUT)
                        Text("Fade").tag(EIVIZ_TRANSITION_FADE)
                        Text("Dip").tag(EIVIZ_TRANSITION_DIP)
                    }
                    Text("Duration (frames)").font(.system(size: 11)).foregroundStyle(EivizTheme.dim)
                    mixerUintField(Binding(
                        get: {
                            mixer.selectedUnit.transitions[safe: index]?.durationFrames ?? preset.durationFrames
                        },
                        set: { frames in
                            var unit = mixer.selectedUnit
                            if index < unit.transitions.count {
                                unit.transitions[index].durationFrames = frames
                                mixer.saveUnit(unit)
                            }
                        }
                    ))
                    Toggle("Swap", isOn: Binding(
                        get: { mixer.selectedUnit.transitions[safe: index]?.swap ?? true },
                        set: { value in
                            var unit = mixer.selectedUnit
                            if index < unit.transitions.count {
                                unit.transitions[index].swap = value
                                mixer.saveUnit(unit)
                            }
                        }
                    ))
                    Button("Use for T-bar") { mixer.tbarPresetIndex = index }
                    Button("−") {
                        var unit = mixer.selectedUnit
                        guard unit.transitions.count > 1, index < unit.transitions.count else { return }
                        mixer.expandedTransitions.remove(unit.transitions[index].id)
                        unit.transitions.remove(at: index)
                        mixer.saveUnit(unit)
                        mixer.tbarPresetIndex = min(mixer.tbarPresetIndex, unit.transitions.count - 1)
                    }
                }
                .padding(.top, 4)
            } label: {
                Text("\(preset.label)  \(preset.durationFrames)f")
                    .font(.system(size: 12, weight: .semibold))
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .padding(4)
            .overlay(
                Rectangle().stroke(selected ? EivizTheme.preview : EivizTheme.stroke, lineWidth: 1)
            )
            Button("TAKE") { mixer.firePreset(preset, index: index) }
                .buttonStyle(MixerButtonStyle())
        }
        .onTapGesture { mixer.tbarPresetIndex = index }
    }

    private var lower: some View {
        VStack(spacing: 8) {
            if mixer.selectedVideoId != nil {
                videoBar
            }
            HStack(alignment: .top, spacing: 8) {
                VStack(alignment: .leading) {
                    Text("Inputs").fontWeight(.bold)
                    List(selection: $mixer.selectedInputId) {
                        ForEach(mixer.session.inputs) { input in
                            Text(input.name).tag(Optional(input.id))
                        }
                    }
                    .scrollContentBackground(.hidden)
                    .background(EivizTheme.list)
                    HStack {
                        Button("Add") {
                            mixer.editingInput = nil
                            mixer.showAddInput = true
                        }
                        Button("Edit") {
                            guard let id = mixer.selectedInputId,
                                  let input = mixer.session.inputs.first(where: { $0.id == id })
                            else { return }
                            mixer.editingInput = input
                            mixer.showAddInput = true
                        }
                        Button("Delete") { mixer.deleteSelectedInput() }
                    }
                    .buttonStyle(MixerButtonStyle())
                }
                .frame(width: 240)
                VStack(alignment: .leading) {
                    HStack {
                        Text("Scenes").fontWeight(.bold)
                        Spacer()
                        Button("+") { mixer.addScene() }
                        Button("−") { mixer.removeScene() }
                        Button("Edit") {
                            mixer.editingScene = mixer.session.scenes.first { $0.id == mixer.selectedSceneId }
                            mixer.showSceneEditor = true
                        }
                    }
                    .buttonStyle(MixerButtonStyle())
                    ScrollView {
                        LazyVGrid(columns: [GridItem(.adaptive(minimum: 176), spacing: 8)], alignment: .leading, spacing: 8) {
                            ForEach(mixer.session.scenes) { scene in
                                sceneTile(scene)
                            }
                        }
                    }
                }
            }
            .frame(maxHeight: .infinity)
        }
    }

    private func sceneTile(_ scene: SceneEntry) -> some View {
        let selected = mixer.selectedSceneId == scene.id
        return VStack(spacing: 0) {
            Text(scene.name)
                .font(.system(size: 12))
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 6)
                .padding(.vertical, 3)
                .background(Color(white: 0.2))
            MetalPreviewRepresentable(role: .monitor(monitorId: scene.monitorId, sourceId: scene.gpuId))
                .frame(width: 176, height: 90)
                .background(Color.black)
        }
        .frame(width: 176)
        .id("scene-\(scene.id)-\(scene.monitorId)")
        .contentShape(Rectangle())
        .overlay(Rectangle().stroke(selected ? EivizTheme.preview : Color(white: 0.33), lineWidth: 2))
        .onTapGesture { mixer.previewScene(scene) }
        .contextMenu {
            Button("Edit") {
                mixer.editingScene = scene
                mixer.showSceneEditor = true
            }
        }
    }

    private var videoBar: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(mixer.videoTitle).fontWeight(.bold)
            HStack {
                Slider(value: $mixer.videoFraction, in: 0 ... 1) { editing in
                    if !editing { mixer.videoSeek(mixer.videoFraction) }
                }
                Button("Restart") { mixer.videoRestart() }
                Button(mixer.videoPlaying ? "❚❚" : "▶") { mixer.videoPlayToggle() }
            }
            .buttonStyle(MixerButtonStyle())
        }
        .padding(8)
        .background(EivizTheme.videoBar)
    }

    private var audioBar: some View {
        HStack(spacing: 12) {
            Text("Audio").fontWeight(.bold)
            ForEach(mixer.session.buses) { bus in
                meter(title: bus.name, id: bus.role == .master ? 0 : EIVIZ_AUDIO_BUS_PEAK_BASE | bus.id)
            }
            ForEach(mixer.selectedUnit.overlays) { slot in
                Toggle(isOn: Binding(
                    get: {
                        mixer.session.units.first { $0.id == mixer.selectedUnitId }?
                            .overlays.first { $0.id == slot.id }?.enabled ?? slot.enabled
                    },
                    set: { mixer.setOverlayEnabled(slot.id, enabled: $0) }
                )) {
                    Text(overlayName(slot))
                }
                .toggleStyle(.checkbox)
            }
            Spacer()
        }
        .padding(8)
        .background(EivizTheme.panel)
    }

    private func overlayName(_ slot: OverlaySlot) -> String {
        mixer.session.scenes.first { $0.gpuId == slot.sceneGpuId }?.name ?? "DSK"
    }

    private func meter(title: String, id: UInt64) -> some View {
        let peak = mixer.peaks[id] ?? (0, 0)
        return VStack(alignment: .leading, spacing: 2) {
            Text(title).font(.system(size: 10))
            HStack(spacing: 2) {
                Rectangle().fill(EivizTheme.status).frame(width: 8, height: CGFloat(4 + peak.0 * 36))
                Rectangle().fill(EivizTheme.status).frame(width: 8, height: CGFloat(4 + peak.1 * 36))
            }
            .frame(height: 40, alignment: .bottom)
        }
    }

    private var statusBar: some View {
        HStack {
            Text(mixer.status).foregroundStyle(EivizTheme.status)
            Spacer()
            if !mixer.warnText.isEmpty {
                Text(mixer.warnText).foregroundStyle(EivizTheme.warn)
            }
            if !mixer.errorText.isEmpty {
                Text(mixer.errorText).foregroundStyle(EivizTheme.warn)
            }
            Text(mixer.resourceText).foregroundStyle(EivizTheme.hud)
        }
        .font(.system(size: 12))
        .padding(.horizontal, 8)
        .padding(.vertical, 4)
        .background(EivizTheme.statusBar)
    }

    private func bus(title: String, color: Color, kind: UInt32) -> some View {
        VStack(spacing: 0) {
            Text(title)
                .font(.system(size: 13, weight: .bold))
                .foregroundStyle(title == "PREVIEW" ? Color(red: 0.07, green: 0.07, blue: 0.07) : Color.white)
                .frame(maxWidth: .infinity)
                .padding(.vertical, 6)
                .background(color)
            MetalPreviewRepresentable(role: .unit(unitId: mixer.selectedUnitId, kind: kind))
                .frame(minWidth: 320, minHeight: 180)
                .background(Color.black)
        }
        .aspectRatio(16.0 / 9.0, contentMode: .fit)
        .clipped()
        .overlay(Rectangle().stroke(color, lineWidth: 2))
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

private extension Array {
    subscript(safe index: Int) -> Element? {
        indices.contains(index) ? self[index] : nil
    }
}
