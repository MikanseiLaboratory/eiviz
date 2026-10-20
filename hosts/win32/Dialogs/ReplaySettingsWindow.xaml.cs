using System;
using System.Collections.Generic;
using System.Collections.ObjectModel;
using System.ComponentModel;
using System.Linq;
using System.Runtime.InteropServices;
using System.Text;
using System.Text.Json.Nodes;
using System.Windows;
using System.Windows.Controls;
using Eiviz.Host.I18n;
using Eiviz.Host.Interop;

namespace Eiviz.Host.Dialogs;

/// <summary>
/// Replay Setup.
///
/// Every list is edited in place: an angle's name, source, record mode, audio and
/// drive are cells of its own row, and a drive's path is a text box in its row. There
/// is no "select something, then use the controls somewhere else", because an operator
/// setting up four cameras should not have to hold a selection in their head while
/// they change a dropdown.
/// </summary>
public partial class ReplaySettingsWindow : Window
{
    private readonly Session _session;
    private readonly ObservableCollection<AngleRow> _angles = [];
    private readonly ObservableCollection<StorageRow> _storages = [];
    private JsonObject? _replay;
    private bool _hasLimits;
    private uint _angleLimit;
    private uint _channelLimit;
    /// <summary>Set once something is edited: closing without applying asks first.</summary>
    private bool _dirty;

    public ReplaySettingsWindow(Session session)
    {
        InitializeComponent();
        _session = session;
        RetentionBox.Items.Add(new ComboBoxItem { Content = Loc.T("replay.retentionStop"), Tag = "StopWhenFull" });
        RetentionBox.Items.Add(new ComboBoxItem { Content = Loc.T("replay.retentionOverwrite"), Tag = "Overwrite" });
        RetentionBox.Items.Add(new ComboBoxItem { Content = Loc.T("replay.retentionProtect"), Tag = "OverwriteUnprotected" });
        RetentionBox.SelectedIndex = 0;
        // The deck's own refresh interval: zero follows the project's setting.
        DeckPresentBox.Items.Add(new ComboBoxItem { Content = Loc.T("replay.deckRefreshFollow"), Tag = "0" });
        foreach (var frames in new[] { 1u, 2u, 3u, 4u, 6u, 8u })
            DeckPresentBox.Items.Add(new ComboBoxItem { Content = Loc.Format("replay.deckRefreshEvery", frames), Tag = frames.ToString() });
        DeckPresentBox.SelectedIndex = 0;
        // Any edit marks the window dirty; the numbers are checked on Apply, where the
        // operator can see the whole row that is wrong.
        QueueBox.TextChanged += MarkDirty;
        BufferBox.TextChanged += MarkDirty;
        ReserveBox.TextChanged += MarkDirty;
        SegmentFramesBox.TextChanged += MarkDirty;
        ProfileBox.SelectionChanged += MarkDirty;
        OverflowBox.SelectionChanged += MarkDirty;
        RetentionBox.SelectionChanged += MarkDirty;
        AutoAddBox.Click += MarkDirty;
        Load();
    }

    private void MarkDirty(object sender, RoutedEventArgs e) => _dirty = true;

    private void MarkDirty(object sender, TextChangedEventArgs e) => _dirty = true;

    private void MarkDirty(object sender, SelectionChangedEventArgs e) => _dirty = true;

    private void Load()
    {
        _replay = ReplayModel.Parse(_session);
        if (_replay is null)
        {
            PlanText.Text = Loc.T("replay.missing");
            ApplyButton.IsEnabled = false;
            return;
        }
        _storages.Clear();
        if (_replay["storage"] is JsonArray storage)
        {
            foreach (var item in storage)
            {
                if (item is JsonObject entry)
                    _storages.Add(new StorageRow { Path = entry["path"]?.GetValue<string>() ?? "" });
            }
        }
        SelectTag(ProfileBox, _replay["format"]?["profile"]?.GetValue<string>() ?? "Sq");
        var buffering = _replay["buffering"] as JsonObject;
        QueueBox.Text = (buffering?["encodeQueueFrames"]?.GetValue<uint>() ?? 6).ToString();
        BufferBox.Text = (buffering?["writeBufferMb"]?.GetValue<uint>() ?? 512).ToString();
        ReserveBox.Text = (buffering?["reserveMb"]?.GetValue<uint>() ?? 2048).ToString();
        SelectTag(OverflowBox, buffering?["overflowPolicy"]?.GetValue<string>() ?? "DropFrames");
        SelectTag(RetentionBox, _replay["retention"]?.GetValue<string>() ?? "StopWhenFull");
        SelectTag(DeckPresentBox, _session.Settings.ReplayPresentInterval.ToString());
        AutoAddBox.IsChecked = ReplayModel.AutoAddToPlaylist(_replay);
        SegmentFramesBox.Text = (_replay["format"]?["segmentFrames"]?.GetValue<uint>() ?? 120).ToString();

        // Which drive each angle records to is read from the drives the document lists.
        var driveOf = new Dictionary<uint, int>();
        if (_replay["storage"] is JsonArray drives)
        {
            for (var index = 0; index < drives.Count; index++)
            {
                if (drives[index] is not JsonObject entry || entry["angleIds"] is not JsonArray ids)
                    continue;
                foreach (var id in ids)
                {
                    if (id is not null)
                        driveOf[id.GetValue<uint>()] = index;
                }
            }
        }
        _angles.Clear();
        foreach (var angle in ReplayModel.Angles(_replay))
        {
            var row = new AngleRow
            {
                Id = angle.Id,
                Name = angle.Label,
                SourceKind = angle.Source,
                SourceId = angle.Source == "Input" ? angle.InputId : angle.SceneId,
                RecordMode = angle.RecordMode,
                Encoder = angle.Encoder,
                AudioUnitId = angle.AudioUnitId,
                DriveIndex = driveOf.TryGetValue(angle.Id, out var index) ? index : -1,
            };
            // The dropdowns are filled before the row reaches the list: a ComboBox reads
            // its selection when its container is created, and a list that is empty at
            // that moment leaves the cell looking unset.
            FillOptions(row);
            row.PropertyChanged += AngleRow_Changed;
            _angles.Add(row);
        }

        AngleList.ItemsSource = _angles;
        StorageList.ItemsSource = _storages;
        RefreshOptions();
        RefreshStorages();
        RefreshFormatText();
        RefreshPlan();
        _dirty = false;
    }

    /// <summary>Fills every dropdown one row shows.</summary>
    private void FillOptions(AngleRow row)
    {
        row.KindOptions =
        [
            new("Scene", Loc.T("replay.sourceScene")),
            new("Input", Loc.T("replay.sourceInput")),
        ];
        row.AudioOptions = AudioOptions();
        row.DriveOptions = DriveOptions();
        row.SourceOptions = SourceOptions(row.SourceKind);
        row.ModeOptions = ModeOptions(row.SourceKind, row.SourceId);
        row.EncoderOptions =
        [
            new("Gpu", Loc.T("replay.encoderGpu")),
            new("Cpu", Loc.T("replay.encoderCpu")),
        ];
        row.EncoderOption = row.EncoderOptions.FirstOrDefault(option => option.Value == row.Encoder) ?? row.EncoderOptions[0];
    }

    /// <summary>
    /// Fills every dropdown a row shows. The source and mode lists depend on the kind
    /// and the source, so they are rebuilt whenever those change.
    /// </summary>
    private void RefreshOptions()
    {
        foreach (var row in _angles)
        {
            FillOptions(row);
            row.NotifyOptions();
        }
    }

    private IReadOnlyList<Option<ulong>> AudioOptions() =>
    [
        new(0UL, Loc.T("replay.audioSilent")),
        .. _session.Units.Select(unit => new Option<ulong>(unit.Id, unit.Name)),
    ];

    /// <summary>
    /// Drives an angle can record to. "No drive" is a real entry: it shows the state
    /// instead of pretending the first drive was chosen.
    /// </summary>
    private IReadOnlyList<Option<int>> DriveOptions() =>
    [
        new(-1, Loc.T("replay.driveUnassigned")),
        .. _storages.Select((storage, index) =>
            new Option<int>(index, $"{index + 1}: {(string.IsNullOrWhiteSpace(storage.Path) ? Loc.T("replay.driveUnset") : storage.Path)}")),
    ];

    /// <summary>Scenes, or the inputs that have a picture: a Replay channel is not a camera.</summary>
    private IReadOnlyList<Option<ulong>> SourceOptions(string kind) =>
        kind == "Input"
            ? [.. _session.Inputs
                .Where(input => input.Kind != InputKind.Replay && input.Kind != InputKind.Audio)
                .Select(input => new Option<ulong>(input.Id, input.Name))]
            : [.. _session.Scenes.Select(scene => new Option<ulong>(scene.Id, scene.Name))];

    /// <summary>Passthrough only exists for an OMT input: a scene is composited.</summary>
    private IReadOnlyList<Option<string>> ModeOptions(string kind, ulong sourceId)
    {
        var omt = kind == "Input"
            && _session.Inputs.FirstOrDefault(input => input.Id == sourceId)?.Kind == InputKind.OMT;
        return omt
            ?
            [
                new("Composite", Loc.T("replay.recordComposite")),
                new("Passthrough", Loc.T("replay.recordPassthrough")),
            ]
            : [new("Composite", Loc.T("replay.recordComposite"))];
    }

    private void AngleRow_Changed(object? sender, PropertyChangedEventArgs e)
    {
        _dirty = true;
        if (sender is not AngleRow row)
            return;
        if (e.PropertyName is nameof(AngleRow.SourceKind) or nameof(AngleRow.SourceId))
        {
            row.SourceOptions = SourceOptions(row.SourceKind);
            row.ModeOptions = ModeOptions(row.SourceKind, row.SourceId);
            row.NotifyOptions();
        }
        if (e.PropertyName is nameof(AngleRow.DriveIndex) or nameof(AngleRow.Name))
            RefreshStorages();
    }

    private void StorageRow_Changed(object? sender, PropertyChangedEventArgs e)
    {
        _dirty = true;
        if (e.PropertyName == nameof(StorageRow.Path))
        {
            // The drive names travel in the angle cells, so a rename has to reach them.
            foreach (var row in _angles)
                row.DriveOptions = DriveOptions();
            foreach (var row in _angles)
                row.NotifyOptions();
        }
    }

    /// <summary>Recomputes what each drive holds and how much room it has.</summary>
    private void RefreshStorages()
    {
        for (var index = 0; index < _storages.Count; index++)
        {
            var names = _angles
                .Where(angle => angle.DriveIndex == index)
                .Select(angle => string.IsNullOrWhiteSpace(angle.Name) ? $"#{angle.Id}" : angle.Name);
            _storages[index].Angles = string.Join(", ", names);
        }
        RefreshFreeSpace();
    }

    /// <summary>
    /// Free space and how long the current settings would record there, per drive. The
    /// mixer answers for a path; a path it cannot reach says so rather than showing a
    /// zero the operator would read as "full".
    /// </summary>
    private void RefreshFreeSpace()
    {
        var unit = _session.Units.FirstOrDefault();
        var profile = ComboTag(ProfileBox, "Sq") switch
        {
            "Hq" => 0u,
            "Lq" => 2u,
            _ => 1u,
        };
        var reserveMb = uint.TryParse(ReserveBox.Text.Trim(), out var reserve) ? reserve : 0u;
        foreach (var row in _storages)
        {
            if (string.IsNullOrWhiteSpace(row.Path))
            {
                row.Free = string.Empty;
                continue;
            }
            if (unit is null)
            {
                row.Free = Loc.T("replay.freeUnknown");
                continue;
            }
            var path = Marshal.StringToCoTaskMemUTF8(row.Path);
            try
            {
                var input = new MixerReplayEstimateIn
                {
                    Width = unit.Width,
                    Height = unit.Height,
                    FpsNum = unit.FpsNum,
                    FpsDen = unit.FpsDen,
                    Profile = profile,
                    AngleCount = (uint)Math.Max(1, _angles.Count),
                    Path = path,
                    PathLen = (nuint)Encoding.UTF8.GetByteCount(row.Path),
                    ReserveBytes = (ulong)reserveMb * 1024 * 1024,
                };
                var output = default(MixerReplayEstimateOut);
                var code = MixerNative.ReplayEstimate(ref input, ref output);
                // A drive the mixer cannot write to says so rather than showing a number
                // that would send the operator chasing space that is already there.
                if (code == 0 && output.PathWritable != 0)
                {
                    row.Free = $"{FormatBytes(output.FreeBytes)}  ({FormatDuration(output.RemainingSecs)})";
                }
                else
                {
                    row.Free = Loc.T("replay.freeUnknown");
                }
            }
            catch (Exception)
            {
                row.Free = Loc.T("replay.freeUnknown");
            }
            finally
            {
                Marshal.FreeCoTaskMem(path);
            }
        }
    }

    private static string FormatBytes(ulong bytes)
    {
        string[] units = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
        double value = bytes;
        var unit = 0;
        while (value >= 1024 && unit < units.Length - 1)
        {
            value /= 1024;
            unit++;
        }
        return unit == 0 ? $"{bytes} {units[unit]}" : $"{value:0.#} {units[unit]}";
    }

    private static string FormatDuration(uint seconds)
    {
        var span = TimeSpan.FromSeconds(seconds);
        return span.TotalHours >= 1
            ? $"{(int)span.TotalHours}:{span.Minutes:00}:{span.Seconds:00}"
            : $"{span.Minutes}:{span.Seconds:00}";
    }

    private void RefreshFormatText()
    {
        var unit = _session.Units.FirstOrDefault();
        FormatText.Text = unit is null
            ? Loc.T("replay.needMixingUnit")
            : $"{unit.Width} x {unit.Height}   {unit.FpsNum}/{unit.FpsDen}";
    }

    private void RefreshPlan()
    {
        _hasLimits = PlanGate.TryReplayLimits(out _angleLimit, out _channelLimit);
        if (_hasLimits)
        {
            PlanText.Text = _angleLimit == 0 || _channelLimit == 0
                ? Loc.T("replay.planDenied")
                : Loc.Format("replay.planLimits", _angleLimit, _channelLimit);
        }
        else
        {
            PlanText.Text = string.Empty;
        }
        var active = _replay is null ? 0 : ReplayModel.ActiveAngle(_replay);
        ActiveAngleText.Text = active == 0
            ? Loc.T("replay.noActiveAngle")
            : Loc.Format("replay.activeAngle", AngleName(active));
    }

    private string AngleName(uint id)
    {
        foreach (var angle in _angles)
        {
            if (angle.Id == id)
                return string.IsNullOrWhiteSpace(angle.Name) ? $"#{id}" : angle.Name;
        }
        return id.ToString();
    }

    // ---- storage rows -------------------------------------------------------------

    private void AddStorageRow_Click(object sender, RoutedEventArgs e) => BrowseInto(null);

    private void BrowseStorageRow_Click(object sender, RoutedEventArgs e)
    {
        if (sender is Button { Tag: StorageRow row })
            BrowseInto(row);
    }

    /// <summary>
    /// Asks for a folder the way every other path in the app is chosen. Picking one is
    /// the whole action: it either fills the row the button belongs to, or adds a row.
    /// </summary>
    private void BrowseInto(StorageRow? row)
    {
        var dialog = new Microsoft.Win32.OpenFolderDialog
        {
            Title = Loc.T("replay.pickStorage"),
            Multiselect = false,
        };
        if (dialog.ShowDialog(this) != true)
            return;
        var path = dialog.FolderName.Trim();
        if (string.IsNullOrWhiteSpace(path))
            return;
        if (_storages.Any(existing => !ReferenceEquals(existing, row)
            && existing.Path.Trim().Equals(path, StringComparison.OrdinalIgnoreCase)))
        {
            Refuse(Loc.T("replay.duplicateStorage"));
            return;
        }
        if (row is null)
        {
            var added = new StorageRow { Path = path };
            added.PropertyChanged += StorageRow_Changed;
            _storages.Add(added);
            // The first drive takes the angles that have none: a setup with one drive
            // should not need a second step per angle.
            if (_storages.Count == 1)
            {
                foreach (var angle in _angles)
                {
                    if (angle.DriveIndex < 0)
                        angle.DriveIndex = 0;
                }
            }
            StatusText.Text = Loc.Format("replay.storageAdded", path);
        }
        else
        {
            row.Path = path;
            StatusText.Text = Loc.Format("replay.storageAdded", path);
        }
        RefreshStorages();
        _dirty = true;
    }

    private void RemoveStorageRow_Click(object sender, RoutedEventArgs e)
    {
        if (sender is not Button { Tag: StorageRow row })
            return;
        if (_storages.Count <= 1)
        {
            Refuse(Loc.T("replay.needStorable"));
            return;
        }
        var index = _storages.IndexOf(row);
        row.PropertyChanged -= StorageRow_Changed;
        _storages.Remove(row);
        // Angles on the removed drive move to the first one, and the ones after it
        // shift down with the list.
        foreach (var angle in _angles)
        {
            if (angle.DriveIndex == index)
                angle.DriveIndex = 0;
            else if (angle.DriveIndex > index)
                angle.DriveIndex -= 1;
        }
        foreach (var angle in _angles)
            angle.DriveOptions = DriveOptions();
        foreach (var angle in _angles)
            angle.NotifyOptions();
        RefreshStorages();
        _dirty = true;
    }

    // ---- angle rows ---------------------------------------------------------------

    private void AddAngleRow_Click(object sender, RoutedEventArgs e)
    {
        if (_storages.Count == 0)
        {
            Refuse(Loc.T("replay.needDriveForAngle"));
            return;
        }
        if (_hasLimits && _angleLimit != uint.MaxValue && _angles.Count >= _angleLimit)
        {
            Refuse(Loc.Format("replay.planLimits", _angleLimit, _channelLimit));
            return;
        }
        // The next unused scene: the common case needs no edits at all, and the row is
        // there to be changed in place if it is not the one they wanted.
        var used = _angles
            .Where(angle => angle.SourceKind == "Scene")
            .Select(angle => angle.SourceId)
            .ToHashSet();
        var scene = _session.Scenes.FirstOrDefault(candidate => !used.Contains(candidate.Id));
        var row = new AngleRow
        {
            Id = _angles.Count == 0 ? 1u : _angles.Max(angle => angle.Id) + 1,
            Name = scene?.Name ?? string.Empty,
            SourceKind = "Scene",
            SourceId = scene?.Id ?? 0,
            RecordMode = "Composite",
            AudioUnitId = 0,
            DriveIndex = _storages.Count == 1 ? 0 : -1,
        };
        row.PropertyChanged += AngleRow_Changed;
        // Filled before it joins the list, for the same reason as on load.
        FillOptions(row);
        _angles.Add(row);
        RefreshStorages();
        RefreshPlan();
        AngleList.SelectedItem = row;
        AngleList.ScrollIntoView(row);
        StatusText.Text = Loc.Format("replay.angleAdded", row.Name);
        _dirty = true;
    }

    private void RemoveAngleRow_Click(object sender, RoutedEventArgs e)
    {
        if (sender is not Button { Tag: AngleRow row })
            return;
        // A clip that plays this angle, or takes its sound from it, would be left
        // pointing at nothing, so the angle waits until those clips are gone.
        var users = ClipsUsingAngle(row.Id);
        if (users > 0)
        {
            Refuse(Loc.Format("replay.angleInUse", users));
            return;
        }
        row.PropertyChanged -= AngleRow_Changed;
        _angles.Remove(row);
        RefreshStorages();
        RefreshPlan();
        _dirty = true;
    }

    /// <summary>How many clips play this angle, or take their sound from it.</summary>
    private int ClipsUsingAngle(uint angleId)
    {
        if (_replay?["playlists"] is not JsonArray playlists)
            return 0;
        var count = 0;
        foreach (var list in playlists)
        {
            if (list is not JsonObject playlist || playlist["clips"] is not JsonArray clips)
                continue;
            foreach (var item in clips)
            {
                if (item is not JsonObject clip)
                    continue;
                var picture = clip["angleId"]?.GetValue<uint>() == angleId;
                var audio = string.Equals(clip["audioKind"]?.GetValue<string>(), "Angle", StringComparison.OrdinalIgnoreCase)
                    && clip["audioAngleId"]?.GetValue<uint>() == angleId;
                if (picture || audio)
                    count++;
            }
        }
        return count;
    }

    // ---- apply --------------------------------------------------------------------

    /// <summary>
    /// Measures the machine and reports what it means for the angle count in this dialog.
    /// The measurement encodes frames and writes a scratch file, so it runs off the UI
    /// thread and the window stays usable while it works.
    /// </summary>
    private async void Capacity_Click(object sender, RoutedEventArgs e)
    {
        CapacityButton.IsEnabled = false;
        CapacityText.Text = Loc.T("replay.capacityRunning");
        try
        {
            var storage = (_replay?["storage"] as JsonArray)?.FirstOrDefault()?["path"]?.GetValue<string>() ?? "";
            var width = _replay?["format"]?["width"]?.GetValue<uint>() ?? 1920;
            var height = _replay?["format"]?["height"]?.GetValue<uint>() ?? 1080;
            var backend = (Application.Current as App)?.Backend;
            var measured = backend is null
                ? null
                : await Task.Run(() => backend.BenchmarkCapacity(storage, width, height));
            if (string.IsNullOrWhiteSpace(measured))
            {
                CapacityText.Text = Loc.T("replay.capacityFailed");
                return;
            }
            var root = JsonNode.Parse(measured)?.AsObject();
            double Number(string name) => root?[name]?.GetValue<double>() ?? 0;
            CapacityText.Text = string.Format(
                Loc.T("replay.capacityResult"),
                Number("encodeFps"),
                Number("diskWriteMbS"),
                Number("diskReadMbS"),
                (int)Number("anglesAt60"),
                Number("totalFps"));
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
    private void Apply_Click(object sender, RoutedEventArgs e)
    {
        if (_replay is null)
            return;
        if (!uint.TryParse(QueueBox.Text.Trim(), out var queue) || queue is < 2 or > 60
            || !uint.TryParse(BufferBox.Text.Trim(), out var buffer) || buffer is < 64 or > 8192
            || !uint.TryParse(ReserveBox.Text.Trim(), out var reserve))
        {
            Refuse(Loc.T("replay.badBuffering"));
            return;
        }
        // The segment length is the operator's setting: the recorder no longer
        // substitutes one when it is missing or out of range.
        if (!uint.TryParse(SegmentFramesBox.Text.Trim(), out var segmentFrames)
            || segmentFrames is < 30 or > 1800)
        {
            Refuse(Loc.T("replay.segmentFramesRange"));
            return;
        }
        var unit = _session.Units.FirstOrDefault();
        if (unit is null)
        {
            Refuse(Loc.T("replay.needMixingUnit"));
            return;
        }
        if (_storages.Count == 0 || _storages.All(storage => string.IsNullOrWhiteSpace(storage.Path)))
        {
            Refuse(Loc.T("replay.needStorable"));
            return;
        }
        // Every angle records somewhere, and every angle records something: the row it
        // is missing from is the one that turns red.
        var unassigned = _angles.FirstOrDefault(angle => angle.DriveIndex < 0 || angle.DriveIndex >= _storages.Count);
        if (unassigned is not null)
        {
            Refuse(Loc.Format("replay.angleNeedsDrive", AngleName(unassigned.Id)));
            AngleList.SelectedItem = unassigned;
            return;
        }
        var sourceless = _angles.FirstOrDefault(angle => angle.SourceId == 0);
        if (sourceless is not null)
        {
            Refuse(Loc.Format("replay.angleNeedsSource", AngleName(sourceless.Id)));
            AngleList.SelectedItem = sourceless;
            return;
        }
        var duplicate = _angles
            .GroupBy(angle => (angle.SourceKind, angle.SourceId))
            .FirstOrDefault(group => group.Count() > 1);
        if (duplicate is not null)
        {
            var first = duplicate.First();
            Refuse(Loc.Format(
                first.SourceKind == "Input" ? "replay.angleDuplicateInput" : "replay.angleDuplicate",
                AngleName(first.Id)));
            AngleList.SelectedItem = first;
            return;
        }
        if (_angles.Count == 0)
        {
            Refuse(Loc.T("replay.needStorable"));
            return;
        }

        var drives = _storages
            .Where(storage => !string.IsNullOrWhiteSpace(storage.Path))
            .ToList();
        var storage = new JsonArray();
        foreach (var drive in drives)
        {
            var ids = new JsonArray();
            foreach (var angle in _angles.Where(angle => ReferenceEquals(_storages[angle.DriveIndex], drive)).OrderBy(angle => angle.Id))
                ids.Add(angle.Id);
            storage.Add(new JsonObject
            {
                ["path"] = drive.Path.Trim(),
                ["angleIds"] = ids,
            });
        }
        _replay["storage"] = storage;
        _replay["enabled"] = true;

        var format = _replay["format"] as JsonObject ?? new JsonObject();
        // The whole format travels: a document that keeps a zero resolution or a zero
        // segment length can never record, and the mixer could only say so much later.
        format["width"] = unit.Width;
        format["height"] = unit.Height;
        format["fpsNum"] = unit.FpsNum;
        format["fpsDen"] = unit.FpsDen;
        format["profile"] = ComboTag(ProfileBox, "Sq");
        format["segmentFrames"] = segmentFrames;
        _replay["format"] = format;
        _replay["buffering"] = new JsonObject
        {
            ["encodeQueueFrames"] = queue,
            ["writeBufferMb"] = buffer,
            ["overflowPolicy"] = ComboTag(OverflowBox, "DropFrames"),
            ["reserveMb"] = reserve,
        };
        _replay["retention"] = ComboTag(RetentionBox, "StopWhenFull");
        if (uint.TryParse(ComboTag(DeckPresentBox, "0"), out var deckInterval))
            _session.Settings.ReplayPresentInterval = MultiviewLayout.ClampPresentInterval(deckInterval);
        _replay["autoAddToPlaylist"] = AutoAddBox.IsChecked == true;

        var angles = new JsonArray();
        foreach (var angle in _angles.OrderBy(angle => angle.Id))
        {
            angles.Add(new JsonObject
            {
                ["id"] = angle.Id,
                ["name"] = string.IsNullOrWhiteSpace(angle.Name) ? $"#{angle.Id}" : angle.Name.Trim(),
                ["source"] = angle.SourceKind,
                ["sceneId"] = angle.SceneId,
                ["inputId"] = angle.InputId,
                ["audio"] = angle.AudioUnitId == 0 ? "Silent" : "Unit",
                ["audioUnitId"] = angle.AudioUnitId,
                ["recordMode"] = angle.RecordMode,
                ["encoder"] = angle.Encoder,
            });
        }
        _replay["angles"] = angles;

        // This window owns the storage, the angles, the format and the buffering. The
        // events, the playlists and which playlist each channel plays belong to the
        // deck: an operator who marked a clip while this window was open must not lose
        // it to an Apply. Sending the snapshot back whole did exactly that, and the
        // mixer then played a playlist whose clip transitions had been reverted.
        if (JsonNode.Parse(_session.ReplayJson) is JsonObject current)
        {
            foreach (var ownedByDeck in new[] { "events", "playlists", "channels", "activeAngleId" })
            {
                if (current[ownedByDeck] is { } value)
                    _replay[ownedByDeck] = value.DeepClone();
            }
            if (current["enabled"] is { } enabled)
                _replay["enabled"] = enabled.DeepClone();
        }

        // Removing an angle strands every clip that plays it, and the control layer
        // refuses the whole change when that happens. Say how many clips would go and
        // let the operator decide, rather than failing with a message they cannot act
        // on or deleting their work without asking.
        var keptAngles = _angles.Select(angle => angle.Id).ToHashSet();
        if (_replay["playlists"] is JsonArray lists)
        {
            var stranded = 0;
            foreach (var list in lists.OfType<JsonObject>())
            {
                if (list["clips"] is not JsonArray clips)
                    continue;
                stranded += clips.OfType<JsonObject>()
                    .Count(clip => !keptAngles.Contains(clip["angleId"]?.GetValue<uint>() ?? 0));
            }
            if (stranded > 0)
            {
                var answer = MessageBox.Show(
                    this,
                    Loc.Format("replay.orphanClips", stranded),
                    Title,
                    MessageBoxButton.OKCancel,
                    MessageBoxImage.Warning);
                if (answer != MessageBoxResult.OK)
                    return;
                foreach (var list in lists.OfType<JsonObject>())
                {
                    if (list["clips"] is not JsonArray clips)
                        continue;
                    for (var index = clips.Count - 1; index >= 0; index--)
                    {
                        if (clips[index] is JsonObject clip
                            && !keptAngles.Contains(clip["angleId"]?.GetValue<uint>() ?? 0))
                        {
                            clips.RemoveAt(index);
                        }
                    }
                }
            }
        }

        var json = ReplayModel.Serialize(_replay);
        _session.ReplayJson = json;
        try
        {
            if (App.IsRemote)
            {
                // A remote core owns the events and playlists too, so sending the whole
                // document from this window would overwrite whatever another operator
                // marked in the meantime. Only the settings travel.
                if (Owner is MainWindow main)
                {
                    main.RemoteMutate(
                        "{\"kind\":\"updateReplaySettings\",\"settings\":" + ReplayModel.SerializeSettings(_replay) + "}",
                        Title);
                }
            }
            else
            {
                // Publishing the session reconfigures the mixer and validates the
                // change, so the refusal (and its reason) comes back with it.
                SessionStore.Publish(_session);
            }
        }
        catch (Exception error)
        {
            Refuse(error.Message);
            return;
        }
        _dirty = false;
        RefreshFreeSpace();
        // The window stays open: an operator tuning four cameras should not have to
        // reopen it between attempts. Closing keeps what was applied.
        StatusText.Text = Loc.T("replay.applied");
    }

    private void Close_Click(object sender, RoutedEventArgs e) => Close();

    protected override void OnClosing(CancelEventArgs e)
    {
        if (_dirty)
        {
            var answer = MessageBox.Show(
                this,
                Loc.T("replay.discardEdits"),
                Loc.T("replay.settingsTitle"),
                MessageBoxButton.YesNo,
                MessageBoxImage.Warning);
            if (answer != MessageBoxResult.Yes)
            {
                e.Cancel = true;
                return;
            }
        }
        base.OnClosing(e);
    }

    private void Refuse(string message) => StatusText.Text = message;

    private static string ComboTag(ComboBox box, string fallback) =>
        (box.SelectedItem as ComboBoxItem)?.Tag as string ?? fallback;

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
}
