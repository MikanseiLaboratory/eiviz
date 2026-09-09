import AppKit
import AVFoundation
import Combine
import Darwin
import EivizMixer
import EivizRemote
import Foundation
import SwiftUI
import UniformTypeIdentifiers

extension UTType {
    static let eivizSession = UTType(exportedAs: "jp.mikanseilaboratory.eiviz.session")
    static let eivizSessionExport = UTType(exportedAs: "jp.mikanseilaboratory.eiviz.session-export")
}

struct RemoteVideoItem: Hashable {
    var transport: OutputTransport
    var address: String
    var label: String

    static func == (lhs: RemoteVideoItem, rhs: RemoteVideoItem) -> Bool {
        lhs.transport == rhs.transport && lhs.address == rhs.address
    }

    func hash(into hasher: inout Hasher) {
        hasher.combine(transport)
        hasher.combine(address)
    }
}

@MainActor
final class MixerController: ObservableObject {
    @Published var session = MixerSessionData.default()
    @Published var selectedUnitId: UInt64 = 1
    @Published var selectedSceneId: UInt64?
    @Published var selectedInputId: UInt64?
    @Published var mix: Float = 0
    @Published var mixByUnit: [UInt64: Float] = [:]
    @Published var tbarLocked = false
    var tbarDragging = false
    @Published var tbarPresetIndex = 0
    @Published var status = ""
    @Published var errorText = ""
    @Published var warnText = ""
    @Published var resourceText = ""
    @Published var peaks: [UInt64: (Float, Float)] = [:]
    @Published var overlayOn: [UUID: Bool] = [:]
    @Published var videoFraction: Double = 0
    @Published var videoPlaying = false
    @Published var videoTitle = ""
    @Published var showSettings = false
    @Published var settingsCategory = 0
    @Published var showPreferences = false
    @Published var showAddInput = false
    @Published var editingInput: InputEntry?
    @Published var showMixingUnit = false
    @Published var showSceneEditor = false
    @Published var showOverlay = false
    @Published var showMultiview = false
    @Published var showMultiviewSlots = false
    @Published var showResources = false
    @Published var showLogs = false
    @Published var showConnect = false
    @Published var editingUnit: MixingUnitEntry?
    @Published var editingScene: SceneEntry?
    @Published var openMultiview: MultiviewLayout?
    @Published var expandedTransitions: Set<UUID> = []
    @Published var kindMenuGroup: [UUID: TransitionGroup] = [:]
    @Published var inputFilter = ListFilter.all
    @Published var sceneFilter = ListFilter.all
    @Published var surfaceEpoch: UInt64 = 0
    @Published var isRemote = false
    @Published var remoteConnected = false
    @Published var remoteRevision: UInt64 = 0
    @Published var remoteVideoCatalogEpoch: UInt64 = 0
    @Published var videoUnavailable = false

    var booted = false
    var pendingOpenPath: String?
    var fatalHandled = false
    var tbarLatching = false
    var meterTimer: Timer?
    var mixTimer: Timer?
    @Published var previewByUnit: [UInt64: UInt64] = [:]
    @Published var programByUnit: [UInt64: UInt64] = [:]
    var inputPreviewWindows: [UInt64: NSWindow] = [:]
    var inputPreviewControllers: [UInt64: NSWindowController] = [:]
    let inputPreviewCloser = InputPreviewCloser()
    var audioInputWindows: [UInt64: NSWindow] = [:]
    var audioInputControllers: [UInt64: NSWindowController] = [:]
    let audioInputCloser = AudioInputCloser()
    var switcherWindows: [UInt64: NSWindow] = [:]
    let switcherCloser = SwitcherCloser()
    var multiviewWindows: [UInt64: NSWindow] = [:]
    let multiviewCloser = SwitcherCloser()
    var videoRoles: [UInt64: (program: Bool, preview: Bool)] = [:]
    var remoteHandle: Int32 = 0
    var remoteReceiveIds: [UInt64: UInt64] = [:]
    var remoteReceiveKeys: [UInt64: String] = [:]
    var remoteEpoch = ""
    var remotePulledDocumentRevision: UInt64 = 0
    var remoteLiveSequence: UInt64 = 0
    var remoteLag = false
    var remoteError = ""
    var remotePreviewKey = ""
    var remoteProgramKey = ""
    var remoteMultiviewKey = ""
    var remotePreviewLive = false
    var remoteProgramLive = false
    var remoteMultiviewLive = false
    var discoveredOmt: [String] = []
    var discoveredNdi: [String] = []
    var remoteDiscoverTask: Task<Void, Never>?
    let remoteMutateGate = RemoteMutateGate()

    var selectedUnit: MixingUnitEntry {
        session.units.first { $0.id == selectedUnitId } ?? session.units[0]
    }

    func boot() {
        guard !booted else { return }
        isRemote = HostRole.isRemote
        guard mixer_ping() == 0x4549_5649 else {
            presentError(L10n.t("error.abiMismatch"), title: L10n.t("action.Metal mixer initialization"))
            return
        }
        if isRemote {
            bootRemote()
            return
        }
        guard fail(mixer_create_with_backend(AppPrefs.shared.renderer.createAbi, 0, session.settings.masterFpsNum, session.settings.masterFpsDen), "Metal mixer initialization") else {
            return
        }
        fail(mixer_set_frame_buffer(min(8, max(1, session.settings.frameBufferFrames))), "Set frame buffer")
        fail(mixer_set_rebar_optimization(session.settings.rebarOptimizationEnabled ? 1 : 0), "Set ReBAR optimization")
        fail(mixer_set_ndi_gpu_upload(session.settings.ndiGpuUploadEnabled ? 1 : 0), "Set NDI GPU upload")
        GpuPresentStore.load()
        FlipBudget.configure(session.settings.flipSwapchainLimit)
        applyBusColors()
        applySession()
        bumpSurfaceEpoch()
        startTimers()
        booted = true
        applyVmixApi()
        publishSession()
        updateStatus()
        openPendingSession()
    }

    private func startTimers() {
        meterTimer = Timer.scheduledTimer(withTimeInterval: 0.05, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.tick() }
        }
        mixTimer = Timer.scheduledTimer(withTimeInterval: 1.0 / 60.0, repeats: true) { [weak self] _ in
            Task { @MainActor in
                if self?.isRemote == true {
                    self?.pollRemote()
                } else {
                    ThumbPump.poll()
                    self?.syncAllUnitBuses()
                }
            }
        }
    }

    private func startRemoteDiscover() {
        remoteDiscoverTask?.cancel()
        remoteDiscoverTask = Task.detached { [weak self] in
            while !Task.isCancelled {
                let omt = MixerFFI.discoverOmt()
                let ndi = MixerFFI.discoverNdi()
                await MainActor.run {
                    self?.applyDiscoveredVideo(omt: omt, ndi: ndi)
                }
                try? await Task.sleep(nanoseconds: 2_000_000_000)
            }
        }
    }

    private func applyDiscoveredVideo(omt: [String], ndi: [String]) {
        let changed = omt != discoveredOmt || ndi != discoveredNdi
        if changed {
            discoveredOmt = omt
            discoveredNdi = ndi
            remoteVideoCatalogEpoch &+= 1
        }
        guard isRemote else { return }
        if AppPrefs.shared.remoteVideoLayout == .multiview {
            if !remoteMultiviewLive {
                remoteMultiviewKey = ""
            }
            if changed || !remoteMultiviewLive {
                syncPublishedVideo()
            }
            return
        }
        if !remotePreviewLive {
            remotePreviewKey = ""
        }
        if !remoteProgramLive {
            remoteProgramKey = ""
        }
        if changed || !remotePreviewLive || !remoteProgramLive {
            syncPublishedVideo()
        }
    }

    private func bootRemote() {
        guard fail(mixer_create_with_backend(AppPrefs.shared.renderer.createAbi, 0, 60, 1), "Metal mixer initialization") else {
            return
        }
        _ = mixer_define_generator(EIVIZ_SRC_BLACK, EIVIZ_GEN_SOLID, 0, 0, 0, 1, 0)
        FlipBudget.configure(0)
        GpuPresentStore.load()
        startRemoteDiscover()
        startTimers()
        booted = true
        status = L10n.t("msg.remoteIdle")
        refreshRemoteWarn()
    }

    func connectRemote(url: String, token: String) {
        let endpoint = url.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !endpoint.isEmpty else { return }
        if remoteHandle != 0 {
            _ = mixer_remote_close(remoteHandle)
            remoteHandle = 0
        }
        remoteConnected = false
        remotePreviewKey = ""
        remoteProgramKey = ""
        remoteMultiviewKey = ""
        remotePreviewLive = false
        remoteProgramLive = false
        remoteMultiviewLive = false
        remoteEpoch = ""
        remotePulledDocumentRevision = 0
        remoteLiveSequence = 0
        let handle = MixerFFI.withCString(endpoint) { urlPtr in
            MixerFFI.withCString(token) { tokenPtr in
                mixer_remote_open(urlPtr, tokenPtr)
            }
        }
        if handle <= 0 {
            presentError(L10n.t("msg.remoteConnectFailed"), title: L10n.t("chrome.connect"))
            refreshRemoteWarn()
            return
        }
        remoteHandle = handle
        KeychainStore.save(account: endpoint, token: token)
        AppPrefs.shared.remoteUrl = endpoint
        AppPrefs.shared.rememberRemote(endpoint)
        pollRemote(force: true)
        bumpSurfaceEpoch()
        refreshRemoteWarn()
    }

    func disconnectRemote() {
        guard isRemote else { return }
        closeAllSwitchers()
        closeAllMultiviews()
        showOverlay = false
        if remoteHandle != 0 {
            _ = mixer_remote_close(remoteHandle)
            remoteHandle = 0
        }
        remoteConnected = false
        remotePreviewKey = ""
        remoteProgramKey = ""
        remoteMultiviewKey = ""
        remotePreviewLive = false
        remoteProgramLive = false
        remoteMultiviewLive = false
        remoteEpoch = ""
        remotePulledDocumentRevision = 0
        remoteLiveSequence = 0
        for id in remoteReceiveIds.values {
            _ = mixer_destroy_source(id)
        }
        remoteReceiveIds.removeAll()
        remoteReceiveKeys.removeAll()
        _ = mixer_destroy_source(MixerRemote.previewSourceId)
        _ = mixer_destroy_source(MixerRemote.programSourceId)
        _ = mixer_destroy_source(MixerRemote.mainMultiviewSourceId)
        bumpSurfaceEpoch()
        refreshRemoteWarn()
    }

    func applyVmixApi() {
        applyHttpApi()
        applyTcpApi()
        applyNativeApi()
    }

    private func applyHttpApi() {
        let settings = session.settings
        let port = settings.vmixApiPort == 0 ? 8088 : settings.vmixApiPort
        let enabled = settings.vmixApiEnabled
        let code = MixerFFI.withCString(settings.vmixApiUser) { user in
            MixerFFI.withCString(settings.vmixApiPassword) { pass in
                mixer_api_configure(enabled ? 1 : 0, port, user, pass)
            }
        }
        if code == 0 {
            return
        }
        if !enabled || code != 5 {
            _ = fail(code, "Configure vMix HTTP API")
            return
        }
        session.settings.vmixApiEnabled = false
        _ = MixerFFI.withCString(settings.vmixApiUser) { user in
            MixerFFI.withCString(settings.vmixApiPassword) { pass in
                mixer_api_configure(0, port, user, pass)
            }
        }
        let ownerText = MixerFFI.listenOwnerText()
        let owner = ownerText.isEmpty ? nil : ownerText
        if let owner {
            HostLog.write("WARN", "vMix HTTP API listen failed on port \(port); in use by \(owner); disabled")
        } else {
            HostLog.write("WARN", "vMix HTTP API listen failed on port \(port); disabled")
        }
        Task { @MainActor in
            self.showHttpListenFailed(port: port, owner: owner)
        }
    }

    private func applyTcpApi() {
        let enabled = session.settings.vmixTcpEnabled
        let code = mixer_tcp_configure(enabled ? 1 : 0)
        if code == 0 {
            return
        }
        if !enabled || code != 5 {
            _ = fail(code, "Configure vMix TCP API")
            return
        }
        session.settings.vmixTcpEnabled = false
        _ = mixer_tcp_configure(0)
        let ownerText = MixerFFI.tcpListenOwnerText()
        let owner = ownerText.isEmpty ? nil : ownerText
        if let owner {
            HostLog.write("WARN", "vMix TCP API listen failed on port 8099; in use by \(owner); disabled")
        } else {
            HostLog.write("WARN", "vMix TCP API listen failed on port 8099; disabled")
        }
        Task { @MainActor in
            self.showTcpListenFailed(owner: owner)
        }
    }

    private func applyNativeApi() {
        if isRemote {
            return
        }
        let prefs = AppPrefs.shared
        let port = prefs.nativeApiPort == 0 ? 9400 : prefs.nativeApiPort
        let bind = prefs.nativeApiBind.trimmingCharacters(in: .whitespacesAndNewlines)
        let host = bind.isEmpty ? "127.0.0.1" : bind
        let enabled = prefs.nativeApiEnabled
        let token = KeychainStore.load(account: "listen")
        let role = prefs.nativeApiRole.isEmpty ? "admin" : prefs.nativeApiRole
        let media = prefs.resolvedMediaDirectory
        let code = MixerFFI.withCString(host) { hostPtr in
            MixerFFI.withCString(token) { tokenPtr in
                MixerFFI.withCString(role) { rolePtr in
                    MixerFFI.withCString(media) { mediaPtr in
                        mixer_ws_configure_owned(enabled ? 1 : 0, hostPtr, port, tokenPtr, rolePtr, mediaPtr)
                    }
                }
            }
        }
        if code == 0 {
            return
        }
        if !enabled || code != 5 {
            _ = fail(code, "Configure Protobuf WebSocket API")
            return
        }
        prefs.nativeApiEnabled = false
        prefs.save()
        _ = MixerFFI.withCString(host) { hostPtr in
            mixer_ws_configure_owned(0, hostPtr, port, "", "", "")
        }
        let ownerText = MixerFFI.wsListenOwnerText()
        let owner = ownerText.isEmpty ? nil : ownerText
        if let owner {
            HostLog.write("WARN", "Protobuf WebSocket API listen failed on port \(port); in use by \(owner); disabled")
        } else {
            HostLog.write("WARN", "Protobuf WebSocket API listen failed on port \(port); disabled")
        }
        Task { @MainActor in
            self.showWsListenFailed(port: port, owner: owner)
        }
    }

    private func showHttpListenFailed(port: UInt32, owner: String?) {
        let message = owner.map { L10n.format("msg.httpListenFailedOwner", "\(port)", $0) }
            ?? L10n.format("msg.httpListenFailed", "\(port)")
        presentError(message, title: L10n.t("settings.webApi"))
    }

    private func showTcpListenFailed(owner: String?) {
        let message = owner.map { L10n.format("msg.tcpListenFailedOwner", $0) }
            ?? L10n.t("msg.tcpListenFailed")
        presentError(message, title: L10n.t("settings.webApi"))
    }

    private func showWsListenFailed(port: UInt32, owner: String?) {
        let message = owner.map { L10n.format("msg.wsListenFailedOwner", "\(port)", $0) }
            ?? L10n.format("msg.wsListenFailed", "\(port)")
        presentError(message, title: L10n.t("settings.webApi"))
    }

    func publishSession() {
        if isRemote { return }
        replaceRuntime()
    }

    func replaceRuntime() {
        if isRemote { return }
        session.selectedUnitId = selectedUnitId
        guard let json = try? SessionFile.encode(session) else { return }
        json.withUnsafeBytes { ptr in
            _ = mixer_session_replace(ptr.bindMemory(to: UInt8.self).baseAddress, json.count, 0)
        }
    }

    func shutdown() {
        remoteDiscoverTask?.cancel()
        remoteDiscoverTask = nil
        mixTimer?.invalidate()
        mixTimer = nil
        meterTimer?.invalidate()
        meterTimer = nil
        closeAllInputPreviews()
        closeAllAudioInputs()
        closeAllSwitchers()
        if remoteHandle != 0 {
            _ = mixer_remote_close(remoteHandle)
            remoteHandle = 0
        }
        remoteConnected = false
        remotePreviewKey = ""
        remoteProgramKey = ""
        remoteMultiviewKey = ""
        remotePreviewLive = false
        remoteProgramLive = false
        remoteMultiviewLive = false
        remoteEpoch = ""
        remotePulledDocumentRevision = 0
        remoteLiveSequence = 0
        for id in remoteReceiveIds.values {
            _ = mixer_destroy_source(id)
        }
        _ = mixer_destroy_source(MixerRemote.previewSourceId)
        _ = mixer_destroy_source(MixerRemote.programSourceId)
        _ = mixer_destroy_source(MixerRemote.mainMultiviewSourceId)
        remoteReceiveIds.removeAll()
        remoteReceiveKeys.removeAll()
        guard booted else { return }
        FlipBudget.reset()
        mixer_destroy()
        booted = false
    }

    func allocateMonitorId() -> UInt64 {
        let id = session.nextMonitorId
        session.nextMonitorId += 1
        return id
    }

    func previewSelectedInput() {
        if isRemote {
            presentError(L10n.t("msg.remoteNoInputPreview"), title: L10n.t("chrome.previewInput"))
            return
        }
        guard let id = selectedInputId,
              let input = session.inputs.first(where: { $0.id == id })
        else {
            let alert = NSAlert()
            alert.messageText = "Select an Input to preview."
            alert.alertStyle = .informational
            alert.runModal()
            return
        }
        openInputPreview(inputId: input.id, name: input.listLabel(in: session, localFiles: !isRemote))
    }

    func openInputPreview(inputId: UInt64, name: String) {
        if isRemote {
            presentError(L10n.t("msg.remoteNoInputPreview"), title: L10n.t("chrome.previewInput"))
            return
        }
        if let existing = inputPreviewWindows[inputId] {
            presentInputPreview(existing)
            return
        }
        let unit = selectedUnit
        let width = CGFloat(960)
        let height = width * CGFloat(max(1, unit.height)) / CGFloat(max(1, unit.width))
        let contentRect = NSRect(x: 0, y: 0, width: width, height: height)
        let window = InputPreviewHostWindow(
            contentRect: contentRect,
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        window.title = name
        window.identifier = NSUserInterfaceItemIdentifier("input-preview-\(inputId)")
        window.contentAspect = CGFloat(max(1, unit.width)) / CGFloat(max(1, unit.height))
        window.contentView = makeInputPreviewContent(
            sourceId: inputId,
            frame: contentRect
        )
        window.isReleasedWhenClosed = false
        window.appearance = NSApp.appearance
        window.backgroundColor = EivizTheme.nsStatusBar
        window.minSize = NSSize(width: 320, height: 180)
        window.tabbingMode = .disallowed
        window.collectionBehavior = [.moveToActiveSpace, .fullScreenPrimary]
        window.setContentSize(NSSize(width: width, height: height))
        window.center()
        inputPreviewCloser.onClose = { [weak self] closedId in
            Task { @MainActor in
                self?.inputPreviewDidClose(closedId)
            }
        }
        window.delegate = inputPreviewCloser
        let controller = NSWindowController(window: window)
        inputPreviewControllers[inputId] = controller
        inputPreviewWindows[inputId] = window
        presentInputPreview(window)
    }

    private func presentInputPreview(_ window: NSWindow) {
        NSApp.activate(ignoringOtherApps: true)
        if window.isMiniaturized {
            window.deminiaturize(nil)
        }
        window.makeKeyAndOrderFront(nil)
        window.orderFrontRegardless()
    }

    func closeInputPreview(_ inputId: UInt64) {
        let window = inputPreviewWindows.removeValue(forKey: inputId)
        inputPreviewControllers.removeValue(forKey: inputId)
        window?.delegate = nil
        window?.close()
    }

    func closeAllInputPreviews() {
        for id in Array(inputPreviewWindows.keys) {
            closeInputPreview(id)
        }
    }

    private func inputPreviewDidClose(_ inputId: UInt64) {
        inputPreviewWindows.removeValue(forKey: inputId)
        inputPreviewControllers.removeValue(forKey: inputId)
    }

    func openAudioInput(_ input: InputEntry) {
        if let existing = audioInputWindows[input.id] {
            presentAudioInput(existing)
            return
        }
        let view = AudioInputSettingsView(inputId: input.id, mixer: self)
        let hosting = NSHostingView(rootView: view)
        hosting.frame = NSRect(x: 0, y: 0, width: 380, height: 220)
        let window = NSWindow(
            contentRect: hosting.frame,
            styleMask: [.titled, .closable, .miniaturizable],
            backing: .buffered,
            defer: false
        )
        window.title = L10n.format("audio.inputTitle", input.listLabel(in: session, localFiles: !isRemote))
        window.identifier = NSUserInterfaceItemIdentifier("audio-input-\(input.id)")
        window.contentView = hosting
        window.isReleasedWhenClosed = false
        window.appearance = NSApp.appearance
        window.backgroundColor = EivizTheme.nsStatusBar
        window.tabbingMode = .disallowed
        window.center()
        audioInputCloser.onClose = { [weak self] closedId in
            Task { @MainActor in
                self?.audioInputDidClose(closedId)
            }
        }
        window.delegate = audioInputCloser
        let controller = NSWindowController(window: window)
        audioInputControllers[input.id] = controller
        audioInputWindows[input.id] = window
        presentAudioInput(window)
    }

    private func presentAudioInput(_ window: NSWindow) {
        NSApp.activate(ignoringOtherApps: true)
        if window.isMiniaturized {
            window.deminiaturize(nil)
        }
        window.makeKeyAndOrderFront(nil)
    }

    func closeAudioInput(_ inputId: UInt64) {
        let window = audioInputWindows.removeValue(forKey: inputId)
        audioInputControllers.removeValue(forKey: inputId)
        window?.delegate = nil
        window?.close()
    }

    func closeAllAudioInputs() {
        for id in Array(audioInputWindows.keys) {
            closeAudioInput(id)
        }
    }

    private func audioInputDidClose(_ inputId: UInt64) {
        audioInputWindows.removeValue(forKey: inputId)
        audioInputControllers.removeValue(forKey: inputId)
    }

    func applyInputAudio(id: UInt64, mask: UInt32, gain: Float, mute: Bool) {
        guard let index = session.inputs.firstIndex(where: { $0.id == id }) else { return }
        session.inputs[index].busMask = session.inputs[index].kind == .mix ? 0 : (mask == 0 ? 1 : mask)
        session.inputs[index].gain = max(0, gain)
        session.inputs[index].mute = mute
        let input = session.inputs[index]
        let applied = audioMask(input)
        if isRemote {
            _ = mixer_remote_audio_set_input(remoteHandle, input.id, applied, input.gain, mute ? 1 : 0)
        } else {
            _ = mixer_audio_set_input(input.id, applied, input.gain, mute ? 1 : 0)
        }
        if let window = audioInputWindows[id] {
            window.title = L10n.format("audio.inputTitle", input.listLabel(in: session, localFiles: !isRemote))
        }
        objectWillChange.send()
    }

    func applySession() {
        if isRemote { return }
        session.mergeTagCatalogs()
        session.assignMonitors()
        replaceRuntime()
        applyStoredBuses()
        selectedUnitId = session.selectedUnitId == 0 ? (session.units.first?.id ?? 1) : session.selectedUnitId
        let previewId = session.units.first { $0.id == selectedUnitId }?.previewSceneId ?? 0
        selectedSceneId = previewId != 0 ? previewId : session.scenes.first?.id
        applyVmixApi()
    }

    private func applyStoredBuses() {
        for unit in session.units {
            let previewId = unit.previewSceneId != 0
                ? unit.previewSceneId
                : (session.scenes.first?.id ?? 0)
            let programId = unit.programSceneId != 0
                ? unit.programSceneId
                : (session.scenes.count > 1 ? session.scenes[1].id : previewId)
            let preview = session.scenes.first { $0.id == previewId }?.gpuId ?? 0
            let program = session.scenes.first { $0.id == programId }?.gpuId ?? preview
            var state = currentState(unit.id)
            state.preview_source = preview
            state.program_source = program
            fail(mixer_unit_set_state(unit.id, &state), "Restore buses")
            applyBusSources(unitId: unit.id, preview: preview, program: program)
        }
    }

    func captureSceneBuses() {
        for index in session.units.indices {
            let id = session.units[index].id
            if let gpu = previewByUnit[id],
               let scene = session.scenes.first(where: { $0.gpuId == gpu })
            {
                session.units[index].previewSceneId = scene.id
            }
            if let gpu = programByUnit[id],
               let scene = session.scenes.first(where: { $0.gpuId == gpu })
            {
                session.units[index].programSceneId = scene.id
            }
        }
    }

    func pushAudio() {
        if isRemote { return }
        var live: [UInt64] = []
        let n = mixer_audio_bus_count()
        if n > 0 {
            for i in 0..<UInt32(n) {
                var info = MixerFFI.zeroed() as EivizAudioBusInfo
                if mixer_audio_bus_get(i, &info) != 0 { continue }
                live.append(info.id)
            }
        }
        let keep = Set(session.buses.map(\.id))
        for id in live where !keep.contains(id) {
            _ = mixer_audio_bus_remove(id)
        }
        for bus in session.buses {
            MixerFFI.withCString(bus.name) { name in
                MixerFFI.withCString(bus.deviceId) { device in
                    _ = mixer_audio_bus_upsert(
                        bus.id,
                        name,
                        bus.role.rawUInt,
                        bus.deviceKind.rawUInt,
                        device,
                        bus.mapLeft,
                        bus.mapRight
                    )
                }
            }
            _ = mixer_audio_set_bus_gain(bus.id, max(0, bus.gain), bus.mute ? 1 : 0)
        }
        for input in session.inputs {
            _ = mixer_audio_set_input(input.id, audioMask(input), max(0, input.gain), input.mute ? 1 : 0)
        }
        for unit in session.units {
            _ = mixer_audio_set_unit_link(unit.id, unit.audioBusId == 0 ? 1 : unit.audioBusId, unit.audioLink.rawUInt)
        }
        _ = mixer_audio_set_headphone_cue(selectedUnitId)
        _ = mixer_audio_set_headphone_copy_master(session.headphoneCopyMaster ? 1 : 0)
    }

}


final class AudioInputCloser: NSObject, NSWindowDelegate {
    var onClose: ((UInt64) -> Void)?

    func windowWillClose(_ notification: Notification) {
        guard let window = notification.object as? NSWindow,
              let raw = window.identifier?.rawValue,
              raw.hasPrefix("audio-input-"),
              let inputId = UInt64(raw.dropFirst("audio-input-".count))
        else { return }
        onClose?(inputId)
    }
}

final class InputPreviewCloser: NSObject, NSWindowDelegate {
    var onClose: ((UInt64) -> Void)?

    func windowWillClose(_ notification: Notification) {
        guard let window = notification.object as? NSWindow,
              let raw = window.identifier?.rawValue,
              raw.hasPrefix("input-preview-"),
              let inputId = UInt64(raw.dropFirst("input-preview-".count))
        else { return }
        onClose?(inputId)
    }

    func windowWillResize(_ sender: NSWindow, to frameSize: NSSize) -> NSSize {
        guard let window = sender as? InputPreviewHostWindow,
              !window.styleMask.contains(.fullScreen),
              let content = window.contentView
        else { return frameSize }
        let extraW = window.frame.width - content.bounds.width
        let extraH = window.frame.height - content.bounds.height
        let contentW = max(320, frameSize.width - extraW)
        let contentH = contentW / max(0.1, window.contentAspect)
        return NSSize(width: contentW + extraW, height: contentH + extraH)
    }
}

final class RemoteMutateGate: @unchecked Sendable {
    let lock = NSLock()
    var revision: UInt64 = 0

    func mutate(handle: Int32, json: String) -> (code: Int32, revision: UInt64) {
        lock.lock()
        defer { lock.unlock() }
        let expected = revision
        let code = MixerRemote.mutate(handle, json, expected: expected)
        let statusJson = MixerRemote.status(handle)
        var nextRevision = expected
        if let data = statusJson.data(using: .utf8),
           let root = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        {
            if let value = root["documentRevision"] as? NSNumber, value.uint64Value != 0 {
                nextRevision = value.uint64Value
            } else if let value = root["revision"] as? NSNumber {
                nextRevision = value.uint64Value
            }
        }
        revision = nextRevision
        return (code, nextRevision)
    }

    func note(_ value: UInt64) {
        lock.lock()
        defer { lock.unlock() }
        if value > revision {
            revision = value
        }
    }
}

final class SwitcherCloser: NSObject, NSWindowDelegate {
    var onClose: ((UInt64) -> Void)?

    func windowWillClose(_ notification: Notification) {
        guard let window = notification.object as? NSWindow,
              let raw = window.identifier?.rawValue
        else { return }
        if raw.hasPrefix("switcher-"), let unitId = UInt64(raw.dropFirst("switcher-".count)) {
            onClose?(unitId)
        } else if raw.hasPrefix("multiview-"), let id = UInt64(raw.dropFirst("multiview-".count)) {
            onClose?(id)
        }
    }
}
