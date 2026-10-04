import EivizMixer
import SwiftUI

struct SceneAnimView: View {
    @EnvironmentObject private var mixer: MixerController
    @Environment(\.dismiss) private var dismiss
    let sceneId: UInt64
    let persist: Bool
    @State private var selectedState: UInt64 = 0
    @State private var selectedSequence: UInt64 = 0
    @State private var live = SceneAnimLive.idle
    @State private var status = ""

    private let tally = Timer.publish(every: 0.2, on: .main, in: .common).autoconnect()
    private static let easings: [(String, UInt32)] = [
        ("Linear", EIVIZ_EASING_LINEAR),
        ("EaseIn", EIVIZ_EASING_IN),
        ("EaseOut", EIVIZ_EASING_OUT),
        ("EaseInOut", EIVIZ_EASING_IN_OUT),
        ("Smoothstep", EIVIZ_EASING_SMOOTHSTEP),
        ("Bezier", EIVIZ_EASING_BEZIER),
        ("Hold", EIVIZ_EASING_HOLD)
    ]
    private static let card = Color(red: 0x24 / 255, green: 0x24 / 255, blue: 0x24 / 255)
    private static let liveFill = Color(red: 0x1E / 255, green: 0x6B / 255, blue: 0x3A / 255)
    private static let moveFill = Color(red: 0x2E / 255, green: 0x5E / 255, blue: 0x8E / 255)
    private static let waitFill = Color(red: 0x3A / 255, green: 0x3A / 255, blue: 0x3A / 255)

    private var sceneIndex: Int? {
        mixer.session.scenes.firstIndex { $0.id == sceneId }
    }

    private var scene: SceneEntry? {
        sceneIndex.map { mixer.session.scenes[$0] }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .top, spacing: 16) {
                stateColumn
                    .frame(maxWidth: .infinity)
                    .layoutPriority(2)
                Divider()
                sequenceColumn
                    .frame(maxWidth: .infinity)
                    .layoutPriority(3)
            }
            HStack {
                Text(status).font(.system(size: 11)).foregroundStyle(.yellow)
                Spacer()
                Button("Close") { dismiss() }
                    .keyboardShortcut(.cancelAction)
            }
        }
        .padding(12)
        .frame(minWidth: 1040, minHeight: 720)
        .background(EivizTheme.dialog)
        .foregroundStyle(EivizTheme.text)
        .buttonStyle(MixerButtonStyle())
        .onAppear {
            selectedState = scene?.states.first?.id ?? 0
            selectedSequence = scene?.sequences.first?.id ?? 0
        }
        .onReceive(tally) { _ in
            if let scene {
                live = mixer.sceneAnimLive(scene)
            }
        }
        .onDisappear {
            if persist, mixer.isRemote, let scene {
                _ = mixer.commitRemoteScene(scene)
            }
        }
    }

    // MARK: States

    private var stateColumn: some View {
        VStack(alignment: .leading, spacing: 6) {
            heading(L10n.t("anim.states"), L10n.t("anim.statesHelp"))
            listBox {
                ForEach(scene?.states ?? []) { state in
                    row(
                        title: displayName(state.name, state.id),
                        detail: motionSummary(state.enter),
                        selected: selectedState == state.id,
                        lit: live.litState == state.id,
                        action: L10n.t("anim.goTo"),
                        onSelect: { selectedState = state.id },
                        onAction: { go(state.id) }
                    )
                }
            }
            HStack {
                Button(L10n.t("anim.newState")) { addState() }
                Button(L10n.t("anim.updateState")) { capture() }
                Button(L10n.t("anim.delete")) { deleteState() }
                Button(L10n.t("anim.saved")) { go(0) }
                    .help(L10n.t("anim.savedHelp"))
            }
            ScrollView {
                if let state = scene?.states.first(where: { $0.id == selectedState }) {
                    stateForm(state)
                }
            }
        }
    }

    @ViewBuilder
    private func stateForm(_ state: SceneState) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            label(L10n.t("anim.name"))
            TextField("", text: Binding(
                get: { state.name },
                set: { value in
                    update { scene in
                        guard let index = scene.states.firstIndex(where: { $0.id == state.id }) else { return }
                        scene.states[index].name = value
                    }
                }
            ))
            motionEditor(state.enter) { motion in
                update { scene in
                    guard let index = scene.states.firstIndex(where: { $0.id == state.id }) else { return }
                    scene.states[index].enter = motion
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    // MARK: Sequences

    private var sequenceColumn: some View {
        VStack(alignment: .leading, spacing: 6) {
            heading(L10n.t("anim.sequences"), L10n.t("anim.sequencesHelp"))
            listBox {
                ForEach(scene?.sequences ?? []) { sequence in
                    row(
                        title: displayName(sequence.name, sequence.id),
                        detail: L10n.format("anim.stepCount", "\(sequence.steps.count)", seconds(totalFrames(sequence))),
                        selected: selectedSequence == sequence.id,
                        lit: live.sequenceId == sequence.id,
                        action: "▶",
                        onSelect: { selectedSequence = sequence.id },
                        onAction: { run(sequence, EIVIZ_SCENE_SEQ_PLAY) }
                    )
                }
            }
            HStack {
                Button(L10n.t("anim.newSequence")) { addSequence() }
                Button(L10n.t("anim.delete")) { deleteSequence() }
                Spacer().frame(width: 12)
                Button(L10n.t("anim.play")) { run(selectedSequenceEntry, EIVIZ_SCENE_SEQ_PLAY) }
                Button(L10n.t("anim.reverse")) { run(selectedSequenceEntry, EIVIZ_SCENE_SEQ_REVERSE) }
                Button(L10n.t("anim.stop")) { run(selectedSequenceEntry, EIVIZ_SCENE_SEQ_STOP) }
            }
            ScrollView {
                if let sequence = selectedSequenceEntry {
                    sequenceForm(sequence)
                }
            }
        }
    }

    private var selectedSequenceEntry: SceneSequence? {
        scene?.sequences.first { $0.id == selectedSequence }
    }

    @ViewBuilder
    private func sequenceForm(_ sequence: SceneSequence) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            label(L10n.t("anim.name"))
            TextField("", text: Binding(
                get: { sequence.name },
                set: { value in
                    updateSequence(sequence.id) { $0.name = value }
                }
            ))
            label(L10n.t("anim.steps"))
            timeline(sequence)
            let total = totalFrames(sequence)
            Text(L10n.format("anim.total", "\(total)", seconds(total)))
                .font(.system(size: 11))
                .foregroundStyle(EivizTheme.dim)
            ForEach(Array(sequence.steps.enumerated()), id: \.offset) { offset, step in
                stepCard(sequence, offset, step)
            }
            Button(L10n.t("anim.addStep")) { addStep(sequence) }
                .padding(.top, 4)
            if sequence.steps.count < 2 {
                note(L10n.t("anim.needTwoSteps"))
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func timeline(_ sequence: SceneSequence) -> some View {
        let segments = timelineSegments(sequence)
        let total = max(1, segments.reduce(0) { $0 + Double($1.frames) })
        let playing = live.sequenceId == sequence.id
        return GeometryReader { geo in
            HStack(spacing: 1) {
                ForEach(Array(segments.enumerated()), id: \.offset) { _, segment in
                    let on = playing && live.stepIndex == segment.step && live.holding == segment.wait
                    Text(segment.label)
                        .font(.system(size: 11))
                        .lineLimit(1)
                        .padding(.horizontal, 4)
                        .frame(
                            width: max(1, geo.size.width * Double(segment.frames) / total - 1),
                            height: 28,
                            alignment: .leading
                        )
                        .background(on ? Self.liveFill : segment.wait ? Self.waitFill : Self.moveFill)
                        .help("\(segment.label) — \(segment.frames) \(L10n.t("anim.frames")) (\(seconds(segment.frames)))")
                }
            }
        }
        .frame(height: 28)
        .padding(.vertical, 4)
    }

    private struct Segment {
        let frames: UInt32
        let label: String
        let step: Int
        let wait: Bool
    }

    private func timelineSegments(_ sequence: SceneSequence) -> [Segment] {
        var segments: [Segment] = []
        for (index, step) in sequence.steps.enumerated() {
            let name = scene?.states.first { $0.id == step.stateId }.map { displayName($0.name, $0.id) } ?? "?"
            segments.append(Segment(frames: moveFrames(step), label: "\(index + 1). \(name)", step: index, wait: false))
            if step.holdFrames > 0 {
                segments.append(Segment(frames: step.holdFrames, label: L10n.t("anim.wait"), step: index, wait: true))
            }
        }
        return segments
    }

    private func stepCard(_ sequence: SceneSequence, _ index: Int, _ step: SequenceStep) -> some View {
        let target = scene?.states.first { $0.id == step.stateId }
        return VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 6) {
                Text("\(index + 1)")
                    .font(.system(size: 11, weight: .bold))
                    .frame(width: 20, height: 20)
                    .background(Circle().fill(Self.moveFill))
                Text(L10n.t("anim.moveTo")).foregroundStyle(EivizTheme.dim)
                Picker("", selection: Binding(
                    get: { step.stateId },
                    set: { value in updateStep(sequence.id, index) { $0.stateId = value } }
                )) {
                    ForEach(scene?.states ?? []) { state in
                        Text(displayName(state.name, state.id)).tag(state.id)
                    }
                }
                .labelsHidden()
                .frame(minWidth: 160)
                Spacer()
                Button("↑") { moveStep(sequence.id, index, -1) }
                    .disabled(index == 0)
                    .help(L10n.t("anim.moveUp"))
                Button("↓") { moveStep(sequence.id, index, 1) }
                    .disabled(index >= sequence.steps.count - 1)
                    .help(L10n.t("anim.moveDown"))
                Button("×") {
                    updateSequence(sequence.id) { $0.steps.remove(at: index) }
                }
                .help(L10n.t("anim.removeStep"))
            }
            VStack(alignment: .leading, spacing: 6) {
                Toggle(
                    L10n.format("anim.useStateTiming", target.map { motionSummary($0.enter) } ?? "-"),
                    isOn: Binding(
                        get: { step.motion == nil },
                        set: { inherit in
                            updateStep(sequence.id, index) {
                                $0.motion = inherit ? nil : (target?.enter ?? Motion())
                            }
                        }
                    )
                )
                if let motion = step.motion {
                    motionEditor(motion) { next in
                        updateStep(sequence.id, index) { $0.motion = next }
                    }
                }
                HStack(spacing: 6) {
                    Text(L10n.t("anim.thenWait"))
                    TextField("", value: Binding(
                        get: { step.holdFrames },
                        set: { value in updateStep(sequence.id, index) { $0.holdFrames = value } }
                    ), format: .number)
                    .frame(width: 64)
                    Text(L10n.t("anim.frames"))
                    Text(seconds(step.holdFrames)).foregroundStyle(EivizTheme.dim)
                }
            }
            .padding(.leading, 26)
        }
        .padding(8)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 4).fill(Self.card))
        .padding(.top, 2)
    }

    // MARK: Motion editor

    private func motionEditor(_ motion: Motion, _ write: @escaping (Motion) -> Void) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            label(L10n.t("anim.moveTime"))
            HStack(spacing: 6) {
                TextField("", value: Binding(
                    get: { max(1, motion.durationFrames) },
                    set: { value in
                        var next = motion
                        next.durationFrames = max(1, value)
                        write(next)
                    }
                ), format: .number)
                .frame(width: 64)
                Text(L10n.t("anim.frames"))
                Text(seconds(max(1, motion.durationFrames))).foregroundStyle(EivizTheme.dim)
            }
            label(L10n.t("anim.curve"))
            Picker("", selection: Binding(
                get: { motion.easing },
                set: { value in
                    var next = motion
                    next.easing = value
                    if value == EIVIZ_EASING_BEZIER {
                        if next.bezier == nil {
                            next.bezier = BezierHandles(x1: 0.42, y1: 0, x2: 0.58, y2: 1)
                        }
                    } else {
                        next.bezier = nil
                    }
                    write(next)
                }
            )) {
                ForEach(Self.easings, id: \.1) { item in
                    Text(item.0).tag(item.1)
                }
            }
            .labelsHidden()
            .frame(width: 160)
            if motion.easing == EIVIZ_EASING_BEZIER {
                HStack {
                    Button("Ease") { write(preset(motion, 0.25, 0.1, 0.25, 1)) }
                    Button("In") { write(preset(motion, 0.42, 0, 1, 1)) }
                    Button("Out") { write(preset(motion, 0, 0, 0.58, 1)) }
                    Button("In Out") { write(preset(motion, 0.42, 0, 0.58, 1)) }
                }
                HStack(spacing: 10) {
                    handleField("X1", motion, \.x1, true, write)
                    handleField("Y1", motion, \.y1, false, write)
                    handleField("X2", motion, \.x2, true, write)
                    handleField("Y2", motion, \.y2, false, write)
                }
            }
        }
    }

    private func handleField(
        _ title: String,
        _ motion: Motion,
        _ key: WritableKeyPath<BezierHandles, Float>,
        _ clampX: Bool,
        _ write: @escaping (Motion) -> Void
    ) -> some View {
        HStack(spacing: 2) {
            Text(title).frame(width: 22, alignment: .leading)
            TextField("", value: Binding(
                get: { Double(motion.bezier?[keyPath: key] ?? 0) },
                set: { raw in
                    var next = motion
                    if next.bezier == nil {
                        next.bezier = BezierHandles(x1: 0.42, y1: 0, x2: 0.58, y2: 1)
                    }
                    let value = clampX ? min(1, max(0, raw)) : raw
                    next.bezier?[keyPath: key] = Float(value)
                    write(next)
                }
            ), format: .number)
            .frame(width: 56)
        }
    }

    private func preset(_ motion: Motion, _ x1: Float, _ y1: Float, _ x2: Float, _ y2: Float) -> Motion {
        var next = motion
        next.easing = EIVIZ_EASING_BEZIER
        next.bezier = BezierHandles(x1: x1, y1: y1, x2: x2, y2: y2)
        return next
    }

    // MARK: Building blocks

    private func heading(_ title: String, _ help: String) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(title).font(.system(size: 14, weight: .bold))
            note(help)
        }
    }

    private func listBox<Content: View>(@ViewBuilder _ content: () -> Content) -> some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                content()
            }
        }
        .frame(height: 150)
        .background(EivizTheme.list)
    }

    private func row(
        title: String,
        detail: String,
        selected: Bool,
        lit: Bool,
        action: String,
        onSelect: @escaping () -> Void,
        onAction: @escaping () -> Void
    ) -> some View {
        HStack {
            VStack(alignment: .leading, spacing: 1) {
                Text(title).fontWeight(.semibold).lineLimit(1)
                Text(detail).font(.system(size: 11)).foregroundStyle(EivizTheme.dim)
            }
            Spacer()
            Button(action, action: onAction)
        }
        .padding(.horizontal, 6)
        .padding(.vertical, 3)
        .background(lit ? Self.liveFill : selected ? Color.accentColor.opacity(0.35) : Color.clear)
        .contentShape(Rectangle())
        .onTapGesture(perform: onSelect)
    }

    private func label(_ text: String) -> some View {
        Text(text)
            .font(.system(size: 11))
            .foregroundStyle(EivizTheme.dim)
            .padding(.top, 8)
    }

    private func note(_ text: String) -> some View {
        Text(text)
            .font(.system(size: 11))
            .foregroundStyle(EivizTheme.dim)
            .fixedSize(horizontal: false, vertical: true)
    }

    // MARK: Edits

    private func update(_ body: (inout SceneEntry) -> Void) {
        guard let index = sceneIndex else { return }
        body(&mixer.session.scenes[index])
        mixer.session.scenes[index].assignLayerIds()
        let scene = mixer.session.scenes[index]
        guard !mixer.isRemote else { return }
        mixer.pushSceneAnim(scene)
        if persist {
            mixer.publishSession()
        }
        status = ""
    }

    private func updateSequence(_ id: UInt64, _ body: (inout SceneSequence) -> Void) {
        update { scene in
            guard let index = scene.sequences.firstIndex(where: { $0.id == id }) else { return }
            body(&scene.sequences[index])
        }
    }

    private func updateStep(_ sequenceId: UInt64, _ index: Int, _ body: (inout SequenceStep) -> Void) {
        updateSequence(sequenceId) { sequence in
            guard sequence.steps.indices.contains(index) else { return }
            body(&sequence.steps[index])
        }
    }

    private func currentLayout(_ scene: inout SceneEntry) -> [LayerKey] {
        scene.assignLayerIds()
        return scene.layers.filter { $0.layerId != 0 }.map {
            LayerKey(layerId: $0.layerId, geom: SceneLayerGeom.from($0))
        }
    }

    private func addState() {
        update { scene in
            let id = (scene.states.map(\.id).max() ?? 0) + 1
            let layers = currentLayout(&scene)
            scene.states.append(SceneState(id: id, name: "State \(scene.states.count + 1)", layers: layers))
            selectedState = id
        }
    }

    private func capture() {
        guard scene?.states.contains(where: { $0.id == selectedState }) == true else { return }
        update { scene in
            guard let index = scene.states.firstIndex(where: { $0.id == selectedState }) else { return }
            let layers = currentLayout(&scene)
            scene.states[index].layers = layers
        }
        status = L10n.t("anim.captured")
    }

    private func deleteState() {
        update { scene in
            guard let index = scene.states.firstIndex(where: { $0.id == selectedState }) else { return }
            let id = scene.states[index].id
            scene.states.remove(at: index)
            for sequence in scene.sequences.indices {
                scene.sequences[sequence].steps.removeAll { $0.stateId == id }
            }
            selectedState = scene.states.first?.id ?? 0
        }
    }

    private func addSequence() {
        guard (scene?.states.count ?? 0) >= 2 else {
            status = L10n.t("anim.needTwoStates")
            return
        }
        update { scene in
            let id = (scene.sequences.map(\.id).max() ?? 0) + 1
            scene.sequences.append(SceneSequence(
                id: id,
                name: "Sequence \(scene.sequences.count + 1)",
                steps: [
                    SequenceStep(stateId: scene.states[0].id),
                    SequenceStep(stateId: scene.states[1].id)
                ]
            ))
            selectedSequence = id
        }
    }

    private func deleteSequence() {
        update { scene in
            scene.sequences.removeAll { $0.id == selectedSequence }
            selectedSequence = scene.sequences.first?.id ?? 0
        }
    }

    private func addStep(_ sequence: SceneSequence) {
        guard let states = scene?.states, let first = states.first else {
            status = L10n.t("anim.needTwoStates")
            return
        }
        let last = sequence.steps.last?.stateId ?? 0
        let pick = states.first { $0.id != last } ?? first
        updateSequence(sequence.id) { $0.steps.append(SequenceStep(stateId: pick.id)) }
    }

    private func moveStep(_ sequenceId: UInt64, _ index: Int, _ delta: Int) {
        updateSequence(sequenceId) { sequence in
            let next = index + delta
            guard sequence.steps.indices.contains(index), sequence.steps.indices.contains(next) else { return }
            sequence.steps.swapAt(index, next)
        }
    }

    // MARK: Playback

    private func go(_ stateId: UInt64) {
        guard let scene else { return }
        if mixer.isRemote, !mixer.commitRemoteScene(scene) {
            return
        }
        mixer.sceneGo(to: stateId, scene: scene)
    }

    private func run(_ sequence: SceneSequence?, _ op: UInt32) {
        guard let scene, let sequence else { return }
        guard sequence.steps.count >= 2 else {
            status = L10n.t("anim.needTwoSteps")
            return
        }
        if mixer.isRemote, !mixer.commitRemoteScene(scene) {
            return
        }
        mixer.sceneSequence(sequence.id, op: op, scene: scene)
    }

    // MARK: Labels

    private func moveFrames(_ step: SequenceStep) -> UInt32 {
        if let own = step.motion {
            return max(1, own.durationFrames)
        }
        return max(1, scene?.states.first { $0.id == step.stateId }?.enter.durationFrames ?? 1)
    }

    private func totalFrames(_ sequence: SceneSequence) -> UInt32 {
        sequence.steps.reduce(0) { $0 + moveFrames($1) + $1.holdFrames }
    }

    private func seconds(_ frames: UInt32) -> String {
        let settings = mixer.session.settings
        let fps = Double(settings.masterFpsNum) / Double(max(1, settings.masterFpsDen))
        return L10n.format("anim.seconds", String(format: "%.2f", Double(frames) / max(1, fps)))
    }

    private func motionSummary(_ motion: Motion) -> String {
        let curve = Self.easings.first { $0.1 == motion.easing }?.0 ?? "?"
        let frames = max(1, motion.durationFrames)
        return "\(frames) f · \(curve) · \(seconds(frames))"
    }

    private func displayName(_ name: String, _ id: UInt64) -> String {
        name.trimmingCharacters(in: .whitespaces).isEmpty ? "\(id)" : name
    }
}
