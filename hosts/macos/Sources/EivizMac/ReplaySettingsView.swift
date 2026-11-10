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
    @State private var angles: [ReplayAngleInfo] = []
    @State private var status = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(planText).font(.callout)
            if !status.isEmpty {
                // Apply and every refusal report here, where it cannot be missed.
                Text(status).fontWeight(.bold).foregroundStyle(EivizTheme.dim)
            }
            GroupBox(L10n.t("replay.storage")) {
                VStack(alignment: .leading, spacing: 6) {
                    Text(L10n.t("replay.storageHelp"))
                        .font(.caption)
                        .foregroundStyle(EivizTheme.dim)
                    // Each drive is edited in its own row: the path is the row's text field,
                    // and the angles that record there are shown in the same row.
                    List {
                        ForEach($storages) { $storage in
                            HStack(spacing: 6) {
                                TextField(L10n.t("settings.replayPath"), text: $storage.path)
                                    .frame(minWidth: 240)
                                    .help(L10n.t("replay.pickStorageHint"))
                                Text(storage.angleIds.map(String.init).joined(separator: ", "))
                                    .foregroundStyle(EivizTheme.dim)
                                Spacer()
                                Button(L10n.t("replay.browse")) { browseInto(storage.id) }
                                Button(L10n.t("replay.removeStorage")) { removeStorageRow(storage.id) }
                            }
                        }
                    }
                    .frame(minHeight: 90)
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
                    }
                    HStack {
                        TextField(L10n.t("settings.replayQueue"), text: $queue).frame(width: 80)
                        TextField(L10n.t("settings.replayBuffer"), text: $buffer).frame(width: 90)
                        TextField(L10n.t("replay.reserve"), text: $reserve).frame(width: 100)
                        Picker(L10n.t("replay.overflow"), selection: $overflow) {
                            Text(L10n.t("replay.overflowDrop")).tag("DropFrames")
                            Text(L10n.t("replay.overflowStop")).tag("StopRecording")
                        }
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
                    }
                    HStack {
                        Text(L10n.t("replay.warnings")).font(.caption)
                        TextField("600", text: $warnSecs).frame(width: 70)
                        TextField("300", text: $alertSecs).frame(width: 70)
                        TextField("60", text: $criticalSecs).frame(width: 70)
                    }
                    Text(L10n.t("replay.warningsHelp")).font(.caption)
                    Text(L10n.t("replay.retentionHelp")).font(.caption)
                }
            }
            GroupBox(L10n.t("replay.angles")) {
                VStack(alignment: .leading, spacing: 6) {
                    Text(L10n.t("replay.anglesHelp")).font(.caption)
                    // Every angle is edited in its own row: name, what it records, how, its
                    // audio and its drive are cells, not a separate panel.
                    List {
                        ForEach($angles) { $angle in
                            HStack(spacing: 6) {
                                TextField(L10n.t("replay.name"), text: $angle.name)
                                    .frame(width: 130)
                                Picker("", selection: $angle.source) {
                                    Text(L10n.t("replay.sourceScene")).tag("Scene")
                                    Text(L10n.t("replay.sourceInput")).tag("Input")
                                }
                                .labelsHidden()
                                .frame(width: 90)
                                .onChange(of: angle.source) { _, _ in resetAngleSource(angle.id) }
                                Picker("", selection: sourceBinding($angle)) {
                                    ForEach(sources(for: angle), id: \.id) { source in
                                        Text(source.name).tag(source.id)
                                    }
                                }
                                .labelsHidden()
                                .frame(width: 170)
                                // Where this angle is encoded: GPU keeps the picture on
                                // the GPU from the scene to the stored packet.
                                Picker("", selection: $angle.encoder) {
                                    Text(L10n.t("replay.encoderGpu")).tag("Gpu")
                                    Text(L10n.t("replay.encoderCpu")).tag("Cpu")
                                }
                                .labelsHidden()
                                .frame(width: 90)
                                .help(L10n.t("replay.encoderHelp"))
                                Picker("", selection: $angle.audioUnitId) {
                                    Text(L10n.t("replay.audioSilent")).tag(UInt64(0))
                                    ForEach(mixer.session.units) { unit in
                                        Text(unit.name).tag(unit.id)
                                    }
                                }
                                .labelsHidden()
                                .frame(width: 140)
                                .help(L10n.t("replay.audioHelp"))
                                Picker("", selection: driveBinding(angle.id)) {
                                    Text(L10n.t("replay.driveUnassigned")).tag(-1)
                                    ForEach(Array(storages.enumerated()), id: \.offset) { index, storage in
                                        Text("\(index + 1): \(storage.path)").tag(index)
                                    }
                                }
                                .labelsHidden()
                                .frame(width: 150)
                                .help(L10n.t("replay.storageHelp"))
                                Button(L10n.t("replay.removeAngle")) { removeAngle(angle.id) }
                            }
                        }
                    }
                    .frame(minHeight: 150)
                    HStack {
                        Button(L10n.t("replay.addAngleRow")) { addAngle() }
                        Text(L10n.t("replay.angleRowHelp")).font(.caption)
                    }
                }
            }
            Toggle(L10n.t("replay.autoAdd"), isOn: $autoAdd)
            Text(L10n.t("replay.autoAddHelp")).font(.caption)
            if let active = document?.activeAngleId, active != 0 {
                Text(L10n.format("replay.activeAngle", document?.angleLabel(active) ?? ""))
            } else {
                Text(L10n.t("replay.noActiveAngle"))
            }
            if !status.isEmpty {
                Text(status).font(.caption)
            }
            Spacer()
            HStack {
                Spacer()
                // Cancelling discards the edits: nothing was published, so the
                // window just goes away.
                Button(L10n.t("dialog.cancel")) { mixer.closeReplaySettings() }
                Button(L10n.t("replay.apply")) { apply() }
                    .keyboardShortcut(.defaultAction)
            }
        }
        .padding(14)
        .frame(minWidth: 620, minHeight: 520)
        .onAppear { load() }
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
                } else {
                    angle.wrappedValue.sceneId = value
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
        } else {
            angle.sceneId = first
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
        let id = (angles.map(\.id).max() ?? 0) + 1
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
        if !usable.isEmpty && angles.isEmpty {
            status = L10n.t("replay.needStorable")
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
        current.setAutoAddToPlaylist(autoAdd)
        document = current
        // Applying either works or says why. A window that closes without a word
        // leaves the operator unable to tell the two apart.
        if mixer.applyReplay(current) {
            AppKitDialog.toast(L10n.t("replay.applied"))
            mixer.closeReplaySettings()
        } else {
            status = L10n.t("replay.publishRefused")
        }
    }
}
