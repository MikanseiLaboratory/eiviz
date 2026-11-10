using Eiviz.Host.I18n;

namespace Eiviz.Host.Dialogs;

/// <summary>
/// The queue of files an export writes, and the progress the deck shows while it does.
///
/// One export runs at a time, because the store does: the angle modes write one file per
/// angle, and those go out one after another rather than all at once. The deck used to keep
/// this queue among its own fields, along with the poll that advances it and the sentences
/// it reports; none of that is window code, and keeping it apart is what stops the window
/// from growing back.
/// </summary>
internal sealed class ReplayExportController
{
    /// <summary>`state` values the module reports, matching `eiviz-pro-api`'s `EXPORT_*`.</summary>
    private const uint Running = 1;
    private const uint Done = 2;
    private const uint Failed = 3;
    private const uint Cancelled = 4;

    private readonly Queue<Item> _queue = new();
    private bool _busy;

    /// <summary>Whether a file is being written right now.</summary>
    public bool IsBusy => _busy;

    /// <summary>Shows one export sentence. The deck puts it on its own status line too.</summary>
    public event Action<string>? Reported;

    /// <summary>Queues one export body, and starts it when nothing else is running.</summary>
    /// <param name="body">The request JSON, ready to hand to the mixer.</param>
    /// <param name="label">The file name, for the sentence that says what is being written.</param>
    /// <param name="backend">The mixer, or null when one is not open.</param>
    public void Start(string body, string label, IEivizBackend? backend)
    {
        _queue.Enqueue(new Item(body, label));
        if (!_busy)
            StartNext(backend);
    }

    /// <summary>
    /// Follows the running export. Called from the deck's poll; does nothing when no export
    /// is queued, so an idle deck costs one bool test per frame.
    /// </summary>
    /// <param name="backend">The mixer, or null when one is not open.</param>
    public void Poll(IEivizBackend? backend)
    {
        if (!_busy || backend is null)
            return;
        if (backend.ReplayExportProgress() is not { Length: > 0 } progress)
            return;
        var step = ReplayExportProgress.Parse(progress);
        switch (step.State)
        {
            case Running:
                Reported?.Invoke(Loc.Format("replay.exportRunning", step.Done, step.Total, step.Skipped));
                return;
            case Done:
                _busy = false;
                if (_queue.Count > 0)
                {
                    StartNext(backend);
                    return;
                }
                Reported?.Invoke(Loc.Format("replay.exportDone", step.Done, step.Skipped));
                return;
            case Failed:
                Stop();
                Reported?.Invoke(Loc.Format("replay.exportError", step.Message));
                return;
            case Cancelled:
                Stop();
                Reported?.Invoke(Loc.T("replay.exportCancelled"));
                return;
            default:
                // Idle: nothing has started yet, so there is nothing to say.
                return;
        }
    }

    /// <summary>Starts the next queued file, or stops when the queue is empty.</summary>
    private void StartNext(IEivizBackend? backend)
    {
        if (_queue.Count == 0)
        {
            _busy = false;
            return;
        }
        var next = _queue.Dequeue();
        if (backend is null)
        {
            Stop();
            Reported?.Invoke(Loc.T("replay.exportFailed"));
            return;
        }
        if (!backend.ReplayExportStart(next.Body, out var error))
        {
            Stop();
            Reported?.Invoke(error.Length > 0 ? error : Loc.T("replay.exportFailed"));
            return;
        }
        _busy = true;
        Reported?.Invoke(Loc.Format("replay.exportStarted", next.Label));
    }

    /// <summary>Stops the queue: nothing more is written after a refusal or a cancel.</summary>
    private void Stop()
    {
        _busy = false;
        _queue.Clear();
    }

    /// <summary>One queued file: what to write and what to call it.</summary>
    private sealed record Item(string Body, string Label);
}

/// <summary>
/// One reading of the export progress the mixer reports, parsed once.
///
/// The deck and the export window both read these numbers, and the JSON keys are a contract
/// with the mixer; naming them where they are parsed keeps a renamed key from silently
/// reading zero.
/// </summary>
internal readonly record struct ReplayExportProgress(
    uint State,
    ulong Done,
    ulong Total,
    ulong Skipped,
    string Message)
{
    /// <summary>Reads the JSON the mixer writes. A malformed report reads as idle.</summary>
    public static ReplayExportProgress Parse(string json)
    {
        try
        {
            using var document = System.Text.Json.JsonDocument.Parse(json);
            var root = document.RootElement;
            ulong Number(string name) =>
                root.TryGetProperty(name, out var value) && value.TryGetUInt64(out var number)
                    ? number
                    : 0;
            return new ReplayExportProgress(
                (uint)Number("state"),
                Number("done"),
                Number("total"),
                Number("skipped"),
                root.TryGetProperty("message", out var message) ? message.GetString() ?? "" : "");
        }
        catch (System.Text.Json.JsonException)
        {
            return default;
        }
    }
}
