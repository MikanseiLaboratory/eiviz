import AppKit
import AVFoundation
import Combine
import Darwin
import EivizMixer
import EivizRemote
import Foundation
import SwiftUI
import UniformTypeIdentifiers

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
    @Published private(set) var surfaceEpoch: UInt64 = 0
    @Published private(set) var isRemote = false
    @Published private(set) var remoteConnected = false
    @Published private(set) var remoteRevision: UInt64 = 0
    @Published var videoUnavailable = false

    private var booted = false
    private var fatalHandled = false
    private var tbarLatching = false
    private var meterTimer: Timer?
    private var mixTimer: Timer?
    @Published private(set) var previewByUnit: [UInt64: UInt64] = [:]
    @Published private(set) var programByUnit: [UInt64: UInt64] = [:]
    private var inputPreviewWindows: [UInt64: NSWindow] = [:]
    private var inputPreviewControllers: [UInt64: NSWindowController] = [:]
    private let inputPreviewCloser = InputPreviewCloser()
    private var switcherWindows: [UInt64: NSWindow] = [:]
    private let switcherCloser = SwitcherCloser()
    private var multiviewWindows: [UInt64: NSWindow] = [:]
    private let multiviewCloser = SwitcherCloser()
    private var videoRoles: [UInt64: (program: Bool, preview: Bool)] = [:]
    private var remoteHandle: Int32 = 0
    private var remoteReceiveIds: [UInt64: UInt64] = [:]
    private var remoteEpoch = ""
    private var remotePulledDocumentRevision: UInt64 = 0
    private var remoteLiveSequence: UInt64 = 0
    private var remoteLag = false
    private var remoteError = ""
    private var remotePreviewKey = ""
    private var remoteProgramKey = ""
    private var remotePreviewLive = false
    private var remoteProgramLive = false

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

    private func bootRemote() {
        guard fail(mixer_create_with_backend(AppPrefs.shared.renderer.createAbi, 0, 60, 1), "Metal mixer initialization") else {
            return
        }
        _ = mixer_define_generator(EIVIZ_SRC_BLACK, EIVIZ_GEN_SOLID, 0, 0, 0, 1, 0)
        FlipBudget.configure(0)
        GpuPresentStore.load()
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
        remotePreviewLive = false
        remoteProgramLive = false
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
        remotePreviewLive = false
        remoteProgramLive = false
        remoteEpoch = ""
        remotePulledDocumentRevision = 0
        remoteLiveSequence = 0
        for id in remoteReceiveIds.values {
            _ = mixer_destroy_source(id)
        }
        remoteReceiveIds.removeAll()
        _ = mixer_destroy_source(MixerRemote.previewSourceId)
        _ = mixer_destroy_source(MixerRemote.programSourceId)
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
        mixTimer?.invalidate()
        mixTimer = nil
        meterTimer?.invalidate()
        meterTimer = nil
        closeAllInputPreviews()
        closeAllSwitchers()
        if remoteHandle != 0 {
            _ = mixer_remote_close(remoteHandle)
            remoteHandle = 0
        }
        remoteConnected = false
        remotePreviewKey = ""
        remoteProgramKey = ""
        remotePreviewLive = false
        remoteProgramLive = false
        remoteEpoch = ""
        remotePulledDocumentRevision = 0
        remoteLiveSequence = 0
        for id in remoteReceiveIds.values {
            _ = mixer_destroy_source(id)
        }
        _ = mixer_destroy_source(MixerRemote.previewSourceId)
        _ = mixer_destroy_source(MixerRemote.programSourceId)
        remoteReceiveIds.removeAll()
        guard booted else { return }
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
        openInputPreview(inputId: input.id, name: input.name)
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

    func applySession() {
        if isRemote { return }
        session.mergeTagCatalogs()
        session.assignMonitors()
        replaceRuntime()
        selectedSceneId = session.scenes.first?.id
        selectedUnitId = session.selectedUnitId == 0 ? (session.units.first?.id ?? 1) : session.selectedUnitId
        applyVmixApi()
    }

    func pushAudio() {
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
                        bus.mapRight,
                        bus.exclusive ? 1 : 0
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

    func cut() {
        takeCut(unitId: selectedUnit.id)
        mix = 0
        tbarLocked = false
        updateStatus()
    }

    private func fireAuto(_ preset: TransitionPreset, unit: MixingUnitEntry, reason: String) {
        if isRemote {
            _ = mixer_remote_auto(
                remoteHandle,
                unit.id,
                preset.kind,
                unit.durationMs(for: preset),
                preset.swap ? 1 : 0,
                preset.keepPreview ? 1 : 0,
                preset.easing,
                preset.direction,
                preset.dipR,
                preset.dipG,
                preset.dipB,
                preset.dipA <= 0 ? 1 : preset.dipA,
                preset.softness,
                preset.param
            )
            return
        }
        let wgsl = resolvedCustomWgsl(preset)
        if wgsl.isEmpty {
            fail(mixer_unit_set_custom_wgsl(unit.id, nil), "custom wgsl")
        } else {
            wgsl.withCString { fail(mixer_unit_set_custom_wgsl(unit.id, $0), "custom wgsl") }
        }
        fail(
            mixer_unit_auto(
                unit.id,
                preset.kind,
                unit.durationMs(for: preset),
                preset.swap ? 1 : 0,
                preset.keepPreview ? 1 : 0,
                preset.easing,
                preset.direction,
                preset.dipR,
                preset.dipG,
                preset.dipB,
                preset.dipA,
                EIVIZ_INCOMING_PREVIEW,
                preset.softness,
                preset.param
            ),
            reason
        )
    }

    func takeCut(unitId: UInt64? = nil) {
        let unit = unit(for: unitId)
        let preset = tbarPreset(for: unit)
        if isRemote {
            _ = mixer_remote_cut(remoteHandle, unit.id, preset.swap ? 1 : 0)
            return
        }
        fail(mixer_unit_cut(unit.id, preset.swap ? 1 : 0, EIVIZ_INCOMING_PREVIEW), "CUT")
        syncUnitBuses(unit.id)
    }

    func auto() {
        let preset = tbarPreset()
        if preset.kind == EIVIZ_TRANSITION_CUT || preset.durationValue <= 1 {
            cut()
            return
        }
        fireAuto(preset, unit: selectedUnit, reason: "AUTO")
        tbarLocked = false
        syncUnitBuses(selectedUnit.id)
    }

    func firePreset(_ preset: TransitionPreset, index: Int) {
        tbarPresetIndex = index
        firePreset(preset, unitId: selectedUnit.id)
        tbarLocked = false
    }

    func firePreset(_ preset: TransitionPreset, unitId: UInt64) {
        let unit = unit(for: unitId)
        if preset.kind == EIVIZ_TRANSITION_CUT || preset.durationValue <= 1 {
            if isRemote {
                _ = mixer_remote_cut(remoteHandle, unit.id, preset.swap ? 1 : 0)
            } else {
                fail(mixer_unit_cut(unit.id, preset.swap ? 1 : 0, EIVIZ_INCOMING_PREVIEW), "TAKE")
            }
        } else {
            fireAuto(preset, unit: unit, reason: "TAKE")
        }
        if !isRemote {
            syncUnitBuses(unit.id)
        }
    }

    func previewScene(_ scene: SceneEntry) {
        previewScene(scene, unitId: selectedUnit.id)
    }

    func previewScene(_ scene: SceneEntry, unitId: UInt64) {
        let unit = unit(for: unitId)
        if isRemote {
            _ = mixer_remote_preview(remoteHandle, unit.id, scene.gpuId)
            selectedSceneId = scene.id
            return
        }
        var state = currentState(unit.id)
        state.preview_source = scene.gpuId
        fail(mixer_unit_set_state(unit.id, &state), "Preview scene")
        applyBusSources(unitId: unit.id, preview: scene.gpuId, program: state.program_source)
        selectedSceneId = scene.id
    }

    func previewingSceneId(for unitId: UInt64) -> UInt64? {
        guard let gpuId = previewByUnit[unitId] else { return nil }
        return session.scenes.first { $0.gpuId == gpuId }?.id
    }

    func programmingSceneId(for unitId: UInt64) -> UInt64? {
        guard let gpuId = programByUnit[unitId] else { return nil }
        return session.scenes.first { $0.gpuId == gpuId }?.id
    }

    func setMix(_ value: Float) {
        if tbarLatching { return }
        if tbarLocked {
            if value < 1 {
                tbarLatching = true
                mix = 1
                tbarLatching = false
            }
            return
        }
        if value >= 0.999 {
            tbarLocked = true
            tbarLatching = true
            mix = 1
            tbarLatching = false
            takeCut()
            return
        }
        mix = value
        setMix(value, unitId: selectedUnit.id)
    }

    func setMix(_ value: Float, unitId: UInt64) {
        if isRemote {
            _ = mixer_remote_set_mix(remoteHandle, unitId, value)
            return
        }
        var state = currentState(unitId)
        state.mix = value
        normalizePixelSortDefaults(unitId)
        var preset = tbarPreset(for: unit(for: unitId))
        TransitionCatalog.applyKindDefaults(&preset)
        state.transition_kind = preset.kind
        state.transition_easing = preset.easing
        state.transition_direction = preset.direction
        state.dip_r = preset.dipR
        state.dip_g = preset.dipG
        state.dip_b = preset.dipB
        state.dip_a = preset.dipA <= 0 ? 1 : preset.dipA
        state.softness = preset.softness
        state.param = preset.param
        let wgsl = resolvedCustomWgsl(preset)
        if wgsl.isEmpty {
            _ = mixer_unit_set_custom_wgsl(unitId, nil)
        } else {
            wgsl.withCString { _ = mixer_unit_set_custom_wgsl(unitId, $0) }
        }
        _ = mixer_unit_set_state(unitId, &state)
    }

    func finishTBar() {
        guard tbarLocked else { return }
        tbarLatching = true
        mix = 0
        tbarLatching = false
        tbarLocked = false
    }

    func addInput(_ input: InputEntry) {
        upsertInput(input, replacing: nil)
    }

    func upsertInput(_ input: InputEntry, replacing: UInt64?) {
        if isRemote {
            if replacing == nil, input.kind == .still || input.kind == .video {
                guard let path = input.pathOrAddress else { return }
                let kind = input.kind == .still ? "still" : "video"
                let code = MixerFFI.withCString(path) { pathPtr in
                    MixerFFI.withCString(kind) { kindPtr in
                        MixerFFI.withCString(input.name) { namePtr in
                            mixer_remote_upload(remoteHandle, pathPtr, kindPtr, namePtr, input.videoLoop ? 1 : 0, remoteRevision)
                        }
                    }
                }
                if code != 0 {
                    presentInputError(L10n.t("msg.uploadFailed"))
                    return
                }
                pollRemote(force: true)
                return
            }
            if input.kind == .mix,
               input.mixSource != .sessionMultiview,
               let unit = session.units.first(where: { $0.id == input.mixTargetId }),
               mixUnitUses(unit, sourceId: replacing ?? input.id)
            {
                presentInputError(L10n.t("msg.mixCycle"), editing: replacing != nil)
                return
            }
            var entry = input
            if let id = replacing {
                entry.id = id
            } else if entry.id == 0 || session.inputs.contains(where: { $0.id == entry.id }) {
                entry.id = session.nextInputId
            }
            _ = mutateRemote(MixerRemote.upsertInput(entry))
            return
        }
        if input.kind == .mix,
           input.mixSource != .sessionMultiview,
           let unit = session.units.first(where: { $0.id == input.mixTargetId }),
           mixUnitUses(unit, sourceId: replacing ?? input.id)
        {
            presentInputError(L10n.t("msg.mixCycle"), editing: replacing != nil)
            return
        }
        var entry = input
        if let id = replacing, let index = session.inputs.firstIndex(where: { $0.id == id }) {
            if !session.inputs[index].isBuiltin {
                _ = mixer_destroy_source(id)
            }
            entry.id = id
            if entry.guid.isEmpty {
                entry.guid = session.inputs[index].guid
            }
            session.inputs[index] = entry
        } else {
            if entry.id == 0 || session.inputs.contains(where: { $0.id == entry.id }) {
                entry.id = session.nextInputId
                session.nextInputId += 1
            }
            session.inputs.append(entry)
        }
        attach(entry)
        selectedInputId = entry.id
        objectWillChange.send()
    }

    func deleteSelectedInput() {
        guard let id = selectedInputId,
              let index = session.inputs.firstIndex(where: { $0.id == id }),
              !session.inputs[index].isBuiltin
        else { return }
        if isRemote {
            _ = mutateRemote(MixerRemote.deleteInput(id))
            return
        }
        closeInputPreview(id)
        videoRoles.removeValue(forKey: id)
        _ = mixer_destroy_source(id)
        session.inputs.remove(at: index)
        for unitIndex in session.units.indices {
            session.units[unitIndex].overlays.removeAll { $0.sourceKind == .input && $0.sceneGpuId == id }
        }
        selectedInputId = nil
        pushOverlays()
    }

    func addScene() {
        if isRemote {
            let scene = SceneEntry(id: session.nextSceneId, name: "Scene \(session.nextSceneId)")
            _ = mutateRemote(MixerRemote.upsertScene(scene))
            return
        }
        let scene = session.addScene(name: "Scene \(session.nextSceneId)", input: nil)
        pushScene(scene)
        previewScene(scene)
    }

    func removeScene() {
        guard let id = selectedSceneId,
              let scene = session.scenes.first(where: { $0.id == id })
        else { return }
        deleteScene(scene)
    }

    func deleteScene(_ scene: SceneEntry) {
        guard session.scenes.count > 1 else { return }
        if isRemote {
            _ = mutateRemote(MixerRemote.deleteScene(scene.id))
            return
        }
        closeInputPreview(scene.gpuId)
        _ = mixer_destroy_scene(scene.gpuId)
        session.scenes.removeAll { $0.id == scene.id }
        for unitIndex in session.units.indices {
            session.units[unitIndex].overlays.removeAll { $0.sourceKind == .scene && $0.sceneGpuId == scene.gpuId }
        }
        pushOverlays()
        if let next = session.scenes.first {
            previewScene(next)
        }
    }

    func cutScene(_ scene: SceneEntry) {
        previewScene(scene)
        cut()
    }

    func toggleSceneLoop(_ scene: SceneEntry) {
        guard let video = sceneVideo(scene),
              let index = session.inputs.firstIndex(where: { $0.id == video.id })
        else { return }
        session.inputs[index].videoLoop.toggle()
        if isRemote {
            _ = mixer_remote_video_loop(remoteHandle, video.id, session.inputs[index].videoLoop ? 1 : 0)
        } else {
            _ = mixer_video_set_loop(video.id, session.inputs[index].videoLoop ? 1 : 0)
        }
        objectWillChange.send()
    }

    func toggleScenePlay(_ scene: SceneEntry) {
        guard let video = sceneVideo(scene) else { return }
        if isRemote {
            _ = mixer_remote_video_play(remoteHandle, video.id, 1)
            objectWillChange.send()
            return
        }
        guard let info = copyVideoInfo(video.id) else { return }
        _ = mixer_video_set_playing(video.id, info.playing == 0 ? 1 : 0)
        objectWillChange.send()
    }

    func toggleSceneAudio(_ scene: SceneEntry) {
        let ids = sceneInputs(scene).map(\.id)
        guard !ids.isEmpty else { return }
        let mute = !sceneInputs(scene).allSatisfy(\.mute)
        for id in ids {
            guard let index = session.inputs.firstIndex(where: { $0.id == id }) else { continue }
            session.inputs[index].mute = mute
            let input = session.inputs[index]
            _ = mixer_audio_set_input(input.id, audioMask(input), max(0, input.gain), mute ? 1 : 0)
        }
        objectWillChange.send()
    }

    func sceneVideo(_ scene: SceneEntry) -> InputEntry? {
        sceneInputs(scene).first { $0.kind == .video }
    }

    func sceneInputs(_ scene: SceneEntry) -> [InputEntry] {
        scene.layers.compactMap { layer in
            session.inputs.first { $0.id == layer.inputId }
        }
    }

    func scenePlaying(_ scene: SceneEntry) -> Bool {
        guard let video = sceneVideo(scene), let info = copyVideoInfo(video.id) else { return false }
        return info.playing != 0
    }

    func saveScene(_ scene: SceneEntry) {
        if let index = session.scenes.firstIndex(where: { $0.id == scene.id }) {
            session.scenes[index] = scene
        }
        pushScene(scene)
    }

    func addUnit() {
        var unit = MixingUnitEntry(id: session.nextUnitId, name: "Mixing Unit \(session.nextUnitId)")
        unit.width = session.settings.defaultWidth
        unit.height = session.settings.defaultHeight
        unit.fpsNum = session.settings.masterFpsNum
        unit.fpsDen = session.settings.masterFpsDen
        unit.transitions = [
            TransitionPreset(kind: EIVIZ_TRANSITION_CUT, durationValue: 1, swap: true),
            TransitionPreset(kind: EIVIZ_TRANSITION_FADE, durationValue: 30, swap: true)
        ]
        editingUnit = unit
        showMixingUnit = true
    }

    func commitUnit(_ unit: MixingUnitEntry) {
        if session.units.contains(where: { $0.id == unit.id }) {
            saveUnit(unit)
            return
        }
        var entry = unit
        entry.id = session.nextUnitId
        if entry.audioBusId == 0 {
            entry.audioBusId = 1
        }
        if entry.transitions.isEmpty {
            entry.transitions = [
                TransitionPreset(kind: EIVIZ_TRANSITION_CUT, durationValue: 1, swap: true),
                TransitionPreset(kind: EIVIZ_TRANSITION_FADE, durationValue: 30, swap: true)
            ]
        }
        guard fail(mixer_create_unit(entry.id, entry.width, entry.height), "Create Mixing Unit") else { return }
        session.nextUnitId += 1
        fail(mixer_unit_configure(entry.id, entry.width, entry.height, entry.fpsNum, entry.fpsDen), "Configure Mixing Unit")
        fail(mixer_audio_set_unit_link(entry.id, entry.audioBusId, entry.audioLink.rawUInt), "Audio link")
        let preview = session.scenes.first?.gpuId ?? UInt64(EIVIZ_SRC_BARS)
        let program = session.scenes.count > 1 ? session.scenes[1].gpuId : preview
        applyBusSources(unitId: entry.id, preview: preview, program: program)
        pushState(unitId: entry.id, program: program, preview: preview, mix: 0, kind: EIVIZ_TRANSITION_FADE)
        session.units.append(entry)
        selectedUnitId = entry.id
        updateStatus()
    }

    func deleteUnit() {
        guard session.units.count > 1 else { return }
        let id = selectedUnitId
        closeSwitcher(id)
        _ = mixer_destroy_unit(id)
        session.units.removeAll { $0.id == id }
        selectedUnitId = session.units[0].id
    }

    func saveUnit(_ unit: MixingUnitEntry) {
        if let index = session.units.firstIndex(where: { $0.id == unit.id }) {
            session.units[index] = unit
        }
        if !isRemote {
            fail(mixer_unit_configure(unit.id, unit.width, unit.height, unit.fpsNum, unit.fpsDen), "Configure Mixing Unit")
            fail(mixer_audio_set_unit_link(unit.id, unit.audioBusId, unit.audioLink.rawUInt), "Audio link")
        }
        selectedUnitId = unit.id
        if let window = switcherWindows[unit.id] {
            window.title = unit.name
            window.level = unit.alwaysOnTop ? .floating : .normal
        }
        updateStatus()
    }

    func setSwitcherAlwaysOnTop(_ unitId: UInt64, _ on: Bool) {
        guard let index = session.units.firstIndex(where: { $0.id == unitId }) else { return }
        session.units[index].alwaysOnTop = on
        switcherWindows[unitId]?.level = on ? .floating : .normal
    }

    func hideSceneOnSwitcher(_ unitId: UInt64, _ sceneId: UInt64) {
        guard let index = session.units.firstIndex(where: { $0.id == unitId }) else { return }
        var unit = session.units[index]
        if unit.switcherSceneFilter == .all {
            unit.switcherSceneFilter = .exclude
            unit.switcherSceneIds = []
        }
        if unit.switcherSceneFilter == .exclude, !unit.switcherSceneIds.contains(sceneId) {
            unit.switcherSceneIds.append(sceneId)
        }
        if unit.switcherSceneFilter == .include {
            unit.switcherSceneIds.removeAll { $0 == sceneId }
        }
        session.units[index] = unit
    }

    func setSwitcherSceneFilter(_ unitId: UInt64, _ filter: SwitcherSceneFilter, ids: [UInt64]) {
        guard let index = session.units.firstIndex(where: { $0.id == unitId }) else { return }
        session.units[index].switcherSceneFilter = filter
        session.units[index].switcherSceneIds = ids
    }

    func toggleSceneCollapsed(_ sceneId: UInt64) {
        guard let index = session.scenes.firstIndex(where: { $0.id == sceneId }) else { return }
        session.scenes[index].previewCollapsed.toggle()
    }

    func openOverlay(for unitId: UInt64) {
        selectedUnitId = unitId
        openOverlay()
    }

    func toggleOverlay(_ slot: OverlaySlot) {
        guard let index = session.units.firstIndex(where: { $0.id == selectedUnitId }),
              let slotIndex = session.units[index].overlays.firstIndex(where: { $0.id == slot.id })
        else { return }
        let enabled = !session.units[index].overlays[slotIndex].enabled
        setOverlayEnabled(slot.id, enabled: enabled)
    }

    func addOutput(_ output: OutputEntry) {
        if isRemote { return }
        var entry = output
        if entry.transport != .omt {
            entry.useGpu = false
        }
        if entry.sourceKind == .multiview, entry.sourceId != 0, entry.sourceId < EIVIZ_MULTIVIEW_BASE {
            entry.sourceId = EIVIZ_MULTIVIEW_BASE | entry.sourceId
        } else if entry.sourceKind == .scene, entry.sourceId != 0, entry.sourceId < EIVIZ_SCENE_BASE {
            entry.sourceId = EIVIZ_SCENE_BASE | entry.sourceId
        }
        if let index = session.outputs.firstIndex(where: { $0.id == entry.id }) {
            session.outputs[index] = entry
        }
        _ = mixer_output_remove(entry.id)
        guard entry.enabled else { return }
        let id = entry.id
        let transport = entry.transport.rawValueU32
        let name = entry.name
        let sourceKind = entry.sourceKind.rawValueU32
        let sourceId = entry.sourceId
        let unitId = entry.unitId
        let useGpu: UInt32 = entry.useGpu ? 1 : 0
        let audioBusId = entry.sourceKind == .multiview ? 0 : entry.audioBusId
        let skipIdle: UInt32 = entry.transport == .omt && entry.skipEncodeWhenNoReceivers ? 1 : 0
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            MixerFFI.withCString(name) { cName in
                let code = mixer_output_add(id, transport, cName, sourceKind, sourceId, unitId, useGpu, audioBusId, skipIdle)
                if code != 0 {
                    DispatchQueue.main.async {
                        _ = self?.fail(code, "Add output")
                    }
                }
            }
        }
    }

    func openNewMultiview() {
        guard FlipBudget.tryOpen(1) else { return }
        let unitId = session.settings.defaultMultiviewUnitId == 0
            ? selectedUnitId
            : session.settings.defaultMultiviewUnitId
        if isRemote {
            var layout = MultiviewLayout(
                id: session.nextMultiviewId,
                name: "Multiview \(session.nextMultiviewId)",
                previewUnitId: unitId,
                programUnitId: unitId,
                labelAnchor: session.settings.multiviewLabelAnchor,
                labelSize: session.settings.multiviewLabelSize,
                labelUnit: session.settings.multiviewLabelUnit
            )
            layout.ensureTiles()
            layout.seedDefaultBuses(unitId)
            if mutateRemote(MixerRemote.upsertMultiview(layout)),
               let added = session.multiviews.first(where: { $0.id == layout.id })
            {
                openMultiviewWindow(added)
            }
            return
        }
        let layout = session.addMultiview(unitId: unitId)
        pushMultiview(layout)
        openMultiviewWindow(layout)
    }

    func openMultiviewWindow(_ layout: MultiviewLayout) {
        pushMultiview(layout)
        openMultiview = layout
        if let existing = multiviewWindows[layout.id] {
            existing.makeKeyAndOrderFront(nil)
            existing.level = layout.alwaysOnTop ? .floating : .normal
            return
        }
        guard FlipBudget.tryOpen(1) else { return }
        let host = NSHostingController(rootView: MultiviewView(layoutId: layout.id).environmentObject(self).environment(\.mixerSurfaceEpoch, surfaceEpoch))
        let window = SwitcherHostWindow(
            contentRect: NSRect(x: 0, y: 0, width: 1280, height: 792),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        window.title = layout.name
        window.identifier = NSUserInterfaceItemIdentifier("multiview-\(layout.id)")
        window.contentViewController = host
        window.isReleasedWhenClosed = false
        window.appearance = NSApp.appearance
        window.backgroundColor = EivizTheme.nsBackground
        window.level = layout.alwaysOnTop ? .floating : .normal
        window.center()
        multiviewCloser.onClose = { [weak self] closedId in
            Task { @MainActor in
                self?.multiviewWindows.removeValue(forKey: closedId)
            }
        }
        window.delegate = multiviewCloser
        window.makeKeyAndOrderFront(nil)
        multiviewWindows[layout.id] = window
        showMultiview = false
    }

    func applyMultiviewWindowLevel(_ layout: MultiviewLayout) {
        multiviewWindows[layout.id]?.level = layout.alwaysOnTop ? .floating : .normal
    }

    func openSwitcher(_ unitId: UInt64? = nil) {
        let unit = unit(for: unitId)
        if let existing = switcherWindows[unit.id] {
            existing.makeKeyAndOrderFront(nil)
            existing.level = unit.alwaysOnTop ? .floating : .normal
            return
        }
        guard FlipBudget.tryOpen(2) else { return }
        let host = NSHostingController(rootView: SwitcherView(unitId: unit.id).environmentObject(self).environment(\.mixerSurfaceEpoch, surfaceEpoch))
        let window = SwitcherHostWindow(
            contentRect: NSRect(x: 0, y: 0, width: 1280, height: 640),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        window.title = unit.name
        window.identifier = NSUserInterfaceItemIdentifier("switcher-\(unit.id)")
        window.contentViewController = host
        window.isReleasedWhenClosed = false
        window.appearance = NSApp.appearance
        window.backgroundColor = EivizTheme.nsBackground
        window.tabbingMode = .disallowed
        window.level = unit.alwaysOnTop ? .floating : .normal
        window.center()
        switcherCloser.onClose = { [weak self] closedId in
            Task { @MainActor in
                self?.switcherWindows.removeValue(forKey: closedId)
            }
        }
        window.delegate = switcherCloser
        switcherWindows[unit.id] = window
        window.makeKeyAndOrderFront(nil)
    }

    func closeSwitcher(_ unitId: UInt64) {
        let window = switcherWindows.removeValue(forKey: unitId)
        window?.delegate = nil
        window?.close()
    }

    func openSceneEditor(_ scene: SceneEntry?) {
        if showSceneEditor {
            editingScene = scene
            return
        }
        guard FlipBudget.tryOpen(1) else { return }
        editingScene = scene
        showSceneEditor = true
    }

    func openOverlay() {
        if showOverlay {
            return
        }
        guard FlipBudget.tryOpen(1) else { return }
        showOverlay = true
    }

    func closeAllSwitchers() {
        for id in Array(switcherWindows.keys) {
            closeSwitcher(id)
        }
    }

    func deleteMultiview(_ id: UInt64) {
        if isRemote {
            _ = mutateRemote(MixerRemote.deleteMultiview(id))
            if openMultiview?.id == id {
                showMultiview = false
                openMultiview = nil
            }
            return
        }
        if let layout = session.multiviews.first(where: { $0.id == id }) {
            _ = mixer_destroy_scene(layout.gpuId)
            _ = mixer_detach_monitor(layout.monitorId)
        }
        session.multiviews.removeAll { $0.id == id }
        if openMultiview?.id == id {
            showMultiview = false
            openMultiview = nil
        }
    }

    func pushMultiview(_ layout: MultiviewLayout) {
        if isRemote {
            _ = mutateRemote(MixerRemote.upsertMultiview(layout))
            return
        }
        guard let index = session.multiviews.firstIndex(where: { $0.id == layout.id }) else { return }
        var item = layout
        item.ensureTiles()
        session.multiviews[index] = item
        var layers: [EivizOverlayDesc] = []
        func layer(_ source: UInt64, _ x: Float, _ y: Float, _ w: Float, _ h: Float, _ z: Int32) {
            var desc = MixerFFI.emptyOverlay()
            desc.source_id = source
            desc.rect = EivizRect(x: x, y: y, width: w, height: h)
            desc.opacity = 1
            desc.z = z
            layers.append(desc)
        }
        for (z, pane) in item.template.panes.enumerated() {
            let source = z < item.tiles.count
                ? item.tiles[z].kind.encoded(item.tiles[z].sourceId)
                : 0
            layer(source, pane.x, pane.y, pane.width, pane.height, Int32(z))
        }
        let names = slotNames(item)
        var owned = names.map { $0.isEmpty ? nil : strdup($0) }
        defer {
            for pointer in owned {
                if let pointer {
                    free(pointer)
                }
            }
        }
        for i in layers.indices where i < owned.count {
            if let pointer = owned[i] {
                layers[i].label = UnsafePointer(pointer)
            }
        }
        layers.withUnsafeMutableBufferPointer { ptr in
            fail(
                mixer_define_scene(item.gpuId, selectedUnit.width, selectedUnit.height, UInt32(ptr.count), ptr.baseAddress),
                "Define Multiview"
            )
        }
        let previewUnit = item.tiles.first(where: { $0.kind == .muPreview })?.sourceId ?? item.previewUnitId
        let programUnit = item.tiles.first(where: { $0.kind == .muProgram })?.sourceId ?? item.programUnitId
        fail(mixer_bind_multiview(item.gpuId, previewUnit == 0 ? 1 : previewUnit, programUnit == 0 ? 1 : programUnit), "Bind Multiview")
        let interval = item.presentInterval == 0 ? session.settings.defaultPresentInterval : item.presentInterval
        _ = mixer_set_monitor_present_interval(item.monitorId, max(1, interval))
    }

    func applyBusColors() {
        let preview = session.settings.previewColor
        let program = session.settings.programColor
        let inactive = session.settings.inactiveColor
        _ = mixer_set_bus_colors(
            preview.r, preview.g, preview.b,
            program.r, program.g, program.b,
            inactive.r, inactive.g, inactive.b
        )
        let fallback = min(200, max(1, session.settings.multiviewLabelSize))
        session.settings.multiviewLabelSize = fallback
        _ = mixer_set_mv_label(
            0,
            fallback,
            session.settings.multiviewLabelUnit == .percent ? 1 : 0,
            session.settings.multiviewLabelAnchor == .top ? 1 : 0
        )
        for layout in session.multiviews {
            let size = layout.resolvedLabelSize(session.settings)
            let percent = layout.resolvedLabelUnit(session.settings) == .percent ? UInt32(1) : 0
            let top = layout.resolvedLabelAnchor(session.settings) == .top ? UInt32(1) : 0
            _ = mixer_set_mv_label(layout.gpuId, size, percent, top)
        }
    }

    private func slotNames(_ layout: MultiviewLayout) -> [String] {
        layout.template.panes.indices.map { index in
            index < layout.tiles.count ? tileLabel(layout.tiles[index]) : ""
        }
    }

    private func tileLabel(_ tile: MvSlot) -> String {
        if !tile.labelFollow {
            return tile.label
        }
        switch tile.kind {
        case .input:
            return session.inputs.first(where: { $0.id == tile.sourceId })?.name ?? ""
        case .scene:
            return session.scenes.first(where: { $0.gpuId == tile.sourceId })?.name ?? ""
        case .muPreview:
            let name = session.units.first(where: { $0.id == tile.sourceId })?.name ?? String(tile.sourceId)
            return "PRV  \(name)"
        case .muProgram:
            let name = session.units.first(where: { $0.id == tile.sourceId })?.name ?? String(tile.sourceId)
            return "PGM  \(name)"
        default:
            return ""
        }
    }

    func setOverlayEnabled(_ id: UUID, enabled: Bool, unitId: UInt64? = nil) {
        let targetId = unitId ?? selectedUnitId
        guard let unitIndex = session.units.firstIndex(where: { $0.id == targetId }),
              let slotIndex = session.units[unitIndex].overlays.firstIndex(where: { $0.id == id })
        else { return }
        let slot = session.units[unitIndex].overlays[slotIndex]
        let unit = session.units[unitIndex]
        var desc = MixerFFI.emptyOverlay()
        desc.source_id = slot.sceneGpuId
        desc.rect = EivizRect(x: slot.x, y: slot.y, width: slot.width, height: slot.height)
        desc.crop = EivizRect(x: slot.cropX, y: slot.cropY, width: slot.cropWidth, height: slot.cropHeight)
        desc.opacity = slot.opacity
        desc.z = slot.z
        desc.audio_follow = slot.audioFollow ? 1 : 0
        desc.hidden = slot.hidden ? 1 : 0
        let ms = slot.durationUnit == EIVIZ_DURATION_MS
            ? max(1, slot.durationValue)
            : unit.durationMs(slot.durationValue)
        if slot.transitionKind == EIVIZ_TRANSITION_CUT || ms <= 1 {
            session.units[unitIndex].overlays[slotIndex].enabled = enabled
            overlayOn[id] = enabled
            if isRemote {
                _ = mixer_remote_overlay_auto(remoteHandle, unit.id, UInt32(slotIndex), 1, enabled ? 1 : 0)
            } else {
                pushOverlays(unitId: unit.id)
            }
            return
        }
        if enabled {
            session.units[unitIndex].overlays[slotIndex].enabled = true
            overlayOn[id] = true
            if isRemote {
                _ = mixer_remote_overlay_auto(remoteHandle, unit.id, UInt32(slotIndex), ms, 1)
                return
            }
            pushOverlays(unitId: unit.id)
            fail(mixer_unit_overlay_auto(unit.id, 1, ms, &desc), "overlay auto")
            return
        }
        session.units[unitIndex].overlays[slotIndex].enabled = false
        overlayOn[id] = false
        if isRemote {
            _ = mixer_remote_overlay_auto(remoteHandle, unit.id, UInt32(slotIndex), ms, 0)
            return
        }
        pushOverlays(forceEnabled: id, unitId: unit.id)
        fail(mixer_unit_overlay_auto(unit.id, 0, ms, &desc), "overlay auto")
        DispatchQueue.main.asyncAfter(deadline: .now() + .milliseconds(Int(ms))) { [weak self] in
            guard let self,
                  let ui = self.session.units.firstIndex(where: { $0.id == unit.id }),
                  let si = self.session.units[ui].overlays.firstIndex(where: { $0.id == id })
            else { return }
            if self.session.units[ui].overlays[si].enabled {
                return
            }
            self.overlayOn[id] = false
            self.pushOverlays(unitId: unit.id)
        }
    }

    func snapshotProgram() {
        saveSnapshot(
            sourceId: selectedUnitId,
            kind: EIVIZ_OUTPUT_PROGRAM,
            name: selectedUnit.name
        )
    }

    func snapshotScene(_ scene: SceneEntry) {
        saveSnapshot(sourceId: scene.gpuId, kind: 0, name: scene.name)
    }

    func snapshotInput(_ input: InputEntry) {
        saveSnapshot(sourceId: input.id, kind: EIVIZ_OUTPUT_SOURCE, name: input.name)
    }

    private func saveSnapshot(sourceId: UInt64, kind: UInt32, name: String) {
        let panel = NSSavePanel()
        panel.allowedContentTypes = [.png, .jpeg]
        panel.allowsOtherFileTypes = false
        panel.nameFieldStringValue = snapshotFileName(name)
        guard panel.runModal() == .OK, let url = panel.url else { return }
        MixerFFI.withCString(url.path) { path in
            _ = fail(mixer_snapshot(sourceId, kind, path), "Screenshot")
        }
    }

    private func snapshotFileName(_ name: String) -> String {
        let cleaned = name.replacingOccurrences(
            of: "[/\\\\?%*|\"<>:]",
            with: "_",
            options: .regularExpression
        )
        return (cleaned.isEmpty ? "eiviz" : cleaned) + ".png"
    }

    func saveSession() {
        let panel = NSSavePanel()
        panel.allowedContentTypes = [.json]
        panel.nameFieldStringValue = "eiviz.json"
        guard panel.runModal() == .OK, let url = panel.url else { return }
        session.selectedUnitId = selectedUnitId
        session.settings.lastSessionPath = nil
        do {
            let json = try SessionFile.encode(session)
            let saved = MixerFFI.withCString(url.path) { path in
                json.withUnsafeBytes { ptr in
                    fail(
                        mixer_session_save(path, ptr.bindMemory(to: UInt8.self).baseAddress, json.count),
                        "Save session"
                    )
                }
            }
            if saved {
                AppPrefs.shared.rememberSession(url.path)
            }
        } catch {
            presentError(L10n.error("Save session", 3), title: L10n.t("action.Save session"))
        }
    }

    func newSession() {
        replaceSession(MixerSessionData.default())
    }

    func loadSession(path: String? = nil) {
        let url: URL
        if let path {
            url = URL(fileURLWithPath: path)
        } else {
            let panel = NSOpenPanel()
            panel.allowedContentTypes = [.json]
            panel.allowsMultipleSelection = false
            guard panel.runModal() == .OK, let picked = panel.url else { return }
            url = picked
        }
        var buffer = [UInt8](repeating: 0, count: 1 << 20)
        let n = MixerFFI.withCString(url.path) { path in
            buffer.withUnsafeMutableBufferPointer { ptr in
                mixer_session_load(path, ptr.baseAddress, ptr.count)
            }
        }
        guard n > 0 else {
            fail(n == 0 ? 5 : n, "Load session")
            return
        }
        do {
            replaceSession(try SessionFile.decode(Data(buffer.prefix(Int(n)))))
            AppPrefs.shared.rememberSession(url.path)
        } catch {
            presentError(L10n.error("Load session", 3), title: L10n.t("action.Load session"))
        }
    }

    func recreateMixer() {
        shutdown()
        boot()
    }

    private func replaceSession(_ loaded: MixerSessionData) {
        closeAllInputPreviews()
        closeAllSwitchers()
        closeAllMultiviews()
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
    }

    private func bumpSurfaceEpoch() {
        surfaceEpoch &+= 1
    }

    private func closeAllMultiviews() {
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

    private func copyVideoInfo(_ id: UInt64) -> EivizVideoInfo? {
        var info = EivizVideoInfo(playing: 0, is_file: 0, position_hns: 0, duration_hns: 0)
        guard mixer_video_copy_info(id, &info) == EIVIZ_OK else { return nil }
        return info
    }

    private func startVideoInput(
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

    private func attachInputs() {
        for input in session.inputs where input.kind != .omt && input.kind != .ndi {
            attach(input)
        }
        for input in session.inputs where input.kind == .omt || input.kind == .ndi {
            attach(input)
        }
    }

    private func attach(_ input: InputEntry) {
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
                videoTitle = input.name
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
        }
        _ = mixer_audio_set_input(input.id, audioMask(input), input.gain, input.mute ? 1 : 0)
    }

    private func audioMask(_ input: InputEntry) -> UInt32 {
        input.kind == .mix ? 0 : (input.busMask == 0 ? 1 : input.busMask)
    }

    private func startCapture(_ input: InputEntry) {
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

    private func pushState(unitId: UInt64, program: UInt64, preview: UInt64, mix: Float, kind: UInt32) {
        var state = MixerFFI.emptyState()
        state.program_source = program
        state.preview_source = preview
        state.mix = mix
        state.transition_kind = kind
        let unit = session.units.first { $0.id == unitId } ?? selectedUnit
        fillAux(&state, unit: unit)
        fail(mixer_unit_set_state(unitId, &state), "Set Mixing Unit state")
    }

    private func currentState(_ unitId: UInt64) -> EivizUnitState {
        var state = MixerFFI.emptyState()
        _ = mixer_unit_get_state(unitId, &state)
        let unit = session.units.first { $0.id == unitId } ?? selectedUnit
        fillAux(&state, unit: unit)
        return state
    }

    private func fillAux(_ state: inout EivizUnitState, unit: MixingUnitEntry, forceEnabled: UUID? = nil) {
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

    private func unit(for id: UInt64?) -> MixingUnitEntry {
        guard let id else { return selectedUnit }
        return session.units.first { $0.id == id } ?? selectedUnit
    }

    private func normalizePixelSortDefaults(_ unitId: UInt64) {
        guard let index = session.units.firstIndex(where: { $0.id == unitId }) else { return }
        for i in session.units[index].transitions.indices {
            TransitionCatalog.applyKindDefaults(&session.units[index].transitions[i])
        }
    }

    private func resolvedCustomWgsl(_ preset: TransitionPreset) -> String {
        if preset.kind != EIVIZ_TRANSITION_CUSTOM {
            return ""
        }
        if let wgsl = preset.customWgsl, !wgsl.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            return wgsl
        }
        return CustomWgslEditor.template
    }

    private func tbarPreset() -> TransitionPreset {
        tbarPreset(for: selectedUnit)
    }

    private func tbarPreset(for unit: MixingUnitEntry) -> TransitionPreset {
        let list = unit.transitions
        guard !list.isEmpty else {
            return TransitionPreset(kind: EIVIZ_TRANSITION_CUT, durationValue: 1, swap: true)
        }
        return list[min(tbarPresetIndex, list.count - 1)]
    }

    private func tick() {
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
            videoTitle = session.inputs.first { $0.id == id }?.name ?? videoTitle
            if info.duration_hns > 0 {
                videoFraction = Double(info.position_hns) / Double(info.duration_hns)
            }
        }
        syncAllUnitBuses()
        updateStatus()
    }

    private func syncAllUnitBuses() {
        for unit in session.units {
            syncUnitBuses(unit.id)
        }
    }

    private func syncUnitBuses(_ unitId: UInt64) {
        var state = MixerFFI.emptyState()
        guard mixer_unit_get_state(unitId, &state) == EIVIZ_OK else { return }
        applyBusSources(unitId: unitId, preview: state.preview_source, program: state.program_source)
        applyMixerMix(unitId: unitId, mix: state.mix)
    }

    private func applyMixerMix(unitId: UInt64, mix value: Float) {
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

    private func applyBusSources(unitId: UInt64, preview: UInt64, program: UInt64) {
        if previewByUnit[unitId] != preview || programByUnit[unitId] != program {
            var nextPreview = previewByUnit
            var nextProgram = programByUnit
            nextPreview[unitId] = preview
            nextProgram[unitId] = program
            previewByUnit = nextPreview
            programByUnit = nextProgram
        }
    }

    private func mixUnitUses(_ unit: MixingUnitEntry, sourceId: UInt64) -> Bool {
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

    private func updateStatus() {
        if isRemote {
            return
        }
        let unit = selectedUnit
        status = "\(unit.width)x\(unit.height) \(unit.fpsLabel)   \(unit.name)"
    }

    private func refreshRemoteWarn() {
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
        } else {
            next = ""
        }
        if warnText != next {
            warnText = next
        }
    }

    private func handleMixerFatal() -> Bool {
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
        alert.runModal()
        NSApplication.shared.terminate(nil)
        return true
    }

    private func saveRecoveredSession() {
        let dir = FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Application Support/eiviz", isDirectory: true)
        do {
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            let url = dir.appendingPathComponent("recovered-session.eiviz.json")
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

    func surfaceRole(kind: UInt32, unitId: UInt64? = nil) -> SurfaceRole {
        let unit = unitId ?? selectedUnitId
        guard isRemote else {
            return .unit(unitId: unit, kind: kind)
        }
        if kind == EIVIZ_OUTPUT_PREVIEW, !selectedRemoteVideo(preview: true).address.isEmpty {
            return .monitor(monitorId: MixerRemote.previewMonitor ^ (unit << 8), sourceId: MixerRemote.previewSourceId)
        }
        if kind == EIVIZ_OUTPUT_PROGRAM, !selectedRemoteVideo(preview: false).address.isEmpty {
            return .monitor(monitorId: MixerRemote.programMonitor ^ (unit << 8), sourceId: MixerRemote.programSourceId)
        }
        return .monitor(monitorId: 0, sourceId: 0)
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

    private func commitRemoteOverlays(unitId: UInt64) {
        guard let unit = session.units.first(where: { $0.id == unitId }) else { return }
        for (index, slot) in unit.overlays.enumerated() {
            let code = MixerRemote.mutate(
                remoteHandle,
                MixerRemote.setOverlaySlot(unitId: unitId, index: UInt32(index), slot: slot),
                expected: remoteRevision
            )
            if code != 0 {
                presentError(remoteMutateError(), title: L10n.t("chrome.overlay"))
                pollRemote(force: true)
                return
            }
            remoteRevision += 1
        }
        pollRemote(force: true)
    }

    @discardableResult
    private func mutateRemote(_ json: String) -> Bool {
        let code = MixerRemote.mutate(remoteHandle, json, expected: remoteRevision)
        if code != 0 {
            presentError(remoteMutateError(), title: L10n.t("chrome.settings"))
            pollRemote(force: true)
            return false
        }
        pollRemote(force: true)
        return true
    }

    private func remoteMutateError() -> String {
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

    private func pollRemote(force: Bool = false) {
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
            let json = MixerRemote.snapshot(remoteHandle)
            if let data = json.data(using: .utf8), let loaded = try? SessionFile.decode(data) {
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
                bumpSurfaceEpoch()
            }
        }
        refreshRemoteWarn()
    }

    private func applyRemoteLiveBuses(_ liveJson: String) {
        guard let data = liveJson.data(using: .utf8),
              let root = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let units = root["units"] as? [String: Any]
        else { return }
        for unit in session.units {
            guard let live = units[String(unit.id)] as? [String: Any] else { continue }
            let preview = (live["previewSource"] as? NSNumber)?.uint64Value ?? 0
            let program = (live["programSource"] as? NSNumber)?.uint64Value ?? 0
            applyBusSources(unitId: unit.id, preview: preview, program: program)
        }
    }

    private func publishedOutputs() -> [OutputEntry] {
        session.outputs.filter {
            $0.enabled
                && ($0.transport == .omt || $0.transport == .ndi)
                && ($0.sourceKind == .muPreview || $0.sourceKind == .muProgram || $0.sourceKind == .multiview)
        }
    }

    private func remotePublishedSource(_ kind: OutputSourceKind, unitId: UInt64) -> OutputEntry? {
        let matches = publishedOutputs().filter { $0.sourceKind == kind && $0.unitId == unitId }
        guard matches.count == 1 else { return nil }
        return matches.first
    }

    private func remotePublishedMultiview(_ layoutGpuId: UInt64) -> UInt64? {
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
        for line in MixerFFI.discover({ mixer_omt_discover($0, $1) }) { add(.omt, line) }
        for line in MixerFFI.discover({ mixer_ndi_discover($0, $1) }) { add(.ndi, line) }
        for output in session.outputs where output.enabled && (output.transport == .omt || output.transport == .ndi) {
            add(output.transport, output.name)
        }
        return items
    }

    func selectedRemoteVideo(preview: Bool) -> RemoteVideoItem {
        let prefs = AppPrefs.shared
        let address = (preview ? prefs.previewVideoAddress : prefs.programVideoAddress)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        let raw = preview ? prefs.previewVideoTransport : prefs.programVideoTransport
        let transport: OutputTransport = raw.uppercased() == "NDI" ? .ndi : .omt
        if !address.isEmpty {
            let prefix = transport == .ndi ? "NDI" : "OMT"
            return RemoteVideoItem(transport: transport, address: address, label: "\(prefix)  \(address)")
        }
        if let output = remotePublishedSource(preview ? .muPreview : .muProgram, unitId: selectedUnitId) {
            let prefix = output.transport == .ndi ? "NDI" : "OMT"
            return RemoteVideoItem(transport: output.transport, address: output.name, label: "\(prefix)  \(output.name)")
        }
        return RemoteVideoItem(transport: .omt, address: "", label: L10n.t("chrome.videoNone"))
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

    private func syncPublishedVideo() {
        let preview = selectedRemoteVideo(preview: true)
        let program = selectedRemoteVideo(preview: false)
        let previewKey = "\(preview.transport.rawValue):\(preview.address)"
        let programKey = "\(program.transport.rawValue):\(program.address)"
        if previewKey != remotePreviewKey {
            remotePreviewKey = previewKey
            remotePreviewLive = connectRemoteChoice(MixerRemote.previewSourceId, preview)
        }
        if programKey != remoteProgramKey {
            remoteProgramKey = programKey
            remoteProgramLive = connectRemoteChoice(MixerRemote.programSourceId, program)
        }
        let outputs = publishedOutputs().filter { $0.sourceKind == .multiview }
        var keep: [UInt64: UInt64] = [:]
        for output in outputs {
            let id = MixerRemote.sourceBase | output.id
            keep[output.id] = id
            if remoteReceiveIds[output.id] == id {
                continue
            }
            if connectRemoteChoice(id, RemoteVideoItem(transport: output.transport, address: output.name, label: output.name)) {
                remoteReceiveIds[output.id] = id
            }
        }
        for (outputId, sourceId) in remoteReceiveIds where keep[outputId] == nil {
            _ = mixer_destroy_source(sourceId)
            remoteReceiveIds.removeValue(forKey: outputId)
        }
        let unavailable = !remotePreviewLive || !remoteProgramLive
        if videoUnavailable != unavailable {
            videoUnavailable = unavailable
        }
        refreshRemoteWarn()
    }

    @discardableResult
    private func connectRemoteChoice(_ id: UInt64, _ item: RemoteVideoItem) -> Bool {
        _ = mixer_destroy_source(id)
        guard !item.address.isEmpty else { return false }
        let code: Int32
        if item.transport == .ndi {
            code = MixerFFI.withCString(item.address) { mixer_ndi_connect(id, $0, 3, 0) }
        } else {
            code = MixerFFI.withCString(item.address) { mixer_omt_connect(id, $0, 1, 3, 0) }
        }
        return code == 0
    }

    @discardableResult
    private func fail(_ code: Int32, _ action: String) -> Bool {
        guard let message = MixerFFI.check(code, action) else { return true }
        presentError(message, title: L10n.t("action.\(action)"))
        return false
    }
}

private final class InputPreviewCloser: NSObject, NSWindowDelegate {
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

private final class SwitcherCloser: NSObject, NSWindowDelegate {
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
