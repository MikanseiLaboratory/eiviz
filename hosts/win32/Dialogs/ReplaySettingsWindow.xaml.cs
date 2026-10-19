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
        SceneBox.ItemsSource = session.Scenes;
        if (session.Scenes.Count > 0)
            SceneBox.SelectedIndex = 0;
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
                SceneName = angle.Source == "Input"
                    ? InputName(angle.InputId)
                    : SceneName(angle.SceneId),
                AudioUnitId = angle.AudioUnitId,
                RecordMode = angle.RecordMode
            });
        }
        RefreshAngles();
        RefreshStorages();
        RefreshAngleAudioBox();
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
    }

    private void RefreshAngleStorageBox()
    {
        var selected = AngleStorageBox.SelectedIndex;
        AngleStorageBox.ItemsSource = _storages
            .Select((storage, index) => $"{index + 1}: {storage.Path}")
            .ToList();
        if (selected >= 0 && selected < _storages.Count)
            AngleStorageBox.SelectedIndex = selected;
        else if (_storages.Count > 0)
            AngleStorageBox.SelectedIndex = 0;
        // The angle list shows the drive each angle records to.
        foreach (var angle in _angles)
            angle.Drive = DriveOf(angle.Id);
        AngleList.ItemsSource = null;
        AngleList.ItemsSource = _angles.ToList();
    }

    private int DriveOf(uint angleId)
    {
        for (var index = 0; index < _storages.Count; index++)
        {
            if (_storages[index].AngleIds.Contains(angleId))
                return index + 1;
        }
        return _storages.Count == 0 ? 0 : 1;
    }

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

    private void AssignStorage_Click(object sender, RoutedEventArgs e)
    {
        if (AngleList.SelectedItem is not AngleRow angle)
        {
            StatusText.Text = Loc.T("replay.selectAngle");
            return;
        }
        if (AngleStorageBox.SelectedIndex < 0 || AngleStorageBox.SelectedIndex >= _storages.Count)
            return;
        foreach (var storage in _storages)
            storage.AngleIds.Remove(angle.Id);
        // Drive 1 is the default, so naming it explicitly is not required.
        if (AngleStorageBox.SelectedIndex > 0)
            _storages[AngleStorageBox.SelectedIndex].AngleIds.Add(angle.Id);
        RefreshStorages();
    }

    private void AngleList_SelectionChanged(object sender, SelectionChangedEventArgs e) =>
        RefreshAngleStorageBox();

    /// <summary>
    /// Angle audio sources: silence, or a mixing unit's program mix. The waveform
    /// and any recorded audio come from whichever is chosen.
    /// </summary>
    private void RefreshAngleAudioBox()
    {
        var selected = (AngleAudioBox.SelectedItem as ComboBoxItem)?.Tag as ulong?;
        AngleAudioBox.Items.Clear();
        AngleAudioBox.Items.Add(new ComboBoxItem { Content = Loc.T("replay.audioSilent"), Tag = 0UL });
        foreach (var unit in _session.Units)
            AngleAudioBox.Items.Add(new ComboBoxItem { Content = unit.Name, Tag = unit.Id });
        var match = selected is { } id
            ? AngleAudioBox.Items.OfType<ComboBoxItem>().FirstOrDefault(item => (ulong?)item.Tag == id)
            : null;
        AngleAudioBox.SelectedItem = match ?? AngleAudioBox.Items[0];
        foreach (var angle in _angles)
            angle.Audio = AudioLabel(angle.AudioUnitId);
        AngleList.ItemsSource = null;
        AngleList.ItemsSource = _angles.ToList();
    }

    private string AudioLabel(ulong unitId)
    {
        if (unitId == 0)
            return Loc.T("replay.audioSilent");
        var unit = _session.Units.FirstOrDefault(item => item.Id == unitId);
        return unit?.Name ?? Loc.Format("replay.audioUnit", unitId);
    }

    private void AssignAudio_Click(object sender, RoutedEventArgs e)
    {
        if (AngleList.SelectedItem is not AngleRow angle)
        {
            StatusText.Text = Loc.T("replay.selectAngle");
            return;
        }
        angle.AudioUnitId = (AngleAudioBox.SelectedItem as ComboBoxItem)?.Tag as ulong? ?? 0;
        RefreshAngleAudioBox();
    }

    private string SceneName(ulong sceneId) =>
        _session.Scenes.FirstOrDefault(scene => scene.Id == sceneId)?.Name
        ?? Loc.Format("replay.sceneMissing", sceneId);

    private string InputName(ulong inputId) =>
        _session.Inputs.FirstOrDefault(input => input.Id == inputId)?.Name
        ?? Loc.Format("replay.inputMissing", inputId);

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

    private void AddAngle_Click(object sender, RoutedEventArgs e)
    {
        if (SceneBox.SelectedItem is not SceneEntry scene)
            return;
        if (_angles.Any(angle => angle.SceneId == scene.Id))
        {
            StatusText.Text = Loc.T("replay.angleDuplicate");
            return;
        }
        var id = _angles.Count == 0 ? 1u : _angles.Max(angle => angle.Id) + 1;
        if (_hasLimits && _angleLimit != uint.MaxValue && _angles.Count >= _angleLimit)
        {
            StatusText.Text = Loc.Format("replay.planLimits", _angleLimit, _channelLimit);
            return;
        }
        _angles.Add(new AngleRow
        {
            Id = id,
            Name = scene.Name,
            SceneId = scene.Id,
            SceneName = scene.Name,
            Audio = Loc.T("replay.audioSilent")
        });
        RefreshAngles();
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
        RefreshAngles();
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

    private void RefreshAngles()
    {
        AngleList.ItemsSource = null;
        AngleList.ItemsSource = _angles.ToList();
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
        format["profile"] = ComboTag(ProfileBox);
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
        public string Name { get; init; } = "";
        /// `Scene` or `Input`: what this angle records from.
        public string Source { get; init; } = "Scene";
        public ulong SceneId { get; init; }
        /// Input this angle records when `Source` is `Input`.
        public ulong InputId { get; init; }
        public string SceneName { get; init; } = "";
        public string Audio { get; set; } = "";
        /// Mixing unit whose program mix feeds this angle. Zero means silent.
        public ulong AudioUnitId { get; set; }
        /// `Composite` or `Passthrough` (OMT inputs only).
        public string RecordMode { get; init; } = "Composite";
        /// 1-based drive the angle records to, for the list column.
        public int Drive { get; set; }
    }

    private sealed class StorageRow
    {
        public string Path { get; init; } = "";
        public HashSet<uint> AngleIds { get; } = [];
        public string Angles { get; set; } = "";
    }
}

