import AppKit
import EivizMixer
import Foundation
import SwiftUI

/// Replay exists only while the project has a Replay input, so every entry point
/// checks that first.
extension MixerController {
    var hasReplayInput: Bool {
        session.inputs.contains { $0.kind == .replay }
    }

    var replayDocument: ReplayDocument? {
        ReplayDocument(session.replay)
    }

    /// Assigns the reserved channel id and creates the replay document when this
    /// is the first channel. Returns false when A-Z are all in use.
    func assignReplayChannel(_ input: inout InputEntry) -> Bool {
        var index: UInt32 = 0
        while let document = replayDocument, document.channels.contains(where: { $0.index == index }) {
            index += 1
        }
        guard index < 26 else { return false }
        input.id = EIVIZ_REPLAY_CHANNEL_BASE | UInt64(index + 1)
        input.name = ReplayChannelInfo(index: index, inputId: input.id).label
        var document = replayDocument ?? ReplayDocument.empty()
        document.addChannel(index: index, inputId: input.id)
        session.replay = document.value
        return true
    }

    /// Drops the channel an input owns and removes the document when it was the
    /// last one, which returns the project to the "no Replay" state.
    func removeReplayChannel(inputId: UInt64) {
        guard session.inputs.contains(where: { $0.id == inputId && $0.kind == .replay }),
              var document = replayDocument
        else { return }
        let index = document.channels.first { $0.inputId == inputId }?.index
        if document.removeChannel(inputId: inputId) {
            session.replay = nil
            // Cues to a channel nobody owns go with Replay, or the session stops
            // loading.
            clearReplayCues(all: true)
            closeReplayWindows()
            return
        }
        if let index {
            // Only the cues that named this channel are dead.
            clearReplayCues(channel: index)
        }
        session.replay = document.value
    }

    /// True when the last channel is about to be removed.
    func isLastReplayChannel(inputId: UInt64) -> Bool {
        guard let document = replayDocument else { return false }
        return document.channels.count == 1 && document.channels.contains { $0.inputId == inputId }
    }

    /// Replay format frame rate, with the session master rate as the fallback the
    /// core also uses.
    func replayFps(_ document: ReplayDocument) -> (num: UInt32, den: UInt32) {
        ReplayTime.resolve(document.fps, session.settings)
    }

    func openReplay() {
        guard hasReplayInput else { return }
        if let existing = replayWindow {
            existing.makeKeyAndOrderFront(nil)
            return
        }
        guard FlipBudget.tryOpen(1) else { return }
        let host = NSHostingController(
            rootView: ReplayView()
                .environmentObject(self)
                .environment(\.mixerSurfaceEpoch, surfaceEpoch)
        )
        let window = SwitcherHostWindow(
            contentRect: NSRect(x: 0, y: 0, width: 1240, height: 780),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        window.title = L10n.t("replay.windowTitle")
        window.identifier = NSUserInterfaceItemIdentifier("replay")
        window.contentViewController = host
        window.isReleasedWhenClosed = false
        window.appearance = NSApp.appearance
        window.backgroundColor = EivizTheme.nsBackground
        window.center()
        replayCloser.onClose = { [weak self] in
            Task { @MainActor in self?.replayWindow = nil }
        }
        window.delegate = replayCloser
        window.makeKeyAndOrderFront(nil)
        replayWindow = window
    }

    /// Closes the recording settings. Applying and cancelling both end here, so the
    /// operator sees the window go away either way.
    func closeReplaySettings() {
        replaySettingsWindow?.close()
        replaySettingsWindow = nil
    }

    func openReplaySettings() {
        guard hasReplayInput else { return }
        if let existing = replaySettingsWindow {
            existing.makeKeyAndOrderFront(nil)
            return
        }
        let host = NSHostingController(
            rootView: ReplaySettingsView()
                .environmentObject(self)
                .environment(\.mixerSurfaceEpoch, surfaceEpoch)
        )
        let window = SwitcherHostWindow(
            contentRect: NSRect(x: 0, y: 0, width: 720, height: 640),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        window.title = L10n.t("replay.setupTitle")
        window.identifier = NSUserInterfaceItemIdentifier("replay-setup")
        window.contentViewController = host
        window.isReleasedWhenClosed = false
        window.appearance = NSApp.appearance
        window.backgroundColor = EivizTheme.nsBackground
        window.center()
        replaySettingsCloser.onClose = { [weak self] in
            Task { @MainActor in self?.replaySettingsWindow = nil }
        }
        window.delegate = replaySettingsCloser
        window.makeKeyAndOrderFront(nil)
        replaySettingsWindow = window
    }

    func closeReplayWindows() {
        replayWindow?.close()
        replaySettingsWindow?.close()
    }

    /// Recorded frame range of one angle, for the export UI.
    func replayAngleRange(_ angleId: UInt32) -> (first: UInt64, last: UInt64)? {
        MixerFFI.replayAngleRange(angleId: angleId)
    }

    /// Starts an MP4 export of recorded segments. Returns false when the mixer or
    /// the Pro module refuses it.
    func startReplayExport(_ json: String) -> Bool {
        MixerFFI.replayExportStart(json) == EIVIZ_OK
    }

    func cancelReplayExport() {
        _ = MixerFFI.replayExportCancel()
    }

    /// Export progress, or nil while none is running.
    func replayExportProgress() -> ReplayExportProgress? {
        let json = MixerFFI.replayExportProgress()
        guard !json.isEmpty else { return nil }
        return ReplayExportProgress(json: json)
    }
    /// Writes the replay document back and pushes it to the core. In remote mode
    /// the core owns the document, so it goes through a mutation instead.
    /// Writes the recording settings. Returns false when the core refused the
    /// document, so the caller can keep the window open and say why.
    @discardableResult
    func applyReplay(_ document: ReplayDocument) -> Bool {
        session.replay = document.value
        if isRemote {
            return mutateRemote(MixerRemote.upsertReplaySettings(document.value))
        }
        return replaceRuntime()
    }

    /// Clears the animation cues that name a Replay channel or clip. A cue whose
    /// channel or clip is gone would stop the session loading, so it goes with
    /// what it named. An empty clip id matches nothing, because that is how "the
    /// newest clip" is written. Returns how many were cleared.
    @discardableResult
    func clearReplayCues(channel: UInt32? = nil, clipUid: String? = nil, all: Bool = false) -> Int {
        var cleared = 0
        for sceneIndex in session.scenes.indices {
            for sequenceIndex in session.scenes[sceneIndex].sequences.indices {
                let steps = session.scenes[sceneIndex].sequences[sequenceIndex].steps
                for stepIndex in steps.indices {
                    guard let action = session.scenes[sceneIndex].sequences[sequenceIndex].steps[stepIndex].replay
                    else { continue }
                    let namedChannel = channel.map { action.channel == $0 } ?? false
                    let namedClip = (clipUid?.isEmpty == false) && action.clipUid == clipUid
                    if !all && !namedChannel && !namedClip { continue }
                    session.scenes[sceneIndex].sequences[sequenceIndex].steps[stepIndex].replay = nil
                    cleared += 1
                }
            }
        }
        if cleared > 0 {
            objectWillChange.send()
        }
        return cleared
    }

    /// The angle that records a scene, when Replay has one.
    func replayAngle(recording sceneId: UInt64) -> ReplayAngleInfo? {
        replayDocument?.angles.first { $0.sceneId == sceneId }
    }

    /// The angle that records a mixing unit's program mix, when Replay has one.
    func replayAngle(listening unitId: UInt64) -> ReplayAngleInfo? {
        guard unitId != 0 else { return nil }
        return replayDocument?.angles.first { $0.audioUnitId == unitId }
    }

    @discardableResult
    func replayEnableRecording(_ on: Bool) -> Bool {
        if isRemote {
            return MixerRemoteReplay.record(handle: remoteHandle, on: on) == 0
        }
        if let document = replayDocument {
            _ = MixerFFI.replayConfigure(document.json)
        }
        return MixerFFI.replayRecord(on) == 0
    }

    /// Returns the mark-in frame when the host owns it. The remote core keeps the
    /// mark, so there is nothing to return there.
    func replayMarkIn() -> UInt64? {
        if isRemote {
            _ = MixerRemoteReplay.markIn(handle: remoteHandle)
            return nil
        }
        return MixerFFI.replayMarkIn()
    }

    /// Takes the Mark Out. The core that owns the recording creates the event from
    /// its own mark, so only the host that took the mark sees the frames.
    func replayMarkOut(name: String) -> ReplayMarkOutcome {
        if isRemote {
            return MixerRemoteReplay.markOut(handle: remoteHandle, name: name) == 0
                ? .remote
                : .refused
        }
        guard let mark = MixerFFI.replayMarkOut() else { return .refused }
        return .local(uuid: mark.uuid, inFrame: mark.inFrame, outFrame: mark.outFrame)
    }

    @discardableResult
    func replayCue(channel: UInt32, clip: ReplayClipInfo) -> Bool {
        if isRemote {
            return MixerRemoteReplay.cue(
                handle: remoteHandle,
                channel: channel,
                clipUid: clip.uid
            ) == 0
        }
        return MixerFFI.replayCue(
            channel: channel,
            angleId: clip.angleId,
            inFrame: clip.inFrame,
            outFrame: clip.outFrame,
            recordingUuid: clip.recordingUuid
        ) == 0
    }

    @discardableResult
    func replayTransport(channel: UInt32, playing: Bool, speedMilli: Int32, seekFrame: Int64) -> Bool {
        if isRemote {
            return MixerRemoteReplay.transport(
                handle: remoteHandle,
                channel: channel,
                playing: playing,
                speedMilli: speedMilli,
                seekFrame: seekFrame
            ) == 0
        }
        return MixerFFI.replayTransport(
            channel: channel,
            playing: playing,
            speedMilli: speedMilli,
            seekFrame: seekFrame
        ) == 0
    }

    @discardableResult
    func replayJumpToNow(channel: UInt32) -> Bool {
        if isRemote {
            return MixerRemoteReplay.jumpToNow(handle: remoteHandle, channel: channel) == 0
        }
        return MixerFFI.replayJumpToNow(channel: channel) == 0
    }

    func replayStats() -> EivizReplayStats? {
        isRemote ? nil : MixerFFI.replayStats()
    }

    /// Replay stats with the per-angle breakdown for the health line.
    func replayStatsWithAngles() -> (stats: EivizReplayStats, angles: [EivizReplayAngleStats])? {
        isRemote ? nil : MixerFFI.replayStatsWithAngles()
    }

    /// Plan limits for the Replay UI. Remote mode cannot read them yet.
    func replayLimits() -> (angles: UInt32, channels: UInt32)? {
        guard !isRemote else { return nil }
        var caps = EivizCapabilities()
        guard mixer_capabilities(&caps) == 0 else { return nil }
        return (caps.replay_angle_limit, caps.replay_channel_limit)
    }
}

final class ReplayCloser: NSObject, NSWindowDelegate {
    var onClose: (() -> Void)?

    func windowWillClose(_ notification: Notification) {
        onClose?()
    }
}
