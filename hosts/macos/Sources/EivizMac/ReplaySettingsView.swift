import AppKit
import EivizMixer
import SwiftUI

/// Project-scoped replay configuration. It only exists while the project has a
/// Replay input, so it is a dedicated window rather than a settings tab.
struct ReplaySettingsView: View {
    @EnvironmentObject private var mixer: MixerController

    @State private var document: ReplayDocument?
    @State private var storages: [ReplayDocument.StorageSpec] = []
    @State private var profile = "Sq"
    /// Segment length in frames, as text so the operator can type it.
    @State private var segmentFrames = "120"
    @State private var warnSecs = "600"
    @State private var alertSecs = "300"
    @State private var criticalSecs = "60"
    @State private var retention = "StopWhenFull"
    @State private var queue = "6"
    @State private var buffer = "512"
    @State private var reserve = "2048"
    @State private var overflow = "DropFrames"
    @State private var autoAdd = false
    @State private var decodeAhead = true
    @State private var profiling = false
    @State private var angles: [ReplayAngleInfo] = []
    @State private var status = ""

    var body: some View {
        // The settings are taller than the smallest window the view allows, so they scroll
        // instead of being clipped at the bottom of a compressed layout. Apply stays pinned
        // below the scroll, so it is reachable without scrolling to the end.
        VStack(spacing: 0) {
            ScrollView {
                VStack(alignment: .leading, spacing: 10) {
                    Text(planText).font(.callout)
                    if !status.isEmpty {
                        // Apply and every refusal report here, where it cannot be missed.
                        Text(status).fontWeight(.bold).foregroundStyle(EivizTheme.dim)
                    }
                    VStack(alignment: .leading, spacing: 6) {
                        Text(L10n.t("replay.storage"))
                            .font(.system(size: 14, weight: .bold))
                        VStack(alignment: .leading, spacing: 6) {
                            Text(L10n.t("replay.storageHelp"))
                                .font(.caption)
                                .foregroundStyle(EivizTheme.dim)
                            // Each drive is edited in its own row: the path is the row's text field,
                            // and the angles that record there are shown in the same row.
                            // The drive table is the Windows one: path, free space, the angles
                            // that record there, and the row's browse and remove buttons.
                            Table($storages) {
                                TableColumn(L10n.t("settings.replayPath")) { $storage in
                                    TextField("", text: $storage.path)
                                        .help(L10n.t("replay.pickStorageHint"))
                                }
                                .width(min: 240, ideal: 520)
                                TableColumn(L10n.t("replay.freeSpace")) { storage in
                                    Text(freeSpaceText(storage.wrappedValue.path, angles: storage.wrappedValue.angleIds))
                                        .foregroundStyle(EivizTheme.dim)
                                }
                                .width(180)
                                TableColumn(L10n.t("replay.angle")) { storage in
                                    Text(storage.wrappedValue.angleIds.map(String.init).joined(separator: ", "))
                                }
                                .width(200)
                                TableColumn("") { storage in
                                    Button(L10n.t("replay.browse")) { browseInto(storage.wrappedValue.id) }
                                }
                                .width(100)
                                TableColumn("") { storage in
                                    Button(L10n.t("replay.removeStorage")) { removeStorageRow(storage.wrappedValue.id) }
                                }
                                .width(80)
                            }
                            .frame(minHeight: 130)
                            Button(L10n.t("replay.addStorageRow")) { browseInto(nil) }
                            HStack {
                                Picker(L10n.t("settings.replayProfile"), selection: $profile) {
                                    Text("HQ").tag("Hq")
                                    Text("SQ").tag("Sq")
                                    Text("LQ").tag("Lq")
                                }
                                Picker(L10n.t("replay.retention"), selection: $retention) {
                                    Text(L10n.t("replay.retentionStop")).tag("StopWhenFull")
                                    Text(L10n.t("replay.retentionOverwrite")).tag("Overwrite")
                                    Text(L10n.t("replay.retentionProtect")).tag("OverwriteUnprotected")
                                }
                                .frame(minWidth: 280, alignment: .leading)
                            }
                            HStack(alignment: .bottom, spacing: 10) {
                                // A filled text field hides its placeholder, so each number keeps its
                                // label above it instead of leaving three bare values.
                                numberField("settings.replayQueue", $queue, width: 90)
                                numberField("settings.replayBuffer", $buffer, width: 90)
                                numberField("replay.reserve", $reserve, width: 100)
                                Spacer(minLength: 4)
                                Picker(L10n.t("replay.overflow"), selection: $overflow) {
                                    Text(L10n.t("replay.overflowDrop")).tag("DropFrames")
                                    Text(L10n.t("replay.overflowStop")).tag("StopRecording")
                                }
                                .frame(minWidth: 230, alignment: .leading)
                            }
                            HStack(spacing: 12) {
                                Button(L10n.t("replay.capacityRun")) {}
                                    .disabled(true)
                                    .frame(minWidth: 150, minHeight: 28)
                                    .help(L10n.t("replay.capacityIdle"))
                                Text("—")
                                    .font(.system(size: 11))
                                    .foregroundStyle(EivizTheme.dim)
                            }
                            HStack(alignment: .top) {
                                // One format for every angle, taken from the first Mixing Unit.
                                // It is shown rather than edited here.
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(L10n.t("replay.formatFromUnit")).font(.caption)
                                    Text(formatText).font(.system(.body, design: .monospaced))
                                }
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(L10n.t("replay.segmentFrames")).font(.caption)
                                    TextField("120", text: $segmentFrames)
                                        .frame(width: 90)
                                        .help(L10n.t("replay.segmentFramesRange"))
                                }
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(L10n.t("replay.deckRefresh")).font(.caption)
                                    Picker("", selection: $mixer.session.settings.replayPresentInterval) {
                                        Text(L10n.t("replay.deckRefreshFollow")).tag(UInt32(0))
                                        ForEach([UInt32(1), 2, 3, 4, 6, 8], id: \.self) { frames in
                                            Text(L10n.format("replay.deckRefreshEvery", "\(frames)"))
                                                .tag(frames)
                                        }
                                    }
                                    .labelsHidden()
                                    .frame(width: 150)
                                    .help(L10n.t("replay.deckRefreshHelp"))
                                }
                            }
                            Text(L10n.t("replay.retentionHelp")).font(.caption)
                        }
                    }
                    VStack(alignment: .leading, spacing: 6) {
                        Text(L10n.t("replay.angles"))
                            .font(.system(size: 14, weight: .bold))
                        VStack(alignment: .leading, spacing: 6) {
                            Text(L10n.t("replay.anglesHelp")).font(.caption)
                            // Every angle is edited in its own row: name, what it records, how, its
                            // audio and its drive are cells, not a separate panel.
                            Table($angles) {
                                TableColumn(L10n.t("replay.name")) { $angle in
                                    TextField("", text: $angle.name)
                                }
                                .width(170)
                                TableColumn(L10n.t("replay.sourceKind")) { $angle in
                                    Picker("", selection: $angle.source) {
                                        Text(L10n.t("replay.sourceScene")).tag("Scene")
                                        Text(L10n.t("replay.sourceInput")).tag("Input")
                                    }
                                    .labelsHidden()
                                    .onChange(of: $angle.wrappedValue.source) { _, _ in
                                        resetAngleSource($angle.wrappedValue.id)
                                    }
                                }
                                .width(100)
                                TableColumn(L10n.t("replay.source")) { $angle in
                                    Picker("", selection: sourceBinding($angle)) {
                                        ForEach(sources(for: $angle.wrappedValue), id: \.id) { source in
                                            Text(source.name).tag(source.id)
                                        }
                                    }
                                    .labelsHidden()
                                }
                                .width(240)
                                TableColumn(L10n.t("replay.encoder")) { $angle in
                                    Picker("", selection: $angle.encoder) {
                                        Text(L10n.t("replay.encoderGpu")).tag("Gpu")
                                        Text(L10n.t("replay.encoderCpu")).tag("Cpu")
                                    }
                                    .labelsHidden()
                                    .help(L10n.t("replay.encoderHelp"))
                                }
                                .width(110)
                                TableColumn(L10n.t("replay.audio")) { $angle in
                                    Picker("", selection: $angle.audioUnitId) {
                                        Text(L10n.t("replay.audioSilent")).tag(UInt64(0))
                                        ForEach(mixer.session.units) { unit in
                                            Text(unit.name).tag(unit.id)
                                        }
                                    }
                                    .labelsHidden()
                                    .help(L10n.t("replay.audioHelp"))
                                }
                                .width(160)
                                TableColumn(L10n.t("replay.drive")) { $angle in
                                    Picker("", selection: driveBinding($angle.wrappedValue.id)) {
                                        Text(L10n.t("replay.driveUnassigned")).tag(-1)
                                        ForEach(Array(storages.enumerated()), id: \.offset) { index, storage in
                                            Text("\(index + 1): \(storage.path)").tag(index)
                                        }
                                    }
                                    .labelsHidden()
                                    .help(L10n.t("replay.storageHelp"))
                                }
                                .width(160)
                                TableColumn("") { $angle in
                                    Button(L10n.t("replay.removeAngle")) {
                                        removeAngle($angle.wrappedValue.id)
                                    }
                                }
                                .width(80)
                            }
                            .frame(minHeight: 210)                            .frame(minHeight: 230)
                            HStack {
                                Button(L10n.t("replay.addAngleRow")) { addAngle() }
                                Text(L10n.t("replay.angleRowHelp")).font(.caption)
                            }
                        }
                    }
                    Toggle(L10n.t("replay.autoAdd"), isOn: $autoAdd)
                    Text(L10n.t("replay.autoAddHelp")).font(.caption)
                    Text(L10n.t("replay.warnings")).font(.system(size: 11))
                    HStack {
                        TextField("600", text: $warnSecs).frame(width: 70)
                        TextField("300", text: $alertSecs).frame(width: 70)
                        TextField("60", text: $criticalSecs).frame(width: 70)
                    }
                    Text(L10n.t("replay.warningsHelp")).font(.caption)
                    Toggle(L10n.t("replay.decodeAhead"), isOn: $decodeAhead)
                    Text(L10n.t("replay.decodeAheadHelp")).font(.caption)
                    Toggle(L10n.t("replay.profiling"), isOn: $profiling)
                    Text(L10n.t("replay.profilingHelp")).font(.caption)
                    if let active = document?.activeAngleId, active != 0 {
                        Text(L10n.format("replay.activeAngle", document?.angleLabel(active) ?? ""))
                    } else {
                        Text(L10n.t("replay.noActiveAngle"))
                    }
                    if !status.isEmpty {
                        Text(status).font(.caption)
                    }
                }
                .padding(14)
            }
            .frame(height: 720)
            Divider()
            HStack(spacing: 8) {
                Spacer()
                // Closing discards the edits: nothing was published, so the window
                // just goes away. Apply keeps it open.
                Button(L10n.t("replay.close")) { mixer.closeReplaySettings() }
                    .frame(width: 130, height: 34)
                    .keyboardShortcut(.cancelAction)
                Button(L10n.t("replay.apply")) { apply() }
                    .frame(width: 130, height: 34)
                    .keyboardShortcut(.defaultAction)
            }
            .padding(.horizontal, 16)
            .padding(.bottom, 14)
        }
        .frame(minWidth: 620, maxWidth: .infinity, minHeight: 520, maxHeight: .infinity)
        .onAppear { load() }
    }

    private func numberField(_ key: String, _ value: Binding<String>, width: CGFloat) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(L10n.t(key)).font(.caption)
            TextField("", text: value).frame(width: width)
        }
    }

    private var planText: String {
        guard let limits = mixer.replayLimits() else { return "" }
        if limits.angles == 0 || limits.channels == 0 {
            return L10n.t("replay.planDenied")
        }
        return L10n.format("replay.planLimits", "\(limits.angles)", "\(limits.channels)")
    }

    /// The format Replay records in: the first Mixing Unit's picture and rate. It is
    /// shown rather than edited, because every angle shares one format.
    private var formatText: String {
        guard let unit = mixer.session.units.first else {
            return L10n.t("replay.needMixingUnit")
        }
        return "\(unit.width) x \(unit.height)   \(unit.fpsNum)/\(unit.fpsDen)"
    }

    private func sceneName(_ sceneId: UInt64) -> String {
        mixer.session.scenes.first { $0.id == sceneId }?.name
            ?? L10n.format("replay.sceneMissing", "\(sceneId)")
    }

    private func load() {
        guard let current = mixer.replayDocument else {
            status = L10n.t("replay.missing")
            return
        }
        document = current
        storages = current.storages
        profile = current.profile
        segmentFrames = "\(current.segmentFrames)"
        let warnings = current.raw["warnings"]?.objectValue
        warnSecs = "\(warnings?["remainingWarnSecs"]?.uint32Value ?? 600)"
        alertSecs = "\(warnings?["remainingAlertSecs"]?.uint32Value ?? 300)"
        criticalSecs = "\(warnings?["remainingCriticalSecs"]?.uint32Value ?? 60)"
        retention = current.retention
        let buffering = current.buffering
        queue = "\(buffering.queue)"
        buffer = "\(buffering.buffer)"
        reserve = "\(buffering.reserve)"
        overflow = buffering.overflow
        autoAdd = current.autoAddToPlaylist
        decodeAhead = current.raw["decodeAhead"]?.boolValue ?? true
        profiling = current.raw["profiling"]?.boolValue ?? false
        angles = current.angles
    }

    private func inputKind(_ inputId: UInt64) -> InputKind? {
        mixer.session.inputs.first { $0.id == inputId }?.kind
    }

    private func inputName(_ inputId: UInt64) -> String {
        mixer.session.inputs.first { $0.id == inputId }?.name
            ?? L10n.format("replay.inputMissing", "\(inputId)")
    }

    /// One entry of the add row's source list.
    private struct SourceOption: Identifiable {
        let id: UInt64
        let name: String
    }

    /// What a row can record: scenes, or the inputs that have a picture. A Replay
    /// channel is not a camera, and an audio input has nothing to record.
    private func sources(for angle: ReplayAngleInfo) -> [SourceOption] {
        if angle.source == "Input" {
            return mixer.session.inputs
                .filter { $0.kind != .replay && $0.kind != .audio }
                .map { SourceOption(id: $0.id, name: $0.name) }
        }
        return mixer.session.scenes.map { SourceOption(id: $0.id, name: $0.name) }
    }

    /// The scene or input a row points at, depending on its kind. The row keeps one id
    /// per kind so switching back and forth does not lose the other choice.
    private func sourceBinding(_ angle: Binding<ReplayAngleInfo>) -> Binding<UInt64> {
        Binding(
            get: {
                angle.wrappedValue.source == "Input"
                    ? angle.wrappedValue.inputId
                    : angle.wrappedValue.sceneId
            },
            set: { value in
                if angle.wrappedValue.source == "Input" {
                    angle.wrappedValue.inputId = value
                    angle.wrappedValue.sceneId = 0
                } else {
                    angle.wrappedValue.sceneId = value
                    angle.wrappedValue.inputId = 0
                }
            }
        )
    }

    /// Which drive a row records to. The document keeps the assignment on the drives, so
    /// this reads and writes there rather than on the angle.
    private func driveBinding(_ angleId: UInt32) -> Binding<Int> {
        Binding(
            get: { driveOf(angleId) },
            set: { index in assignDrive(angleId, index) }
        )
    }

    /// A new kind starts from the first source of that kind.
    private func resetAngleSource(_ angleId: UInt32) {
        guard let index = angles.firstIndex(where: { $0.id == angleId }) else { return }
        var angle = angles[index]
        let first = sources(for: angle).first?.id ?? 0
        if angle.source == "Input" {
            angle.inputId = first
            angle.sceneId = 0
        } else {
            angle.sceneId = first
            angle.inputId = 0
        }
        angles[index] = angle
    }

    /// Moves an angle to a drive. The assignment lives on the drives, so it is rewritten
    /// there: an angle belongs to exactly one.
    private func assignDrive(_ angleId: UInt32, _ index: Int) {
        for slot in storages.indices {
            storages[slot].angleIds.removeAll { $0 == angleId }
        }
        if index >= 0 && index < storages.count {
            storages[index].angleIds.append(angleId)
        }
    }

    private func driveIndex(_ storage: ReplayDocument.StorageSpec) -> Int {
        storages.firstIndex(of: storage) ?? 0
    }

    /// Drive an angle records to. Angles that name no drive report zero, which the
    /// list shows as such: Apply refuses a setup with an unassigned angle.
    private func driveOf(_ angleId: UInt32) -> Int {
        storages.firstIndex { $0.angleIds.contains(angleId) } ?? -1
    }

    /// Asks for a folder the way every other path on this host is chosen. Picking one
    /// fills the row it was asked from, or adds a row when none was named.
    private func browseInto(_ storageId: String?) {
        let panel = NSOpenPanel()
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.allowsMultipleSelection = false
        panel.canCreateDirectories = true
        panel.prompt = L10n.t("replay.pickStorage")
        panel.message = L10n.t("replay.pickStorageHint")
        guard panel.runModal() == .OK, let url = panel.url else { return }
        let trimmed = url.path.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        guard !storages.contains(where: { $0.id != storageId && $0.path.caseInsensitiveCompare(trimmed) == .orderedSame }) else {
            status = L10n.t("replay.duplicateStorage")
            return
        }
        if let storageId, let index = storages.firstIndex(where: { $0.id == storageId }) {
            storages[index].path = trimmed
        } else {
            storages.append(ReplayDocument.StorageSpec(path: trimmed, angleIds: []))
            // The first drive takes the angles that have none: a setup with one drive
            // should not need a second step per angle.
            if storages.count == 1 {
                storages[0].angleIds = angles.map(\.id).sorted()
            }
        }
        status = L10n.format("replay.storageAdded", trimmed)
    }

    /// Removes a drive by its row, moving any angle that recorded there to the first one.
    private func removeStorageRow(_ storageId: String) {
        guard storages.count > 1, let index = storages.firstIndex(where: { $0.id == storageId }) else {
            status = L10n.t("replay.needStorable")
            return
        }
        let removed = storages.remove(at: index)
        for angle in removed.angleIds {
            assignDrive(angle, 0)
        }
    }

    /// Audio source label for the angle list.
    private func audioLabel(_ unitId: UInt64) -> String {
        if unitId == 0 {
            return L10n.t("replay.audioSilent")
        }
        return mixer.session.units.first { $0.id == unitId }?.name
            ?? L10n.format("replay.audioUnit", "\(unitId)")
    }

    /// Adds a row for a new angle. It starts on the next unused scene, so the common case
    /// needs no edits at all, and the row is there to be changed in place if it is not the
    /// one they wanted.
    private func addAngle() {
        guard !storages.isEmpty else {
            status = L10n.t("replay.needDriveForAngle")
            return
        }
        if let limits = mixer.replayLimits(), limits.angles != .max, angles.count >= Int(limits.angles) {
            status = L10n.format("replay.planLimits", "\(limits.angles)", "\(limits.channels)")
            return
        }
        let used = Set(angles.filter { $0.source == "Scene" }.map(\.sceneId))
        let scene = mixer.session.scenes.first { !used.contains($0.id) }
        guard var current = document else { return }
        let id = current.reserveAngleId()
        document = current
        angles.append(
            ReplayAngleInfo(
                id: id,
                name: scene?.name ?? "",
                source: "Scene",
                sceneId: scene?.id ?? 0,
                inputId: 0,
                    audioUnitId: 0
            )
        )
        // A single drive takes the new angle: a setup with one drive should not need a
        // second step per angle.
        if storages.count == 1 {
            storages[0].angleIds.append(id)
        }
        status = L10n.format("replay.angleAdded", scene?.name ?? "")
    }

    /// Removes one angle by id, from its own row's button.
    private func removeAngle(_ angleId: UInt32) {
        guard let angle = angles.first(where: { $0.id == angleId }) else { return }
        // A clip that plays this angle, or takes its sound from it, would be left
        // pointing at nothing, so the angle waits until those clips are gone.
        let users = clipCount(using: angle.id)
        if users > 0 {
            status = L10n.format("replay.angleInUse", "\(users)")
            return
        }
        angles.removeAll { $0.id == angle.id }
        // A drive that still listed it would fail validation on Apply.
        for index in storages.indices {
            storages[index].angleIds.removeAll { $0 == angle.id }
        }
    }

    /// How many clips play this angle, or take their sound from it.
    private func clipCount(using angleId: UInt32) -> Int {
        guard let document else { return 0 }
        var count = 0
        for playlistId in document.playlistIds {
            for clip in document.clips(playlistId: playlistId) {
                let picture = clip.angleId == angleId
                let audio = clip.audioKind == "Angle" && clip.audioAngleId == angleId
                if picture || audio {
                    count += 1
                }
            }
        }
        return count
    }

    /// Free space for one drive, from the same mixer model the recorder uses. The
    /// settings table is where an operator sees a full drive before pressing Apply.
    private func freeSpaceText(_ path: String, angles: [UInt32]) -> String {
        let trimmed = path.trimmingCharacters(in: .whitespaces)
        guard !trimmed.isEmpty, let unit = mixer.session.units.first else { return "—" }
        let count = UInt32(max(1, angles.count))
        guard let estimate = MixerFFI.replayEstimate(
            path: trimmed,
            width: unit.width,
            height: unit.height,
            fpsNum: unit.fpsNum,
            fpsDen: unit.fpsDen,
            profile: profileCode,
            angleCount: count,
            reserveBytes: UInt64(reserve) ?? 0
        ), estimate.path_writable != 0 else { return "—" }
        return ByteCountFormatter.string(
            fromByteCount: Int64(estimate.free_bytes), countStyle: .file)
    }

    private var profileCode: UInt32 {
        switch profile {
        case "Hq": return 0
        case "Lq": return 2
        default: return 1
        }
    }

    private func apply() {
        guard var current = document else { return }
        guard let queueValue = UInt32(queue), (2...60).contains(queueValue),
              let bufferValue = UInt32(buffer), (64...8192).contains(bufferValue),
              let reserveValue = UInt32(reserve)
        else {
            status = L10n.t("replay.badBuffering")
            return
        }
        // The segment length is the operator's setting: the recorder no longer
        // substitutes one when it is missing or out of range.
        guard let segmentValue = UInt32(segmentFrames), (30...1800).contains(segmentValue) else {
            status = L10n.t("replay.segmentFramesRange")
            return
        }
        guard let unit = mixer.session.units.first else {
            // Every angle records one format, and the format comes from the Mixing
            // Unit. Without one there is nothing to record in.
            status = L10n.t("replay.needMixingUnit")
            return
        }
        let usable = storages.filter { !$0.path.trimmingCharacters(in: .whitespaces).isEmpty }
        if let problem = ReplayDeckMath.setupProblem(angles: angles, storages: storages) {
            let name = { (id: UInt32) in
                angles.first(where: { $0.id == id })?.label ?? "\(id)"
            }
            switch problem {
            case .noStorage, .noAngles:
                status = L10n.t("replay.needStorable")
            case let .angleNeedsDrive(id):
                status = L10n.format("replay.angleNeedsDrive", name(id))
            case let .angleNeedsSource(id):
                status = L10n.format("replay.angleNeedsSource", name(id))
            case let .angleDuplicate(id, kind):
                status = L10n.format(
                    kind == "Input" ? "replay.angleDuplicateInput" : "replay.angleDuplicate",
                    name(id))
            case let .duplicateStorage(path):
                status = L10n.format("replay.duplicateStorage", path)
            }
            return
        }
        // Only angles that still exist are assigned, so a removed one cannot be
        // left behind on a drive and fail validation.
        let known = Set(angles.map { $0.id })
        for index in storages.indices {
            storages[index].angleIds.removeAll { !known.contains($0) }
        }
        // One drive takes every angle: the operator added the drive for this
        // recording, and a recording never guesses which disk to write to.
        if storages.count == 1 {
            storages[0].angleIds = angles.map(\.id).sorted()
        }
        current.setStorages(storages)
        var format = current.raw["format"]?.objectValue ?? [:]
        // The whole format travels: a document that keeps a zero resolution or a zero
        // segment length can never record, and the mixer would only be able to say so
        // much later.
        format["width"] = .number(Double(unit.width))
        format["height"] = .number(Double(unit.height))
        format["fpsNum"] = .number(Double(unit.fpsNum))
        format["fpsDen"] = .number(Double(unit.fpsDen))
        format["profile"] = .string(profile)
        format["segmentFrames"] = .number(Double(segmentValue))
        current.raw["format"] = .object(format)
        guard let warnValue = UInt32(warnSecs),
              let alertValue = UInt32(alertSecs),
              let criticalValue = UInt32(criticalSecs)
        else {
            status = L10n.t("replay.warningsHelp")
            return
        }
        current.raw["warnings"] = .object([
            "remainingWarnSecs": .number(Double(warnValue)),
            "remainingAlertSecs": .number(Double(alertValue)),
            "remainingCriticalSecs": .number(Double(criticalValue)),
        ])
        current.raw["buffering"] = .object([
            "encodeQueueFrames": .number(Double(queueValue)),
            "writeBufferMb": .number(Double(bufferValue)),
            "overflowPolicy": .string(overflow),
            "reserveMb": .number(Double(reserveValue)),
        ])
        current.setRetention(retention)
        current.setAngles(angles)
        current.refreshEnabled()
        current.setAutoAddToPlaylist(autoAdd)
        current.raw["decodeAhead"] = .bool(decodeAhead)
        current.raw["profiling"] = .bool(profiling)
        document = current
        // Applying either works or says why. A window that closes without a word
        // leaves the operator unable to tell the two apart.
        if mixer.applyReplay(current) {
            // The deck refresh interval lives in the session settings, not the replay
            // document, so applying has to publish the session too.
            mixer.publishSession()
            if let gpuCount = ReplayDeckMath.gpuAnglesOverCapacity(angles) {
                AppKitDialog.toast(L10n.format(
                    "replay.gpuAnglesOverCapacity",
                    "\(gpuCount)",
                    "\(ReplayDeckMath.gpuAnglesPerEncoder)"))
            } else {
                AppKitDialog.toast(L10n.t("replay.applied"))
            }
            // Windows keeps the settings window open after Apply: an operator tuning four
            // cameras should not have to reopen it between attempts. Closing is its own
            // button, and it is what discards.
        } else {
            status = L10n.t("replay.publishRefused")
        }
    }
}
