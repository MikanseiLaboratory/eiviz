using System.IO;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Windows;
using System.Windows.Automation;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Threading;
using Eiviz.Host.I18n;
using Eiviz.Host.Interop;
using Eiviz.Host.Preview;

namespace Eiviz.Host.Dialogs;

/// <summary>
/// Live replay operator surface. Mark In/Out and the transport sit together at
/// the bottom, the angle multiview and the event/playlist lists fill the middle,
/// and the recording timecode is always visible.
/// </summary>
public partial class ReplayWindow : Window
{
    private const uint MarkUnknown = uint.MaxValue;

    // The live document: App.ReplaceDocument swaps the Session object whenever the mixer reports a change, so a window that keeps the instance it was opened with writes its edits back into an outdated copy.
    private Session _session => ((App)Application.Current).Session;
    /// <summary>The window that opened the deck, for the messages it sends back.</summary>
    private readonly MainWindow? _main;
    private readonly DispatcherTimer _timer = new() { Interval = TimeSpan.FromMilliseconds(200) };
    private readonly List<SwapchainHost> _angleHosts = [];
    /// <summary>
    /// Monitor ids of the two deck monitors: what the channel is sending out and
    /// the angle being recorded. They are retargeted only when the source changes,
    /// because a retarget every poll would tear the picture.
    /// </summary>
    private ulong _outputMonitorId;
    private ulong _liveMonitorId;
    private ulong _outputSource;
    private ulong _liveSource;
    private JsonObject? _replay;
    private string _lastJson = "";
    private ulong _playlistId;
    /// <summary>The project's playlists, as the picker shows them.</summary>
    private IReadOnlyList<ReplayPlaylistInfo> _playlistChoices = [];
    private uint _channel;
    private ulong? _inFrame;
    /// <summary>True while the channel follows the recording edge instead of a clip.</summary>
    private bool _live = true;
    /// <summary>True while the playlist goes round again at its end.</summary>
    private bool _looping;
    /// <summary>
    /// Paints the remaining time between the 200 ms status ticks. A countdown that
    /// steps five times a second reads as a stopped clock; the position is known, so
    /// it is advanced by the elapsed time instead of waiting for the next tick.
    /// </summary>
    private readonly DispatcherTimer _headerTimer = new() { Interval = TimeSpan.FromMilliseconds(33) };
    private ulong _headerOut;
    private ulong _headerPosition;
    private long _headerAt;
    private bool _headerPlaying;    /// <summary>The operator picked a camera in this window since it last loaded.</summary>
    private bool _angleEdited;
    /// <summary>Events this window deleted, so a merge does not put them back.</summary>
    private readonly HashSet<ulong> _deletedEvents = [];
    /// <summary>Clips this window deleted, by uid.</summary>
    private readonly HashSet<string> _deletedClips = [];
    /// <summary>True while the input picture is a grid of every angle.</summary>
    private bool _multiview;
    /// <summary>True while the seek slider is being moved to match the channel.</summary>
    private bool _sliderGuard;
    private ulong _outFrame;
    /// <summary>
    /// True between Mark In and Mark Out. The mark itself stays on screen after it
    /// is taken, so this, not <c>_inFrame</c>, decides whether Mark Out may run.
    /// </summary>
    private bool _markOpen;
    private bool _playing;
    /// <summary>
    /// True while the mixer is recording. A Remote cannot read it, so it stays null
    /// there and the deck keeps both buttons available.
    /// </summary>
    private bool? _recording;
    private int _speedMilli = 1000;
    private bool _loadingTransition;
    private TagCheckPanel? _clipTags;
    private readonly List<ClipAudioChoice> _clipAudio = [];
    /// <summary>
    /// A message the operator has to see, held over the state line for a few ticks.
    /// The state line is rewritten every poll, so a notice that is not repeated
    /// would vanish before it could be read.
    /// </summary>
    private string _notice = "";
    private int _noticeTicks;
    /// Export states, matching the module's `EXPORT_*`.
    private const uint ExportIdle = 0;
    private const uint ExportRunning = 1;
    private const uint ExportDone = 2;
    private const uint ExportFailed = 3;
    private const uint ExportCancelled = 4;
    private readonly Queue<ExportJobSpec> _exportQueue = new();
    private bool _exporting;

    private static IEivizBackend? Backend => (Application.Current as App)?.Backend;

    /// <summary>When the operator last dragged the waveform, so a drag does not flood
    /// the mixer with one seek per pixel.</summary>
    private long _lastSeekAt;

    public ReplayWindow(Session session, MainWindow? main = null)
    {
        InitializeComponent();
        // The waveform doubles as the seek bar.
        Waveform.SeekRequested += OnWaveformSeek;
        _headerTimer.Tick += (_, _) => UpdateHeader();
        Closed += (_, _) => _headerTimer.Stop();

        _main = main;
        Loaded += OnLoaded;
        Closed += OnClosed;
        _timer.Tick += (_, _) => Tick();
    }

    private void OnLoaded(object sender, RoutedEventArgs e)
    {
        _replay = ReplayModel.Parse(_session);
        RefreshExportModes();
        if (_replay is null)
        {
            MessageBox.Show(this, Loc.T("replay.missing"), Title);
            Close();
            return;
        }
        _lastJson = _session.ReplayJson;
        RefreshPlaylistChoices();
        // The deck monitors: the channel's own picture (what goes out) and the angle
        // being recorded (what comes in).
        _outputMonitorId = _session.NextMonitorId++;
        _liveMonitorId = _session.NextMonitorId++;
        OutputMonitor.Loaded += (_, _) => RetargetMonitors();
        LiveMonitor.Loaded += (_, _) => RetargetMonitors();
        // The same rule the main window applies to its preview surfaces: a monitor is
        // presented every N master frames, N coming from the session. Presenting every
        // frame costs GPU time for a picture an eye cannot tell apart, and the deck is
        // one of several consumers of the same texture.
        ApplyDeckPresentInterval();
        BuildChannels();
        BuildAngles();
        // The speed the deck starts on is lit, the way a controller's transport shows it.
        RefreshSpeedButtons();
        RefreshEvents();
        RefreshPlaylist();
        Tick();
        _timer.Start();
        _headerTimer.Start();
    }

    private void OnClosed(object? sender, EventArgs e)
    {
        _timer.Stop();
        _headerTimer.Stop();
        foreach (var host in _angleHosts)
            host.ReleaseNative();
        _angleHosts.Clear();
        OutputMonitor.ReleaseNative();
        LiveMonitor.ReleaseNative();
    }

    /// <summary>
    /// Marks the clip the channel is playing and keeps the transport label honest.
    /// Playback is the mixer's state, not the window's: a playlist that is not
    /// looping stops on its own, and the deck must say so the moment it happens.
    /// </summary>
    private void RefreshOnAir()
    {
        if (_replay is null)
            return;
        var state = Backend?.ReplayChannelState(_channel);
        if (state is { } mixer)
        {
            var wasPlaying = _playing;
            _playing = mixer.Playing != 0;
            _headerOut = mixer.OutFrame;
            _headerPosition = mixer.PositionFrame;
            _headerPlaying = mixer.Playing > 0;
            _headerAt = System.Diagnostics.Stopwatch.GetTimestamp();
            if (wasPlaying != _playing)
            {
                UpdatePlayButton();
                UpdatePauseButton();
            }
        }
        // While a clip plays, the header shows how much of it is left instead of the
        // recording timecode: that is the number the operator acts on during a replay.
        if (state is { Playing: > 0, OutFrame: > 0 } air && air.OutFrame > air.PositionFrame)
        {
            var fps = _replay is null ? 0.0 : ReplayModel.Rate(ReplayModel.Fps(_replay, _session));
            if (fps > 0)
            {
                RecordTcLabel.Text = Loc.T("replay.clipRemaining");
                RecordTcText.Text = "-" + ReplayModel.Timecode(air.OutFrame - air.PositionFrame, ReplayModel.Fps(_replay!, _session));
            }
        }
        else
        {
            RecordTcLabel.Text = Loc.T("replay.recordTc");
        }
        // The waveform follows the picture, not the recorder: while a clip plays the
        // middle of the strip is the frame on air, so the operator sees what is behind
        // and ahead of the playhead.
        if (state is { Playing: > 0 } live)
        {
            Waveform.PositionFrame = live.PositionFrame;
        }
        var rows = PlaylistList.ItemsSource as List<ClipRow> ?? [];
        var playing = state is { Playing: > 0 };
        foreach (var row in rows)
        {
            var onAir = playing
                && state is { } now
                && now.InFrame == row.InFrame
                && now.OutFrame == row.OutFrame
                && now.AngleId == row.AngleId;
            var marker = onAir ? "▶" : "";
            if (row.Marker != marker)
            {
                row.IsLoading = true;
                row.Marker = marker;
                row.IsLoading = false;
            }
        }
    }

    /// <summary>
    /// Points the deck monitors at the channel's output and the selected angle's
    /// live picture. Called on load and whenever either source changes.
    /// </summary>
    private void RetargetMonitors()
    {
        if (_replay is null)
            return;
        var channel = ReplayModel.Channels(_replay).FirstOrDefault(item => item.Index == _channel);
        var source = channel?.SourceId ?? 0;
        if (source != _outputSource)
        {
            _outputSource = source;
            if (source != 0)
                OutputMonitor.RetargetMonitor(_outputMonitorId, source);
        }
        OutputLabel.Text = channel is null
            ? Loc.T("replay.noChannels")
            : Loc.Format("replay.outputLabel", channel.Label, _playing ? Loc.T("replay.playing") : Loc.T("replay.stopped"));
        // Whether the channel reaches the program is the operator's business: the deck
        // reports what the channel is sending, not where it is used.

        var angle = ActiveAngle();
        var live = angle?.LiveSourceId ?? 0;
        if (live != _liveSource)
        {
            _liveSource = live;
            if (live != 0)
                LiveMonitor.RetargetMonitor(_liveMonitorId, live);
        }
        LiveLabel.Text = angle is null
            ? Loc.T("replay.noActiveAngle")
            : Loc.Format("replay.inputLabel", angle.Label);
    }

    /// <summary>Angle the operator is watching, or null when Replay has none.</summary>
    private ReplayAngleInfo? ActiveAngle()
    {
        if (_replay is null)
            return null;
        var active = ReplayModel.ActiveAngle(_replay);
        foreach (var angle in ReplayModel.Angles(_replay))
        {
            if (angle.Id == active)
                return angle;
        }
        return ReplayModel.Angles(_replay).FirstOrDefault();
    }

    /// <summary>
    /// Fills the playlist picker from the document and shows the one this channel plays.
    /// A project can hold several playlists, so the picker is what says which one the
    /// deck below is showing.
    /// </summary>
    private void RefreshPlaylistChoices()
    {
        if (_replay is null)
            return;
        var lists = ReplayModel.Playlists(_replay);
        if (lists.Count == 0)
        {
            ReplayModel.EnsurePlaylist(_replay);
            lists = ReplayModel.Playlists(_replay);
        }
        var current = ReplayModel.ChannelPlaylist(_replay, _channel) ?? lists[0].Id;
        _playlistId = current;
        _playlistChoices = lists;
        PlaylistBox.ItemsSource = lists;
        PlaylistBox.DisplayMemberPath = nameof(ReplayPlaylistInfo.Label);
        PlaylistBox.SelectedItem = lists.FirstOrDefault(list => list.Id == current) ?? lists[0];
        PlaylistNameBox.Text = lists.FirstOrDefault(list => list.Id == current)?.Name ?? "";
        PlaylistRemoveButton.IsEnabled = lists.Count > 1;
    }

    private void Playlist_Changed(object sender, SelectionChangedEventArgs e)
    {
        if (_replay is null || PlaylistBox.SelectedItem is not ReplayPlaylistInfo choice)
            return;
        if (choice.Id == _playlistId)
            return;
        // Switching the picker moves the *channel*: that is what plays a playlist.
        _playlistId = choice.Id;
        ReplayModel.SetChannelPlaylist(_replay, _channel, choice.Id);
        PlaylistNameBox.Text = choice.Name;
        SaveDocument();
        RefreshPlaylist();
    }

    private void PlaylistName_Changed(object sender, RoutedEventArgs e)
    {
        if (_replay is null)
            return;
        var name = PlaylistNameBox.Text.Trim();
        if (name.Length == 0)
        {
            Warn(Loc.T("replay.playlistNameEmpty"));
            RefreshPlaylistChoices();
            return;
        }
        if (!ReplayModel.RenamePlaylist(_replay, _playlistId, name))
            return;
        SaveDocument();
        RefreshPlaylistChoices();
    }

    private void PlaylistAdd_Click(object sender, RoutedEventArgs e)
    {
        if (_replay is null)
            return;
        var id = ReplayModel.AddPlaylist(_replay, Loc.Format("replay.playlistDefaultName", ReplayModel.Playlists(_replay).Count + 1));
        // A new playlist is for this channel: creating one and then having to point the
        // channel at it would be two steps for one intention.
        ReplayModel.SetChannelPlaylist(_replay, _channel, id);
        _playlistId = id;
        SaveDocument();
        RefreshPlaylistChoices();
        RefreshPlaylist();
        Warn(Loc.Format("replay.playlistAdded", PlaylistNameBox.Text));
    }

    private void PlaylistRemove_Click(object sender, RoutedEventArgs e)
    {
        if (_replay is null || _playlistChoices.Count <= 1)
        {
            Warn(Loc.T("replay.playlistLast"));
            return;
        }
        var users = ReplayModel.ChannelsUsingPlaylist(_replay, _playlistId);
        if (users > 1)
        {
            Warn(Loc.Format("replay.playlistInUse", users));
            return;
        }
        var clips = ReplayModel.Clips(_replay, _playlistId).Count;
        if (clips > 0)
        {
            var answer = MessageBox.Show(
                this,
                Loc.Format("replay.playlistDeleteClips", clips),
                Title,
                MessageBoxButton.YesNo,
                MessageBoxImage.Warning);
            if (answer != MessageBoxResult.Yes)
                return;
        }
        ReplayModel.RemovePlaylist(_replay, _playlistId);
        var remaining = ReplayModel.Playlists(_replay);
        var next = remaining.FirstOrDefault();
        if (next is not null)
            ReplayModel.SetChannelPlaylist(_replay, _channel, next.Id);
        SaveDocument();
        RefreshPlaylistChoices();
        RefreshPlaylist();
    }

    /// <summary>
    /// The keys an operator works by, the way a replay controller has them: mark, play,
    /// step, cue. A field being typed into keeps its own keys, so a name can contain an
    /// i or an o.
    /// </summary>
    private void Window_PreviewKeyDown(object sender, KeyEventArgs e)
    {
        if (Keyboard.FocusedElement is TextBox or ComboBox)
            return;
        switch (e.Key)
        {
            case Key.Space:
                Play_Click(this, new RoutedEventArgs());
                break;
            case Key.I:
                MarkIn_Click(this, new RoutedEventArgs());
                break;
            case Key.O:
                MarkOut_Click(this, new RoutedEventArgs());
                break;
            case Key.L:
                Live_Click(this, new RoutedEventArgs());
                break;
            case Key.Left:
                StepBack_Click(this, new RoutedEventArgs());
                break;
            case Key.Right:
                StepForward_Click(this, new RoutedEventArgs());
                break;
            case Key.Enter:
                CueSelectedClip();
                break;
            case Key.Delete:
                ClipRemove_Click(this, new RoutedEventArgs());
                break;
            default:
                return;
        }
        e.Handled = true;
    }

    private void Tick()
    {
        // The document can change under us: the remote core accepts edits from
        // another host, and the vMix API can create events.
        if (_session.ReplayJson != _lastJson)
        {
            var parsed = ReplayModel.Parse(_session);
            if (parsed is not null)
            {
                _replay = parsed;
                // The document now on air is the truth, including the camera another
                // writer selected while this window was open.
                _angleEdited = false;
                _deletedEvents.Clear();
                _deletedClips.Clear();
                RefreshPlaylistChoices();
                _lastJson = _session.ReplayJson;
                BuildAngles();
                RefreshEvents();
                RefreshPlaylist();
                RefreshDefaultTransition();
            }
        }
        RefreshStatus();
        RefreshOnAir();
        RetargetMonitors();
        RefreshExport();
    }

    /// <summary>
    /// Export progress, and the queue that follows it. Several files (one per
    /// angle) are written one after another, because the store runs one export at
    /// a time.
    /// </summary>
    /// <summary>
    /// Follows a running export. The window shows the progress; the deck's status line
    /// carries the same sentence so an operator watching the picture still sees it.
    /// </summary>
    private void RefreshExport()
    {
        if (!_exporting)
            return;
        var progress = Backend?.ReplayExportProgress() ?? "";
        if (string.IsNullOrEmpty(progress))
            return;
        using var document = JsonDocument.Parse(progress);
        var root = document.RootElement;
        var state = root.TryGetProperty("state", out var stateValue) ? stateValue.GetUInt32() : 0;
        var done = root.TryGetProperty("done", out var doneValue) ? doneValue.GetUInt64() : 0;
        var total = root.TryGetProperty("total", out var totalValue) ? totalValue.GetUInt64() : 0;
        var skipped = root.TryGetProperty("skipped", out var skippedValue) ? skippedValue.GetUInt64() : 0;
        var message = root.TryGetProperty("message", out var messageValue) ? messageValue.GetString() ?? "" : "";
        switch (state)
        {
            case ExportRunning:
                Report(Loc.Format("replay.exportRunning", done, total, skipped));
                return;
            case ExportDone:
                _exporting = false;
                if (_exportQueue.Count > 0)
                {
                    StartNextExport();
                    return;
                }
                Report(Loc.Format("replay.exportDone", done, skipped));
                return;
            case ExportFailed:
                _exporting = false;
                _exportQueue.Clear();
                Report(Loc.Format("replay.exportError", message));
                return;
            case ExportCancelled:
                _exporting = false;
                _exportQueue.Clear();
                Report(Loc.T("replay.exportCancelled"));
                return;
        }
    }

    /// <summary>One export sentence for the window and the deck's own status line.</summary>
    private void Report(string text)
    {
        _exportWindow?.ShowProgress(text);
        ExportStatusText.Text = text;
    }

    private void StartNextExport()
    {
        if (_exportQueue.Count == 0)
        {
            _exporting = false;
            return;
        }
        var next = _exportQueue.Dequeue();
        if (Backend is not { } backend)
        {
            _exporting = false;
            _exportQueue.Clear();
            Report(Loc.T("replay.exportFailed"));
            return;
        }
        if (!backend.ReplayExportStart(next.Json, out var error))
        {
            _exporting = false;
            _exportQueue.Clear();
            Report(error.Length > 0 ? error : Loc.T("replay.exportFailed"));
            return;
        }
        _exporting = true;
        Report(Loc.Format("replay.exportStarted", next.Label));
    }
    /// <summary>The export settings window, while it is open.</summary>
    private ReplayExportWindow? _exportWindow;

    /// <summary>
    /// Opens the export settings. The deck keeps one button: the mode, the codec and
    /// the path live in the window, and so does the progress of a running export.
    /// </summary>
    private void Export_Click(object sender, RoutedEventArgs e)
    {
        if (_replay is null)
            return;
        if (_exportWindow is { } open)
        {
            open.Activate();
            return;
        }
        if (Backend?.IsRemote == true)
        {
            Warn(Loc.T("replay.exportLocalOnly"));
            return;
        }
        var modes = new List<ExportModeChoice>
        {
            new("Clip", Loc.T("replay.exportClip")),
            new("Playlist", Loc.T("replay.exportPlaylist")),
            new("Angle", Loc.T("replay.exportAngle")),
            new("AllAngles", Loc.T("replay.exportAllAngles")),
        };
        var window = new ReplayExportWindow(
            modes,
            BuildSegments,
            ExportFormat,
            () => [.. ExportAngles().Select(angle => (angle.Id, angle.Label))],
            () => _recording == true,
            (body, label) => StartExport(body, label)) { Owner = this };
        window.Closed += (_, _) => _exportWindow = null;
        _exportWindow = window;
        window.Show();
    }

    /// <summary>Hands one export body to the mixer and reports it in the window.</summary>
    private void StartExport(string body, string label)
    {
        _exportQueue.Enqueue(new ExportJobSpec(body, label));
        if (_exporting)
            return;
        StartNextExport();
    }
    /// <summary>Segments the chosen mode writes, in order.</summary>
    private List<(uint AngleId, ulong From, ulong To)> BuildSegments(string mode)
    {
        var result = new List<(uint, ulong, ulong)>();
        if (_replay is null)
            return result;
        switch (mode)
        {
            case "Clip":
                if (PlaylistList.SelectedItem is ClipRow row
                    && Clips().FirstOrDefault(item => item.Uid == row.Uid) is { } selected)
                {
                    result.Add((selected.AngleId, selected.InFrame, selected.OutFrame));
                }
                break;
            case "Playlist":
                foreach (var clip in Clips())
                    result.Add((clip.AngleId, clip.InFrame, clip.OutFrame));
                break;
            case "Angle":
            case "AllAngles":
                foreach (var angle in ExportAngles())
                {
                    var range = Backend?.ReplayAngleRange(angle.Id);
                    if (range is { } bounds)
                        result.Add((angle.Id, bounds.First, bounds.Last));
                    if (mode == "Angle")
                        break;
                }
                break;
        }
        return result;
    }

    /// <summary>Angles in the order an export should walk them: the one on air first.</summary>
    private List<ReplayAngleInfo> ExportAngles()
    {
        if (_replay is null)
            return [];
        var active = ReplayModel.ActiveAngle(_replay);
        var angles = ReplayModel.Angles(_replay).ToList();
        if (active == 0)
            return angles;
        return [.. angles.OrderBy(angle => angle.Id == active ? 0 : 1)];
    }

    private (uint Width, uint Height, uint FpsNum, uint FpsDen, uint Bitrate) ExportFormat()
    {
        if (_replay is null)
            return (1920, 1080, 60, 1, 20_000_000);
        var format = _replay["format"] as JsonObject;
        var width = format?["width"]?.GetValue<uint>() ?? 1920;
        var height = format?["height"]?.GetValue<uint>() ?? 1080;
        var (fpsNum, fpsDen) = ReplayModel.Fps(_replay, _session);
        var profile = format?["profile"]?.GetValue<string>() ?? "Sq";
        // Matches the recording profiles: HQ carries the most detail.
        var bitrate = profile switch
        {
            "Hq" => 40_000_000u,
            "Lq" => 8_000_000u,
            _ => 20_000_000u
        };
        return (width, height, fpsNum, fpsDen, bitrate);
    }

    private void RefreshExportModes()
    {
    }


    private sealed record ExportJobSpec(string Json, string Label);

    private unsafe void RefreshStatus()
    {
        if (_replay is null)
            return;
        var fps = ReplayModel.Fps(_replay, _session);
        MixerReplayStats stats = default;
        // One entry per angle the plan allows: Enterprise is unlimited, so this is the
        // mixer-side cap (32) rather than a number that happened to fit Professional.
        var angleBuffer = stackalloc MixerReplayAngleStats[32];
        MixerReplayAngleStats selected = default;
        var selectedAngle = ActiveAngleId();
        var hasSelected = false;
        // A Remote drives Replay on the machine that owns the recording, so there is
        // no recording here to measure. Reporting "idle" would be a lie.
        var remote = Backend?.IsRemote == true;
        if (remote)
        {
            StateText.Text = Loc.T("replay.stateRemote");
            HealthText.Text = Loc.T("replay.remoteMonitor");
            // The recording timecode lives with the recording, not here.
            RecordTcText.Text = "--:--:--:--";
        }
        else if (MixerNative.ReplayCopyStats(&stats, (nint)angleBuffer, 32) == 0 && stats.State != 0)
        {
            _recording = stats.State == 1;
            StateText.Text = stats.State switch
            {
                1 => RecordingText(stats),
                2 => Loc.T("replay.stateStopped"),
                _ => Loc.T("replay.stateIdle")
            };
            RecordTcText.Text = Stamp(stats.RecordingFrame, string.IsNullOrEmpty(MixerNative.ReplayRecordingUuid()) ? null : MixerNative.ReplayRecordingUuid());
            if (stats.FramesDroppedRecent > 0)
                StateText.Text += "  " + Loc.Format("replay.dropped", stats.FramesDroppedRecent);
            var count = Math.Min(stats.AngleCount, 8u);
            for (var index = 0; index < count; index++)
            {
                if (angleBuffer[index].AngleId == selectedAngle)
                {
                    selected = angleBuffer[index];
                    hasSelected = true;
                }
            }
            RefreshHealth(stats, selected, hasSelected);
        }
        else
        {
            _recording = false;
            StateText.Text = Loc.T("replay.stateIdle");
            HealthText.Text = "";
        }
        // The master frame rate, from the mixer's own frame counter. When a preview looks
        // slow this is the first thing to read: it says whether the mixer is behind or
        // only the window is.
        var runtime = default(MixerRuntimeStats);
        unsafe
        {
            if (MixerNative.CopyRuntimeStats(&runtime) == 0)
                UpdateFps(runtime.FramesRendered);
        }
        MarkOutButton.IsEnabled = _markOpen;
        // The deck shows which of the two it is doing. A Remote cannot read the
        // state, so both stay available there.
        if (!remote)
        {
            RecordButton.IsEnabled = true;
        }
        // A state an operator cannot see is a state they will doubt. The record button
        // burns while it records, and the mark buttons light while a mark is open.
        SetLit(RecordButton, _recording == true, Color.FromRgb(0xE0, 0x52, 0x52));
        SetLit(MarkInButton, _markOpen, Color.FromRgb(0x7F, 0xD1, 0x4B));
        SetLit(CancelMarkButton, _markOpen, Color.FromRgb(0xE0, 0x52, 0x52));
        SetLit(LiveButton, _live, Color.FromRgb(0x7F, 0xD1, 0x4B));

        // Waveform of the angle the operator is watching, with the marks on it. The
        // peaks come from the recording machine, so a Remote has none to draw.
        var angles = ReplayModel.Angles(_replay);
        if (angles.Count > 0 && !remote)
        {
            var active = ReplayModel.ActiveAngle(_replay);
            var angleId = active != 0 ? active : angles[0].Id;
            Waveform.Refresh(angleId, ReplayModel.Rate(fps));
        }
        // A mark whose frame is unknown cannot be placed on the waveform.
        var placed = HasLocalInFrame;
        Waveform.HasIn = placed;
        Waveform.InFrame = placed ? _inFrame!.Value : 0;
        Waveform.HasOut = placed && _outFrame != 0;
        Waveform.OutFrame = _outFrame;
        Waveform.PositionFrame = remote ? 0 : stats.RecordingFrame;
        SyncRecordingSlider();

        // A refused command must not look like it worked.
        if (_noticeTicks > 0)
        {
            _noticeTicks--;
            StateText.Text = _notice;
        }
        // Timecodes live in the session. The files live on the drive, and a drive
        // that no longer has them has to say so instead of playing black in silence.
        if (!remote && _replay is not null)
        {
            var missing = ReplayModel.MissingRecordingCount(_replay, MixerNative.ReplayRecordingUuid());
            if (missing > 0)
            {
                var warning = Loc.Format("replay.mediaMissing", missing);
                StateText.Text = string.IsNullOrEmpty(StateText.Text)
                    ? warning
                    : StateText.Text + "  " + warning;
            }
        }
    }

    /// <summary>Counts the remaining clip time down between status ticks.</summary>
    private void UpdateHeader()
    {
        // The playhead clock and the whole-recording slider follow the transport even
        // while it is parked, so a jog or a seek reads where it landed.
        UpdatePlayhead();
        SyncRecordingSlider();
        if (_replay is null || _headerOut == 0 || !_headerPlaying)
            return;
        var fps = ReplayModel.Fps(_replay, _session);
        var rate = ReplayModel.Rate(fps);
        if (rate <= 0)
            return;
        var position = _headerPosition;
        if (_headerAt != 0)
        {
            var elapsed = (System.Diagnostics.Stopwatch.GetTimestamp() - _headerAt)
                / (double)System.Diagnostics.Stopwatch.Frequency;
            var advanced = elapsed * rate * (_speedMilli / 1000.0);
            if (advanced > 0)
                position += (ulong)advanced;
        }
        if (_headerOut <= position || _headerOut <= _headerPosition)
            return;
        RecordTcLabel.Text = Loc.T("replay.clipRemaining");
        RecordTcText.Text = "-" + ReplayModel.Timecode(_headerOut - position, fps);
    }

    /// <summary>
    /// Writes the clock at the playhead. The transport's own state is the truth: it
    /// is what the decoder is showing, and it follows every cue, seek and jog.
    /// </summary>
    private void UpdatePlayhead()
    {
        if (_replay is null)
        {
            PlayheadTcText.Text = "--:--:--:--";
            return;
        }
        var state = Backend?.ReplayChannelState(_channel);
        if (state is not { OutFrame: > 0 } air)
        {
            PlayheadTcText.Text = "--:--:--:--";
            return;
        }
        PlayheadTcText.Text = Stamp(air.PositionFrame, TransportRecording());
    }
    /// <summary>Holds the picture; presses again to continue from the same frame.</summary>
    private void PlaylistPause_Click(object sender, RoutedEventArgs e)
    {
        var playing = !_playing;
        if (Backend?.ReplayTransport(_channel, playing, _speedMilli, -1) != true)
        {
            Warn(Refusal(Loc.T("replay.commandRefused")));
            return;
        }
        _playing = playing;
        UpdatePlayButton();
        UpdatePauseButton();
        RefreshOnAir();
    }

    private void UpdatePauseButton()
    {
        PlaylistPauseButton.Content = Loc.T(_playing ? "replay.pause" : "replay.resume");
        PlaylistPauseButton.ToolTip = Loc.T(_playing ? "replay.pauseHint" : "replay.resumeHint");
    }

    /// <summary>
    /// Commits an in or out point typed into a clip or event row. The cell shows a
    /// timecode and accepts one back, or a plain frame number.
    /// </summary>
    /// <summary>
    /// A range cell can be scrubbed with the mouse as well as typed into: dragging up
    /// adds frames, dragging down removes them, and the value is committed on release.
    /// </summary>
    private void RangeCell_Loaded(object sender, RoutedEventArgs e)
    {
        if (sender is not TextBox box)
            return;
        // Rows are recycled, so the handlers are attached once per cell.
        box.Loaded -= RangeCell_Loaded;
        // The cell shows and accepts the recording's wall-clock timecode, so what the
        // operator sees matches the clock the pictures were recorded on.
        var uuid = RowRecording(box);
        NumericDrag.AttachTimecode(
            box,
            2f,
            text => _replay is not null && ReplayModel.TryParseTimestamp(
                text, ReplayModel.RecordingStart(_replay, uuid), ReplayModel.Fps(_replay, _session), out var frames)
                ? frames
                : null,
            frames => box.Text = Stamp((ulong)Math.Max(0, Math.Round(frames)), uuid),
            () => CommitRange(box));
    }
    /// <summary>The recording the row a range cell belongs to was cut from.</summary>
    private static string? RowRecording(TextBox? box) => box?.Tag switch
    {
        EventRow row => row.RecordingUuid,
        ClipRow clip => clip.RecordingUuid,
        _ => null
    };

    private void RangeCell_LostFocus(object sender, RoutedEventArgs e) => CommitRange(sender as TextBox);

    private void RangeCell_KeyDown(object sender, KeyEventArgs e)
    {
        if (e.Key != Key.Enter)
            return;
        CommitRange(sender as TextBox);
        e.Handled = true;
    }

    private void CommitRange(TextBox? box)
    {
        if (box?.Tag is null || _replay is null)
            return;
        var fps = ReplayModel.Fps(_replay, _session);
        if (!ReplayModel.TryParseTimestamp(box.Text, ReplayModel.RecordingStart(_replay, RowRecording(box)), fps, out var frames))
        {
            Warn(Loc.T("replay.badTimecode"));
            RefreshEvents();
            RefreshPlaylist();
            return;
        }
        if (box.Tag is EventRow eventRow)
        {
            if (_replay["events"] is not JsonArray events)
                return;
            var entry = events.OfType<JsonObject>()
                .FirstOrDefault(item => (item["id"]?.GetValue<ulong>() ?? 0) == eventRow.Id);
            if (entry is null)
                return;
            var inFrame = box.Name == "EventInBox" ? frames : entry["inFrame"]?.GetValue<ulong>() ?? 0;
            var outFrame = box.Name == "EventOutBox" ? frames : entry["outFrame"]?.GetValue<ulong>() ?? 0;
            if (outFrame <= inFrame)
            {
                Warn(Loc.T("replay.badRange"));
                RefreshEvents();
                return;
            }
            entry["inFrame"] = inFrame;
            entry["outFrame"] = outFrame;
            SaveDocument();
            RefreshEvents();
            return;
        }
        if (box.Tag is ClipRow clipRow)
        {
            var playlist = ReplayModel.Playlist(_replay, _playlistId);
            if (playlist?["clips"] is not JsonArray clips)
                return;
            var clip = clips.OfType<JsonObject>()
                .FirstOrDefault(item => (item["uid"]?.GetValue<string>() ?? "") == clipRow.Uid);
            if (clip is null)
                return;
            var inFrame = box.Name == "ClipInBox" ? frames : clip["inFrame"]?.GetValue<ulong>() ?? 0;
            var outFrame = box.Name == "ClipOutBox" ? frames : clip["outFrame"]?.GetValue<ulong>() ?? 0;
            if (outFrame <= inFrame)
            {
                Warn(Loc.T("replay.badRange"));
                RefreshPlaylist();
                return;
            }
            clip["inFrame"] = inFrame;
            clip["outFrame"] = outFrame;
            playlist["revision"] = (playlist["revision"]?.GetValue<ulong>() ?? 0) + 1;
            SaveDocument();
            RefreshPlaylist();
        }
    }
    /// <summary>
    /// What the recorder is doing, in the operator's words. A disk that ran out of
    /// room says so; a store that has not measured free space yet reports zero,
    /// which must not read as "full".
    /// </summary>
    private static string RecordingText(MixerReplayStats stats)
    {
        if ((stats.Issues & ReplayIssues.StorageFull) != 0)
            return Loc.T("replay.stateRecordingNoSpace");
        if (stats.RemainingSecs == uint.MaxValue)
            return Loc.T("replay.stateRecordingUnknown");
        return Loc.Format(
            "replay.stateRecording",
            TimeSpan.FromSeconds(stats.RemainingSecs).ToString(@"h\:mm\:ss"));
    }

    private uint ActiveAngleId()
    {
        if (_replay is null)
            return 0;
        var active = ReplayModel.ActiveAngle(_replay);
        if (active != 0)
            return active;
        var angles = ReplayModel.Angles(_replay);
        return angles.Count > 0 ? angles[0].Id : 0;
    }

    /// <summary>
    /// Where the frames went: what the readback delivered, what the encoder
    /// accepted, and what the disk is doing. Without this an operator only sees
    /// that a recording is short, not which stage lost the picture.
    /// </summary>
    private void RefreshHealth(
        MixerReplayStats stats,
        MixerReplayAngleStats angle,
        bool hasAngle)
    {
        if (!hasAngle)
        {
            // Without a selected angle there is no delivery figure, but why the
            // recorder is unhappy is still worth reading.
            HealthText.Text = ReplayIssues.Text(stats.Issues);
            return;
        }
        HealthText.Text = Loc.Format(
            "replay.healthDelivery",
            angle.FramesWritten,
            stats.FramesOffered,
            stats.FramesPushed,
            angle.DroppedReadback)
            + "  "
            + Loc.Format(
                "replay.healthDisk",
                angle.DroppedEncode,
                angle.DroppedWrite,
                angle.Segments,
                angle.EncodeQueueLen,
                angle.EncodeQueueCap,
                angle.WriteLatencyP95Ms);
        // The numbers say how much was lost; the issues say why.
        var issues = ReplayIssues.Text(stats.Issues);
        if (issues.Length > 0)
            HealthText.Text += "  " + issues;
    }

    private void BuildChannels()
    {
        if (_replay is null)
            return;
        var channels = ReplayModel.Channels(_replay);
        _channel = channels.Count > 0 ? channels[0].Index : 0;
        ChannelBox.ItemsSource = channels.Select(channel => new ChannelChoice(channel.Index, channel.Label)).ToList();
        ChannelBox.DisplayMemberPath = nameof(ChannelChoice.Label);
        ChannelBox.SelectedIndex = 0;
    }

    private void ChannelBox_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (ChannelBox.SelectedItem is ChannelChoice choice)
            _channel = choice.Index;
    }

    /// <summary>
    /// One numbered button per camera, the way a replay controller labels its cameras:
    /// the picture of the selected one is the monitor above, so the strip stays a row of
    /// numbers instead of a row of expensive surfaces.
    /// </summary>
    private void BuildAngles()
    {
        if (_replay is null)
            return;
        AnglePanel.Children.Clear();
        foreach (var host in _angleHosts)
            host.ReleaseNative();
        _angleHosts.Clear();

        var angles = ReplayModel.Angles(_replay);
        if (angles.Count == 0)
        {
            AnglePanel.Children.Add(new TextBlock
            {
                Text = Loc.T("replay.noAngles"),
                Margin = new Thickness(8),
                TextWrapping = TextWrapping.Wrap
            });
            return;
        }
        var active = ReplayModel.ActiveAngle(_replay);
        var index = 0;
        foreach (var angle in angles)
        {
            index++;
            var selected = angle.Id == active;
            var button = new Button
            {
                Style = (Style)FindResource("AngleButton"),
                Tag = angle.Id,
                // The number is what an operator calls the camera; the name is the tooltip.
                Content = $"{index}",
                FontWeight = selected ? FontWeights.Bold : FontWeights.Normal,
                BorderThickness = new Thickness(selected ? 3 : 1),
                BorderBrush = selected ? Brushes.LimeGreen : (Brush)FindResource("ButtonBorder"),
                ToolTip = $"{index}: {angle.Label}",
            };
            button.Click += (sender, _) =>
            {
                if (sender is Button { Tag: uint id })
                    SelectAngle(id);
            };
            // Automation and tests address a camera by its angle, and a bare number is
            // ambiguous.
            AutomationProperties.SetAutomationId(button, $"AngleTile{angle.Id}");
            AutomationProperties.SetName(button, $"{index}: {angle.Label}");
            AnglePanel.Children.Add(button);
        }
    }

    /// <summary>
    /// Records which angle the operator is watching. Event auto-add uses it, and
    /// the control layer refuses to guess.
    /// </summary>
    private void SelectAngle(uint angleId)
    {
        if (_replay is null || ReplayModel.ActiveAngle(_replay) == angleId)
            return;
        _replay["activeAngleId"] = angleId;
        _angleEdited = true;
        SaveDocument();
        BuildAngles();
    }

    private void RefreshEvents()
    {
        if (_replay is null)
            return;
        var fps = ReplayModel.Fps(_replay, _session);
        // The rows are rebuilt only when the document changed under us, and an edit made
        // here writes straight back, so the list is not replaced while it is being typed
        // into.
        var existing = EventList.ItemsSource as List<EventRow> ?? [];
        var rows = new List<EventRow>();
        foreach (var entry in ReplayModel.Events(_replay))
        {
            var previous = existing.FirstOrDefault(row => row.Id == entry.Id);
            var row = previous ?? new EventRow { Id = entry.Id };
            row.IsLoading = true;
            row.InFrame = entry.InFrame;
            row.RecordingUuid = entry.RecordingUuid;
            row.InText = Stamp(entry.InFrame, entry.RecordingUuid);
            row.OutText = Stamp(entry.OutFrame, entry.RecordingUuid);
            row.DurationText = ReplayModel.Duration(entry.InFrame, entry.OutFrame, fps);
            row.Name = entry.Name;
            row.Protected = entry.Protected;
            row.IsLoading = false;
            if (previous is null)
                row.PropertyChanged += EventRow_Changed;
            rows.Add(row);
        }
        EventList.ItemsSource = rows;
        EventEmptyText.Visibility = rows.Count == 0 ? Visibility.Visible : Visibility.Collapsed;
    }

    /// <summary>Writes an event's name or protection back to the document.</summary>
    private void EventRow_Changed(object? sender, System.ComponentModel.PropertyChangedEventArgs e)
    {
        if (sender is not EventRow row || row.IsLoading || _replay is null)
            return;
        if (_replay["events"] is not JsonArray events)
            return;
        foreach (var item in events)
        {
            if (item is not JsonObject entry || entry["id"]?.GetValue<ulong>() != row.Id)
                continue;
            if (e.PropertyName == nameof(EventRow.Name))
                entry["name"] = row.Name.Trim();
            if (e.PropertyName == nameof(EventRow.Protected))
                entry["protected"] = row.Protected;
            SaveDocument();
            return;
        }
    }

    /// <summary>Deletes the event the row belongs to.</summary>
    private void EventDeleteRow_Click(object sender, RoutedEventArgs e)
    {
        if (sender is not Button { Tag: EventRow row } || _replay is null)
            return;
        if (row.Protected)
        {
            Warn(Loc.T("replay.eventProtected"));
            return;
        }
        if (_replay["events"] is not JsonArray events)
            return;
        for (var index = 0; index < events.Count; index++)
        {
            if (events[index] is JsonObject entry && entry["id"]?.GetValue<ulong>() == row.Id)
            {
                events.RemoveAt(index);
                _deletedEvents.Add(row.Id);
                SaveDocument();
                RefreshEvents();
                return;
            }
        }
    }

    /// <summary>Cues the clip this event stands for: the newest clip of that recording.</summary>
    private void EventCue_Click(object sender, RoutedEventArgs e)
    {
        if (sender is not Button { Tag: EventRow row } || _replay is null)
            return;
        // The event is a range of the recording, not a playlist entry, so cue it where
        // it starts on the channel that is showing it.
        var target = (long)Math.Min(row.InFrame, long.MaxValue);
        if (Backend?.ReplayTransport(_channel, false, _speedMilli, target) != true)
        {
            Warn(Refusal(Loc.T("replay.cueRefused")));
            return;
        }
        _live = false;
        RefreshStatus();
    }

    private void RefreshPlaylist()
    {
        if (_replay is null)
            return;
        var fps = ReplayModel.Fps(_replay, _session);
        var angles = ReplayModel.Angles(_replay);
        // The rows are rebuilt only when the document changed under us; an edit made here
        // writes straight back, so the list is not replaced while it is being typed into.
        var existing = PlaylistList.ItemsSource as List<ClipRow> ?? [];
        var rows = new List<ClipRow>();
        foreach (var clip in ReplayModel.Clips(_replay, _playlistId))
        {
            var row = existing.FirstOrDefault(candidate => candidate.Uid == clip.Uid)
                ?? new ClipRow { Uid = clip.Uid, InFrame = clip.InFrame, OutFrame = clip.OutFrame, AngleId = clip.AngleId };
            row.IsLoading = true;
            row.RecordingUuid = clip.RecordingUuid;
            row.InText = Stamp(clip.InFrame, clip.RecordingUuid);
            row.OutText = Stamp(clip.OutFrame, clip.RecordingUuid);
            var duration = ReplayModel.Duration(clip.InFrame, clip.OutFrame, fps);
            row.DurationText = ClipIsExpired(clip) ? duration + " " + Loc.T("replay.expired") : duration;
            row.AngleOptions = [.. angles.Select(angle => new Option<uint>(angle.Id, angle.Label))];
            row.AngleOption = row.AngleOptions.FirstOrDefault(option => option.Value == clip.AngleId);
            row.SpeedOptions =
            [
                new(250u, "25%"),
                new(500u, "50%"),
                new(750u, "75%"),
                new(1000u, "100%"),
                new(1500u, "150%"),
                new(2000u, "200%"),
            ];
            row.SpeedOption = row.SpeedOptions.FirstOrDefault(option => option.Value == (uint)Math.Round(clip.Speed * 1000))
                ?? row.SpeedOptions[3];
            // The mixing unit's names, limited to the ones the replay path can draw: it
            // composes two decoded frames, where the rest are GPU warps.
            row.TransitionOptions =
            [
                new("Cut", Loc.T("replay.transitionCut")),
                new("Dissolve", Loc.T("replay.transitionDissolve")),
                new("Dip", Loc.T("replay.transitionDip")),
                new("Wipe", Loc.T("replay.transitionWipe")),
                new("Slide", Loc.T("replay.transitionSlide")),
                new("Push", Loc.T("replay.transitionPush")),
            ];
            row.TransitionOption = row.TransitionOptions.FirstOrDefault(option => option.Value == clip.Transition.Kind)
                ?? row.TransitionOptions[0];
            row.DirectionOptions =
            [
                new("Left", Loc.T("replay.directionLeft")),
                new("Right", Loc.T("replay.directionRight")),
                new("Up", Loc.T("replay.directionUp")),
                new("Down", Loc.T("replay.directionDown")),
            ];
            row.DirectionOption = row.DirectionOptions.FirstOrDefault(option => option.Value == clip.Transition.Direction)
                ?? row.DirectionOptions[0];
            row.TimingOptions =
            [
                new("Overwrap", Loc.T("replay.timingOverwrap")),
                new("Additive", Loc.T("replay.timingAdditive")),
            ];
            row.TimingOption = row.TimingOptions.FirstOrDefault(option => option.Value == clip.Transition.Timing)
                ?? row.TimingOptions[0];
            row.TransitionFrames = clip.Transition.DurationFrames.ToString();
            row.GainDb = clip.GainDb.ToString("0.#");
            row.PreservePitch = clip.PreservePitch;
            row.AudioOptions = ClipAudioOptions();
            row.AudioOption = row.AudioOptions.FirstOrDefault(choice =>
                choice.Kind == clip.AudioKind
                && (choice.Kind != "Angle" || choice.AngleId == clip.AudioAngleId)) ?? row.AudioOptions[0];
            row.TagsText = clip.Tags.Count == 0 ? "" : "🏷 " + string.Join(", ", clip.Tags);
            row.IsLoading = false;
            if (existing.All(candidate => candidate.Uid != clip.Uid))
                row.PropertyChanged += ClipRow_Changed;
            rows.Add(row);
        }
        PlaylistList.ItemsSource = rows;
        PlaylistEmptyText.Visibility = rows.Count == 0 ? Visibility.Visible : Visibility.Collapsed;
        // The picker shows how many clips each playlist holds, so it is rebuilt with the
        // list: a count left over from before a delete is a lie about the project.
        RefreshPlaylistChoices();
    }

    /// <summary>
    /// Audio sources a clip can take: follow the picture's angle, silence, or another
    /// angle's recording.
    /// </summary>
    private List<ClipAudioChoice> ClipAudioOptions()
    {
        var choices = new List<ClipAudioChoice>
        {
            new("FollowAngle", 0, Loc.T("replay.audioFollow")),
            new("Silent", 0, Loc.T("replay.audioSilent")),
        };
        if (_replay is not null)
        {
            foreach (var angle in ReplayModel.Angles(_replay))
                choices.Add(new ClipAudioChoice("Angle", angle.Id, angle.Label));
        }
        return choices;
    }

    /// <summary>Writes a clip edit back to the document, one field at a time.</summary>
    private void ClipRow_Changed(object? sender, System.ComponentModel.PropertyChangedEventArgs e)
    {
        if (sender is not ClipRow row || row.IsLoading)
            return;
        var clip = FindClipJson(row.Uid);
        if (clip is null)
            return;
        switch (e.PropertyName)
        {
            case nameof(ClipRow.AngleOption) when row.AngleOption is { } angle:
                clip["angleId"] = angle.Value;
                break;
            case nameof(ClipRow.SpeedOption) when row.SpeedOption is { } speed:
                clip["speedMilli"] = speed.Value;
                break;
            case nameof(ClipRow.TransitionOption) when row.TransitionOption is { } transition:
                SetClipTransition(clip, transition.Value, null, null);
                break;
            case nameof(ClipRow.TimingOption) when row.TimingOption is { } timing:
                SetClipTransition(clip, null, timing.Value, null);
                break;
            case nameof(ClipRow.TransitionFrames):
                SetClipTransition(clip, null, null, ParseFrames(row.TransitionFrames));
                break;
            case nameof(ClipRow.GainDb):
                clip["gainDb"] = ParseGain(row.GainDb);
                break;
            case nameof(ClipRow.PreservePitch):
                clip["preservePitch"] = row.PreservePitch;
                break;
            case nameof(ClipRow.AudioOption) when row.AudioOption is { } audio:
                clip["audioKind"] = audio.Kind;
                clip["audioAngleId"] = audio.AngleId;
                break;
            default:
                return;
        }
        SaveDocument();
    }

    /// <summary>The clip object in the document, so a cell can write to it.</summary>
    private JsonObject? FindClipJson(string uid)
    {
        var playlist = _replay is null ? null : ReplayModel.Playlist(_replay, _playlistId);
        if (playlist?["clips"] is not JsonArray clips)
            return null;
        foreach (var item in clips)
        {
            if (item is JsonObject clip && clip["uid"]?.GetValue<string>() == uid)
                return clip;
        }
        return null;
    }

    /// <summary>
    /// Writes the transition, keeping the parts the caller did not change. A cut has no
    /// length, so choosing it clears the frames instead of leaving a stale duration.
    /// </summary>
    private static void SetClipTransition(JsonObject clip, string? kind, string? timing, uint? frames)
    {
        var current = ReplayTransitionInfo.Read(clip["transition"]);
        var nextKind = kind ?? current.Kind;
        var nextTiming = timing ?? current.Timing;
        var nextFrames = frames ?? current.DurationFrames;
        if (nextKind == "Cut")
            nextFrames = 0;
        clip["transition"] = new JsonObject
        {
            ["kind"] = nextKind,
            ["timing"] = nextTiming,
            ["durationFrames"] = nextFrames,
        };
    }

    private static uint ParseFrames(string text) =>
        uint.TryParse(text.Trim(), out var frames) ? Math.Min(frames, 600u) : 0u;

    private static double ParseGain(string text) =>
        double.TryParse(text.Trim(), out var gain) ? Math.Clamp(gain, -60, 12) : 0;

    private void ClipCueRow_Click(object sender, RoutedEventArgs e)
    {
        if (sender is not Button { Tag: ClipRow row })
            return;
        if (Backend?.ReplayCue(_channel, row.Uid) != true)
        {
            Warn(Refusal(Loc.T("replay.cueRefused")));
            return;
        }
        RefreshStatus();
    }

    private void ClipDeleteRow_Click(object sender, RoutedEventArgs e)
    {
        if (sender is not Button { Tag: ClipRow row })
            return;
        RemoveClip(row.Uid);
        RefreshPlaylist();
    }

    private void ClipTagRow_Click(object sender, RoutedEventArgs e)
    {
        if (sender is not Button { Tag: ClipRow row })
            return;
        AddTagToClip(row.Uid);
    }

    /// <summary>
    /// Opens the tag editor for one clip. The row's button is what names the clip, so the
    /// operator does not have to select it first.
    /// </summary>
    private void AddTagToClip(string uid)
    {
        if (_replay is null)
            return;
        var clip = ReplayModel.Clips(_replay, _playlistId).FirstOrDefault(candidate => candidate.Uid == uid);
        if (clip is null)
            return;
        var tags = new List<string>(clip.Tags);
        _clipTags = new TagCheckPanel(ClipTagPanel, _session.ReplayTags, tags, this, () =>
        {
            WriteClipTags(uid, _clipTags?.Selected ?? []);
        });
        _clipTags.PromptAdd();
    }

    private string AngleLabel(uint angleId)
    {
        if (_replay is null)
            return angleId.ToString();
        foreach (var angle in ReplayModel.Angles(_replay))
        {
            if (angle.Id == angleId)
                return angle.Label;
        }
        return angleId == 0 ? "-" : angleId.ToString();
    }

    /// <summary>
    /// Reloads the document and the deck's repaint interval after Replay Setup applies.
    /// The deck otherwise keeps the snapshot it opened with, and the next edit here
    /// publishes that snapshot back over the settings.
    /// </summary>
    internal void ApplySessionSettings()
    {
        if (!IsLoaded)
            return;
        var parsed = ReplayModel.Parse(_session);
        if (parsed is null)
            return;
        _replay = parsed;
        _angleEdited = false;
        _deletedEvents.Clear();
        _deletedClips.Clear();
        _lastJson = _session.ReplayJson;
        RefreshPlaylistChoices();
        BuildAngles();
        RefreshAngleGrid();
        RefreshEvents();
        RefreshPlaylist();
        RefreshDefaultTransition();
        RetargetMonitors();
        ApplyDeckPresentInterval();
    }

    private void ApplyDeckPresentInterval()
    {
        var interval = _session.Settings.ResolvedReplayPresentInterval();
        OutputMonitor.PresentInterval = interval;
        LiveMonitor.PresentInterval = interval;
        OutputMonitor.ApplyPresentInterval();
        LiveMonitor.ApplyPresentInterval();
    }

    private void SaveDocument()
    {
        if (_replay is null)
            return;
        // The channel has to name the playlist it plays. The mixer builds the transport's
        // running order from that name; a nameless channel has no next clip, so a cue
        // plays one clip and stops. Writing it down is what makes "play the playlist"
        // mean the whole list.
        ReplayModel.SetChannelPlaylist(_replay, _channel, _playlistId);
        // Settings stay with the session. This window writes events, playlists and the
        // channel's playlist onto that document, so an Apply from Replay Setup is not
        // replaced by the copy this window loaded earlier.
        var json = ReplayModel.Serialize(
            ReplayModel.WithDeckEdits(_session, _replay, _angleEdited, _deletedEvents, _deletedClips));
        var previous = _session.ReplayJson;
        _session.ReplayJson = json;
        _lastJson = json;
        try
        {
            if (App.IsRemote)
            {
                if (_main is { } main)
                    main.RemoteMutate(
                        "{\"kind\":\"updateReplayDeck\",\"deck\":" + ReplayModel.SerializeDeck(_replay) + "}",
                        Title);
                return;
            }
            SessionStore.NextPublishSource = "deck";
            SessionStore.Publish(_session);
            _angleEdited = false;
            _deletedEvents.Clear();
            _deletedClips.Clear();
        }
        catch (Exception error)
        {
            _session.ReplayJson = previous;
            _lastJson = previous;
            // The core refused the document, so this edit is not on air. Say which
            // edit and why, instead of leaving the window showing something the
            // mixer never took.
            HostLog.WriteException(error);
            Warn(string.IsNullOrWhiteSpace(error.Message)
                ? Loc.T("replay.publishRefused")
                : error.Message);
        }
    }

    private void MarkIn_Click(object sender, RoutedEventArgs e) => MarkIn();

    private void MarkOut_Click(object sender, RoutedEventArgs e) => MarkOut();

    /// <summary>
    /// Opens the recording settings. The deck is where an operator notices that
    /// storage or angles are missing, so the way in belongs here as well.
    /// </summary>
    private void ReplaySetup_Click(object sender, RoutedEventArgs e)
    {
        if (_main is { } main)
            main.OpenReplaySettings();
        else
            Warn(Loc.T("replay.openSetupUnavailable"));
    }

    /// <summary>
    /// A replay deck is driven from the keyboard, so marking, transport and the
    /// return to live all work anywhere in the window except where the operator is
    /// typing.
    /// </summary>
    protected override void OnPreviewKeyDown(KeyEventArgs e)
    {
        base.OnPreviewKeyDown(e);
        if (e.Handled || IsTyping())
            return;
        switch (e.Key)
        {
            case Key.I:
                MarkIn();
                e.Handled = true;
                break;
            case Key.O:
                MarkOut();
                e.Handled = true;
                break;
            case Key.Space:
                TogglePlay();
                e.Handled = true;
                break;
            case Key.Left:
                Step(-1);
                e.Handled = true;
                break;
            case Key.Right:
                Step(1);
                e.Handled = true;
                break;
            case Key.L:
                Live();
                e.Handled = true;
                break;
        }
    }

    /// <summary>
    /// True while the keyboard belongs to a field that wants the character itself,
    /// which also keeps a combo box's prefix search working.
    /// </summary>
    private static bool IsTyping() =>
        Keyboard.FocusedElement is TextBoxBase or ComboBox;

    private void CancelMark_Click(object sender, RoutedEventArgs e)
    {
        _inFrame = null;
        _outFrame = 0;
        _markOpen = false;
        InTcText.Text = "--:--:--:--";
        OutTcText.Text = "--:--:--:--";
        DurationTcText.Text = "--:--:--:--";
        MarkOutButton.IsEnabled = false;
    }

    private void MarkIn()
    {
        if (_replay is null)
            return;
        var remote = Backend?.IsRemote == true;
        var frame = Backend?.ReplayMarkIn();
        // Local mode returns the frame it marked. Nothing came back and the mark is
        // ours to keep, so there is no recorded timeline yet: say so instead of
        // opening a range that cannot be closed.
        if (frame is null && !remote)
        {
            Warn(Refusal(Loc.T("replay.markInFailed")));
            return;
        }
        _inFrame = frame ?? MarkUnknown;
        _markOpen = true;
        SetLit(MarkInButton, true, Color.FromRgb(0x7F, 0xD1, 0x4B));
        SetLit(CancelMarkButton, true, Color.FromRgb(0xE0, 0x52, 0x52));
        // A new Mark In starts a new range: the old Mark Out no longer applies.
        _outFrame = 0;
        _markOpen = true;
        var fps = ReplayModel.Fps(_replay, _session);
        InTcText.Text = frame is { } value ? Stamp(value, TransportRecording()) : Loc.T("replay.markInRemote");
        OutTcText.Text = "--:--:--:--";
        DurationTcText.Text = "--:--:--:--";
        MarkOutButton.IsEnabled = true;
    }

    private void MarkOut()
    {
        if (_replay is null || !_markOpen)
            return;
        var name = Loc.Format("replay.eventName", ReplayModel.Events(_replay).Count + 1);
        if (Backend?.ReplayMarkOut(name, out var created) != true)
        {
            // The mark was refused, so the range is left open for another try. The
            // mixer's own reason is what the operator acts on ("the mark is empty:
            // nothing was recorded after Mark In" is a different problem from a
            // refused write).
            Warn(Refusal(Loc.T("replay.markOutFailed")));
            return;
        }
        if (created is not { } mark)
        {
            // Remote mode: the core created the event from its own mark, and this
            // host never sees the frames. The range is taken, so it closes here, and
            // the event arrives with the session document.
            _markOpen = false;
            MarkOutButton.IsEnabled = false;
            _inFrame = null;
            _outFrame = 0;
            InTcText.Text = "--:--:--:--";
            OutTcText.Text = "--:--:--:--";
            DurationTcText.Text = "--:--:--:--";
            return;
        }
        CreateEvent(mark.Uuid, mark.InFrame, mark.OutFrame, name);
        // Keep the range on screen after it is taken: the operator has to see what
        // was just cut, and the waveform and the step buttons use the same range.
        var fps = ReplayModel.Fps(_replay, _session);
        _inFrame = mark.InFrame;
        _outFrame = mark.OutFrame;
        InTcText.Text = Stamp(mark.InFrame, mark.Uuid);
        OutTcText.Text = Stamp(mark.OutFrame, mark.Uuid);
        DurationTcText.Text = ReplayModel.Duration(mark.InFrame, mark.OutFrame, fps);
        _markOpen = false;
        MarkOutButton.IsEnabled = false;
        RefreshEvents();
        RefreshPlaylist();
    }

    /// <summary>
    /// Shows a message over the state line for a few polls, so a command the core
    /// refused is not mistaken for one that worked.
    /// </summary>
    /// <summary>
    /// The wall-clock timecode of a frame in a recording, falling back to the time
    /// from the recording's start when the session does not know the clock. Events and
    /// clips carry their recording's id; the transport uses the on-air clip's.
    /// </summary>
    private string Stamp(ulong frame, string? uuid)
    {
        if (_replay is null)
            return ReplayModel.Timecode(frame, (60, 1));
        var fps = ReplayModel.Fps(_replay, _session);
        var start = ReplayModel.RecordingStart(_replay, uuid);
        return ReplayModel.Timestamp(start, frame, fps);
    }

    /// <summary>The recording the transport is on: the clip on air, else the recorder.</summary>
    private string? TransportRecording()
    {
        if (Backend?.ReplayChannelState(_channel) is { } state && state.OutFrame > 0 && _replay is not null)
        {
            var clip = ReplayModel.Clips(_replay, _playlistId)
                .FirstOrDefault(item => item.InFrame <= state.PositionFrame && item.OutFrame >= state.PositionFrame);
            if (clip is not null)
                return clip.RecordingUuid;
        }
        var uuid = MixerNative.ReplayRecordingUuid();
        return string.IsNullOrEmpty(uuid) ? null : uuid;
    }
    private void Warn(string message)
    {
        _notice = message;
        _noticeTicks = 10;
        StateText.Text = message;
    }

    /// <summary>
    /// Local mode keeps the event bookkeeping in the host. The shape written here
    /// matches <c>ReplayEventDto</c> and <c>ClipDto</c> exactly so the core reads
    /// it back unchanged.
    /// </summary>
    private void CreateEvent(string uuid, ulong inFrame, ulong outFrame, string name)
    {
        if (_replay is null)
            return;
        var id = ReplayModel.ReserveEventId(_replay);
        var events = _replay["events"] as JsonArray ?? new JsonArray();
        events.Add(new JsonObject
        {
            ["id"] = id,
            ["recordingUuid"] = uuid,
            ["inFrame"] = inFrame,
            ["outFrame"] = outFrame,
            ["name"] = name,
            ["tags"] = new JsonArray(),
            ["protected"] = false
        });
        _replay["events"] = events;

        if (!ReplayModel.AutoAddToPlaylist(_replay))
        {
            SaveDocument();
            return;
        }
        var angleId = ReplayModel.ActiveAngle(_replay);
        if (angleId == 0)
        {
            // The document has no angles at all, so there is no camera to name. The
            // notice line says so; a modal box would block the deck.
            Warn(Loc.T("replay.needAngle"));
            SaveDocument();
            return;
        }
        ReplayModel.AppendClip(
            _replay,
            ReplayModel.EnsurePlaylist(_replay),
            id,
            uuid,
            inFrame,
            outFrame,
            angleId);
        SaveDocument();
    }

    /// <summary>
    /// Files an event that was marked with auto-add off onto the playlist that is
    /// open. The event list is the only place an event exists as a clip source, so
    /// without this an operator who turned auto-add off could never use an event.
    /// </summary>
    private void EventAddToPlaylist_Click(object sender, RoutedEventArgs e)
    {
        if (sender is not Button { Tag: EventRow row } || _replay is null)
            return;
        var eventInfo = ReplayModel.Events(_replay).FirstOrDefault(item => item.Id == row.Id);
        if (eventInfo is null)
            return;
        var angleId = ReplayModel.ActiveAngle(_replay);
        if (angleId == 0)
        {
            // Which camera the clip shows is the operator's call, exactly as it is
            // for auto-add: guessing would file the range against the wrong one.
            Warn(Loc.T("replay.needAngle"));
            return;
        }
        var uid = ReplayModel.AppendClip(
            _replay,
            _playlistId,
            eventInfo.Id,
            eventInfo.RecordingUuid,
            eventInfo.InFrame,
            eventInfo.OutFrame,
            angleId);
        if (uid is null)
        {
            Warn(Loc.T("replay.playlistMissing"));
            return;
        }
        SaveDocument();
        RefreshPlaylist();
        Warn(Loc.Format("replay.addedToPlaylist", string.IsNullOrWhiteSpace(eventInfo.Name) ? row.Name : eventInfo.Name));
    }

    private void EventDelete_Click(object sender, RoutedEventArgs e)
    {
        if (_replay is null || EventList.SelectedItem is not EventRow row)
        {
            Warn(Loc.T("replay.needEvent"));
            return;
        }
        var events = _replay["events"] as JsonArray ?? new JsonArray();
        for (var index = events.Count - 1; index >= 0; index--)
        {
            if (events[index] is JsonObject entry && (entry["id"]?.GetValue<ulong>() ?? 0) == row.Id)
                events.RemoveAt(index);
        }
        SaveDocument();
        RefreshEvents();
        RefreshPlaylist();
    }

    private void EventList_DoubleClick(object sender, MouseButtonEventArgs e) => AddSelectedEventToPlaylist();

    private void AddSelectedEventToPlaylist()
    {
        if (_replay is null || EventList.SelectedItem is not EventRow row)
            return;
        ReplayEventInfo? source = null;
        foreach (var entry in ReplayModel.Events(_replay))
        {
            if (entry.Id == row.Id)
                source = entry;
        }
        if (source is null)
            return;
        var playlistId = ReplayModel.EnsurePlaylist(_replay);
        var playlist = ReplayModel.Playlist(_replay, playlistId);
        if (playlist is null)
            return;
        var angleId = ReplayModel.ActiveAngle(_replay);
        if (angleId == 0)
        {
            MessageBox.Show(this, Loc.T("replay.needAngle"), Title);
            return;
        }
        var clips = playlist["clips"] as JsonArray ?? new JsonArray();
        clips.Add(new JsonObject
        {
            // Adding the same event twice is allowed, so the second clip needs a uid
            // of its own: the uid is what a channel plays by.
            ["uid"] = ReplayModel.UniqueClipUid(_replay, source.Id),
            ["source"] = "Angle",
            ["angleId"] = angleId,
            ["recordingUuid"] = source.RecordingUuid,
            ["inFrame"] = source.InFrame,
            ["outFrame"] = source.OutFrame,
            ["speed"] = 1.0,
            ["gainDb"] = 0.0,
            ["transition"] = new JsonObject { ["kind"] = "Cut", ["timing"] = "Overwrap", ["durationFrames"] = 0U }
        });
        playlist["clips"] = clips;
        playlist["revision"] = (playlist["revision"]?.GetValue<ulong>() ?? 0) + 1;
        SaveDocument();
        RefreshPlaylist();
    }

    private void ClipCue_Click(object sender, RoutedEventArgs e) => CueSelectedClip();

    /// <summary>
    /// Default transition for clips added later. The value is stored on the
    /// playlist, and a new clip starts on it so the operator does not edit every
    /// clip by hand.
    /// </summary>
    private void DefaultTransition_Changed(object sender, RoutedEventArgs e)
    {
        if (_loadingTransition || _replay is null)
            return;
        var playlist = ReplayModel.Playlist(_replay, _playlistId);
        if (playlist is null)
            return;
        var kind = (DefaultKindBox.SelectedItem as ComboBoxItem)?.Tag as string ?? "Cut";
        // Choosing a dissolve has to open the frame box in the same gesture: leaving it
        // disabled until something else refreshes the deck hides the only control the
        // operator needs next.
        DefaultFramesBox.IsEnabled = kind != "Cut";
        if (!uint.TryParse(DefaultFramesBox.Text, out var frames))
            frames = 0;
        // A cut carries no duration, so the frame box is ignored for one.
        if (kind == "Cut")
            frames = 0;
        playlist["defaultTransition"] = new JsonObject
        {
            ["kind"] = kind,
            ["timing"] = (DefaultTimingBox.SelectedItem as ComboBoxItem)?.Tag as string ?? "Overwrap",
            ["durationFrames"] = frames
        };
        SaveDocument();
    }

    /// <summary>Shows the playlist's default transition in the editors.</summary>
    private void RefreshDefaultTransition()
    {
        var playlist = _replay is null ? null : ReplayModel.Playlist(_replay, _playlistId);
        var fallback = playlist?["defaultTransition"] as JsonObject;
        var kind = fallback?["kind"]?.GetValue<string>() ?? "Cut";
        var timing = fallback?["timing"]?.GetValue<string>() ?? "Overwrap";
        var frames = fallback?["durationFrames"]?.GetValue<uint>() ?? 0;
        // Selecting an item raises SelectionChanged, and that handler saves the
        // document. Filling the editors from the document is not an edit, so the
        // handler is held off while it happens.
        _loadingTransition = true;
        try
        {
            SelectTag(DefaultKindBox, kind);
            SelectTag(DefaultTimingBox, timing);
            DefaultFramesBox.Text = frames.ToString();
        }
        finally
        {
            _loadingTransition = false;
        }
        DefaultFramesBox.IsEnabled = kind != "Cut";
    }

    private sealed record ClipAudioChoice(string Kind, uint AngleId, string Label);

    private IReadOnlyList<ReplayClipInfo> Clips() =>
        _replay is null ? [] : ReplayModel.Clips(_replay, _playlistId);

    /// <summary>
    /// Tag check boxes for the selected clip, with the session's replay catalog as
    /// the suggestion list.
    /// </summary>
    private void RefreshClipTags(string uid)
    {
        var tags = ClipTags(uid);
        _clipTags = new TagCheckPanel(ClipTagPanel, _session.ReplayTags, tags, this, () =>
        {
            if (PlaylistList.SelectedItem is ClipRow selected)
                WriteClipTags(selected.Uid, _clipTags?.Selected ?? []);
        });
    }

    private List<string> ClipTags(string uid)
    {
        if (_replay is null)
            return [];
        foreach (var clip in ReplayModel.Clips(_replay, _playlistId))
        {
            if (clip.Uid == uid)
                return [.. clip.Tags];
        }
        return [];
    }

    private void WriteClipTags(string uid, IReadOnlyList<string> tags)
    {
        if (_replay is null)
            return;
        var playlist = ReplayModel.Playlist(_replay, _playlistId);
        if (playlist?["clips"] is not JsonArray clips)
            return;
        foreach (var item in clips)
        {
            if (item is not JsonObject clip || (clip["uid"]?.GetValue<string>() ?? "") != uid)
                continue;
            var array = new JsonArray();
            foreach (var tag in tags)
                array.Add(tag);
            clip["tags"] = array;
        }
        SaveDocument();
        RefreshPlaylist();
    }

    private ReplayTransitionInfo? FindClip(string uid)
    {
        if (_replay is null)
            return null;
        foreach (var clip in ReplayModel.Clips(_replay, _playlistId))
        {
            if (clip.Uid == uid)
                return clip.Transition;
        }
        return null;
    }

    private static void SelectTag(ComboBox box, string tag)
    {
        foreach (ComboBoxItem item in box.Items)
        {
            if (Equals(item.Tag, tag))
            {
                box.SelectedItem = item;
                return;
            }
        }
        if (box.Items.Count > 0)
            box.SelectedIndex = 0;
    }

    private static string ComboTag(ComboBox box, string fallback) =>
        (box.SelectedItem as ComboBoxItem)?.Tag as string ?? fallback;

    private void CueSelectedClip()
    {
        if (PlaylistList.SelectedItem is not ClipRow row)
        {
            // A silent no-op reads as a broken button, so say what is missing.
            Warn(Loc.T("replay.needClip"));
            return;
        }
        if (Backend?.ReplayCue(_channel, row.Uid) != true)
            Warn(Refusal(Loc.T("replay.cueRefused")));
    }

    private void ClipRemove_Click(object sender, RoutedEventArgs e)
    {
        if (PlaylistList.SelectedItem is not ClipRow row)
        {
            Warn(Loc.T("replay.needClip"));
            return;
        }
        RemoveClip(row.Uid);
        RefreshPlaylist();
    }

    /// <summary>Removes one clip from the playlist the deck is showing.</summary>
    private void RemoveClip(string uid)
    {
        if (_replay is null)
            return;
        var playlist = ReplayModel.Playlist(_replay, _playlistId);
        if (playlist?["clips"] is not JsonArray clips)
            return;
        for (var index = clips.Count - 1; index >= 0; index--)
        {
            if (clips[index] is JsonObject clip && (clip["uid"]?.GetValue<string>() ?? "") == uid)
            {
                clips.RemoveAt(index);
                _deletedClips.Add(uid);
            }
        }
        playlist["revision"] = (playlist["revision"]?.GetValue<ulong>() ?? 0) + 1;
        // An animation step that cued this clip has nothing left to fire.
        ReplayCues.Clear(_session.Scenes, clipUid: uid);
        SaveDocument();
    }

    private void Record_Click(object sender, RoutedEventArgs e)
    {
        if (_replay is null)
            return;
        // The deck may hold a document from before the last Apply, and the control
        // layer refuses a recording whose settings are missing. Read the current one
        // so the reason names what is actually missing.
        var current = ReplayModel.Parse(_session);
        if (current is null)
        {
            Warn(Loc.T("replay.missing"));
            return;
        }
        _replay = current;
        // A recorder without storage or angles writes nothing, so the reason is
        // named here instead of leaving the mixer's bare refusal on screen.
        if (!ReplayModel.Enabled(_replay)
            || ReplayModel.Storages(_replay).Count == 0
            || ReplayModel.Angles(_replay).Count == 0)
        {
            Warn(Loc.T("replay.needSetup"));
            return;
        }
        if (!App.IsRemote)
            MixerNative.ThrowIfFailed(MixerNative.ReplayConfigure(ReplayModel.Serialize(_replay)), "Replay configure");
        if (Backend?.ReplayRecord(true) != true)
        {
            Warn(Refusal(Loc.T("replay.recordRefused")));
            return;
        }
        // Link the recording the session is about to write: the id the clips will
        // carry is written down with the drive it went to, so playback and the deck
        // look it up instead of searching the drives for it.
        var uuid = MixerNative.ReplayRecordingUuid();
        var storage = ReplayModel.Storages(_replay).FirstOrDefault() ?? "";
        var format = _replay["format"] as JsonObject;
        if (ReplayModel.AddRecording(
                _replay,
                uuid,
                storage,
                ReplayModel.Angles(_replay).Select(angle => angle.Id).ToList(),
                format?["width"]?.GetValue<uint>() ?? 0,
                format?["height"]?.GetValue<uint>() ?? 0,
                format?["fpsNum"]?.GetValue<uint>() ?? 0,
                format?["fpsDen"]?.GetValue<uint>() ?? 0,
                0,
                MixerNative.ReplayStartedUnixMs()))
        {
            SaveDocument();
        }
    }

    private void RecordToggle_Click(object sender, RoutedEventArgs e)
    {
        if (_recording == true)
        {
            StopRecording();
            return;
        }
        Record_Click(sender, e);
    }

    private void Stop_Click(object sender, RoutedEventArgs e) => StopRecording();

    /// <summary>
    /// Stops the recorder. Stopping one that is not running is not an error, so it stays
    /// quiet instead of reporting a refusal the operator cannot act on.
    /// </summary>
    private void StopRecording()
    {
        if (_recording != true)
            return;
        if (Backend?.ReplayRecord(false) != true)
            Warn(Refusal(Loc.T("replay.commandRefused")));
    }

    /// <summary>
    /// Plays the playlist from its head. The transport takes the next clip as each one
    /// ends, so one press plays the whole list with its transitions, and a clip added
    /// while it runs is picked up at the end.
    /// </summary>
    private void PlaylistPlay_Click(object sender, RoutedEventArgs e)
    {
        if (_replay is null)
            return;
        var clips = ReplayModel.Clips(_replay, _playlistId);
        if (clips.Count == 0)
        {
            Warn(Loc.T("replay.playlistEmpty"));
            return;
        }
        // The running order comes from the document, so the channel has to be pointed at
        // the list on screen before the cue: a channel left on another playlist (or on
        // none) would play one clip and stop, which reads as "it did not play".
        if (ReplayModel.ChannelPlaylist(_replay, _channel) != _playlistId)
        {
            SaveDocument();
        }
        if (Backend?.ReplayCue(_channel, clips[0].Uid) != true)
        {
            Warn(Refusal(Loc.T("replay.cueRefused")));
            return;
        }
        _live = false;
        if (_multiview)
            RefreshAngleGrid();
        if (Backend?.ReplayTransport(_channel, true, _speedMilli, -1) != true)
        {
            Warn(Refusal(Loc.T("replay.commandRefused")));
            return;
        }
        _playing = true;
        UpdatePlayButton();
        RefreshStatus();
    }

    /// <summary>
    /// Switches the input picture between the selected angle and a multiview of every
    /// angle, the way the main window shows its sources at once.
    /// </summary>
    private void Multiview_Click(object sender, RoutedEventArgs e)
    {
        _multiview = !_multiview;
        AngleGridScroller.Visibility = _multiview ? Visibility.Visible : Visibility.Hidden;
        // Hidden, not Collapsed: the picture still measures, so the row keeps its
        // height and toggling does not resize the whole deck.
        LiveBox.Visibility = _multiview ? Visibility.Hidden : Visibility.Visible;
        SetLit(MultiviewButton, _multiview, Color.FromRgb(0x7F, 0xD1, 0x4B));
        RefreshAngleGrid();
    }

    /// <summary>
    /// One live tile per angle. Each tile is its own monitor on the angle's scene, which
    /// is the picture the recorder is taking; a click makes that angle the active one.
    /// </summary>
    private void RefreshAngleGrid()
    {
        if (!_multiview || _replay is null)
            return;
        AngleGridPanel.Children.Clear();
        foreach (var host in _angleHosts)
            host.ReleaseNative();
        _angleHosts.Clear();
        if (Backend?.IsRemote == true)
        {
            AngleGridPanel.Children.Add(new TextBlock
            {
                Text = Loc.T("replay.remoteAngles"),
                Margin = new Thickness(8),
                TextWrapping = TextWrapping.Wrap
            });
            return;
        }
        var active = ReplayModel.ActiveAngle(_replay);
        foreach (var angle in ReplayModel.Angles(_replay))
        {
            var selected = angle.Id == active;
            var surface = new SwapchainHost { Width = 220, Height = 124, Margin = new Thickness(4) };
            FrameworkElement picture = surface;
            if (_live)
            {
                var monitorId = _session.NextMonitorId++;
                surface.Loaded += (_, _) => surface.RetargetMonitor(monitorId, angle.LiveSourceId);
            }
            else if (TryProxyTile(angle.Id) is { } proxy)
            {
                picture = proxy;
            }
            else
            {
                // Jog and search show the proxy. A recording without one stays a label;
                // the full-size picture is not decoded into the tile.
                picture = new TextBlock
                {
                    Text = Loc.T("replay.proxyMissing"),
                    Width = 220,
                    Height = 124,
                    Margin = new Thickness(4),
                    TextWrapping = TextWrapping.Wrap,
                    TextAlignment = TextAlignment.Center,
                    VerticalAlignment = VerticalAlignment.Center
                };
            }
            var label = new TextBlock
            {
                Text = angle.Label,
                Margin = new Thickness(6, 0, 6, 4),
                FontWeight = selected ? FontWeights.Bold : FontWeights.Normal
            };
            var tile = new Border
            {
                BorderThickness = new Thickness(selected ? 3 : 1),
                BorderBrush = selected ? Brushes.LimeGreen : (Brush)FindResource("ButtonBorder"),
                Margin = new Thickness(2),
                Cursor = Cursors.Hand,
                Tag = angle.Id,
                Child = new StackPanel { Children = { picture, label } }
            };
            tile.MouseLeftButtonUp += (clickSender, _) =>
            {
                if (clickSender is Border { Tag: uint id })
                    SelectAngle(id);
            };
            AutomationProperties.SetAutomationId(tile, $"AngleTile{angle.Id}");
            AutomationProperties.SetName(tile, angle.Label);
            AngleGridPanel.Children.Add(tile);
            if (_live)
                _angleHosts.Add(surface);
        }
    }

    /// <summary>
    /// Loops the playlist. The transport goes round again at the end instead of parking
    /// on the last frame; the button stays lit while it does.
    /// </summary>
    private void PlaylistLoop_Click(object sender, RoutedEventArgs e)
    {
        _looping = !_looping;
        if (Backend?.ReplayLoop(_channel, _looping) != true)
        {
            _looping = false;
            Warn(Refusal(Loc.T("replay.commandRefused")));
            return;
        }
        SetLit(PlaylistLoopButton, _looping, Color.FromRgb(0x7F, 0xD1, 0x4B));
    }

    /// <summary>Stops the playlist where it is; the picture stays, so it can resume.</summary>
    private void PlaylistStop_Click(object sender, RoutedEventArgs e)
    {
        if (Backend?.ReplayTransport(_channel, false, _speedMilli, -1) != true)
        {
            Warn(Refusal(Loc.T("replay.commandRefused")));
            return;
        }
        _playing = false;
        UpdatePlayButton();
        RefreshStatus();
    }

    private void Live_Click(object sender, RoutedEventArgs e) => Live();

    /// <summary>
    /// Jumps to the live edge of the recording and plays from there. The deck's LIVE
    /// button parks on the edge; this one goes there and starts, which is what an
    /// operator wants when they have been reviewing and the show has moved on.
    /// </summary>
    private void JumpLive_Click(object sender, RoutedEventArgs e)
    {
        if (Backend?.ReplayJumpToNow(_channel) != true)
        {
            Warn(Refusal(Loc.T("replay.commandRefused")));
            return;
        }
        // Play from the edge: the transport takes the recording on from now.
        if (Backend.ReplayTransport(_channel, true, _speedMilli, -1) != true)
        {
            Warn(Refusal(Loc.T("replay.commandRefused")));
            return;
        }
        _live = true;
        _playing = true;
        UpdatePlayButton();
        UpdatePauseButton();
        if (_multiview)
            RefreshAngleGrid();
    }

    /// <summary>
    /// Moves the channel along the whole recording. The slider runs over the angle's
    /// range, so the left end is the first recorded frame and the right end the newest
    /// one; a drag seeks, and the transport keeps its play state.
    /// </summary>
    private void RecordingSlider_ValueChanged(object sender, RoutedPropertyChangedEventArgs<double> e)
    {
        if (_sliderGuard || _replay is null)
            return;
        var range = Backend?.ReplayAngleRange(ActiveAngleId());
        if (range is not { } bounds || bounds.Last <= bounds.First)
            return;
        var span = (double)(bounds.Last - bounds.First);
        var frame = bounds.First + (ulong)Math.Round(span * e.NewValue / RecordingSlider.Maximum);
        var target = (long)Math.Min(frame, long.MaxValue);
        if (Backend?.ReplayTransport(_channel, _playing, _speedMilli, target) != true)
            return;
        _live = false;
        Waveform.PositionFrame = frame;
        UpdatePlayhead();
        if (_multiview)
            RefreshAngleGrid();
    }

    /// <summary>Puts the slider where the channel now is, without seeking.</summary>
    private void SyncRecordingSlider()
    {
        if (_replay is null)
            return;
        var range = Backend?.ReplayAngleRange(ActiveAngleId());
        if (range is not { } bounds || bounds.Last <= bounds.First)
        {
            _sliderGuard = true;
            RecordingSlider.Value = RecordingSlider.Maximum;
            _sliderGuard = false;
            return;
        }
        var position = Backend?.ReplayChannelState(_channel)?.PositionFrame ?? bounds.Last;
        var span = (double)(bounds.Last - bounds.First);
        var value = position <= bounds.First
            ? 0.0
            : Math.Min(RecordingSlider.Maximum, RecordingSlider.Maximum * (position - bounds.First) / span);
        _sliderGuard = true;
        RecordingSlider.Value = value;
        _sliderGuard = false;
    }


    /// <summary>Returns the channel to the recording edge.</summary>
    private void Live()
    {
        if (Backend?.ReplayJumpToNow(_channel) != true)
            Warn(Refusal(Loc.T("replay.commandRefused")));
        _live = true;
        _playing = false;
        UpdatePlayButton();
        if (_multiview)
            RefreshAngleGrid();
    }

    /// <summary>
    /// A clip whose frames are no longer in the open recording. A different
    /// recording, or a store that is not open, is a missing file rather than expiry.
    /// </summary>
    private bool ClipIsExpired(ReplayClipInfo clip)
    {
        var uuid = MixerNative.ReplayRecordingUuid();
        if (string.IsNullOrEmpty(uuid)
            || !string.Equals(clip.RecordingUuid, uuid, StringComparison.OrdinalIgnoreCase))
            return false;
        var range = Backend?.ReplayAngleRange(clip.AngleId);
        return range is { } bounds && clip.OutFrame < bounds.First;
    }

    /// <summary>
    /// The jog tile. The proxy is decoded into the tile; a missing proxy stays a
    /// label and the full-size picture is not decoded here.
    /// </summary>
    private FrameworkElement? TryProxyTile(uint angleId)
    {
        var frame = Backend?.ReplayChannelState(_channel)?.PositionFrame ?? 0UL;
        var buffer = new byte[480 * 270 * 2];
        uint width;
        uint height;
        unsafe
        {
            fixed (byte* ptr = buffer)
            {
                if (MixerNative.ReplayCopyProxy(angleId, frame, ptr, (nuint)buffer.Length, &width, &height) == 0)
                    return null;
            }
        }
        if (width == 0 || height == 0 || (ulong)width * height * 2 > (ulong)buffer.Length)
            return null;
        var bitmap = new System.Windows.Media.Imaging.WriteableBitmap(
            (int)width, (int)height, 96, 96, System.Windows.Media.PixelFormats.Bgra32, null);
        bitmap.Lock();
        unsafe
        {
            var dst = (byte*)bitmap.BackBuffer;
            var stride = bitmap.BackBufferStride;
            for (var y = 0; y < height; y++)
            {
                for (var x = 0; x < width; x += 2)
                {
                    var i = ((int)y * (int)width + (int)x) * 2;
                    var u = (int)buffer[i] - 128;
                    var y0 = (int)buffer[i + 1];
                    var v = (int)buffer[i + 2] - 128;
                    var y1 = (int)buffer[i + 3];
                    WriteBgra(dst, stride, (int)x, (int)y, y0, u, v);
                    if (x + 1 < width)
                        WriteBgra(dst, stride, (int)x + 1, (int)y, y1, u, v);
                }
            }
        }
        bitmap.AddDirtyRect(new System.Windows.Int32Rect(0, 0, (int)width, (int)height));
        bitmap.Unlock();
        return new System.Windows.Controls.Image
        {
            Source = bitmap,
            Width = 220,
            Height = 124,
            Margin = new Thickness(4),
            Stretch = System.Windows.Media.Stretch.Uniform
        };
    }

    private static unsafe void WriteBgra(byte* dst, int stride, int x, int y, int luma, int u, int v)
    {
        var c = luma + (int)(1.402 * v);
        var g = luma - (int)(0.344136 * u + 0.714136 * v);
        var b = luma + (int)(1.772 * u);
        var p = dst + y * stride + x * 4;
        p[0] = (byte)Math.Clamp(b, 0, 255);
        p[1] = (byte)Math.Clamp(g, 0, 255);
        p[2] = (byte)Math.Clamp(c, 0, 255);
        p[3] = 255;
    }

    private void Play_Click(object sender, RoutedEventArgs e) => TogglePlay();

    /// <summary>Starts or pauses the channel, from the button or the space bar.</summary>
    private void TogglePlay()
    {
        var playing = !_playing;
        if (Backend?.ReplayTransport(_channel, playing, _speedMilli, -1) == true)
        {
            _playing = playing;
        }
        else
        {
            // The channel never started, so the button must not say "Pause".
            Warn(Refusal(Loc.T("replay.playRefused")));
        }
        UpdatePlayButton();
    }

    /// <summary>
    /// The mixer's own reason for a refusal when it has one, so the operator reads
    /// what happened instead of a generic sentence.
    /// </summary>
    private static string Refusal(string fallback)
    {
        var detail = App.IsRemote ? "" : MixerNative.LastErrorText();
        return string.IsNullOrWhiteSpace(detail) ? fallback : $"{fallback}  {detail}";
    }

    private void StepBack_Click(object sender, RoutedEventArgs e) => Step(-1);

    private void StepForward_Click(object sender, RoutedEventArgs e) => Step(1);

    /// <summary>
    /// Seeks the channel to the frame under the pointer. A drag emits one call per mouse
    /// move, and the mixer only needs the latest, so it is throttled to about one per
    /// frame.
    /// </summary>
    private void OnWaveformSeek(ulong frame)
    {
        var now = Environment.TickCount64;
        if (now - _lastSeekAt < 16)
            return;
        _lastSeekAt = now;
        var target = (long)Math.Min(frame, long.MaxValue);
        if (Backend?.ReplayTransport(_channel, _playing, _speedMilli, target) != true)
            return;
        _live = false;
        Waveform.PositionFrame = frame;
        if (_multiview)
            RefreshAngleGrid();
    }

    /// <summary>
    /// Measures the machine and says how many angles it can record. The measurement encodes
    /// frames and writes a scratch file, so it runs off the UI thread; the deck keeps
    /// playing underneath.
    /// </summary>
    private async void Capacity_Click(object sender, RoutedEventArgs e)
    {
        if (_replay is null)
            return;
        CapacityButton.IsEnabled = false;
        CapacityText.Text = Loc.T("replay.capacityRunning");
        try
        {
            var storage = (_replay["storage"] as System.Text.Json.Nodes.JsonArray)?
                .FirstOrDefault()?["path"]?.GetValue<string>() ?? "";
            var width = _replay["format"]?["width"]?.GetValue<uint>() ?? 1920;
            var height = _replay["format"]?["height"]?.GetValue<uint>() ?? 1080;
            var backend = Backend;
            var measured = backend is null
                ? null
                : await Task.Run(() => backend.BenchmarkCapacity(storage, width, height));
            if (string.IsNullOrWhiteSpace(measured))
            {
                CapacityText.Text = Loc.T("replay.capacityFailed");
                return;
            }
            var root = System.Text.Json.Nodes.JsonNode.Parse(measured)?.AsObject();
            double Number(string name) => root?[name]?.GetValue<double>() ?? 0;
            var gpu = Number("encodeFps");
            var cpu = Number("encodeFpsCpu");
            var faster = gpu >= cpu ? "GPU" : "CPU";
            var ratio = cpu > 0 && gpu > 0
                ? Loc.Format("replay.capacityResultCpu", Math.Max(gpu, cpu) / Math.Min(gpu, cpu), faster)
                : "";
            var readback = Number("readbackMbS");
            var cpuAngles = (int)Number("anglesAt60Cpu");
            CapacityText.Text = string.Format(
                Loc.T("replay.capacityResult"),
                gpu,
                Number("diskWriteMbS"),
                Number("diskReadMbS"),
                (int)Number("anglesAt60"),
                Number("totalFps"),
                cpu)
                + string.Format(Loc.T("replay.capacityReadback"), readback, cpuAngles)
                + (ratio.Length > 0 ? "　" + ratio : "");
        }
        catch (Exception error)
        {
            HostLog.Write("ERROR", $"capacity bench: {error}");
            CapacityText.Text = Loc.T("replay.capacityFailed");
        }
        finally
        {
            CapacityButton.IsEnabled = true;
        }
    }
    private void Step(int delta)
    {
        // The mixer owns the position, so it does the step. Doing it here needed a local
        // mark and jumped to it, which is not what the buttons say they do.
        if (Backend?.ReplayStep(_channel, delta) == true)
        {
            _playing = false;
            UpdatePlayButton();
            return;
        }
        if (!HasLocalInFrame)
        {
            Warn(Loc.T("replay.stepNeedsLocalMark"));
            return;
        }
        SeekMarked(delta);
    }

    /// <summary>
    /// Moves the channel by `delta` frames from Mark In, stopping at Mark Out when
    /// one was taken. The mixer clamps a seek to the clip on its own, so this only
    /// keeps the operator inside the mark they made.
    /// </summary>
    private void SeekMarked(long delta)
    {
        var low = (long)_inFrame!.Value;
        var high = _outFrame > (ulong)low ? (long)_outFrame : long.MaxValue;
        var target = Math.Clamp(low + delta, 0, high);
        if (Backend?.ReplayTransport(_channel, false, _speedMilli, target) != true)
            Warn(Loc.T("replay.commandRefused"));
        _live = false;
        _playing = false;
        UpdatePlayButton();
        if (_multiview)
            RefreshAngleGrid();
    }

    private void Speed_Click(object sender, RoutedEventArgs e)
    {
        if (sender is not Button { Tag: string tag } || !int.TryParse(tag, out var milli))
            return;
        _speedMilli = milli;
        if (Backend?.ReplayTransport(_channel, _playing, _speedMilli, -1) != true)
            Warn(Loc.T("replay.commandRefused"));
        RefreshSpeedButtons();
    }

    /// <summary>
    /// Marks the speed the deck is playing at, the way a replay controller lights the
    /// button it is on. The state comes from the channel, so a speed set from another
    /// host shows up here too.
    /// </summary>
    private void RefreshSpeedButtons()
    {
        foreach (var (button, milli) in new[]
                 {
                     (Speed25Button, 250),
                     (Speed50Button, 500),
                     (Speed75Button, 750),
                     (Speed100Button, 1000),
                 })
        {
            var active = milli == _speedMilli;
            button.FontWeight = active ? FontWeights.Bold : FontWeights.Normal;
            button.BorderThickness = new Thickness(active ? 2 : 1);
            button.BorderBrush = active ? Brushes.LimeGreen : (Brush)FindResource("ButtonBorder");
        }
    }

    private void JogSlider_ValueChanged(object sender, RoutedPropertyChangedEventArgs<double> e)
    {
        if (!HasLocalInFrame)
        {
            Warn(Loc.T("replay.stepNeedsLocalMark"));
            return;
        }
        var frames = (long)Math.Round(e.NewValue);
        if (frames == 0)
            return;
        SeekMarked(frames);
    }

    /// <summary>
    /// Steps and jogging need the actual mark frame, which only the local mixer
    /// hands back. The remote core keeps it and ignores a seek it cannot place.
    /// </summary>
    private bool HasLocalInFrame => _inFrame is { } value && value != MarkUnknown;

    private ulong _fpsFrames;
    private long _fpsTicks;

    /// <summary>
    /// Master frame rate from the mixer's composed-frame counter. The rate is measured
    /// over the tick, so it reads what the mixer is doing rather than what was asked for.
    /// </summary>
    private void UpdateFps(ulong frames)
    {
        var now = System.Diagnostics.Stopwatch.GetTimestamp();
        if (_fpsTicks == 0 || _fpsFrames == 0)
        {
            _fpsFrames = frames;
            _fpsTicks = now;
            return;
        }
        var seconds = (now - _fpsTicks) / (double)System.Diagnostics.Stopwatch.Frequency;
        if (seconds < 1.0)
            return;
        var delta = frames >= _fpsFrames ? frames - _fpsFrames : 0;
        FpsText.Text = Loc.Format("replay.masterFps", delta / seconds);
        _fpsFrames = frames;
        _fpsTicks = now;
    }

    /// <summary>
    /// Lights a control that is in a state, the way a controller's buttons light: the
    /// record button while it records, the mark button while a mark is open. A deck that
    /// only changes a word elsewhere leaves the operator guessing.
    /// </summary>
    private static void SetLit(Control control, bool lit, Color on)
    {
        control.BorderThickness = new Thickness(lit ? 2 : 1);
        control.BorderBrush = lit ? new SolidColorBrush(on) : (Brush)control.FindResource("ButtonBorder");
        control.FontWeight = lit ? FontWeights.Bold : FontWeights.Normal;
    }

    private void UpdatePlayButton()
    {
        PlayButton.Content = Loc.T(_playing ? "replay.pause" : "replay.play");
        PlayButton.ToolTip = Loc.T(_playing ? "replay.pauseHint" : "replay.playHint");
        RefreshSpeedButtons();
    }

    private sealed record ChannelChoice(uint Index, string Label);

    /// <summary>
    /// One event, edited in its own row. The name and the protection flag are cells, so
    /// an operator renaming or protecting an event does it where the event is.
    /// </summary>
    private sealed class EventRow : System.ComponentModel.INotifyPropertyChanged
    {
        private string _name = "";
        private bool _protected;
        private bool _loading = true;

        public ulong Id { get; init; }

        /// <summary>The recording this event was cut from; its timecode uses that clock.</summary>
        public string RecordingUuid { get; set; } = "";

        /// <summary>The range the operator marked, for cueing the event itself.</summary>
        public ulong InFrame { get; set; }

        public string InText { get; set; } = "";

        public string OutText { get; set; } = "";

        public string DurationText { get; set; } = "";

        /// <summary>In, out and length, read-only: they come from the mark.</summary>

        public string Name
        {
            get => _name;
            set => Set(ref _name, value);
        }

        public bool Protected
        {
            get => _protected;
            set => Set(ref _protected, value);
        }

        /// <summary>True while the window fills the row, so no write goes back to the document.</summary>
        public bool IsLoading
        {
            get => _loading;
            set => _loading = value;
        }

        public event System.ComponentModel.PropertyChangedEventHandler? PropertyChanged;

        /// <summary>What a screen reader and the automation tree read out.</summary>
        public override string ToString() =>
            string.IsNullOrWhiteSpace(Name) ? $"{InText} → {OutText}" : $"{Name}  ({InText} → {OutText})";

        private void Set<T>(ref T field, T value, [System.Runtime.CompilerServices.CallerMemberName] string? name = null)
        {
            if (System.Collections.Generic.EqualityComparer<T>.Default.Equals(field, value))
                return;
            field = value;
            PropertyChanged?.Invoke(this, new System.ComponentModel.PropertyChangedEventArgs(name));
        }
    }

    /// <summary>
    /// One clip, edited in its own row. Every field an operator changes — angle, speed,
    /// transition, length, gain, pitch, audio — is a cell of the row it belongs to, so
    /// nothing is "select a clip, then use the controls somewhere else".
    /// </summary>
    private sealed class ClipRow : System.ComponentModel.INotifyPropertyChanged
    {
        /// <summary>The recording this clip was cut from; its timecode uses that clock.</summary>
        public string RecordingUuid { get; set; } = "";

        private Option<uint>? _angle;
        private Option<uint>? _speed;
        private Option<string>? _transition;
        private Option<string>? _direction;
        private Option<string>? _timing;
        private ClipAudioChoice? _audio;
        private string _transitionFrames = "0";
        private string _gainDb = "0";
        private bool _preservePitch;
        private bool _loading = true;
        private string _marker = "";

        public string Uid { get; init; } = "";

        /// <summary>The clip's own range, so the row can be matched with the channel.</summary>
        public ulong InFrame { get; init; }

        public ulong OutFrame { get; init; }

        public uint AngleId { get; init; }

        /// <summary>Marker shown while the channel is playing this clip.</summary>
        public string Marker
        {
            get => _marker;
            set => Set(ref _marker, value);
        }

        public string InText { get; set; } = "";

        public string OutText { get; set; } = "";

        public string DurationText { get; set; } = "";

        /// <summary>In, out and length, read-only: they come from the mark.</summary>

        public IReadOnlyList<Option<uint>> AngleOptions { get; set; } = [];

        public IReadOnlyList<Option<uint>> SpeedOptions { get; set; } = [];

        public IReadOnlyList<Option<string>> TransitionOptions { get; set; } = [];

        public IReadOnlyList<Option<string>> TimingOptions { get; set; } = [];

        /// <summary>Which way a Wipe, Slide or Push travels.</summary>
        public IReadOnlyList<Option<string>> DirectionOptions { get; set; } = [];

        public IReadOnlyList<ClipAudioChoice> AudioOptions { get; set; } = [];

        public string TagsText { get; set; } = "";

        public Option<uint>? AngleOption
        {
            get => _angle;
            set => Set(ref _angle, value);
        }

        public Option<uint>? SpeedOption
        {
            get => _speed;
            set => Set(ref _speed, value);
        }

        public Option<string>? TransitionOption
        {
            get => _transition;
            set
        {
            if (_transition == value)
                return;
            _transition = value;
            PropertyChanged?.Invoke(this, new System.ComponentModel.PropertyChangedEventArgs(nameof(TransitionOption)));
            PropertyChanged?.Invoke(this, new System.ComponentModel.PropertyChangedEventArgs(nameof(DirectionVisibility)));
            PropertyChanged?.Invoke(this, new System.ComponentModel.PropertyChangedEventArgs(nameof(LengthVisibility)));
        }
        }

        public Option<string>? DirectionOption
        {
            get => _direction;
            set => Set(ref _direction, value);
        }

        /// <summary>
        /// Direction and length only mean something for some transitions: a cut has no
        /// edge to move and no time to spend. The cells hide instead of showing a
        /// setting the mixer would ignore, the way the main window does it.
        /// </summary>
        public Visibility DirectionVisibility =>
            _transition is { Value: "Wipe" or "Slide" or "Push" } ? Visibility.Visible : Visibility.Collapsed;

        public Visibility LengthVisibility =>
            _transition is { Value: "Cut" } ? Visibility.Collapsed : Visibility.Visible;
        public Option<string>? TimingOption
        {
            get => _timing;
            set => Set(ref _timing, value);
        }

        public ClipAudioChoice? AudioOption
        {
            get => _audio;
            set => Set(ref _audio, value);
        }

        /// <summary>Frames the transition lasts, as text so it can be typed.</summary>
        public string TransitionFrames
        {
            get => _transitionFrames;
            set => Set(ref _transitionFrames, value);
        }

        public string GainDb
        {
            get => _gainDb;
            set => Set(ref _gainDb, value);
        }

        public bool PreservePitch
        {
            get => _preservePitch;
            set => Set(ref _preservePitch, value);
        }

        /// <summary>True while the window fills the row: no write goes back to the document.</summary>
        public bool IsLoading
        {
            get => _loading;
            set => _loading = value;
        }

        public event System.ComponentModel.PropertyChangedEventHandler? PropertyChanged;

        /// <summary>What a screen reader and the automation tree read out.</summary>
        public override string ToString() =>
            $"{InText} → {OutText}  {AngleOption?.Label ?? ""}  {SpeedOption?.Label ?? ""}";

        private void Set<T>(ref T field, T value, [System.Runtime.CompilerServices.CallerMemberName] string? name = null)
        {
            if (System.Collections.Generic.EqualityComparer<T>.Default.Equals(field, value))
                return;
            field = value;
            PropertyChanged?.Invoke(this, new System.ComponentModel.PropertyChangedEventArgs(name));
        }
    }
}