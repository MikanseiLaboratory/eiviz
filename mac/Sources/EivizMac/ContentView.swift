import AppKit
import EivizMixer
import SwiftUI

struct CustomWgslEdit: Identifiable {
    let id = UUID()
    let index: Int
    let text: String
}

struct ContentView: View {
    @EnvironmentObject private var mixer: MixerController
    @ObservedObject private var prefs = AppPrefs.shared
    @State private var customWgslEdit: CustomWgslEdit?

    var body: some View {
        VStack(spacing: 0) {
            topBar
            VSplitView {
                VSplitView {
                    HSplitView {
                        bus(title: previewTitle, color: mixer.session.settings.previewColor, kind: EIVIZ_OUTPUT_PREVIEW)
                        transitions
                            .frame(minWidth: 220, idealWidth: 260)
                        bus(title: programTitle, color: mixer.session.settings.programColor, kind: EIVIZ_OUTPUT_PROGRAM)
                    }
                    lower
                }
                .padding(8)
                audioBar
                    .frame(minHeight: 80)
            }
            statusBar
        }
        .background(EivizTheme.background)
        .foregroundStyle(EivizTheme.text)
        .id("\(prefs.language)-\(prefs.theme)-\(prefs.localeRevision)")
        .sheet(isPresented: $mixer.showSettings) { SettingsView() }
        .sheet(isPresented: $mixer.showPreferences) { PreferencesView() }
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
        .sheet(isPresented: $mixer.showLogs) { LogsView() }
        .sheet(item: $customWgslEdit) { edit in
            CustomWgslEditor(
                text: edit.text,
                onSave: { wgsl in
                    updateTransition(edit.index) { $0.customWgsl = wgsl }
                    wgsl.withCString { _ = mixer_unit_set_custom_wgsl(mixer.selectedUnitId, $0) }
                    customWgslEdit = nil
                },
                onCancel: { customWgslEdit = nil }
            )
        }
    }

    private var topBar: some View {
        HStack {
            Button(L10n.t("chrome.new")) { mixer.newSession() }
            Button(L10n.t("chrome.save")) { mixer.saveSession() }
            Button(L10n.t("chrome.load")) { mixer.loadSession() }
            Menu {
                ForEach(AppPrefs.shared.existingSessions(), id: \.self) { path in
                    Button(URL(fileURLWithPath: path).lastPathComponent) { mixer.loadSession(path: path) }
                }
            } label: {
                Text("▾")
            }
            Spacer()
            Text(L10n.t("chrome.mixingUnit"))
            Picker("", selection: $mixer.selectedUnitId) {
                ForEach(mixer.session.units) { unit in
                    Text(unit.displayName).tag(unit.id)
                }
            }
            .frame(width: 280)
            Button(L10n.t("chrome.add")) { mixer.addUnit() }
            Button(L10n.t("chrome.edit")) {
                mixer.editingUnit = mixer.selectedUnit
                mixer.showMixingUnit = true
            }
            Button(L10n.t("chrome.delete")) { mixer.deleteUnit() }
            Button(L10n.t("chrome.open")) { mixer.openSwitcher() }
            Button(L10n.t("chrome.resources")) { mixer.showResources = true }
            Button(L10n.t("chrome.logs")) { mixer.showLogs = true }
            Button(L10n.t("chrome.settings")) { mixer.showSettings = true }
            Button(L10n.t("chrome.preferences")) { mixer.showPreferences = true }
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
            .frame(minHeight: 80)
            .frame(maxHeight: .infinity)
            Slider(
                value: Binding(
                    get: { Double(mixer.mix) },
                    set: { mixer.setMix(Float($0)) }
                ),
                in: 0 ... 1,
                onEditingChanged: { editing in
                    mixer.tbarDragging = editing
                    if !editing { mixer.finishTBar() }
                }
            )
            .tint(mixer.session.settings.previewColor.color)
            .frame(width: 220)
            .frame(maxWidth: .infinity)
            .padding(.horizontal, 8)
            .padding(.vertical, 8)
            Color.clear.frame(maxHeight: .infinity)
        }
        .padding(.horizontal, 12)
        .frame(minWidth: 220, idealWidth: 260)
    }

    private var previewTitle: String {
        if let id = mixer.previewingSceneId(for: mixer.selectedUnitId),
           let scene = mixer.session.scenes.first(where: { $0.id == id }) {
            return "PREVIEW — \(scene.name)"
        }
        return "PREVIEW"
    }

    private var programTitle: String {
        if let id = mixer.programmingSceneId(for: mixer.selectedUnitId),
           let scene = mixer.session.scenes.first(where: { $0.id == id }) {
            return "PROGRAM — \(scene.name)"
        }
        return "PROGRAM"
    }

    private func dipColorBinding(index: Int, fallback: TransitionPreset) -> Binding<Color> {
        Binding(
            get: {
                let item = mixer.selectedUnit.transitions[safe: index] ?? fallback
                return Color(red: Double(item.dipR), green: Double(item.dipG), blue: Double(item.dipB))
            },
            set: { color in
                #if canImport(AppKit)
                let ns = NSColor(color)
                guard let rgb = ns.usingColorSpace(.sRGB) else { return }
                updateTransition(index) {
                    $0.dipR = Float(rgb.redComponent)
                    $0.dipG = Float(rgb.greenComponent)
                    $0.dipB = Float(rgb.blueComponent)
                    $0.dipA = 1
                }
                #endif
            }
        )
    }

    private func updateTransition(_ index: Int, _ body: (inout TransitionPreset) -> Void) {
        var unit = mixer.selectedUnit
        guard index < unit.transitions.count else { return }
        body(&unit.transitions[index])
        mixer.saveUnit(unit)
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
                                updateTransition(index) {
                                    $0.kind = value
                                    if value == EIVIZ_TRANSITION_CUSTOM && ($0.customWgsl ?? "").isEmpty {
                                        $0.customWgsl = CustomWgslEditor.template
                                    }
                                }
                                if value == EIVIZ_TRANSITION_CUSTOM {
                                    let wgsl = mixer.selectedUnit.transitions[safe: index]?.customWgsl
                                        ?? CustomWgslEditor.template
                                    wgsl.withCString { _ = mixer_unit_set_custom_wgsl(mixer.selectedUnitId, $0) }
                                }
                            }
                    )) {
                        Text("Cut").tag(EIVIZ_TRANSITION_CUT)
                        Text("Fade").tag(EIVIZ_TRANSITION_FADE)
                        Text("Dip").tag(EIVIZ_TRANSITION_DIP)
                        Text("Wipe").tag(EIVIZ_TRANSITION_WIPE)
                        Text("Slide").tag(EIVIZ_TRANSITION_SLIDE)
                        Text("Push").tag(EIVIZ_TRANSITION_PUSH)
                        Text("Iris").tag(EIVIZ_TRANSITION_IRIS)
                        Text("Blinds").tag(EIVIZ_TRANSITION_BLINDS)
                        Text("Zoom").tag(EIVIZ_TRANSITION_ZOOM)
                        Text("Additive").tag(EIVIZ_TRANSITION_ADDITIVE)
                        Text("Custom WGSL").tag(EIVIZ_TRANSITION_CUSTOM)
                    }
                    if (mixer.selectedUnit.transitions[safe: index] ?? preset).hasDuration {
                        Text("Duration").font(.system(size: 11)).foregroundStyle(EivizTheme.dim)
                        mixerUintField(Binding(
                            get: { mixer.selectedUnit.transitions[safe: index]?.durationValue ?? preset.durationValue },
                            set: { value in updateTransition(index) { $0.durationValue = value } }
                        ))
                        Picker("", selection: Binding(
                            get: { mixer.selectedUnit.transitions[safe: index]?.durationUnit ?? preset.durationUnit },
                            set: { value in updateTransition(index) { $0.durationUnit = value } }
                        )) {
                            Text("Frames").tag(EIVIZ_DURATION_FRAMES)
                            Text("Milliseconds").tag(EIVIZ_DURATION_MS)
                        }
                    }
                    if (mixer.selectedUnit.transitions[safe: index] ?? preset).hasEasing {
                        Text("Easing").font(.system(size: 11)).foregroundStyle(EivizTheme.dim)
                        Picker("", selection: Binding(
                            get: { mixer.selectedUnit.transitions[safe: index]?.easing ?? preset.easing },
                            set: { value in updateTransition(index) { $0.easing = value } }
                        )) {
                            Text("Linear").tag(EIVIZ_EASING_LINEAR)
                            Text("EaseIn").tag(EIVIZ_EASING_IN)
                            Text("EaseOut").tag(EIVIZ_EASING_OUT)
                            Text("EaseInOut").tag(EIVIZ_EASING_IN_OUT)
                            Text("Smoothstep").tag(EIVIZ_EASING_SMOOTHSTEP)
                        }
                    }
                    if (mixer.selectedUnit.transitions[safe: index] ?? preset).hasDirection {
                        Text("Direction").font(.system(size: 11)).foregroundStyle(EivizTheme.dim)
                        Picker("", selection: Binding(
                            get: { mixer.selectedUnit.transitions[safe: index]?.direction ?? preset.direction },
                            set: { value in updateTransition(index) { $0.direction = value } }
                        )) {
                            Text("Left").tag(EIVIZ_DIR_LEFT)
                            Text("Right").tag(EIVIZ_DIR_RIGHT)
                            Text("Up").tag(EIVIZ_DIR_UP)
                            Text("Down").tag(EIVIZ_DIR_DOWN)
                        }
                    }
                    Toggle("Swap", isOn: Binding(
                        get: { mixer.selectedUnit.transitions[safe: index]?.swap ?? true },
                        set: { value in updateTransition(index) { $0.swap = value } }
                    ))
                    Toggle("Keep Preview Scene", isOn: Binding(
                        get: { mixer.selectedUnit.transitions[safe: index]?.keepPreview ?? true },
                        set: { value in updateTransition(index) { $0.keepPreview = value } }
                    ))
                    if (mixer.selectedUnit.transitions[safe: index] ?? preset).hasDipColor {
                        Text((mixer.selectedUnit.transitions[safe: index] ?? preset).kind == EIVIZ_TRANSITION_PUSH ? "Fill color" : "Dip color")
                            .font(.system(size: 11)).foregroundStyle(EivizTheme.dim)
                        ColorPicker("", selection: dipColorBinding(index: index, fallback: preset), supportsOpacity: false)
                            .labelsHidden()
                    }
                    if (mixer.selectedUnit.transitions[safe: index] ?? preset).hasCustomWgsl {
                        Button((mixer.selectedUnit.transitions[safe: index]?.customWgsl ?? "").isEmpty ? "Edit WGSL…" : "Edit WGSL (set)") {
                            customWgslEdit = CustomWgslEdit(
                                index: index,
                                text: mixer.selectedUnit.transitions[safe: index]?.customWgsl ?? preset.customWgsl ?? ""
                            )
                        }
                    }
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
                Text("\(preset.label)  \(preset.durationLabel)")
                    .font(.system(size: 12, weight: .semibold))
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .padding(4)
            .overlay(
                Rectangle().stroke(selected ? mixer.session.settings.previewColor.color : EivizTheme.stroke, lineWidth: 1)
            )
            Button("TAKE") { mixer.firePreset(preset, index: index) }
                .buttonStyle(MixerButtonStyle())
        }
        .onTapGesture { mixer.tbarPresetIndex = index }
        .onAppear {
            if preset.kind == EIVIZ_TRANSITION_STINGER {
                updateTransition(index) { $0.kind = EIVIZ_TRANSITION_FADE }
            }
        }
    }

    private var lower: some View {
        VStack(spacing: 8) {
            if mixer.selectedVideoId != nil {
                videoBar
            }
            HSplitView {
                VStack(alignment: .leading) {
                    Text("Inputs").fontWeight(.bold)
                    List(mixer.session.inputs, selection: $mixer.selectedInputId) { input in
                        Text(input.name)
                            .tag(input.id)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .contentShape(Rectangle())
                            .onTapGesture { mixer.selectedInputId = input.id }
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
                        Button("Preview") { mixer.previewSelectedInput() }
                        Button("Delete") { mixer.deleteSelectedInput() }
                    }
                    .buttonStyle(MixerButtonStyle())
                }
                .frame(minWidth: 160)
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
                        WrapFlowLayout(spacing: 8) {
                            ForEach(mixer.session.scenes) { scene in
                                sceneTile(scene)
                            }
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    .scrollClipDisabled()
                }
            }
            .frame(maxHeight: .infinity)
        }
    }

    private func sceneTile(_ scene: SceneEntry) -> some View {
        let preview = mixer.previewingSceneId(for: mixer.selectedUnitId) == scene.id
        let program = mixer.programmingSceneId(for: mixer.selectedUnitId) == scene.id
        let number = (mixer.session.scenes.firstIndex(where: { $0.id == scene.id }) ?? 0) + 1
        let video = mixer.sceneVideo(scene)
        let loopOn = video?.videoLoop == true
        let playing = mixer.scenePlaying(scene)
        let muted = mixer.sceneInputs(scene).allSatisfy(\.mute)
        return VStack(spacing: 0) {
            HStack(spacing: 6) {
                Text("\(number)")
                    .font(.system(size: 11, weight: .bold))
                Text(scene.name)
                    .font(.system(size: 12))
                    .lineLimit(1)
                    .frame(maxWidth: .infinity, alignment: .leading)
                Button("X") { mixer.deleteScene(scene) }
                    .buttonStyle(MixerTileButtonStyle())
            }
            .padding(.horizontal, 6)
            .padding(.vertical, 3)
            .background(Color(white: 0.2))
            .contentShape(Rectangle())
            .onTapGesture { mixer.previewScene(scene) }
            MetalPreviewRepresentable(
                role: .monitor(monitorId: scene.monitorId, sourceId: scene.gpuId),
                presentInterval: 1,
                onClick: { mixer.previewScene(scene) }
            )
            .frame(width: 176, height: 90)
            HStack(spacing: 1) {
                sceneChip("CUT") { mixer.cutScene(scene) }
                sceneChip("Loop") { mixer.toggleSceneLoop(scene) }
                    .opacity(video == nil ? 0.35 : (loopOn ? 1 : 0.55))
                    .disabled(video == nil)
                sceneChip(playing ? "❚❚" : "▶") { mixer.toggleScenePlay(scene) }
                    .disabled(video == nil)
                sceneChip("Aud") { mixer.toggleSceneAudio(scene) }
                    .opacity(muted ? 0.45 : 1)
                sceneChip("Prev") { mixer.openInputPreview(inputId: scene.gpuId, name: scene.name) }
                sceneChip("Set") {
                    mixer.editingScene = scene
                    mixer.showSceneEditor = true
                }
            }
            .padding(2)
        }
        .frame(width: 176)
        .id("scene-\(scene.id)-\(scene.monitorId)")
        .background(Rectangle().stroke(
            program ? mixer.session.settings.programColor.color
                : preview ? mixer.session.settings.previewColor.color
                : mixer.session.settings.inactiveColor.color,
            lineWidth: 2
        ))
        .contextMenu {
            Button("Edit") {
                mixer.editingScene = scene
                mixer.showSceneEditor = true
            }
        }
    }

    private func sceneChip(_ title: String, action: @escaping () -> Void) -> some View {
        Button(title, action: action)
            .buttonStyle(MixerTileButtonStyle())
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
        HStack(alignment: .bottom, spacing: 12) {
            Text("Audio").fontWeight(.bold)
            HSplitView {
                HStack(alignment: .bottom, spacing: 12) {
                    ForEach(mixer.session.buses) { bus in
                        meter(title: bus.name, id: bus.role == .master ? 0 : EIVIZ_AUDIO_BUS_PEAK_BASE | bus.id)
                    }
                    Spacer(minLength: 0)
                }
                HStack(alignment: .bottom, spacing: 12) {
                    Spacer(minLength: 0)
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
                }
                .frame(minWidth: 80)
            }
            Menu {
                ForEach(mixer.session.multiviews) { layout in
                    Button(layout.name) { mixer.openMultiviewWindow(layout) }
                }
                Divider()
                Button(L10n.t("chrome.newMultiview")) { mixer.openNewMultiview() }
            } label: {
                Text(L10n.t("chrome.multiview"))
            }
            .buttonStyle(MixerButtonStyle())
            Button(L10n.t("chrome.overlay")) { mixer.showOverlay = true }
                .buttonStyle(MixerButtonStyle())
        }
        .padding(8)
        .background(EivizTheme.panel)
    }

    private func overlayName(_ slot: OverlaySlot) -> String {
        if slot.sourceKind == .input {
            return mixer.session.inputs.first { $0.id == slot.sceneGpuId }?.name ?? "Input"
        }
        return mixer.session.scenes.first { $0.gpuId == slot.sceneGpuId }?.name ?? "Scene"
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

    private func bus(title: String, color: RgbColor, kind: UInt32) -> some View {
        VStack(spacing: 0) {
            Text(title)
                .font(.system(size: 13, weight: .bold))
                .foregroundStyle(color.headerForeground)
                .frame(maxWidth: .infinity)
                .padding(.vertical, 6)
                .background(color.color)
            MetalPreviewRepresentable(role: .unit(unitId: mixer.selectedUnitId, kind: kind))
                .frame(minWidth: 320, minHeight: 180)
        }
        .aspectRatio(
            CGFloat(mixer.selectedUnit.width) / max(1, CGFloat(mixer.selectedUnit.height)),
            contentMode: .fit
        )
        .background(Rectangle().stroke(color.color, lineWidth: 2))
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

private extension Array {
    subscript(safe index: Int) -> Element? {
        indices.contains(index) ? self[index] : nil
    }
}
