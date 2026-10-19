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

    private readonly Session _session;
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
    private readonly List<ExportModeChoice> _exportModes = [];
    private readonly Queue<ExportJobSpec> _exportQueue = new();
    private bool _exporting;

    private static IEivizBackend? Backend => (Application.Current as App)?.Backend;

    public ReplayWindow(Session session)
    {
        InitializeComponent();
        _session = session;
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
        BuildChannels();
        BuildAngles();
        RefreshEvents();
        RefreshPlaylist();
        Tick();
        _timer.Start();
    }

    private void OnClosed(object? sender, EventArgs e)
    {
        _timer.Stop();
        foreach (var host in _angleHosts)
            host.ReleaseNative();
        _angleHosts.Clear();
        OutputMonitor.ReleaseNative();
        LiveMonitor.ReleaseNative();
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
                RefreshPlaylistChoices();
                _lastJson = _session.ReplayJson;
                BuildAngles();
                RefreshEvents();
                RefreshPlaylist();
                RefreshDefaultTransition();
            }
        }
        RefreshStatus();
        RetargetMonitors();
        RefreshExport();
    }

    /// <summary>
    /// Export progress, and the queue that follows it. Several files (one per
    /// angle) are written one after another, because the store runs one export at
    /// a time.
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
                ExportCancelButton.IsEnabled = true;
                ExportButton.IsEnabled = false;
                ExportStatusText.Text = Loc.Format("replay.exportRunning", done, total, skipped);
                return;
            case ExportDone:
                _exporting = false;
                ExportCancelButton.IsEnabled = false;
                ExportButton.IsEnabled = true;
                if (_exportQueue.Count > 0)
                {
                    StartNextExport();
                    return;
                }
                ExportStatusText.Text = Loc.Format("replay.exportDone", done, skipped);
                return;
            case ExportFailed:
                _exporting = false;
                _exportQueue.Clear();
                ExportCancelButton.IsEnabled = false;
                ExportButton.IsEnabled = true;
                ExportStatusText.Text = Loc.Format("replay.exportError", message);
                return;
            case ExportCancelled:
                _exporting = false;
                _exportQueue.Clear();
                ExportCancelButton.IsEnabled = false;
                ExportButton.IsEnabled = true;
                ExportStatusText.Text = Loc.T("replay.exportCancelled");
                return;
        }
    }

    private void StartNextExport()
    {
        if (_exportQueue.Count == 0)
        {
            _exporting = false;
            ExportButton.IsEnabled = true;
            return;
        }
        var next = _exportQueue.Dequeue();
        if (Backend is not { } backend)
        {
            _exporting = false;
            _exportQueue.Clear();
            ExportButton.IsEnabled = true;
            ExportStatusText.Text = Loc.T("replay.exportFailed");
            return;
        }
        if (!backend.ReplayExportStart(next.Json, out var error))
        {
            _exporting = false;
            _exportQueue.Clear();
            ExportButton.IsEnabled = true;
            ExportStatusText.Text = error.Length > 0 ? error : Loc.T("replay.exportFailed");
            return;
        }
        _exporting = true;
        ExportButton.IsEnabled = false;
        ExportCancelButton.IsEnabled = true;
        ExportStatusText.Text = Loc.Format("replay.exportStarted", next.Label);
    }

    private void Export_Click(object sender, RoutedEventArgs e)
    {
        if (_replay is null || ExportModeBox.SelectedItem is not ExportModeChoice mode)
            return;
        var path = ExportPathBox.Text.Trim();
        if (path.Length == 0)
        {
            ExportStatusText.Text = Loc.T("replay.exportNeedPath");
            return;
        }
        var segments = string.Join(",", BuildSegments(mode.Key).Select(segment =>
            $"{{\"angleId\":{segment.AngleId},\"fromFrame\":{segment.From},\"toFrame\":{segment.To}}}"));
        if (segments.Length == 0)
        {
            ExportStatusText.Text = Loc.T("replay.exportNothing");
            return;
        }
        _exportQueue.Clear();
        var (width, height, fpsNum, fpsDen, bitrate) = ExportFormat();
        string Body(string target) =>
            $"{{\"path\":{JsonSerializer.Serialize(target)},\"width\":{width},\"height\":{height}," +
            $"\"fpsNum\":{fpsNum},\"fpsDen\":{fpsDen},\"bitrate\":{bitrate},\"sampleRate\":48000," +
            $"\"channels\":2,\"withAudio\":true,\"segments\":[{segments}]}}";
        if (mode.Key == "AllAngles")
        {
            // One file per angle, so a multi-camera export stays usable.
            foreach (var angle in ExportAngles())
            {
                var target = Path.Combine(
                    Path.GetDirectoryName(path) ?? "",
                    $"{Path.GetFileNameWithoutExtension(path)}-angle{angle.Id}{Path.GetExtension(path)}");
                        _exportQueue.Enqueue(new ExportJobSpec(Body(target), $"{angle.Label} → {Path.GetFileName(target)}"));
            }
        }
        else
        {
            _exportQueue.Enqueue(new ExportJobSpec(Body(path), Path.GetFileName(path)));
        }
        StartNextExport();
    }

    private void ExportBrowse_Click(object sender, RoutedEventArgs e)
    {
        var dialog = new Microsoft.Win32.SaveFileDialog
        {
            FileName = $"replay-{DateTime.Now:yyyyMMdd-HHmmss}.mp4",
            Filter = "MP4|*.mp4",
            OverwritePrompt = true
        };
        if (dialog.ShowDialog(this) == true)
            ExportPathBox.Text = dialog.FileName;
    }

    private void ExportCancel_Click(object sender, RoutedEventArgs e)
    {
        Backend?.ReplayExportCancel();
        _exportQueue.Clear();
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
        _exportModes.Clear();
        _exportModes.Add(new ExportModeChoice("Clip", Loc.T("replay.exportClip")));
        _exportModes.Add(new ExportModeChoice("Playlist", Loc.T("replay.exportPlaylist")));
        _exportModes.Add(new ExportModeChoice("Angle", Loc.T("replay.exportAngle")));
        _exportModes.Add(new ExportModeChoice("AllAngles", Loc.T("replay.exportAllAngles")));
        ExportModeBox.ItemsSource = null;
        ExportModeBox.ItemsSource = _exportModes;
        ExportModeBox.DisplayMemberPath = nameof(ExportModeChoice.Label);
        ExportModeBox.SelectedIndex = 0;
    }

    private sealed record ExportModeChoice(string Key, string Label);

    private sealed record ExportJobSpec(string Json, string Label);

    private unsafe void RefreshStatus()
    {
        if (_replay is null)
            return;
        var fps = ReplayModel.Fps(_replay, _session);
        MixerReplayStats stats = default;
        var angleBuffer = stackalloc MixerReplayAngleStats[8];
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
        else if (MixerNative.ReplayCopyStats(&stats, (nint)angleBuffer, 8) == 0 && stats.State != 0)
        {
            _recording = stats.State == 1;
            StateText.Text = stats.State switch
            {
                1 => RecordingText(stats),
                2 => Loc.T("replay.stateStopped"),
                _ => Loc.T("replay.stateIdle")
            };
            RecordTcText.Text = ReplayModel.Timecode(stats.RecordingFrame, fps);
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
        MarkOutButton.IsEnabled = _markOpen;
        // The deck shows which of the two it is doing. A Remote cannot read the
        // state, so both stay available there.
        if (!remote)
        {
            RecordButton.IsEnabled = _recording != true;
            StopButton.IsEnabled = _recording == true;
        }
        else
        {
            RecordButton.IsEnabled = true;
            StopButton.IsEnabled = true;
        }

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

        // A refused command must not look like it worked.
        if (_noticeTicks > 0)
        {
            _noticeTicks--;
            StateText.Text = _notice;
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
            row.InText = ReplayModel.Timecode(entry.InFrame, fps);
            row.OutText = ReplayModel.Timecode(entry.OutFrame, fps);
            row.DurationText = ReplayModel.Duration(entry.InFrame, entry.OutFrame, fps);
            row.Name = entry.Name;
            row.Protected = entry.Protected;
            row.IsLoading = false;
            if (previous is null)
                row.PropertyChanged += EventRow_Changed;
            rows.Add(row);
        }
        EventList.ItemsSource = rows;
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
        var clip = ReplayModel.Clips(_replay, _playlistId)
            .FirstOrDefault(candidate => ReplayModel.EventIdOf(candidate.Uid) == row.Id);
        if (clip is null)
        {
            Warn(Loc.T("replay.eventNotInPlaylist"));
            return;
        }
        if (Backend?.ReplayCue(_channel, clip.Uid) != true)
        {
            Warn(Refusal(Loc.T("replay.cueRefused")));
            return;
        }
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
            var row = existing.FirstOrDefault(candidate => candidate.Uid == clip.Uid) ?? new ClipRow { Uid = clip.Uid };
            row.IsLoading = true;
            row.InText = ReplayModel.Timecode(clip.InFrame, fps);
            row.OutText = ReplayModel.Timecode(clip.OutFrame, fps);
            row.DurationText = ReplayModel.Duration(clip.InFrame, clip.OutFrame, fps);
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
            row.TransitionOptions =
            [
                new("Cut", Loc.T("replay.transitionCut")),
                new("Dissolve", Loc.T("replay.transitionDissolve")),
            ];
            row.TransitionOption = row.TransitionOptions.FirstOrDefault(option => option.Value == clip.Transition.Kind)
                ?? row.TransitionOptions[0];
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

    private void SaveDocument()
    {
        if (_replay is null)
            return;
        var json = ReplayModel.Serialize(_replay);
        _session.ReplayJson = json;
        _lastJson = json;
        if (App.IsRemote)
        {
            if (Owner is MainWindow main)
                main.RemoteMutate("{\"kind\":\"upsertReplaySettings\",\"replay\":" + json + "}", Title);
            return;
        }
        try
        {
            SessionStore.Publish(_session);
        }
        catch (Exception error)
        {
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
        if (Owner is MainWindow main)
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
        // A new Mark In starts a new range: the old Mark Out no longer applies.
        _outFrame = 0;
        _markOpen = true;
        var fps = ReplayModel.Fps(_replay, _session);
        InTcText.Text = frame is { } value ? ReplayModel.Timecode(value, fps) : Loc.T("replay.markInRemote");
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
        InTcText.Text = ReplayModel.Timecode(mark.InFrame, fps);
        OutTcText.Text = ReplayModel.Timecode(mark.OutFrame, fps);
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
        var playlistId = ReplayModel.EnsurePlaylist(_replay);
        var playlist = ReplayModel.Playlist(_replay, playlistId);
        if (playlist is null)
        {
            SaveDocument();
            return;
        }
        var clips = playlist["clips"] as JsonArray ?? new JsonArray();
        // A new clip starts on the playlist's default transition, so an operator
        // who sets "always dissolve" gets it without editing every clip.
        var fallback = playlist["defaultTransition"] as JsonObject;
        clips.Add(new JsonObject
        {
            ["uid"] = ReplayModel.UniqueClipUid(_replay, id),
            ["source"] = "Angle",
            ["angleId"] = angleId,
            ["sourceEvent"] = id,
            ["recordingUuid"] = uuid,
            ["inFrame"] = inFrame,
            ["outFrame"] = outFrame,
            ["speed"] = 1.0,
            ["gainDb"] = 0.0,
            ["transition"] = fallback is null
                ? new JsonObject { ["kind"] = "Cut", ["timing"] = "Overwrap", ["durationFrames"] = 0U }
                : new JsonObject
                {
                    ["kind"] = fallback["kind"]?.GetValue<string>() ?? "Cut",
                    ["timing"] = fallback["timing"]?.GetValue<string>() ?? "Overwrap",
                    ["durationFrames"] = fallback["durationFrames"]?.GetValue<uint>() ?? 0
                }
        });
        playlist["clips"] = clips;
        playlist["revision"] = (playlist["revision"]?.GetValue<ulong>() ?? 0) + 1;
        SaveDocument();
    }

    private void EventProtect_Click(object sender, RoutedEventArgs e)
    {
        if (_replay is null || EventList.SelectedItem is not EventRow row)
        {
            Warn(Loc.T("replay.needEvent"));
            return;
        }
        foreach (var item in _replay["events"] as JsonArray ?? new JsonArray())
        {
            if (item is JsonObject entry && (entry["id"]?.GetValue<ulong>() ?? 0) == row.Id)
                entry["protected"] = !row.Protected;
        }
        SaveDocument();
        RefreshEvents();
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
        // A clip keeps its own recording, range and angle, so it survives the event
        // it came from; only the reference to an event that is gone goes away.
        foreach (var playlist in _replay["playlists"] as JsonArray ?? new JsonArray())
        {
            if (playlist is not JsonObject list || list["clips"] is not JsonArray clips)
                continue;
            var changed = false;
            foreach (var item in clips)
            {
                if (item is not JsonObject clip || (clip["sourceEvent"]?.GetValue<ulong>() ?? 0) != row.Id)
                    continue;
                clip.Remove("sourceEvent");
                changed = true;
            }
            if (changed)
                list["revision"] = (list["revision"]?.GetValue<ulong>() ?? 0) + 1;
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
            ["sourceEvent"] = source.Id,
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
        SelectTag(DefaultKindBox, kind);
        SelectTag(DefaultTimingBox, timing);
        DefaultFramesBox.Text = frames.ToString();
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
                clips.RemoveAt(index);
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
            Warn(Refusal(Loc.T("replay.recordRefused")));
    }

    private void Stop_Click(object sender, RoutedEventArgs e)
    {
        // Stopping a recorder that is not running is not an error, so it stays
        // quiet instead of reporting a refusal the operator cannot act on.
        if (_recording != true)
            return;
        if (Backend?.ReplayRecord(false) != true)
            Warn(Refusal(Loc.T("replay.commandRefused")));
    }

    private void Live_Click(object sender, RoutedEventArgs e) => Live();

    /// <summary>Returns the channel to the recording edge.</summary>
    private void Live()
    {
        if (Backend?.ReplayJumpToNow(_channel) != true)
            Warn(Refusal(Loc.T("replay.commandRefused")));
        _playing = false;
        UpdatePlayButton();
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

    private void Step(int delta)
    {
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
        _playing = false;
        UpdatePlayButton();
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

    private void UpdatePlayButton()
    {
        // The transport is icons: swap the glyph, and let the tooltip carry the word.
        PlayButton.Content = _playing ? "\uE769" : "\uE768";
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

        /// <summary>In, out and length, read-only: they come from the mark.</summary>
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
        private Option<uint>? _angle;
        private Option<uint>? _speed;
        private Option<string>? _transition;
        private Option<string>? _timing;
        private ClipAudioChoice? _audio;
        private string _transitionFrames = "0";
        private string _gainDb = "0";
        private bool _preservePitch;
        private bool _loading = true;

        public string Uid { get; init; } = "";

        /// <summary>In, out and length, read-only: they come from the mark.</summary>
        public string InText { get; set; } = "";

        public string OutText { get; set; } = "";

        public string DurationText { get; set; } = "";

        /// <summary>In, out and length, read-only: they come from the mark.</summary>

        public IReadOnlyList<Option<uint>> AngleOptions { get; set; } = [];

        public IReadOnlyList<Option<uint>> SpeedOptions { get; set; } = [];

        public IReadOnlyList<Option<string>> TransitionOptions { get; set; } = [];

        public IReadOnlyList<Option<string>> TimingOptions { get; set; } = [];

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
            set => Set(ref _transition, value);
        }

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

        private void Set<T>(ref T field, T value, [System.Runtime.CompilerServices.CallerMemberName] string? name = null)
        {
            if (System.Collections.Generic.EqualityComparer<T>.Default.Equals(field, value))
                return;
            field = value;
            PropertyChanged?.Invoke(this, new System.ComponentModel.PropertyChangedEventArgs(name));
        }
    }
}
