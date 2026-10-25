import Foundation
import Combine

/// The queue of files an export writes, and the progress it reports while it does.
///
/// One export runs at a time, because the store does: the angle modes write one file per
/// angle, and those go out one after another. The deck used to keep this queue among its
/// own `@State`, along with the poll that advances it and the sentences it shows; none of
/// that is view code, and keeping it apart is what stops the window from growing back.
///
/// It is an `ObservableObject` like `MixerController`, so the view watches it the same way
/// it watches everything else in this host.
@MainActor
final class ReplayExportRunner: ObservableObject {
    /// One file to write: the request, and the name to show for it.
    private struct Job {
        let json: String
        let label: String
    }

    private var queue: [Job] = []

    /// True while a file is being written, so the deck disables the button.
    @Published private(set) var busy = false
    /// The sentence the deck shows: what is being written, or how it ended.
    @Published private(set) var status = ""

    /// Puts a sentence on the status line without touching the queue, for the checks the
    /// window makes before anything is queued ("choose a file first").
    func report(_ sentence: String) {
        status = sentence
    }

    /// Queues one export and starts it when nothing else is running.
    ///
    /// Returns false when the mixer refused to start it. The queue is left alone: a caller
    /// that queued several files (one per angle) should stop adding them, and a caller that
    /// queued one should see the refusal rather than a queue it silently discarded.
    @discardableResult
    func start(json: String, label: String, mixer: MixerController) -> Bool {
        queue.append(Job(json: json, label: label))
        guard !busy else { return true }
        return startNext(mixer: mixer)
    }

    /// Drops everything queued and stops the running export.
    ///
    /// The mixer keeps writing until it has drained what it already had, so `busy` stays
    /// true and the poll waits for it to report `cancelled`. Clearing the flag here instead
    /// let the operator start a second export while the first was still running, and the
    /// later `done` of the first one was then read as the second one finishing — which
    /// skipped the next queued file.
    func cancel(mixer: MixerController) {
        queue = []
        mixer.cancelReplayExport()
    }

    /// Follows the running export. Called from the deck's poll; does nothing while idle.
    func poll(mixer: MixerController) {
        guard busy, let progress = mixer.replayExportProgress() else { return }
        switch progress.state {
        case .running:
            status = L10n.format(
                "replay.exportRunning",
                "\(progress.done)",
                "\(progress.total)",
                "\(progress.skipped)")
        case .done:
            if queue.isEmpty {
                busy = false
                status = L10n.format(
                    "replay.exportDone",
                    "\(progress.done)",
                    "\(progress.skipped)")
            } else {
                startNext(mixer: mixer)
            }
        case .failed:
            stop(reporting: L10n.format("replay.exportError", progress.message))
        case .cancelled:
            stop(reporting: L10n.t("replay.exportCancelled"))
        case .idle:
            break
        }
    }

    /// Starts the next queued file, or stops when the queue is empty.
    @discardableResult
    private func startNext(mixer: MixerController) -> Bool {
        guard !queue.isEmpty else {
            busy = false
            return true
        }
        let job = queue.removeFirst()
        guard mixer.startReplayExport(job.json) else {
            // A refusal is the mixer saying this file cannot be written at all. Stop
            // rather than working through the rest of the queue, which would fail the
            // same way; the caller sees false and can tell the operator.
            busy = false
            status = L10n.t("replay.exportFailed")
            return false
        }
        busy = true
        status = L10n.format("replay.exportStarted", job.label)
        return true
    }

    /// Stops the queue after a failure the mixer reported while writing.
    ///
    /// Unlike a refusal to start, this one has already produced a sentence, and nothing
    /// more should be written after it: the disk or the encoder is the problem, and the
    /// next file would hit the same one.
    private func stop(reporting sentence: String) {
        busy = false
        queue = []
        status = sentence
    }
}
