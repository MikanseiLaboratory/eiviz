import AppKit
import AVFoundation
import Combine
import Darwin
import EivizMixer
import EivizRemote
import Foundation
import SwiftUI
import UniformTypeIdentifiers

extension MixerController {
    func saveSession() {
        if let path = MixerFFI.sessionCurrentPath(), canOverwrite(path) {
            writeSession(to: path, export: false)
            return
        }
        saveSessionAs()
    }

    func saveSessionAs() {
        let panel = NSSavePanel()
        panel.allowedContentTypes = [UTType.eivizSession]
        panel.allowsOtherFileTypes = false
        panel.nameFieldStringValue = suggestedSessionName()
        guard panel.runModal() == .OK, let url = panel.url else { return }
        writeSession(to: url.path, export: false)
    }

    func saveRemoteSession() {
        guard isRemote, remoteHandle != 0, remoteConnected else { return }
        let (n, bytes) = MixerFFI.copyUtf8(startCap: 8192) {
            mixer_remote_save_session(remoteHandle, $0, $1)
        }
        guard n >= 0 else {
            fail(n < 0 ? -n : n, "Save session")
            return
        }
        struct SavedRemote: Decodable {
            let path: String
            let historyCount: UInt32
        }
        guard let payload = try? JSONDecoder().decode(SavedRemote.self, from: Data(bytes)) else {
            presentError(L10n.error("Save session", 3), title: L10n.t("action.Save session"))
            return
        }
        AppKitDialog.toast(L10n.format("msg.remoteSaved", payload.path, "\(payload.historyCount)"))
    }

    func exportSession() {
        let panel = NSSavePanel()
        panel.allowedContentTypes = [UTType.eivizSessionExport]
        panel.allowsOtherFileTypes = false
        panel.nameFieldStringValue = "session.eivzx"
        guard panel.runModal() == .OK, let url = panel.url else { return }
        writeSession(to: url.path, export: true)
    }

    func suggestedSessionName() -> String {
        if let path = MixerFFI.sessionCurrentPath() ?? AppPrefs.shared.recentSessions.first {
            return URL(fileURLWithPath: path).deletingPathExtension().lastPathComponent + ".eivz"
        }
        return "session.eivz"
    }

    func canOverwrite(_ path: String) -> Bool {
        let lower = path.lowercased()
        return lower.hasSuffix(".eivz") && !lower.hasSuffix(".eivzx")
    }

    func writeSession(to path: String, export: Bool) {
        syncAllUnitBuses()
        captureSceneBuses()
        session.selectedUnitId = selectedUnitId
        session.settings.lastSessionPath = nil
        do {
            let json = try SessionFile.encode(session)
            let saved = MixerFFI.withCString(path) { cPath in
                json.withUnsafeBytes { ptr in
                    fail(
                        export
                            ? mixer_session_export(
                                cPath,
                                ptr.bindMemory(to: UInt8.self).baseAddress,
                                json.count
                            )
                            : mixer_session_save(
                                cPath,
                                ptr.bindMemory(to: UInt8.self).baseAddress,
                                json.count
                            ),
                        export ? "Export session" : "Save session"
                    )
                }
            }
            if saved {
                AppPrefs.shared.rememberSession(path)
                if !export {
                    AppKitDialog.toast(L10n.t("msg.saved"))
                }
            }
        } catch {
            presentError(
                L10n.error(export ? "Export session" : "Save session", 3),
                title: L10n.t(export ? "action.Export session" : "action.Save session")
            )
        }
    }

    func newSession() {
        _ = mixer_session_clear_current()
        replaceSession(MixerSessionData.default())
    }

    func loadSession(path: String? = nil) {
        let fromPanel = path == nil
        let url: URL
        if let path {
            url = URL(fileURLWithPath: path)
        } else {
            let panel = NSOpenPanel()
            panel.allowedContentTypes = [UTType.eivizSession, UTType.eivizSessionExport]
            panel.allowsOtherFileTypes = false
            panel.allowsMultipleSelection = false
            guard panel.runModal() == .OK, let picked = panel.url else { return }
            url = picked
        }
        var historyIndex: UInt32?
        if fromPanel {
            let (historyN, historyBytes) = MixerFFI.withCString(url.path) { cPath in
                MixerFFI.copyUtf8 { mixer_session_history(cPath, $0, $1) }
            }
            guard historyN >= 0 else {
                fail(historyN == 0 ? 5 : historyN, "Load session")
                return
            }
            guard let entries = Self.parseHistory(historyBytes) else {
                presentError(L10n.error("Load session", 3), title: L10n.t("action.Load session"))
                return
            }
            if !entries.isEmpty {
                switch pickHistory(entries) {
                case .cancel:
                    return
                case .latest:
                    break
                case .revision(let index):
                    historyIndex = index
                }
            }
        }
        if historyIndex == nil, MixerFFI.sessionHasAssets(url.path) {
            guard let dest = pickImportDestinations(exportPath: url.path) else { return }
            let (n, bytes) = MixerFFI.withCString(url.path) { export in
                MixerFFI.withCString(dest.session) { sessionPath in
                    MixerFFI.withCString(dest.media) { media in
                        MixerFFI.copyUtf8 { mixer_session_import(export, sessionPath, media, $0, $1) }
                    }
                }
            }
            guard n > 0 else {
                fail(n == 0 ? 5 : n, "Load session")
                return
            }
            do {
                replaceSession(try SessionFile.decode(Data(bytes)))
                AppPrefs.shared.rememberSession(dest.session)
            } catch {
                presentError(L10n.error("Load session", 3), title: L10n.t("action.Load session"))
            }
            return
        }
        let (n, bytes) = MixerFFI.withCString(url.path) { cPath in
            if let historyIndex {
                MixerFFI.copyUtf8 { mixer_session_load_rev(cPath, historyIndex, $0, $1) }
            } else {
                MixerFFI.copyUtf8 { mixer_session_load(cPath, $0, $1) }
            }
        }
        guard n > 0 else {
            fail(n == 0 ? 5 : n, "Load session")
            return
        }
        do {
            replaceSession(try SessionFile.decode(Data(bytes)))
            AppPrefs.shared.rememberSession(url.path)
        } catch {
            presentError(L10n.error("Load session", 3), title: L10n.t("action.Load session"))
        }
    }

    func loadLastSession() {
        guard let path = AppPrefs.shared.existingSessions().first else {
            presentError(L10n.t("msg.noLastSession"), title: L10n.t("chrome.loadLast"))
            return
        }
        loadSession(path: path)
    }

    func relinkMedia() {
        guard let directory = pickRelinkDirectory() else { return }
        let before = Dictionary(uniqueKeysWithValues: session.inputs.map { ($0.id, $0.pathOrAddress) })
        if isRemote {
            _ = mutateRemote(MixerRemote.relinkMedia([directory]))
            let count = session.inputs.filter { before[$0.id] != $0.pathOrAddress }.count
            presentRelinked(count)
            return
        }
        let count = relinkMissingMedia(directories: [directory])
        publishSession()
        applySession()
        objectWillChange.send()
        presentRelinked(count)
    }

    func relinkInputFile(_ input: InputEntry) {
        guard input.kind == .still || input.kind == .video else {
            presentError(L10n.t("msg.selectInputRelink"), title: L10n.t("input.relinkFile"))
            return
        }
        guard let path = pickRelinkFile(for: input) else { return }
        applyRelinkPath(input, path: path)
        presentRelinked(1)
    }

    func presentRelinked(_ count: Int) {
        let alert = NSAlert()
        alert.messageText = L10n.t("input.relink")
        alert.informativeText = L10n.format("msg.relinked", "\(count)")
        alert.alertStyle = .informational
        alert.addButton(withTitle: L10n.t("dialog.ok"))
        AppKitDialog.elevate(alert)
        alert.runModal()
    }

    func pickRelinkDirectory() -> String? {
        if isRemote {
            let alert = NSAlert()
            alert.messageText = L10n.t("input.relinkFolder")
            alert.informativeText = L10n.t("input.relinkDir")
            let field = NSTextField(string: "")
            field.frame = NSRect(x: 0, y: 0, width: 320, height: 24)
            alert.accessoryView = field
            alert.addButton(withTitle: L10n.t("dialog.ok"))
            alert.addButton(withTitle: L10n.t("dialog.cancel"))
            AppKitDialog.elevate(alert)
            guard alert.runModal() == .alertFirstButtonReturn else { return nil }
            let text = field.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
            return text.isEmpty ? nil : text
        }
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = false
        panel.message = L10n.t("input.relinkFolder")
        AppKitDialog.apply(panel)
        guard panel.runModal() == .OK, let url = panel.url else { return nil }
        return url.path
    }

    func pickRelinkFile(for input: InputEntry) -> String? {
        if isRemote {
            let alert = NSAlert()
            alert.messageText = L10n.t("input.relinkFile")
            alert.informativeText = L10n.t("input.relinkFileHost")
            let field = NSTextField(string: input.pathOrAddress ?? "")
            field.frame = NSRect(x: 0, y: 0, width: 320, height: 24)
            alert.accessoryView = field
            alert.addButton(withTitle: L10n.t("dialog.ok"))
            alert.addButton(withTitle: L10n.t("dialog.cancel"))
            AppKitDialog.elevate(alert)
            guard alert.runModal() == .alertFirstButtonReturn else { return nil }
            let text = field.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
            return text.isEmpty ? nil : text
        }
        let panel = NSOpenPanel()
        panel.canChooseDirectories = false
        panel.canChooseFiles = true
        panel.allowsMultipleSelection = false
        panel.message = L10n.t("input.relinkFile")
        panel.allowedContentTypes = input.kind == .still ? [.image] : [.movie]
        AppKitDialog.apply(panel)
        guard panel.runModal() == .OK, let url = panel.url else { return nil }
        return url.path
    }

    func applyRelinkPath(_ input: InputEntry, path: String) {
        if isRemote {
            var next = input
            next.pathOrAddress = path
            _ = mutateRemote(MixerRemote.upsertInput(next))
            return
        }
        guard let index = session.inputs.firstIndex(where: { $0.id == input.id }) else { return }
        session.inputs[index].pathOrAddress = path
        publishSession()
        applySession()
        objectWillChange.send()
    }

    func pickImportDestinations(exportPath: String) -> (session: String, media: String)? {
        let hint = NSAlert()
        hint.messageText = L10n.t("chrome.importExport")
        hint.informativeText = L10n.t("chrome.importExportHint")
        hint.alertStyle = .informational
        hint.addButton(withTitle: L10n.t("dialog.ok"))
        hint.addButton(withTitle: L10n.t("dialog.cancel"))
        AppKitDialog.elevate(hint)
        guard hint.runModal() == .alertFirstButtonReturn else { return nil }
        let exportUrl = URL(fileURLWithPath: exportPath)
        let parent = exportUrl.deletingLastPathComponent()
        let stem = exportUrl.deletingPathExtension().lastPathComponent
        let sessionPanel = NSSavePanel()
        sessionPanel.allowedContentTypes = [UTType.eivizSession]
        sessionPanel.allowsOtherFileTypes = false
        sessionPanel.directoryURL = parent
        sessionPanel.nameFieldStringValue = stem + ".eivz"
        sessionPanel.message = L10n.t("chrome.importSession")
        AppKitDialog.apply(sessionPanel)
        guard sessionPanel.runModal() == .OK, let sessionUrl = sessionPanel.url else { return nil }
        let mediaPanel = NSOpenPanel()
        mediaPanel.canChooseDirectories = true
        mediaPanel.canCreateDirectories = true
        mediaPanel.canChooseFiles = false
        mediaPanel.allowsMultipleSelection = false
        mediaPanel.directoryURL = parent
        mediaPanel.message = L10n.t("chrome.importMedia")
        mediaPanel.prompt = L10n.t("dialog.ok")
        AppKitDialog.apply(mediaPanel)
        guard mediaPanel.runModal() == .OK, let mediaUrl = mediaPanel.url else { return nil }
        return (sessionUrl.path, mediaUrl.path)
    }

    func showInputInFinder(_ input: InputEntry) {
        guard let path = input.pathOrAddress?.trimmingCharacters(in: .whitespacesAndNewlines),
              !path.isEmpty
        else { return }
        let url = URL(fileURLWithPath: path)
        if FileManager.default.fileExists(atPath: path) {
            NSWorkspace.shared.activateFileViewerSelecting([url])
            return
        }
        let parent = url.deletingLastPathComponent().path
        guard FileManager.default.fileExists(atPath: parent) else { return }
        NSWorkspace.shared.selectFile(nil, inFileViewerRootedAtPath: parent)
    }

    func promptMissingMedia() {
        if isRemote { return }
        guard session.inputs.contains(where: \.isMissingMedia) else { return }
        while true {
            let leftover = session.inputs.filter(\.isMissingMedia)
            if leftover.isEmpty { break }
            let alert = NSAlert()
            alert.messageText = L10n.t("input.missingTitle")
            let lines = leftover.map { "\($0.name) — \($0.pathOrAddress ?? "")" }.joined(separator: "\n")
            alert.informativeText = L10n.t("input.missingHint") + "\n\n" + lines
            alert.alertStyle = .warning
            alert.addButton(withTitle: L10n.t("input.relinkFolder"))
            alert.addButton(withTitle: L10n.t("input.relinkFile"))
            alert.addButton(withTitle: L10n.t("dialog.ok"))
            AppKitDialog.elevate(alert)
            let choice = alert.runModal()
            if choice == .alertFirstButtonReturn {
                guard let directory = pickRelinkDirectory() else { continue }
                _ = relinkMissingMedia(directories: [directory])
                publishSession()
                applySession()
                objectWillChange.send()
                continue
            }
            if choice == .alertSecondButtonReturn {
                guard let input = pickMissingInput(leftover),
                      let path = pickRelinkFile(for: input)
                else { continue }
                applyRelinkPath(input, path: path)
                continue
            }
            break
        }
    }

    func pickMissingInput(_ leftover: [InputEntry]) -> InputEntry? {
        if leftover.count == 1 { return leftover[0] }
        let alert = NSAlert()
        alert.messageText = L10n.t("input.relinkFile")
        let popup = NSPopUpButton(frame: NSRect(x: 0, y: 0, width: 360, height: 24), pullsDown: false)
        for input in leftover {
            popup.addItem(withTitle: "\(input.listLabel(in: session, localFiles: true)) — \(input.pathOrAddress ?? "")")
        }
        alert.accessoryView = popup
        alert.addButton(withTitle: L10n.t("dialog.ok"))
        alert.addButton(withTitle: L10n.t("dialog.cancel"))
        AppKitDialog.elevate(alert)
        guard alert.runModal() == .alertFirstButtonReturn else { return nil }
        let index = popup.indexOfSelectedItem
        guard leftover.indices.contains(index) else { return nil }
        return leftover[index]
    }

    func relinkMissingMedia(directories: [String]) -> Int {
        let unique = uniqueFilenames(directories: directories)
        var count = 0
        for index in session.inputs.indices {
            guard session.inputs[index].isMissingMedia else { continue }
            guard let name = session.inputs[index].pathOrAddress.flatMap({ URL(fileURLWithPath: $0).lastPathComponent }),
                  !name.isEmpty,
                  let found = unique[name]
            else { continue }
            session.inputs[index].pathOrAddress = found
            count += 1
        }
        return count
    }

    func uniqueFilenames(directories: [String]) -> [String: String] {
        var found: [String: [String]] = [:]
        for directory in directories {
            collectFiles(directory: directory, into: &found, depth: 0)
        }
        var unique: [String: String] = [:]
        for (name, paths) in found {
            let distinct = Array(Set(paths))
            if distinct.count == 1 {
                unique[name] = distinct[0]
            }
        }
        return unique
    }

    func collectFiles(directory: String, into found: inout [String: [String]], depth: Int) {
        if depth > 16 { return }
        let fm = FileManager.default
        guard let entries = try? fm.contentsOfDirectory(atPath: directory) else { return }
        for name in entries {
            let path = (directory as NSString).appendingPathComponent(name)
            var isDir: ObjCBool = false
            guard fm.fileExists(atPath: path, isDirectory: &isDir) else { continue }
            if isDir.boolValue {
                collectFiles(directory: path, into: &found, depth: depth + 1)
                continue
            }
            found[name, default: []].append(path)
        }
    }

    func openSessionFromSystem(path: String) {
        if isRemote { return }
        if !booted {
            pendingOpenPath = path
            return
        }
        loadSession(path: path)
    }

    func openPendingSession() {
        guard let path = pendingOpenPath else { return }
        pendingOpenPath = nil
        loadSession(path: path)
    }

    func recreateMixer() {
        shutdown()
        boot()
    }

    func reconnectRemoteVideo() {
        guard isRemote else { return }
        remotePreviewKey = ""
        remoteProgramKey = ""
        remoteMultiviewKey = ""
        remotePreviewLive = false
        remoteProgramLive = false
        remoteMultiviewLive = false
        for id in remoteReceiveIds.values {
            _ = mixer_destroy_source(id)
        }
        remoteReceiveIds.removeAll()
        remoteReceiveKeys.removeAll()
        syncPublishedVideo()
        bumpSurfaceEpoch()
    }

    func replaceSession(_ loaded: MixerSessionData) {
        closeAllInputPreviews()
        closeAllSwitchers()
        closeAllMultiviews()
        FlipBudget.reset()
        mixer_destroy()
        session = loaded
        selectedUnitId = loaded.selectedUnitId == 0 ? 1 : loaded.selectedUnitId
        mix = 0
        inputFilter = .all
        sceneFilter = .all
        guard fail(mixer_create_with_backend(AppPrefs.shared.renderer.createAbi, 0, session.settings.masterFpsNum, session.settings.masterFpsDen), "Metal mixer initialization") else {
            return
        }
        fail(mixer_set_frame_buffer(min(8, max(1, session.settings.frameBufferFrames))), "Set frame buffer")
        fail(mixer_set_rebar_optimization(session.settings.rebarOptimizationEnabled ? 1 : 0), "Set ReBAR optimization")
        fail(mixer_set_ndi_gpu_upload(session.settings.ndiGpuUploadEnabled ? 1 : 0), "Set NDI GPU upload")
        FlipBudget.configure(session.settings.flipSwapchainLimit)
        applyBusColors()
        applySession()
        bumpSurfaceEpoch()
        promptMissingMedia()
    }

    func bumpSurfaceEpoch() {
        surfaceEpoch &+= 1
    }

    func closeAllMultiviews() {
        for id in Array(multiviewWindows.keys) {
            let window = multiviewWindows.removeValue(forKey: id)
            window?.delegate = nil
            window?.close()
        }
        showMultiview = false
        openMultiview = nil
    }

    var selectedVideoId: UInt64? {
        guard let id = selectedInputId,
              session.inputs.first(where: { $0.id == id })?.kind == .video
        else { return nil }
        return id
    }

    private var fileVideoId: UInt64? {
        selectedVideoId ?? session.inputs.first { $0.kind == .video }?.id
    }

    func copyVideoInfo(_ id: UInt64) -> EivizVideoInfo? {
        var info = EivizVideoInfo(playing: 0, is_file: 0, position_hns: 0, duration_hns: 0)
        guard mixer_video_copy_info(id, &info) == EIVIZ_OK else { return nil }
        return info
    }

    func startVideoInput(
        id: UInt64,
        path: String,
        capture: UInt32,
        width: UInt32 = 0,
        height: UInt32 = 0,
        fpsNum: UInt32 = 0,
        fpsDen: UInt32 = 0,
        loop: Bool,
        playing: Bool,
        frameBuffer: UInt32 = 3,
    ) {
        if capture == 0 && !FileManager.default.fileExists(atPath: path) {
            presentInputError(L10n.missingFile("Video start"))
            return
        }
        MixerFFI.withCString(path) { cstr in
            fail(
                mixer_video_start(
                    id,
                    cstr,
                    capture,
                    EIVIZ_FMT_BGRA,
                    width,
                    height,
                    fpsNum,
                    fpsDen,
                    max(1, min(8, frameBuffer == 0 ? 3 : frameBuffer))
                ),
                capture == 0 ? "Video start" : "UVC start"
            )
        }
        _ = mixer_video_set_loop(id, loop ? 1 : 0)
        _ = mixer_video_set_playing(id, playing ? 1 : 0)
    }

    func videoPlayToggle() {
        guard let id = fileVideoId else { return }
        videoPlaying.toggle()
        if isRemote {
            _ = mixer_remote_video_play(remoteHandle, id, videoPlaying ? 1 : 0)
        } else {
            _ = mixer_video_set_playing(id, videoPlaying ? 1 : 0)
        }
    }

    func videoRestart() {
        guard let id = fileVideoId else { return }
        if isRemote {
            _ = mixer_remote_video_seek(remoteHandle, id, 0)
            _ = mixer_remote_video_play(remoteHandle, id, 1)
        } else {
            _ = mixer_video_seek(id, 0)
            _ = mixer_video_set_playing(id, 1)
        }
        videoPlaying = true
        videoFraction = 0
    }

    func videoSeek(_ value: Double) {
        guard let id = fileVideoId else { return }
        if isRemote {
            _ = mixer_remote_video_seek(remoteHandle, id, Int64((max(0, min(1, value)) * 10_000_000).rounded()))
            videoFraction = value
            return
        }
        guard let info = copyVideoInfo(id) else { return }
        let duration = max(info.duration_hns, 1)
        let hns = Int64((max(0, min(1, value)) * Double(duration)).rounded())
        _ = mixer_video_seek(id, hns)
        videoFraction = value
    }

    func attachInputs() {
        for input in session.inputs where input.kind != .omt && input.kind != .ndi {
            attach(input)
        }
        for input in session.inputs where input.kind == .omt || input.kind == .ndi {
            attach(input)
        }
    }

    func attach(_ input: InputEntry) {
        switch input.kind {
        case .color, .bars:
            fail(
                mixer_define_generator(
                    input.id,
                    input.kind == .bars ? EIVIZ_GEN_BARS : EIVIZ_GEN_SOLID,
                    input.colorR,
                    input.colorG,
                    input.colorB,
                    1,
                    input.scroll ? 1 : 0
                ),
                "Define colour generator"
            )
            _ = mixer_generator_set_tone(input.id, input.toneHz, input.toneLevelDbfs)
        case .black:
            break
        case .still:
            if let path = input.pathOrAddress {
                guard FileManager.default.fileExists(atPath: path) else {
                    presentInputError(L10n.missingFile("Still load"))
                    return
                }
                MixerFFI.withCString(path) { cstr in
                    fail(mixer_load_still(input.id, cstr), "Still load")
                }
            }
        case .video:
            if let path = input.pathOrAddress {
                startVideoInput(
                    id: input.id,
                    path: path,
                    capture: 0,
                    loop: input.videoLoop,
                    playing: input.videoStartsPlaying,
                    frameBuffer: input.frameBufferFrames
                )
                videoTitle = input.listLabel(in: session, localFiles: !isRemote)
                videoPlaying = input.videoStartsPlaying
            }
        case .omt:
            if let address = input.pathOrAddress {
                MixerFFI.withCString(address) { cstr in
                    fail(
                        mixer_omt_connect(
                            input.id,
                            cstr,
                            input.useGpu ? 1 : 0,
                            max(1, min(8, input.frameBufferFrames)),
                            input.omtQuality.rawUInt
                        ),
                        "OMT connect"
                    )
                }
                _ = mixer_set_live_save(
                    input.id,
                    input.bandwidthSave.rawUInt,
                    input.keepFullOnMultiview ? EIVIZ_SAVE_FLAG_MULTIVIEW : 0
                )
            }
        case .ndi:
            if let address = input.pathOrAddress {
                MixerFFI.withCString(address) { cstr in
                    fail(
                        mixer_ndi_connect(
                            input.id,
                            cstr,
                            max(1, min(8, input.frameBufferFrames)),
                            input.ndiBandwidth.rawUInt
                        ),
                        "NDI connect"
                    )
                }
            }
        case .uvc:
            if let deviceId = input.pathOrAddress {
                startCapture(input)
            }
        case .mix:
            if input.mixTargetId != 0 {
                fail(
                    mixer_define_mix_input(
                        input.id,
                        input.mixTargetId,
                        input.mixSource.sourceKind,
                        max(1, min(8, input.frameBufferFrames)),
                        input.mixAudioBusId
                    ),
                    "Define Mix Input"
                )
            }
        case .audio:
            let deviceId = input.audioDeviceId.isEmpty ? (input.pathOrAddress ?? "") : input.audioDeviceId
            MixerFFI.withCString(deviceId) { device in
                MixerFFI.withCString(input.audioProcessExe) { exe in
                    MixerFFI.withCString(input.audioProcessAumid) { aumid in
                        fail(
                            mixer_audio_capture_start(
                                input.id,
                                input.audioDeviceKind.rawUInt,
                                device,
                                input.audioCaptureMode.rawUInt,
                                input.audioMapLeft,
                                input.audioMapRight,
                                exe,
                                aumid
                            ),
                            "Audio capture start"
                        )
                    }
                }
            }
        }
        _ = mixer_audio_set_input(input.id, audioMask(input), input.gain, input.mute ? 1 : 0)
    }

    func audioMask(_ input: InputEntry) -> UInt32 {
        input.kind == .mix ? 0 : (input.busMask == 0 ? 1 : input.busMask)
    }

    func startCapture(_ input: InputEntry) {
        guard let deviceId = input.pathOrAddress else { return }
        let start = {
            self.startVideoInput(
                id: input.id,
                path: deviceId,
                capture: 1,
                width: input.captureWidth,
                height: input.captureHeight,
                fpsNum: input.captureFpsNum,
                fpsDen: input.captureFpsDen,
                loop: false,
                playing: true,
                frameBuffer: input.frameBufferFrames,
            )
        }
        switch AVCaptureDevice.authorizationStatus(for: .video) {
        case .authorized:
            start()
        case .notDetermined:
            Task { @MainActor in
                if await AVCaptureDevice.requestAccess(for: .video) {
                    start()
                } else {
                    presentError(L10n.t("error.cameraDenied"), title: L10n.t("action.UVC start"))
                }
            }
        default:
            presentError(L10n.t("error.cameraDenied"), title: L10n.t("action.UVC start"))
        }
    }

    func pushScene(_ scene: SceneEntry) {
        if isRemote { return }
        var layers = scene.layers.map { layer -> EivizOverlayDesc in
            var desc = MixerFFI.emptyOverlay()
            desc.source_id = layer.inputId
            desc.rect = EivizRect(x: layer.x, y: layer.y, width: layer.width, height: layer.height)
            desc.crop = EivizRect(x: layer.cropX, y: layer.cropY, width: layer.cropWidth, height: layer.cropHeight)
            desc.opacity = layer.opacity
            desc.z = layer.z
            desc.audio_follow = layer.audioFollow ? 1 : 0
            desc.hidden = layer.hidden ? 1 : 0
            return desc
        }
        let count = UInt32(layers.count)
        layers.withUnsafeMutableBufferPointer { ptr in
            fail(
                mixer_define_scene(scene.gpuId, selectedUnit.width, selectedUnit.height, count, ptr.baseAddress),
                "Define scene"
            )
        }
    }

    func pushOverlays(forceEnabled: UUID? = nil, unitId: UInt64? = nil) {
        if isRemote {
            commitRemoteOverlays(unitId: unitId ?? selectedUnitId)
            return
        }
        let unit = session.units.first { $0.id == (unitId ?? selectedUnitId) } ?? selectedUnit
        var state = MixerFFI.emptyState()
        _ = mixer_unit_get_state(unit.id, &state)
        fillAux(&state, unit: unit, forceEnabled: forceEnabled)
        fail(mixer_unit_set_state(unit.id, &state), "Overlays")
    }

    func pushState(unitId: UInt64, program: UInt64, preview: UInt64, mix: Float, kind: UInt32) {
        var state = MixerFFI.emptyState()
        state.program_source = program
        state.preview_source = preview
        state.mix = mix
        state.transition_kind = kind
        let unit = session.units.first { $0.id == unitId } ?? selectedUnit
        fillAux(&state, unit: unit)
        fail(mixer_unit_set_state(unitId, &state), "Set Mixing Unit state")
    }

    func currentState(_ unitId: UInt64) -> EivizUnitState {
        var state = MixerFFI.emptyState()
        _ = mixer_unit_get_state(unitId, &state)
        let unit = session.units.first { $0.id == unitId } ?? selectedUnit
        fillAux(&state, unit: unit)
        return state
    }

    func fillAux(_ state: inout EivizUnitState, unit: MixingUnitEntry, forceEnabled: UUID? = nil) {
        let enabled = unit.overlays.filter { $0.enabled || $0.id == forceEnabled }.prefix(8)
        state.overlay_count = UInt32(enabled.count)
        for (index, slot) in enabled.enumerated() {
            var desc = MixerFFI.emptyOverlay()
            desc.source_id = slot.sceneGpuId
            desc.rect = EivizRect(x: slot.x, y: slot.y, width: slot.width, height: slot.height)
            desc.crop = EivizRect(x: slot.cropX, y: slot.cropY, width: slot.cropWidth, height: slot.cropHeight)
            desc.opacity = slot.opacity
            desc.z = slot.z
            desc.audio_follow = slot.audioFollow ? 1 : 0
            desc.hidden = slot.hidden ? 1 : 0
            MixerFFI.setOverlay(&state, index: index, desc)
        }
    }

    func unit(for id: UInt64?) -> MixingUnitEntry {
        guard let id else { return selectedUnit }
        return session.units.first { $0.id == id } ?? selectedUnit
    }

    func normalizePixelSortDefaults(_ unitId: UInt64) {
        guard let index = session.units.firstIndex(where: { $0.id == unitId }) else { return }
        for i in session.units[index].transitions.indices {
            TransitionCatalog.applyKindDefaults(&session.units[index].transitions[i])
        }
    }

    func resolvedCustomWgsl(_ preset: TransitionPreset) -> String {
        if preset.kind != EIVIZ_TRANSITION_CUSTOM {
            return ""
        }
        if let wgsl = preset.customWgsl, !wgsl.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            return wgsl
        }
        return CustomWgslEditor.template
    }

    func tbarPreset() -> TransitionPreset {
        tbarPreset(for: selectedUnit)
    }

    func tbarPreset(for unit: MixingUnitEntry) -> TransitionPreset {
        let list = unit.transitions
        guard !list.isEmpty else {
            return TransitionPreset(kind: EIVIZ_TRANSITION_CUT, durationValue: 1, swap: true)
        }
        return list[min(tbarPresetIndex, list.count - 1)]
    }

    func tick() {
        if handleMixerFatal() { return }
        if isRemote {
            return
        }
        var buffer = [EivizAudioPeak](repeating: MixerFFI.zeroed(), count: 32)
        let n = buffer.withUnsafeMutableBufferPointer { ptr in
            mixer_copy_audio_peaks(ptr.baseAddress, UInt32(ptr.count))
        }
        if n > 0 {
            var next: [UInt64: (Float, Float)] = [:]
            for peak in buffer.prefix(Int(n)) {
                next[peak.source_id] = (peak.left, peak.right)
            }
            peaks = next
        }
        var stats = MixerFFI.zeroed() as EivizMixerStats
        _ = mixer_copy_stats(&stats)
        FlipBudget.observeLost(stats.surface_lost)
        if stats.frame_budget_ms > 0 {
            let hud = HostResources.hud(renderMs: stats.render_ms, budgetMs: stats.frame_budget_ms)
            resourceText = hud.text
            warnText = hud.warn
        }
        if let id = fileVideoId, let info = copyVideoInfo(id) {
            videoPlaying = info.playing != 0
            videoTitle = session.inputs.first { $0.id == id }?.listLabel(in: session, localFiles: !isRemote) ?? videoTitle
            if info.duration_hns > 0 {
                videoFraction = Double(info.position_hns) / Double(info.duration_hns)
            }
        }
        syncAllUnitBuses()
        updateStatus()
    }

    func syncAllUnitBuses() {
        for unit in session.units {
            syncUnitBuses(unit.id)
        }
    }

    func syncUnitBuses(_ unitId: UInt64) {
        var state = MixerFFI.emptyState()
        guard mixer_unit_get_state(unitId, &state) == EIVIZ_OK else { return }
        applyBusSources(unitId: unitId, preview: state.preview_source, program: state.program_source)
        applyMixerMix(unitId: unitId, mix: state.mix)
    }

    func applyMixerMix(unitId: UInt64, mix value: Float) {
        if mixByUnit[unitId].map({ abs($0 - value) > 0.002 }) ?? true {
            var next = mixByUnit
            next[unitId] = value
            mixByUnit = next
        }
        guard unitId == selectedUnitId, !tbarLocked, !tbarDragging, !tbarLatching else { return }
        guard abs(mix - value) > 0.002 else { return }
        tbarLatching = true
        mix = value
        tbarLatching = false
    }

    func applyBusSources(unitId: UInt64, preview: UInt64, program: UInt64) {
        if previewByUnit[unitId] != preview || programByUnit[unitId] != program {
            var nextPreview = previewByUnit
            var nextProgram = programByUnit
            nextPreview[unitId] = preview
            nextProgram[unitId] = program
            previewByUnit = nextPreview
            programByUnit = nextProgram
        }
        if let index = session.units.firstIndex(where: { $0.id == unitId }) {
            if let scene = session.scenes.first(where: { $0.gpuId == preview }) {
                session.units[index].previewSceneId = scene.id
            }
            if let scene = session.scenes.first(where: { $0.gpuId == program }) {
                session.units[index].programSceneId = scene.id
            }
        }
    }

    func mixUnitUses(_ unit: MixingUnitEntry, sourceId: UInt64) -> Bool {
        if unit.overlays.contains(where: { $0.sceneGpuId == sourceId }) {
            return true
        }
        var state = EivizUnitState()
        guard mixer_unit_get_state(unit.id, &state) == EIVIZ_OK else {
            return false
        }
        if state.program_source == sourceId || state.preview_source == sourceId {
            return true
        }
        return session.scenes.contains { scene in
            (scene.gpuId == state.program_source || scene.gpuId == state.preview_source)
                && scene.layers.contains { $0.inputId == sourceId }
        }
    }

    func updateStatus() {
        if isRemote {
            return
        }
        let unit = selectedUnit
        status = "\(unit.width)x\(unit.height) \(unit.fpsLabel)   \(unit.name)"
    }

    func refreshRemoteWarn() {
        let next: String
        if remoteHandle == 0 {
            next = L10n.t("msg.remoteIdle")
        } else if !remoteConnected {
            next = L10n.t("msg.remoteDisconnected")
        } else if !remoteError.isEmpty {
            next = remoteError
        } else if remoteLag {
            next = L10n.t("msg.remoteResync")
        } else if videoUnavailable {
            next = L10n.t("msg.videoUnavailable")
        } else if let error = remoteVideoError() {
            next = error
        } else if !remoteVideoReady() {
            next = L10n.t("msg.videoWaiting")
        } else {
            next = ""
        }
        if warnText != next {
            warnText = next
        }
    }

    func remoteVideoReady() -> Bool {
        if AppPrefs.shared.remoteVideoLayout == .multiview {
            return MixerFFI.sourceStatus(MixerRemote.mainMultiviewSourceId).hasVideo
        }
        return MixerFFI.sourceStatus(MixerRemote.previewSourceId).hasVideo
            && MixerFFI.sourceStatus(MixerRemote.programSourceId).hasVideo
    }

    func remoteVideoError() -> String? {
        if AppPrefs.shared.remoteVideoLayout == .multiview {
            let error = MixerFFI.sourceErrorText(MixerRemote.mainMultiviewSourceId)
            return error.isEmpty ? nil : error
        }
        let preview = MixerFFI.sourceErrorText(MixerRemote.previewSourceId)
        if !preview.isEmpty { return preview }
        let program = MixerFFI.sourceErrorText(MixerRemote.programSourceId)
        if !program.isEmpty { return program }
        return nil
    }

    func handleMixerFatal() -> Bool {
        if fatalHandled { return true }
        let fatal = MixerFFI.takeFatalText()
        guard !fatal.isEmpty else { return false }
        fatalHandled = true
        meterTimer?.invalidate()
        saveRecoveredSession()
        let alert = NSAlert()
        alert.messageText = L10n.t("error.mixerFatal")
        alert.alertStyle = .critical
        alert.addButton(withTitle: L10n.t("dialog.ok"))
        AppKitDialog.elevate(alert)
        alert.runModal()
        NSApplication.shared.terminate(nil)
        return true
    }

    func saveRecoveredSession() {
        let dir = FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Application Support/eiviz", isDirectory: true)
        do {
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            let url = dir.appendingPathComponent("recovered-session.eivz")
            session.selectedUnitId = selectedUnitId
            session.settings.lastSessionPath = nil
            let json = try SessionFile.encode(session)
            let saved = MixerFFI.withCString(url.path) { path in
                json.withUnsafeBytes { ptr in
                    mixer_session_save(
                        path,
                        ptr.bindMemory(to: UInt8.self).baseAddress,
                        json.count
                    )
                }
            }
            if saved != EIVIZ_OK {
                HostLog.write("ERROR", "recovered session save failed: \(saved)")
            }
        } catch {
            HostLog.write("ERROR", "recovered session save failed: \(error)")
        }
    }

    func presentError(_ message: String, title: String) {
        let alert = NSAlert()
        alert.messageText = title
        alert.informativeText = message
        alert.alertStyle = .warning
        alert.addButton(withTitle: L10n.t("dialog.ok"))
        AppKitDialog.elevate(alert)
        alert.runModal()
    }

    func presentInputError(_ message: String, editing: Bool = false) {
        presentError(message, title: L10n.t(editing ? "msg.editInput" : "msg.addInput"))
    }

    func addCatalogTag(input: Bool) {
        guard let name = TextPrompt.ask(title: L10n.t("tag.add"), prompt: L10n.t("tag.name"), initial: "") else { return }
        var catalog = input ? session.inputTags : session.sceneTags
        let result = TagCatalog.tryAdd(&catalog, name)
        if !result.ok {
            presentError(L10n.t("tag.duplicate"), title: L10n.t("tag.add"))
            return
        }
        if input {
            session.inputTags = catalog
        } else {
            session.sceneTags = catalog
        }
    }

    func promptTagForCheck(input: Bool) -> String? {
        guard let name = TextPrompt.ask(title: L10n.t("tag.add"), prompt: L10n.t("tag.name"), initial: "") else { return nil }
        var catalog = input ? session.inputTags : session.sceneTags
        let result = TagCatalog.tryAdd(&catalog, name)
        if result.normalized.isEmpty {
            return nil
        }
        if result.ok {
            if input {
                session.inputTags = catalog
            } else {
                session.sceneTags = catalog
            }
        }
        return result.normalized
    }

    func renameCatalogTag(input: Bool, current: String) {
        guard let name = TextPrompt.ask(title: L10n.t("tag.rename"), prompt: L10n.t("tag.name"), initial: current) else { return }
        var catalog = input ? session.inputTags : session.sceneTags
        var owners = input ? session.inputs.map(\.tags) : session.scenes.map(\.tags)
        if !TagCatalog.rename(&catalog, owners: &owners, current: current, next: name) {
            presentError(L10n.t("tag.duplicate"), title: L10n.t("tag.rename"))
            return
        }
        if input {
            session.inputTags = catalog
            for i in session.inputs.indices {
                session.inputs[i].tags = owners[i]
            }
            if inputFilter.mode == .tag && inputFilter.tag == current {
                inputFilter = .tag(name)
            }
        } else {
            session.sceneTags = catalog
            for i in session.scenes.indices {
                session.scenes[i].tags = owners[i]
            }
            if sceneFilter.mode == .tag && sceneFilter.tag == current {
                sceneFilter = .tag(name)
            }
        }
    }

    func deleteCatalogTag(input: Bool, name: String) {
        guard TextPrompt.confirm(title: L10n.t("tag.delete"), message: L10n.format("tag.deleteConfirm", name)) else { return }
        var catalog = input ? session.inputTags : session.sceneTags
        var owners = input ? session.inputs.map(\.tags) : session.scenes.map(\.tags)
        TagCatalog.remove(&catalog, owners: &owners, name: name)
        if input {
            session.inputTags = catalog
            for i in session.inputs.indices {
                session.inputs[i].tags = owners[i]
            }
            if inputFilter.mode == .tag && inputFilter.tag == name {
                inputFilter = .all
            }
        } else {
            session.sceneTags = catalog
            for i in session.scenes.indices {
                session.scenes[i].tags = owners[i]
            }
            if sceneFilter.mode == .tag && sceneFilter.tag == name {
                sceneFilter = .all
            }
        }
    }

    func mergeInputTags(_ tags: [String]) {
        var catalog = session.inputTags
        TagCatalog.mergeInto(&catalog, tags)
        session.inputTags = catalog
    }

    func mergeSceneTags(_ tags: [String]) {
        var catalog = session.sceneTags
        TagCatalog.mergeInto(&catalog, tags)
        session.sceneTags = catalog
    }

    func surfaceRoleMainMultiview() -> SurfaceRole {
        .monitor(
            monitorId: MixerRemote.mainMultiviewMonitor,
            sourceId: remoteMultiviewLive ? MixerRemote.mainMultiviewSourceId : EIVIZ_SRC_BLACK
        )
    }

    func surfaceRole(kind: UInt32, unitId: UInt64? = nil) -> SurfaceRole {
        if isRemote {
            if kind == EIVIZ_OUTPUT_PREVIEW {
                return .monitor(
                    monitorId: MixerRemote.previewMonitor,
                    sourceId: remotePreviewLive ? MixerRemote.previewSourceId : EIVIZ_SRC_BLACK
                )
            }
            return .monitor(
                monitorId: MixerRemote.programMonitor,
                sourceId: remoteProgramLive ? MixerRemote.programSourceId : EIVIZ_SRC_BLACK
            )
        }
        let unit = unitId ?? selectedUnitId
        return .unit(unitId: unit, kind: kind)
    }

    func surfaceRoleMultiview(_ layout: MultiviewLayout) -> SurfaceRole {
        guard isRemote else {
            return .monitor(monitorId: layout.monitorId, sourceId: layout.gpuId)
        }
        if let source = remotePublishedMultiview(layout.gpuId) {
            return .monitor(monitorId: MixerRemote.multiviewMonitor ^ (layout.id << 8), sourceId: source)
        }
        return .monitor(monitorId: 0, sourceId: 0)
    }

    func remoteMultiviewUnavailable(_ layout: MultiviewLayout) -> Bool {
        isRemote && remotePublishedMultiview(layout.gpuId) == nil
    }

    func commitRemoteScene(_ scene: SceneEntry) -> Bool {
        mutateRemote(MixerRemote.upsertScene(scene))
    }

    func commitRemoteOverlays(unitId: UInt64) {
        guard let unit = session.units.first(where: { $0.id == unitId }) else { return }
        for (index, slot) in unit.overlays.enumerated() {
            _ = mutateRemote(
                MixerRemote.setOverlaySlot(unitId: unitId, index: UInt32(index), slot: slot),
                applyDocument: false
            )
        }
    }

    func discoverInput(kind: String, query: String = "") -> String {
        guard isRemote, remoteHandle != 0 else {
            return kind == "uvc" || kind == "uvcModes" ? "[]" : ""
        }
        return MixerRemote.discover(remoteHandle, kind: kind, query: query)
    }

    @discardableResult
    func mutateRemote(_ json: String, applyDocument: Bool = true) -> Bool {
        let handle = remoteHandle
        let gate = remoteMutateGate
        Task.detached { [weak self] in
            let (code, nextRevision) = gate.mutate(handle: handle, json: json)
            await MainActor.run {
                guard let self else { return }
                if self.remoteRevision < nextRevision {
                    self.remoteRevision = nextRevision
                }
                self.pollRemote(force: applyDocument, applyDocument: applyDocument)
                if code != 0 {
                    self.presentError(self.remoteMutateError(), title: L10n.t("chrome.settings"))
                }
            }
        }
        return true
    }

    func remoteMutateError() -> String {
        let statusJson = MixerRemote.status(remoteHandle)
        if let data = statusJson.data(using: .utf8),
           let root = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
           let error = root["error"] as? String,
           !error.isEmpty
        {
            return error
        }
        return L10n.t("msg.revisionConflict")
    }

    @discardableResult
    func mutateRemoteSettings() -> Bool {
        mutateRemote(MixerRemote.setSettings(session))
    }

    func pollRemote(force: Bool = false, applyDocument: Bool = true) {
        guard remoteHandle != 0 else {
            if remoteConnected { remoteConnected = false }
            refreshRemoteWarn()
            return
        }
        let statusJson = MixerRemote.status(remoteHandle)
        var connected = false
        var lag = false
        var error = ""
        var epoch = remoteEpoch
        var documentRevision = remoteRevision
        var sequence = remoteLiveSequence
        if let data = statusJson.data(using: .utf8),
           let root = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        {
            connected = root["connected"] as? Bool ?? false
            lag = root["lag"] as? Bool ?? false
            error = root["error"] as? String ?? ""
            if let value = root["documentRevision"] as? NSNumber, value.uint64Value != 0 {
                documentRevision = value.uint64Value
                if remoteRevision != documentRevision {
                    remoteRevision = documentRevision
                }
            } else if let revision = root["revision"] as? NSNumber {
                let value = revision.uint64Value
                documentRevision = value
                if remoteRevision != value {
                    remoteRevision = value
                }
            }
            if let value = root["sequence"] as? NSNumber {
                sequence = value.uint64Value
            }
            if let value = root["epoch"] as? String {
                epoch = value
            }
        }
        if remoteConnected != connected {
            remoteConnected = connected
        }
        remoteLag = lag
        remoteError = error
        let nextStatus: String
        if !connected {
            nextStatus = L10n.t("msg.remoteDisconnected")
        } else if !error.isEmpty {
            nextStatus = error
        } else if lag {
            nextStatus = L10n.t("msg.remoteResync")
        } else {
            nextStatus = L10n.format("msg.remoteConnected", "\(remoteRevision)")
        }
        if status != nextStatus {
            status = nextStatus
        }
        if force || sequence != remoteLiveSequence {
            let liveJson = MixerRemote.live(remoteHandle)
            if !liveJson.isEmpty {
                remoteLiveSequence = sequence
                if let mixValue = MixerRemote.mix(from: liveJson, unitId: selectedUnitId), !tbarDragging, !tbarLocked, mix != mixValue {
                    mix = mixValue
                }
                applyRemoteLiveBuses(liveJson)
            }
        }
        let docChanged = force || epoch != remoteEpoch || documentRevision != remotePulledDocumentRevision
        if connected, docChanged {
            if applyDocument {
                if let data = MixerRemote.snapshot(remoteHandle).data(using: .utf8),
                   let loaded = try? SessionFile.decode(data)
                {
                    let keepUnit = selectedUnitId
                    let keepScene = selectedSceneId
                    let keepInput = selectedInputId
                    session = loaded
                    selectedUnitId = session.units.contains(where: { $0.id == keepUnit }) ? keepUnit : (session.units.first?.id ?? 1)
                    selectedSceneId = keepScene
                    selectedInputId = keepInput
                    remoteEpoch = epoch
                    remotePulledDocumentRevision = documentRevision
                    syncPublishedVideo()
                }
            } else {
                remoteEpoch = epoch
                remotePulledDocumentRevision = documentRevision
            }
        }
        remoteMutateGate.note(remoteRevision)
        refreshRemoteWarn()
    }

    func applyRemoteLiveBuses(_ liveJson: String) {
        guard let data = liveJson.data(using: .utf8),
              let root = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return }
        if let units = root["units"] as? [String: Any] {
            for unit in session.units {
                guard let live = units[String(unit.id)] as? [String: Any] else { continue }
                let preview = (live["previewSource"] as? NSNumber)?.uint64Value ?? 0
                let program = (live["programSource"] as? NSNumber)?.uint64Value ?? 0
                applyBusSources(unitId: unit.id, preview: preview, program: program)
            }
        }
        if let list = root["peaks"] as? [[String: Any]] {
            var next: [UInt64: (Float, Float)] = [:]
            for peak in list {
                let id = (peak["id"] as? NSNumber)?.uint64Value ?? 0
                let left = (peak["left"] as? NSNumber)?.floatValue ?? 0
                let right = (peak["right"] as? NSNumber)?.floatValue ?? 0
                next[id] = (left, right)
            }
            peaks = next
        }
    }

    func publishedOutputs() -> [OutputEntry] {
        session.outputs.filter {
            $0.enabled
                && ($0.transport == .omt || $0.transport == .ndi)
                && ($0.sourceKind == .muPreview || $0.sourceKind == .muProgram || $0.sourceKind == .multiview)
        }
    }

    func remotePublishedSource(_ kind: OutputSourceKind, unitId: UInt64) -> OutputEntry? {
        let matches = publishedOutputs().filter { $0.sourceKind == kind && $0.unitId == unitId }
        guard matches.count == 1 else { return nil }
        return matches.first
    }

    func remotePublishedUnique(_ kind: OutputSourceKind) -> OutputEntry? {
        let matches = publishedOutputs().filter { $0.sourceKind == kind }
        guard matches.count == 1 else { return nil }
        return matches.first
    }

    func remotePublishedMultiview(_ layoutGpuId: UInt64) -> UInt64? {
        let matches = publishedOutputs().filter { $0.sourceKind == .multiview && $0.sourceId == layoutGpuId }
        guard matches.count == 1, let output = matches.first else { return nil }
        return MixerRemote.sourceBase | output.id
    }

    func remoteVideoItems() -> [RemoteVideoItem] {
        var items = [RemoteVideoItem(transport: .omt, address: "", label: L10n.t("chrome.videoNone"))]
        var seen = Set<String>()
        func add(_ transport: OutputTransport, _ address: String) {
            let trimmed = address.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !trimmed.isEmpty else { return }
            let key = "\(transport.rawValue):\(trimmed)"
            guard seen.insert(key).inserted else { return }
            let prefix = transport == .ndi ? "NDI" : "OMT"
            items.append(RemoteVideoItem(transport: transport, address: trimmed, label: "\(prefix)  \(trimmed)"))
        }
        for line in discoveredOmt { add(.omt, line) }
        for line in discoveredNdi { add(.ndi, line) }
        return items
    }

    func selectedRemoteVideo(preview: Bool) -> RemoteVideoItem {
        let prefs = AppPrefs.shared
        let address = (preview ? prefs.previewVideoAddress : prefs.programVideoAddress)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        let raw = preview ? prefs.previewVideoTransport : prefs.programVideoTransport
        let transport: OutputTransport = raw.uppercased() == "NDI" ? .ndi : .omt
        if !address.isEmpty {
            return resolveRemoteVideo(RemoteVideoItem(
                transport: transport,
                address: address,
                label: "\(transport == .ndi ? "NDI" : "OMT")  \(address)"
            ))
        }
        if let output = remotePublishedSource(preview ? .muPreview : .muProgram, unitId: selectedUnitId) {
            return resolveRemoteVideo(RemoteVideoItem(
                transport: output.transport,
                address: output.name,
                label: "\(output.transport == .ndi ? "NDI" : "OMT")  \(output.name)"
            ))
        }
        return RemoteVideoItem(transport: .omt, address: "", label: L10n.t("chrome.videoNone"))
    }

    func selectedRemoteMultiview() -> RemoteVideoItem {
        let prefs = AppPrefs.shared
        let address = prefs.multiviewVideoAddress.trimmingCharacters(in: .whitespacesAndNewlines)
        let transport: OutputTransport = prefs.multiviewVideoTransport.uppercased() == "NDI" ? .ndi : .omt
        if !address.isEmpty {
            return resolveRemoteVideo(RemoteVideoItem(
                transport: transport,
                address: address,
                label: "\(transport == .ndi ? "NDI" : "OMT")  \(address)"
            ))
        }
        if let output = remotePublishedUnique(.multiview) {
            return resolveRemoteVideo(RemoteVideoItem(
                transport: output.transport,
                address: output.name,
                label: "\(output.transport == .ndi ? "NDI" : "OMT")  \(output.name)"
            ))
        }
        return RemoteVideoItem(transport: .omt, address: "", label: L10n.t("chrome.videoNone"))
    }

    func setRemoteVideoLayout(_ layout: RemoteVideoLayout) {
        guard AppPrefs.shared.remoteVideoLayout != layout else { return }
        AppPrefs.shared.remoteVideoLayout = layout
        AppPrefs.shared.save()
        remotePreviewKey = ""
        remoteProgramKey = ""
        remoteMultiviewKey = ""
        syncPublishedVideo()
        bumpSurfaceEpoch()
    }

    func setRemoteVideo(preview: Bool, item: RemoteVideoItem) {
        if preview {
            AppPrefs.shared.previewVideoAddress = item.address
            AppPrefs.shared.previewVideoTransport = item.transport == .ndi ? "NDI" : "OMT"
        } else {
            AppPrefs.shared.programVideoAddress = item.address
            AppPrefs.shared.programVideoTransport = item.transport == .ndi ? "NDI" : "OMT"
        }
        AppPrefs.shared.save()
        syncPublishedVideo()
        bumpSurfaceEpoch()
    }

    func setRemoteMultiview(_ item: RemoteVideoItem) {
        AppPrefs.shared.multiviewVideoAddress = item.address
        AppPrefs.shared.multiviewVideoTransport = item.transport == .ndi ? "NDI" : "OMT"
        AppPrefs.shared.save()
        syncPublishedVideo()
        bumpSurfaceEpoch()
    }

    func remoteVideoBindKey(_ item: RemoteVideoItem) -> String {
        if item.transport == .ndi {
            return "\(item.transport.rawValue):\(item.address)"
        }
        return "\(item.transport.rawValue):\(item.address):gpu=\(AppPrefs.shared.remoteOmtUseGpu)"
    }

    func syncPublishedVideo() {
        let layout = AppPrefs.shared.remoteVideoLayout
        if layout == .multiview {
            if remotePreviewLive {
                _ = mixer_destroy_source(MixerRemote.previewSourceId)
                remotePreviewLive = false
                remotePreviewKey = ""
            }
            if remoteProgramLive {
                _ = mixer_destroy_source(MixerRemote.programSourceId)
                remoteProgramLive = false
                remoteProgramKey = ""
            }
            let item = selectedRemoteMultiview()
            let key = remoteVideoBindKey(item)
            if key != remoteMultiviewKey {
                remoteMultiviewKey = key
                remoteMultiviewLive = connectRemoteChoice(MixerRemote.mainMultiviewSourceId, item)
            }
        } else {
            if remoteMultiviewLive {
                _ = mixer_destroy_source(MixerRemote.mainMultiviewSourceId)
                remoteMultiviewLive = false
                remoteMultiviewKey = ""
            }
            let preview = selectedRemoteVideo(preview: true)
            let program = selectedRemoteVideo(preview: false)
            let previewKey = remoteVideoBindKey(preview)
            let programKey = remoteVideoBindKey(program)
            if previewKey != remotePreviewKey {
                remotePreviewKey = previewKey
                remotePreviewLive = connectRemoteChoice(MixerRemote.previewSourceId, preview)
            }
            if programKey != remoteProgramKey {
                remoteProgramKey = programKey
                remoteProgramLive = connectRemoteChoice(MixerRemote.programSourceId, program)
            }
        }
        let outputs = publishedOutputs().filter { $0.sourceKind == .multiview }
        var keep: [UInt64: UInt64] = [:]
        for output in outputs {
            let id = MixerRemote.sourceBase | output.id
            let item = RemoteVideoItem(transport: output.transport, address: output.name, label: output.name)
            let key = remoteVideoBindKey(item)
            keep[output.id] = id
            if remoteReceiveIds[output.id] == id, remoteReceiveKeys[output.id] == key {
                continue
            }
            if connectRemoteChoice(id, item) {
                remoteReceiveIds[output.id] = id
                remoteReceiveKeys[output.id] = key
            }
        }
        for (outputId, sourceId) in remoteReceiveIds where keep[outputId] == nil {
            _ = mixer_destroy_source(sourceId)
            remoteReceiveIds.removeValue(forKey: outputId)
            remoteReceiveKeys.removeValue(forKey: outputId)
        }
        let unavailable = layout == .multiview
            ? !remoteMultiviewLive
            : !remotePreviewLive || !remoteProgramLive
        if videoUnavailable != unavailable {
            videoUnavailable = unavailable
        }
        refreshRemoteWarn()
    }

    @discardableResult
    func connectRemoteChoice(_ id: UInt64, _ item: RemoteVideoItem) -> Bool {
        let resolved = resolveRemoteVideo(item)
        _ = mixer_destroy_source(id)
        guard canConnectRemoteVideo(resolved) else { return false }
        let code: Int32
        if resolved.transport == .ndi {
            code = MixerFFI.withCString(resolved.address) { mixer_ndi_connect(id, $0, 1, 0) }
        } else {
            code = MixerFFI.withCString(resolved.address) {
                mixer_omt_connect(id, $0, AppPrefs.shared.remoteOmtUseGpu ? 1 : 0, 1, 0)
            }
        }
        return code == 0
    }

    func canConnectRemoteVideo(_ item: RemoteVideoItem) -> Bool {
        let address = item.address.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !address.isEmpty else { return false }
        if item.transport == .ndi { return true }
        return address.contains("://")
    }

    func resolveRemoteVideo(_ item: RemoteVideoItem) -> RemoteVideoItem {
        let address = item.address.trimmingCharacters(in: .whitespacesAndNewlines)
        if address.isEmpty {
            return item
        }
        if item.transport == .ndi || address.contains("://") {
            return item
        }
        let pool = item.transport == .ndi ? discoveredNdi : discoveredOmt
        let matches = pool.filter { addressRefersTo($0, name: address) }
        guard matches.count == 1, let found = matches.first else {
            return RemoteVideoItem(transport: item.transport, address: "", label: L10n.t("chrome.videoNone"))
        }
        let prefix = item.transport == .ndi ? "NDI" : "OMT"
        return RemoteVideoItem(transport: item.transport, address: found, label: "\(prefix)  \(found)")
    }

    func addressRefersTo(_ discovered: String, name: String) -> Bool {
        if discovered.compare(name, options: .caseInsensitive) == .orderedSame {
            return true
        }
        if let slash = discovered.lastIndex(of: "/"), slash < discovered.index(before: discovered.endIndex) {
            let tail = discovered[discovered.index(after: slash)...]
            if tail.compare(name, options: .caseInsensitive) == .orderedSame {
                return true
            }
        }
        return discovered.range(of: "(\(name))", options: .caseInsensitive) != nil
    }

    @discardableResult
    func fail(_ code: Int32, _ action: String) -> Bool {
        guard let message = MixerFFI.check(code, action) else { return true }
        presentError(message, title: L10n.t("action.\(action)"))
        return false
    }

    private struct SessionHistoryEntry: Decodable {
        let index: UInt32
        let unixMs: UInt64
        let revision: UInt64
    }

    private enum HistoryPick {
        case cancel
        case latest
        case revision(UInt32)
    }

    private static func parseHistory(_ bytes: [UInt8]) -> [SessionHistoryEntry]? {
        let data = Data(bytes)
        if data.isEmpty || data == Data("[]".utf8) {
            return []
        }
        return try? JSONDecoder().decode([SessionHistoryEntry].self, from: data)
    }

    private func pickHistory(_ entries: [SessionHistoryEntry]) -> HistoryPick {
        let popup = NSPopUpButton(frame: NSRect(x: 0, y: 0, width: 380, height: 24), pullsDown: false)
        popup.addItem(withTitle: L10n.t("history.latest"))
        popup.lastItem?.tag = -1
        let formatter = DateFormatter()
        formatter.dateStyle = .short
        formatter.timeStyle = .short
        for (offset, entry) in entries.enumerated() {
            let when = formatter.string(
                from: Date(timeIntervalSince1970: TimeInterval(entry.unixMs) / 1000)
            )
            popup.addItem(
                withTitle: L10n.format("history.entry", "\(offset + 1)", when)
            )
            popup.lastItem?.tag = Int(entry.index)
        }
        popup.selectItem(at: 0)
        let alert = NSAlert()
        alert.messageText = L10n.t("history.title")
        alert.alertStyle = .informational
        alert.accessoryView = popup
        alert.addButton(withTitle: L10n.t("history.open"))
        alert.addButton(withTitle: L10n.t("history.cancel"))
        AppKitDialog.elevate(alert)
        guard alert.runModal() == .alertFirstButtonReturn else { return .cancel }
        let tag = popup.selectedTag()
        return tag < 0 ? .latest : .revision(UInt32(tag))
    }
}
