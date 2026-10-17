using System.IO;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Windows;
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
        _playlistId = ReplayModel.EnsurePlaylist(_replay);
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
        var live = angle?.SceneGpuId ?? 0;
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
                _playlistId = ReplayModel.EnsurePlaylist(_replay);
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
        // Angle previews are the recording machine's scenes. A Remote has no local
        // render of them, so it says where they are instead of showing dead tiles.
        if (Backend?.IsRemote == true)
        {
            AnglePanel.Children.Add(new TextBlock
            {
                Text = Loc.T("replay.remoteAngles"),
                Margin = new Thickness(8),
                TextWrapping = TextWrapping.Wrap
            });
            return;
        }
        // Many angles cost many present surfaces. Past the threshold the core
        // composes them into one shot instead, and a click picks the angle.
        if (angles.Count > ReplayMultiview.CompactThreshold
            && ReplayMultiview.Define(angles, 640, 360))
        {
            BuildCompactAngles(angles, active);
            return;
        }
        foreach (var angle in angles)
        {
            var selected = angle.Id == active;
            var label = new TextBlock
            {
                Text = angle.Label,
                Margin = new Thickness(6, 2, 6, 4),
                FontWeight = selected ? FontWeights.Bold : FontWeights.Normal
            };
            var surface = new SwapchainHost
            {
                Width = 256,
                Height = 144,
                Margin = new Thickness(6, 0, 6, 0)
            };
            var monitorId = _session.NextMonitorId++;
            surface.Loaded += (_, _) => surface.RetargetMonitor(monitorId, angle.SceneGpuId);
            var tile = new Border
            {
                BorderThickness = new Thickness(selected ? 3 : 1),
                BorderBrush = selected ? Brushes.LimeGreen : (Brush)FindResource("ButtonBorder"),
                Margin = new Thickness(4),
                Cursor = Cursors.Hand,
                Tag = angle.Id,
                Child = new StackPanel { Children = { surface, label } }
            };
            tile.MouseLeftButtonUp += (sender, _) =>
            {
                if (sender is Border { Tag: uint id })
                    SelectAngle(id);
            };
            AnglePanel.Children.Add(tile);
            _angleHosts.Add(surface);
        }
    }

    /// <summary>
    /// One composed surface for every angle. Clicking a slot makes that angle the
    /// active one, which is what events and auto-add use.
    /// </summary>
    private void BuildCompactAngles(IReadOnlyList<ReplayAngleInfo> angles, uint active)
    {
        var surface = new SwapchainHost
        {
            Width = 640,
            Height = 360,
            Margin = new Thickness(6, 0, 6, 0)
        };
        var monitorId = _session.NextMonitorId++;
        surface.Loaded += (_, _) => surface.RetargetMonitor(monitorId, ReplayMultiview.SceneId);
        surface.SurfaceClickedAt += (_, point) =>
        {
            if (ReplayMultiview.SlotAt(point.X, point.Y, angles.Count) is { } index)
                SelectAngle(angles[(int)index].Id);
        };
        var caption = new TextBlock
        {
            Text = Loc.Format("replay.compactAngles", angles.Count),
            Margin = new Thickness(6, 2, 6, 4)
        };
        var tile = new Border
        {
            BorderThickness = new Thickness(1),
            BorderBrush = (Brush)FindResource("ButtonBorder"),
            Margin = new Thickness(4),
            Child = new StackPanel { Children = { surface, caption } }
        };
        AnglePanel.Children.Add(tile);
        _angleHosts.Add(surface);
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
        var rows = new List<EventRow>();
        foreach (var entry in ReplayModel.Events(_replay))
        {
            rows.Add(new EventRow
            {
                Id = entry.Id,
                InText = ReplayModel.Timecode(entry.InFrame, fps),
                OutText = ReplayModel.Timecode(entry.OutFrame, fps),
                DurationText = ReplayModel.Duration(entry.InFrame, entry.OutFrame, fps),
                AngleText = AngleLabel(ReplayModel.ActiveAngle(_replay)),
                Name = entry.Protected ? "🔒 " + entry.Name : entry.Name,
                Protected = entry.Protected
            });
        }
        EventList.ItemsSource = rows;
    }

    private void RefreshPlaylist()
    {
        if (_replay is null)
            return;
        var fps = ReplayModel.Fps(_replay, _session);
        var rows = new List<ClipRow>();
        foreach (var clip in ReplayModel.Clips(_replay, _playlistId))
        {
            rows.Add(new ClipRow
            {
                Uid = clip.Uid,
                InText = ReplayModel.Timecode(clip.InFrame, fps),
                OutText = ReplayModel.Timecode(clip.OutFrame, fps),
                AngleText = AngleLabel(clip.AngleId),
                SpeedText = $"{clip.Speed * 100:0}%",
                TransitionText = clip.Transition.Label
            });
        }
        PlaylistList.ItemsSource = rows;
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
            // The mark was refused, so the range is left open for another try.
            Warn(Loc.T("replay.markOutFailed"));
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
    /// Loads the selected clip's transition into the editors. A cut carries no
    /// duration, so the frame box disables itself.
    /// </summary>
    private void PlaylistList_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (_replay is null || PlaylistList.SelectedItem is not ClipRow row)
            return;
        var transition = FindClip(row.Uid);
        if (transition is null)
            return;
        _loadingTransition = true;
        SelectTag(TransitionKindBox, transition.Kind);
        SelectTag(TransitionTimingBox, transition.Timing);
        TransitionFramesBox.Text = transition.DurationFrames.ToString();
        TransitionFramesBox.IsEnabled = transition.Kind != "Cut";
        PreservePitchBox.IsChecked = Clips().FirstOrDefault(clip => clip.Uid == row.Uid)?.PreservePitch ?? false;
        if (Clips().FirstOrDefault(clip => clip.Uid == row.Uid) is { } selected)
        {
            RefreshClipAudioBox(selected);
            ClipGainBox.Text = selected.GainDb.ToString("0.#");
        }
        _loadingTransition = false;
        RefreshClipTags(row.Uid);
    }

    /// <summary>
    /// Fills the clip audio choices: follow the picture's angle, silence, or
    /// another angle's recording.
    /// </summary>
    private void RefreshClipAudioBox(ReplayClipInfo clip)
    {
        _clipAudio.Clear();
        _clipAudio.Add(new ClipAudioChoice("FollowAngle", 0, Loc.T("replay.audioFollow")));
        _clipAudio.Add(new ClipAudioChoice("Silent", 0, Loc.T("replay.audioSilent")));
        foreach (var angle in ReplayModel.Angles(_replay!))
            _clipAudio.Add(new ClipAudioChoice("Angle", angle.Id, angle.Label));
        ClipAudioBox.ItemsSource = null;
        ClipAudioBox.ItemsSource = _clipAudio;
        ClipAudioBox.DisplayMemberPath = nameof(ClipAudioChoice.Label);
        var match = _clipAudio.FirstOrDefault(choice =>
            choice.Kind == clip.AudioKind
            && (choice.Kind != "Angle" || choice.AngleId == clip.AudioAngleId));
        ClipAudioBox.SelectedItem = match ?? _clipAudio[0];
    }

    /// <summary>
    /// Writes the clip's audio source. A clip may deliberately be silent, or take
    /// another angle's sound.
    /// </summary>
    private void ClipAudio_Changed(object sender, SelectionChangedEventArgs e)
    {
        if (_loadingTransition || _replay is null || PlaylistList.SelectedItem is not ClipRow row)
            return;
        if (ClipAudioBox.SelectedItem is not ClipAudioChoice choice)
            return;
        var playlist = ReplayModel.Playlist(_replay, _playlistId);
        if (playlist?["clips"] is not JsonArray clips)
            return;
        foreach (var item in clips)
        {
            if (item is not JsonObject clip || (clip["uid"]?.GetValue<string>() ?? "") != row.Uid)
                continue;
            clip["audio"] = choice.Kind;
            clip["audioAngleId"] = choice.AngleId;
        }
        SaveDocument();
        RefreshPlaylist();
    }

    /// <summary>
    /// Writes the clip's gain. The value is in dB so it reads like a fader, and it
    /// is refused rather than clamped when it is nonsense.
    /// </summary>
    private void ClipGain_Changed(object sender, RoutedEventArgs e)
    {
        if (_loadingTransition || _replay is null || PlaylistList.SelectedItem is not ClipRow row)
            return;
        if (!float.TryParse(ClipGainBox.Text, out var gain) || !float.IsFinite(gain)
            || gain < -60.0f || gain > 12.0f)
        {
            ExportStatusText.Text = Loc.T("replay.gainRange");
            // Put the stored value back so the box never shows a number that is not
            // in the document.
            var current = Clips().FirstOrDefault(item => item.Uid == row.Uid);
            ClipGainBox.Text = (current?.GainDb ?? 0.0).ToString("0.#");
            return;
        }
        var playlist = ReplayModel.Playlist(_replay, _playlistId);
        if (playlist?["clips"] is not JsonArray clips)
            return;
        foreach (var item in clips)
        {
            if (item is not JsonObject clip || (clip["uid"]?.GetValue<string>() ?? "") != row.Uid)
                continue;
            clip["gainDb"] = gain;
        }
        SaveDocument();
        RefreshPlaylist();
    }

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
    /// Writes the pitch flag of the selected clip. Slow motion otherwise drops the
    /// pitch along with the speed.
    /// </summary>
    private void PreservePitch_Changed(object sender, RoutedEventArgs e)
    {
        if (_loadingTransition || _replay is null || PlaylistList.SelectedItem is not ClipRow row)
            return;
        var playlist = ReplayModel.Playlist(_replay, _playlistId);
        if (playlist?["clips"] is not JsonArray clips)
            return;
        foreach (var item in clips)
        {
            if (item is not JsonObject clip || (clip["uid"]?.GetValue<string>() ?? "") != row.Uid)
                continue;
            clip["preservePitch"] = PreservePitchBox.IsChecked == true;
        }
        SaveDocument();
        RefreshPlaylist();
    }

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

    private void AddTag_Click(object sender, RoutedEventArgs e)
    {
        if (_clipTags is null)
        {
            Warn(Loc.T("replay.needClip"));
            return;
        }
        _clipTags.PromptAdd();
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

    /// <summary>
    /// Writes the editors back to the selected clip. Overwrap overlaps the two
    /// clips; Additive inserts the duration between them.
    /// </summary>
    private void Transition_Changed(object sender, RoutedEventArgs e)
    {
        if (_loadingTransition || _replay is null || PlaylistList.SelectedItem is not ClipRow row)
            return;
        var playlist = ReplayModel.Playlist(_replay, _playlistId);
        if (playlist?["clips"] is not JsonArray clips)
            return;
        var kind = ComboTag(TransitionKindBox, "Cut");
        if (!uint.TryParse(TransitionFramesBox.Text.Trim(), out var frames))
            frames = 0;
        frames = Math.Min(frames, 600);
        if (kind == "Cut")
            frames = 0;
        foreach (var item in clips)
        {
            if (item is not JsonObject clip || (clip["uid"]?.GetValue<string>() ?? "") != row.Uid)
                continue;
            clip["transition"] = new JsonObject
            {
                ["kind"] = kind,
                ["timing"] = ComboTag(TransitionTimingBox, "Overwrap"),
                ["durationFrames"] = frames
            };
        }
        playlist["revision"] = (playlist["revision"]?.GetValue<ulong>() ?? 0) + 1;
        TransitionFramesBox.IsEnabled = kind != "Cut";
        SaveDocument();
        RefreshPlaylist();
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

    private void PlaylistList_DoubleClick(object sender, MouseButtonEventArgs e) => CueSelectedClip();

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
        if (_replay is null || PlaylistList.SelectedItem is not ClipRow row)
        {
            Warn(Loc.T("replay.needClip"));
            return;
        }
        var playlist = ReplayModel.Playlist(_replay, _playlistId);
        if (playlist?["clips"] is not JsonArray clips)
            return;
        for (var index = clips.Count - 1; index >= 0; index--)
        {
            if (clips[index] is JsonObject clip && (clip["uid"]?.GetValue<string>() ?? "") == row.Uid)
                clips.RemoveAt(index);
        }
        playlist["revision"] = (playlist["revision"]?.GetValue<ulong>() ?? 0) + 1;
        // An animation step that cued this clip has nothing left to fire.
        ReplayCues.Clear(_session.Scenes, clipUid: row.Uid);
        SaveDocument();
        RefreshPlaylist();
    }

    private void Record_Click(object sender, RoutedEventArgs e)
    {
        if (_replay is null)
            return;
        // A recorder without storage or angles writes nothing, so the reason is
        // named here instead of leaving the mixer's bare refusal on screen.
        if (ReplayModel.Storages(_replay).Count == 0 || ReplayModel.Angles(_replay).Count == 0)
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

    private void UpdatePlayButton() =>
        PlayButton.Content = Loc.T(_playing ? "replay.pause" : "replay.play");

    private sealed record ChannelChoice(uint Index, string Label);

    private sealed class EventRow
    {
        public ulong Id { get; init; }
        public string InText { get; init; } = "";
        public string OutText { get; init; } = "";
        public string DurationText { get; init; } = "";
        public string AngleText { get; init; } = "";
        public string Name { get; init; } = "";
        public bool Protected { get; init; }
    }

    private sealed class ClipRow
    {
        public string Uid { get; init; } = "";
        public string InText { get; init; } = "";
        public string OutText { get; init; } = "";
        public string AngleText { get; init; } = "";
        public string SpeedText { get; init; } = "";
        public string TransitionText { get; init; } = "";
    }
}
