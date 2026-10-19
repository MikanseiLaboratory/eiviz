using System.Text.Json.Nodes;
using System.Windows;
using System.Windows.Controls;
using Eiviz.Host.I18n;
using Eiviz.Host.Interop;

namespace Eiviz.Host.Dialogs;

/// <summary>
/// Project-scoped replay configuration. It exists only while the project has a
/// Replay input, which is why it is a dedicated window instead of a settings tab.
/// </summary>
public partial class ReplaySettingsWindow : Window
{
    private readonly Session _session;
    private readonly List<AngleRow> _angles = [];
    /// Recording drives. An angle may name the drive it records to; every other
    /// angle goes to the first one.
    private readonly List<StorageRow> _storages = [];
    private JsonObject? _replay;
    private uint _angleLimit;
    private uint _channelLimit;
    private bool _hasLimits;

    public ReplaySettingsWindow(Session session)
    {
        InitializeComponent();
        _session = session;
        // What an angle records: a scene's composited output, or an input's own
        // picture without composing a scene for it.
        SourceKindBox.Items.Add(new ComboBoxItem { Content = Loc.T("replay.sourceScene"), Tag = "Scene" });
        SourceKindBox.Items.Add(new ComboBoxItem { Content = Loc.T("replay.sourceInput"), Tag = "Input" });
        SourceKindBox.SelectedIndex = 0;
        RetentionBox.Items.Add(new ComboBoxItem { Content = Loc.T("replay.retentionStop"), Tag = "StopWhenFull" });
        RetentionBox.Items.Add(new ComboBoxItem { Content = Loc.T("replay.retentionOverwrite"), Tag = "Overwrite" });
        RetentionBox.Items.Add(new ComboBoxItem { Content = Loc.T("replay.retentionProtect"), Tag = "OverwriteUnprotected" });
        RetentionBox.SelectedIndex = 0;
        Load();
    }

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
                if (item is not JsonObject entry)
                    continue;
                var row = new StorageRow { Path = entry["path"]?.GetValue<string>() ?? "" };
                if (entry["angleIds"] is JsonArray ids)
                {
                    foreach (var id in ids)
                    {
                        if (id is not null)
                            row.AngleIds.Add(id.GetValue<uint>());
                    }
                }
                _storages.Add(row);
            }
        }
        SelectTag(ProfileBox, _replay["format"]?["profile"]?.GetValue<string>() ?? "Sq");
        var buffering = _replay["buffering"] as JsonObject;
        QueueBox.Text = (buffering?["encodeQueueFrames"]?.GetValue<uint>() ?? 6).ToString();
        BufferBox.Text = (buffering?["writeBufferMb"]?.GetValue<uint>() ?? 512).ToString();
        ReserveBox.Text = (buffering?["reserveMb"]?.GetValue<uint>() ?? 2048).ToString();
        SelectTag(OverflowBox, buffering?["overflowPolicy"]?.GetValue<string>() ?? "DropFrames");
        SelectTag(RetentionBox, _replay["retention"]?.GetValue<string>() ?? "StopWhenFull");
        AutoAddBox.IsChecked = ReplayModel.AutoAddToPlaylist(_replay);

        _angles.Clear();
        foreach (var angle in ReplayModel.Angles(_replay))
        {
            _angles.Add(new AngleRow
            {
                Id = angle.Id,
                Name = angle.Label,
                Source = angle.Source,
                SceneId = angle.SceneId,
                InputId = angle.InputId,
                SourceName = angle.Source == "Input"
                    ? InputName(angle.InputId)
                    : SceneName(angle.SceneId),
                AudioUnitId = angle.AudioUnitId,
                RecordMode = angle.RecordMode
            });
        }
        // The add row starts on scenes, which is what most projects record.
        SelectTag(SourceKindBox, "Scene");
        RefreshAngleSourceBox();
        RefreshAngles();
        RefreshStorages();
        RefreshAngleAudioBox();
        RefreshFormatText();
        SegmentFramesBox.Text = (_replay["format"]?["segmentFrames"]?.GetValue<uint>() ?? 120).ToString();
        RefreshPlan();
    }

    private void RefreshStorages()
    {
        foreach (var storage in _storages)
            storage.Angles = storage.AngleIds.Count == 0
                ? ""
                : string.Join(",", storage.AngleIds.OrderBy(id => id));
        StorageList.ItemsSource = null;
        StorageList.ItemsSource = _storages.ToList();
        if (StorageList.SelectedIndex < 0 && _storages.Count > 0)
            StorageList.SelectedIndex = 0;
        RefreshAngleStorageBox();
        RefreshAddDriveBox();
        RefreshAngles();
    }

    /// <summary>
    /// Drive list of the add row and the edit row. Both name the drives the same
    /// way, and neither invents one: a recording needs a drive the operator chose.
    /// </summary>
    private void RefreshAddDriveBox()
    {
        var selected = AddDriveBox.SelectedIndex;
        var labels = _storages.Select((storage, index) => $"{index + 1}: {storage.Path}").ToList();
        AddDriveBox.ItemsSource = labels;
        if (selected >= 0 && selected < labels.Count)
            AddDriveBox.SelectedIndex = selected;
        else if (labels.Count > 0)
            AddDriveBox.SelectedIndex = 0;
    }

    private void RefreshAngleStorageBox()
    {
        var selected = AngleStorageBox.SelectedIndex;
        var labels = _storages.Select((storage, index) => $"{index + 1}: {storage.Path}").ToList();
        AngleStorageBox.ItemsSource = labels;
        if (selected >= 0 && selected < labels.Count)
            AngleStorageBox.SelectedIndex = selected;
        else if (labels.Count > 0)
            AngleStorageBox.SelectedIndex = 0;
    }

    /// <summary>Index of the drive an angle records to, or -1 when none claims it.</summary>
    private int DriveIndexOf(uint angleId)
    {
        for (var index = 0; index < _storages.Count; index++)
        {
            if (_storages[index].AngleIds.Contains(angleId))
                return index;
        }
        return -1;
    }

    /// <summary>1-based drive for the list column; 0 when the angle has none.</summary>
    private int DriveOf(uint angleId) => DriveIndexOf(angleId) + 1;

    /// <summary>
    /// Asks for a folder the way every other path in the app is chosen, and files it
    /// straight away: picking a drive is the whole action, so there is no second
    /// "add" step to forget.
    /// </summary>
    private void BrowseStorage_Click(object sender, RoutedEventArgs e)
    {
        var dialog = new Microsoft.Win32.OpenFolderDialog
        {
            Title = Loc.T("replay.pickStorage"),
            Multiselect = false
        };
        if (dialog.ShowDialog(this) != true)
            return;
        AddStorage(dialog.FolderName);
    }

    /// <summary>Files a drive. The same folder twice is refused, not duplicated.</summary>
    private void AddStorage(string path)
    {
        var trimmed = path.Trim();
        if (string.IsNullOrWhiteSpace(trimmed))
            return;
        if (_storages.Any(storage => storage.Path.Equals(trimmed, StringComparison.OrdinalIgnoreCase)))
        {
            StatusText.Text = Loc.T("replay.duplicateStorage");
            return;
        }
        _storages.Add(new StorageRow { Path = trimmed });
        RefreshStorages();
        StatusText.Text = Loc.Format("replay.storageAdded", trimmed);
    }

    private void RemoveStorage_Click(object sender, RoutedEventArgs e)
    {
        if (StorageList.SelectedItem is not StorageRow row)
        {
            StatusText.Text = Loc.T("replay.selectStorage");
            return;
        }
        if (_storages.Count <= 1)
        {
            StatusText.Text = Loc.T("replay.needStorable");
            return;
        }
        _storages.Remove(row);
        // Angles that named the removed drive fall back to the first one.
        foreach (var angle in _angles)
        {
            if (row.AngleIds.Contains(angle.Id))
            {
                foreach (var storage in _storages)
                    storage.AngleIds.Remove(angle.Id);
            }
        }
        RefreshStorages();
    }

    /// <summary>
    /// The angle list with the drive each angle records to, and the edit row filled
    /// from the selection. Nothing is guessed here: an angle without a drive shows 0
    /// and Apply refuses it.
    /// </summary>
    private void RefreshAngles()
    {
        foreach (var angle in _angles)
        {
            angle.SourceName = angle.Source == "Input" ? InputName(angle.InputId) : SceneName(angle.SceneId);
            angle.Audio = AudioLabel(angle.AudioUnitId);
            angle.Drive = DriveOf(angle.Id);
        }
        var selected = (AngleList.SelectedItem as AngleRow)?.Id;
        AngleList.ItemsSource = null;
        AngleList.ItemsSource = _angles.ToList();
        if (selected is { } id)
        {
            var row = _angles.FirstOrDefault(angle => angle.Id == id);
            if (row is not null)
                AngleList.SelectedItem = row;
        }
        RefreshAngleEditors();
    }

    /// <summary>
    /// Fills the edit row from the selected angle, and disables it when nothing is
    /// selected so an edit cannot land on the wrong row.
    /// </summary>
    private void RefreshAngleEditors()
    {
        var angle = AngleList.SelectedItem as AngleRow;
        var enabled = angle is not null;
        AngleNameBox.IsEnabled = enabled;
        AngleModeBox.IsEnabled = enabled;
        AngleAudioBox.IsEnabled = enabled;
        AngleStorageBox.IsEnabled = enabled;
        if (angle is null)
        {
            AngleNameBox.Text = "";
            return;
        }
        AngleNameBox.Text = angle.Name;
        RefreshAngleModeBox();
        SelectAudio(AngleAudioBox, angle.AudioUnitId);
        var index = DriveIndexOf(angle.Id);
        AngleStorageBox.SelectedIndex = index >= 0 && index < AngleStorageBox.Items.Count ? index : -1;
    }

    /// <summary>
    /// Record modes the selected angle may use. Passthrough means "store the frames
    /// as they arrive", so it exists only for an OMT input; a scene is composited.
    /// </summary>
    private void RefreshAngleModeBox()
    {
        var angle = AngleList.SelectedItem as AngleRow;
        var omt = angle is not null
            && angle.Source == "Input"
            && _session.Inputs.FirstOrDefault(input => input.Id == angle.InputId)?.Kind == InputKind.OMT;
        var previous = angle?.RecordMode ?? "Composite";
        AngleModeBox.Items.Clear();
        AngleModeBox.Items.Add(new ComboBoxItem
        {
            Content = Loc.T("replay.recordComposite"),
            Tag = "Composite"
        });
        if (omt)
        {
            AngleModeBox.Items.Add(new ComboBoxItem
            {
                Content = Loc.T("replay.recordPassthrough"),
                Tag = "Passthrough"
            });
        }
        SelectTag(AngleModeBox, omt && previous == "Passthrough" ? "Passthrough" : "Composite");
    }

    /// <summary>Record modes the add row may use, for the source it is pointing at.</summary>
    private void RefreshAddModeBox()
    {
        var kind = ComboTag(SourceKindBox, "Scene");
        var omt = kind == "Input"
            && SourceBox.SelectedItem is InputEntry input
            && input.Kind == InputKind.OMT;
        var previous = ComboTag(AddModeBox, "Composite");
        AddModeBox.Items.Clear();
        AddModeBox.Items.Add(new ComboBoxItem
        {
            Content = Loc.T("replay.recordComposite"),
            Tag = "Composite"
        });
        if (omt)
        {
            AddModeBox.Items.Add(new ComboBoxItem
            {
                Content = Loc.T("replay.recordPassthrough"),
                Tag = "Passthrough"
            });
        }
        SelectTag(AddModeBox, omt && previous == "Passthrough" ? "Passthrough" : "Composite");
    }

    /// <summary>
    /// Sources the add row can pick: scenes, or the inputs that have a picture. A
    /// Replay channel is not a camera, and an audio input has nothing to record.
    /// </summary>
    private void RefreshAngleSourceBox()
    {
        var kind = ComboTag(SourceKindBox, "Scene");
        if (kind == "Input")
        {
            SourceBox.ItemsSource = _session.Inputs
                .Where(input => input.Kind != InputKind.Replay && input.Kind != InputKind.Audio)
                .ToList();
        }
        else
        {
            SourceBox.ItemsSource = _session.Scenes.ToList();
        }
        SourceBox.DisplayMemberPath = "Name";
        if (SourceBox.Items.Count > 0)
            SourceBox.SelectedIndex = 0;
        RefreshAddModeBox();
    }

    private void SourceKind_Changed(object sender, SelectionChangedEventArgs e)
    {
        if (SourceKindBox is not null)
            RefreshAngleSourceBox();
    }

    private void Source_Changed(object sender, SelectionChangedEventArgs e)
    {
        if (AddModeBox is not null)
            RefreshAddModeBox();
    }

    private void AngleList_SelectionChanged(object sender, SelectionChangedEventArgs e) =>
        RefreshAngleEditors();

    private void AngleName_Changed(object sender, RoutedEventArgs e)
    {
        if (AngleList.SelectedItem is not AngleRow angle)
            return;
        var name = AngleNameBox.Text.Trim();
        if (name.Length == 0)
            name = angle.Source == "Input" ? InputName(angle.InputId) : SceneName(angle.SceneId);
        angle.Name = name;
        RefreshAngles();
    }

    private void AngleMode_Changed(object sender, SelectionChangedEventArgs e)
    {
        if (AngleList.SelectedItem is not AngleRow angle)
            return;
        angle.RecordMode = ComboTag(AngleModeBox, "Composite");
        RefreshAngles();
    }

    private void AngleAudio_Changed(object sender, SelectionChangedEventArgs e)
    {
        if (AngleList.SelectedItem is not AngleRow angle)
            return;
        angle.AudioUnitId = (AngleAudioBox.SelectedItem as ComboBoxItem)?.Tag as ulong? ?? 0;
        RefreshAngles();
    }

    /// <summary>Moves the selected angle to the drive the operator picked.</summary>
    private void AngleStorage_Changed(object sender, SelectionChangedEventArgs e)
    {
        if (AngleList.SelectedItem is not AngleRow angle)
            return;
        var index = AngleStorageBox.SelectedIndex;
        if (index < 0 || index >= _storages.Count)
            return;
        foreach (var storage in _storages)
            storage.AngleIds.Remove(angle.Id);
        _storages[index].AngleIds.Add(angle.Id);
        RefreshStorages();
    }

    /// <summary>
    /// Angle audio sources: silence, or a mixing unit's program mix. The waveform
    /// and any recorded audio come from whichever is chosen.
    /// </summary>
    private void RefreshAngleAudioBox()
    {
        foreach (var box in new[] { AngleAudioBox, AddAudioBox })
        {
            if (box is null)
                continue;
            var selected = (box.SelectedItem as ComboBoxItem)?.Tag as ulong?;
            box.Items.Clear();
            box.Items.Add(new ComboBoxItem { Content = Loc.T("replay.audioSilent"), Tag = 0UL });
            foreach (var unit in _session.Units)
                box.Items.Add(new ComboBoxItem { Content = unit.Name, Tag = unit.Id });
            var match = selected is { } id
                ? box.Items.OfType<ComboBoxItem>().FirstOrDefault(item => (ulong?)item.Tag == id)
                : null;
            box.SelectedItem = match ?? box.Items[0];
        }
        foreach (var angle in _angles)
            angle.Audio = AudioLabel(angle.AudioUnitId);
        AngleList.ItemsSource = null;
        AngleList.ItemsSource = _angles.ToList();
    }

    /// <summary>Selects the audio entry for a unit id in one combo box.</summary>
    private static void SelectAudio(ComboBox box, ulong unitId)
    {
        foreach (ComboBoxItem item in box.Items)
        {
            if ((ulong?)item.Tag == unitId)
            {
                box.SelectedItem = item;
                return;
            }
        }
        if (box.Items.Count > 0)
            box.SelectedIndex = 0;
    }

    private string AudioLabel(ulong unitId)
    {
        if (unitId == 0)
            return Loc.T("replay.audioSilent");
        var unit = _session.Units.FirstOrDefault(item => item.Id == unitId);
        return unit?.Name ?? Loc.Format("replay.audioUnit", unitId);
    }

    private string SceneName(ulong sceneId) =>
        _session.Scenes.FirstOrDefault(scene => scene.Id == sceneId)?.Name
        ?? Loc.Format("replay.sceneMissing", sceneId);

    private string InputName(ulong inputId) =>
        _session.Inputs.FirstOrDefault(input => input.Id == inputId)?.Name
        ?? Loc.Format("replay.inputMissing", inputId);

    /// <summary>
    /// The format Replay records in: the first Mixing Unit's picture and rate. It is
    /// shown rather than edited, because every angle shares one format.
    /// </summary>
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
            PlanText.Text = "";
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
                return angle.Name;
        }
        return id.ToString();
    }

    private void Retention_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (StatusText is not null)
            StatusText.Text = Loc.T("replay.retentionHelp");
    }

    private void AutoAdd_Click(object sender, RoutedEventArgs e) => RefreshPlan();

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

    private static string ComboTag(ComboBox box) =>
        (box.SelectedItem as ComboBoxItem)?.Tag as string ?? "";

    /// <summary>
    /// Adds an angle from the add row: what to record (a scene or an input), how
    /// (composite, or passthrough for an OMT input), its audio, and the drive it
    /// records to. Every refusal names what is missing instead of doing nothing.
    /// </summary>
    private void AddAngle_Click(object sender, RoutedEventArgs e)
    {
        if (_storages.Count == 0)
        {
            StatusText.Text = Loc.T("replay.needDriveForAngle");
            return;
        }
        if (_hasLimits && _angleLimit != uint.MaxValue && _angles.Count >= _angleLimit)
        {
            StatusText.Text = Loc.Format("replay.planLimits", _angleLimit, _channelLimit);
            return;
        }
        var kind = ComboTag(SourceKindBox, "Scene");
        var mode = ComboTag(AddModeBox, "Composite");
        var id = _angles.Count == 0 ? 1u : _angles.Max(angle => angle.Id) + 1;
        var audioUnitId = (AddAudioBox.SelectedItem as ComboBoxItem)?.Tag as ulong? ?? 0;
        AngleRow row;
        if (kind == "Input")
        {
            if (SourceBox.SelectedItem is not InputEntry input)
            {
                StatusText.Text = Loc.T("replay.selectSource");
                return;
            }
            if (_angles.Any(angle => angle.Source == "Input" && angle.InputId == input.Id))
            {
                StatusText.Text = Loc.T("replay.angleDuplicateInput");
                return;
            }
            row = new AngleRow
            {
                Id = id,
                Name = input.Name,
                Source = "Input",
                InputId = input.Id,
                SourceName = input.Name,
                RecordMode = mode,
                AudioUnitId = audioUnitId
            };
        }
        else
        {
            if (SourceBox.SelectedItem is not SceneEntry scene)
            {
                StatusText.Text = Loc.T("replay.selectSource");
                return;
            }
            if (_angles.Any(angle => angle.Source == "Scene" && angle.SceneId == scene.Id))
            {
                StatusText.Text = Loc.T("replay.angleDuplicate");
                return;
            }
            row = new AngleRow
            {
                Id = id,
                Name = scene.Name,
                Source = "Scene",
                SceneId = scene.Id,
                SourceName = scene.Name,
                RecordMode = mode,
                AudioUnitId = audioUnitId
            };
        }
        _angles.Add(row);
        var drive = AddDriveBox.SelectedIndex >= 0 ? AddDriveBox.SelectedIndex : 0;
        _storages[drive].AngleIds.Add(id);
        RefreshStorages();
        StatusText.Text = Loc.Format("replay.angleAdded", row.Name);
    }

    private void RemoveAngle_Click(object sender, RoutedEventArgs e)
    {
        if (AngleList.SelectedItem is not AngleRow row)
            return;
        // A clip that plays this angle, or takes its sound from it, would be left
        // pointing at nothing, so the angle waits until those clips are gone.
        var users = ClipsUsingAngle(row.Id);
        if (users > 0)
        {
            StatusText.Text = Loc.Format("replay.angleInUse", users);
            return;
        }
        _angles.RemoveAll(angle => angle.Id == row.Id);
        // A drive that still listed it would fail validation on Apply.
        foreach (var storage in _storages)
            storage.AngleIds.Remove(row.Id);
        RefreshStorages();
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
                var picture = (clip["angleId"]?.GetValue<uint>() ?? 0) == angleId;
                var audio = (clip["audio"]?.GetValue<string>() ?? "FollowAngle") == "Angle"
                    && (clip["audioAngleId"]?.GetValue<uint>() ?? 0) == angleId;
                if (picture || audio)
                    count++;
            }
        }
        return count;
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
            // Every angle records one format, and the format comes from the Mixing
            // Unit. Without one there is nothing to record in.
            Refuse(Loc.T("replay.needMixingUnit"));
            return;
        }
        // Keep each drive's angle assignments with its path: dropping blank rows
        // must not shift the assignments onto another drive.
        var drives = _storages
            .Where(storage => !string.IsNullOrWhiteSpace(storage.Path))
            .Select(storage => (Path: storage.Path.Trim(), storage.AngleIds))
            .ToList();
        if (drives.Count > 0 && _angles.Count == 0)
        {
            // Recording with no angle writes nothing, so refuse instead of
            // storing a configuration that cannot run.
            Refuse(Loc.T("replay.needStorable"));
            return;
        }
        // One drive takes every angle: the operator added the drive for this
        // recording, and a recording never guesses which disk to write to.
        if (drives.Count == 1)
        {
            foreach (var angle in _angles)
            {
                if (!drives[0].AngleIds.Contains(angle.Id))
                    drives[0].AngleIds.Add(angle.Id);
            }
        }
        // A drive and an angle are a recording setup, so the document says so. The
        // control layer refuses the other combinations, and a host that wrote them
        // would leave a project that can never record.
        _replay["enabled"] = drives.Count > 0 && _angles.Count > 0;
        var storages = new JsonArray();
        // Only angles that still exist are assigned, so a removed one cannot be
        // left behind on a drive.
        var known = _angles.Select(angle => angle.Id).ToHashSet();
        foreach (var drive in drives)
        {
            var ids = new JsonArray();
            foreach (var id in drive.AngleIds.OrderBy(id => id))
            {
                if (known.Contains(id))
                    ids.Add(id);
            }
            storages.Add(new JsonObject
            {
                ["path"] = drive.Path,
                ["angleIds"] = ids
            });
        }
        _replay["storage"] = storages;
        var format = _replay["format"] as JsonObject ?? new JsonObject();
        // The whole format travels: a document that keeps a zero resolution or a
        // zero segment length can never record, and the mixer would only be able to
        // say so much later.
        format["width"] = unit.Width;
        format["height"] = unit.Height;
        format["fpsNum"] = unit.FpsNum;
        format["fpsDen"] = unit.FpsDen;
        format["profile"] = ComboTag(ProfileBox);
        format["segmentFrames"] = segmentFrames;
        _replay["format"] = format;
        _replay["buffering"] = new JsonObject
        {
            ["encodeQueueFrames"] = queue,
            ["writeBufferMb"] = buffer,
            ["overflowPolicy"] = ComboTag(OverflowBox),
            ["reserveMb"] = reserve
        };
        _replay["retention"] = ComboTag(RetentionBox);
        _replay["autoAddToPlaylist"] = AutoAddBox.IsChecked == true;

        var angles = new JsonArray();
        foreach (var angle in _angles)
        {
            angles.Add(new JsonObject
            {
                ["id"] = angle.Id,
                ["name"] = angle.Name,
                ["source"] = angle.Source,
                ["sceneId"] = angle.SceneId,
                ["inputId"] = angle.InputId,
                ["audio"] = angle.AudioUnitId == 0 ? "Silent" : "Unit",
                ["audioUnitId"] = angle.AudioUnitId,
                ["recordMode"] = angle.RecordMode
            });
        }
        _replay["angles"] = angles;
        var active = ReplayModel.ActiveAngle(_replay);
        if (active != 0 && !_angles.Any(angle => angle.Id == active))
            _replay.Remove("activeAngleId");

        var json = ReplayModel.Serialize(_replay);
        _session.ReplayJson = json;
        try
        {
            if (App.IsRemote)
            {
                // A remote core owns the events and playlists too, so sending the
                // whole document from this window would overwrite whatever another
                // operator marked in the meantime. Only the settings travel.
                if (Owner is MainWindow main)
                    main.RemoteMutate(
                        "{\"kind\":\"updateReplaySettings\",\"settings\":" + ReplayModel.SerializeSettings(_replay) + "}",
                        Title);
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
            HostLog.WriteException(error);
            Refuse(string.IsNullOrWhiteSpace(error.Message)
                ? Loc.T("replay.publishRefused")
                : error.Message);
            return;
        }
        // The deck opens this window non-modally, so there is no dialog result to
        // set: setting one throws and leaves the window open with no confirmation.
        // Applying closes the window and the confirmation lands on the window the
        // operator is looking at next.
        if (Owner is Window owner)
            StatusToast.Show(owner, Loc.T("replay.applied"));
        Close();
    }

    /// <summary>
    /// Discards the edits. Nothing was published, so closing is the whole job; the
    /// operator needs the window to actually go away.
    /// </summary>
    private void Cancel_Click(object sender, RoutedEventArgs e) => Close();

    /// <summary>
    /// Reports why the settings were not taken. The operator pressed Apply, so the
    /// answer is put in front of them instead of in a line at the bottom of a
    /// scrolling window.
    /// </summary>
    private void Refuse(string message)
    {
        StatusText.Text = message;
        MessageBox.Show(this, message, Title, MessageBoxButton.OK, MessageBoxImage.Warning);
    }

    private sealed class AngleRow
    {
        public uint Id { get; init; }
        public string Name { get; set; } = "";
        /// `Scene` or `Input`: what this angle records from.
        public string Source { get; init; } = "Scene";
        public ulong SceneId { get; init; }
        /// Input this angle records when `Source` is `Input`.
        public ulong InputId { get; init; }
        /// Name of the scene or input it records, for the list column.
        public string SourceName { get; set; } = "";
        /// `Composite` or `Passthrough` (OMT inputs only).
        public string RecordMode { get; set; } = "Composite";
        public string SourceLabel => Source == "Input"
            ? Loc.T("replay.sourceInput")
            : Loc.T("replay.sourceScene");
        public string ModeLabel => RecordMode == "Passthrough"
            ? Loc.T("replay.recordPassthrough")
            : Loc.T("replay.recordComposite");
        public string Audio { get; set; } = "";
        /// Mixing unit whose program mix feeds this angle. Zero means silent.
        public ulong AudioUnitId { get; set; }
        /// 1-based drive the angle records to, for the list column. 0 means none.
        public int Drive { get; set; }
    }

    private sealed class StorageRow
    {
        public string Path { get; init; } = "";
        public HashSet<uint> AngleIds { get; } = [];
        public string Angles { get; set; } = "";
    }
}

