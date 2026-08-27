using System.Windows;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Input;
using System.Windows.Threading;
using Eiviz.Host.Dialogs;
using Eiviz.Host.Interop;
using Eiviz.Host.Media;
using Eiviz.Host.Preview;

namespace Eiviz.Host;

public partial class MainWindow : Window
{
    private SceneEntry? _selectedScene;
    private int _tbarPresetIndex;
    private bool _tbarLatching;
    private bool _tbarLocked;
    private bool _suppressUnitChange;
    private OverlayWindow? _overlay;
    private ResourceMonitorWindow? _resourcesWindow;
    private readonly List<MultiviewWindow> _multiviews = [];
    private readonly HashSet<int> _transitionExpanded = [];
    private readonly DispatcherTimer _meterTimer = new() { Interval = TimeSpan.FromMilliseconds(50) };
    private readonly Dictionary<ulong, MeterStrip> _meters = [];
    private readonly ResourceMonitor _resources = new();
    private bool _videoSeeking;
    private bool _videoSeekSuppress;

    public MainWindow()
    {
        InitializeComponent();
        InputList.ItemsSource = _session.Inputs;
        UnitBox.ItemsSource = _session.Units;
        _suppressUnitChange = true;
        UnitBox.SelectedIndex = 0;
        _suppressUnitChange = false;
        RebuildScenes();
        RebuildTransitions();
        RebuildOverlayToggles();
        RebuildMeters();
        UpdateStatus();
        _meterTimer.Tick += (_, _) => TickMeters();
        _meterTimer.Start();
        Closed += (_, _) =>
        {
            _meterTimer.Stop();
            _resources.Dispose();
        };
        Loaded += (_, _) =>
        {
            ApplyAspect();
            WaveOutMonitor.EnsureStarted();
            if (_session.Scenes.Count > 0)
                SelectScene(_session.Scenes[0]);
        };
    }

    private Session _session => ((App)Application.Current).Session;

    private CommandQueue Commands => ((App)Application.Current).Commands;

    private MixingUnitEntry SelectedUnit =>
        UnitBox.SelectedItem as MixingUnitEntry ?? _session.Units[0];

    private uint SceneWidth => SelectedUnit.Width;
    private uint SceneHeight => SelectedUnit.Height;

    private void RebuildScenes()
    {
        ScenePanel.Children.Clear();
        foreach (var scene in _session.Scenes)
        {
            var tile = new SceneTile();
            tile.SceneSelected += (_, selected) => SelectScene(selected);
            tile.SceneEditRequested += (_, selected) => OpenSceneEditor(selected);
            tile.Bind(scene, _selectedScene?.Id == scene.Id);
            ScenePanel.Children.Add(tile);
        }
    }

    private void SelectScene(SceneEntry scene)
    {
        _selectedScene = scene;
        Commands.TryEnqueue(new PreviewSceneCommand(SelectedUnit.Id, scene.GpuId));
        foreach (SceneTile tile in ScenePanel.Children)
            tile.SetSelected(tile.Scene?.Id == scene.Id);
    }

    private void FirePreset(TransitionPreset preset)
    {
        var unit = SelectedUnit;
        if (preset.Kind == MixerNative.TransitionCut || preset.DurationFrames <= 1)
            Commands.TryEnqueue(new CutCommand(unit.Id, preset.Swap));
        else
            Commands.TryEnqueue(new AutoCommand(unit.Id, preset.Kind, unit.DurationMs(preset.DurationFrames), preset.Swap));
    }

    private TransitionPreset TbarPreset()
    {
        var list = SelectedUnit.Transitions;
        if (list.Count == 0)
            return new TransitionPreset { Kind = MixerNative.TransitionCut, Swap = true };
        var index = Math.Clamp(_tbarPresetIndex, 0, list.Count - 1);
        return list[index];
    }

    private void TBar_ValueChanged(object sender, RoutedPropertyChangedEventArgs<double> e)
    {
        if (_tbarLatching)
            return;
        if (_tbarLocked)
        {
            if (e.NewValue < 1)
            {
                _tbarLatching = true;
                TBar.Value = 1;
                _tbarLatching = false;
            }
            return;
        }
        var mix = (float)e.NewValue;
        if (mix >= 0.999f)
        {
            _tbarLocked = true;
            _tbarLatching = true;
            TBar.Value = 1;
            _tbarLatching = false;
            Commands.TryEnqueue(new CutCommand(SelectedUnit.Id, TbarPreset().Swap));
            return;
        }
        Commands.TryEnqueue(new SetMixCommand(SelectedUnit.Id, mix));
    }

    private void TBar_MouseUp(object sender, MouseButtonEventArgs e) => FinishTBar();

    private void TBar_LostMouseCapture(object sender, MouseEventArgs e) => FinishTBar();

    private void FinishTBar()
    {
        if (!_tbarLocked)
            return;
        _tbarLatching = true;
        TBar.Value = 0;
        _tbarLatching = false;
        _tbarLocked = false;
    }

    private void AddTransition_Click(object sender, RoutedEventArgs e)
    {
        SelectedUnit.Transitions.Add(new TransitionPreset());
        _tbarPresetIndex = SelectedUnit.Transitions.Count - 1;
        _transitionExpanded.Add(_tbarPresetIndex);
        RebuildTransitions();
    }

    private void RebuildTransitions()
    {
        TransitionPanel.Children.Clear();
        var unit = SelectedUnit;
        var keep = _transitionExpanded.Where(i => i < unit.Transitions.Count).ToHashSet();
        _transitionExpanded.Clear();
        foreach (var i in keep)
            _transitionExpanded.Add(i);
        for (var i = 0; i < unit.Transitions.Count; i++)
        {
            var index = i;
            var preset = unit.Transitions[i];
            var selected = index == _tbarPresetIndex;
            var row = new DockPanel { Margin = new Thickness(0, 0, 0, 4) };
            var fire = new Button { Content = "TAKE", Width = 48, Height = 22, Margin = new Thickness(4, 0, 0, 0), FontSize = 11 };
            fire.Click += (_, e) =>
            {
                e.Handled = true;
                _tbarPresetIndex = index;
                FirePreset(preset);
                RebuildTransitions();
            };
            DockPanel.SetDock(fire, Dock.Right);
            row.Children.Add(fire);

            var expander = new Expander
            {
                IsExpanded = _transitionExpanded.Contains(index),
                Foreground = System.Windows.Media.Brushes.White,
                Background = new System.Windows.Media.SolidColorBrush(System.Windows.Media.Color.FromRgb(0x22, 0x22, 0x22)),
                BorderBrush = selected
                    ? new System.Windows.Media.SolidColorBrush(System.Windows.Media.Color.FromRgb(0xE8, 0x77, 0x22))
                    : new System.Windows.Media.SolidColorBrush(System.Windows.Media.Color.FromRgb(0x44, 0x44, 0x44)),
                BorderThickness = new Thickness(1),
                Padding = new Thickness(2),
                Header = $"{preset.Label}  {preset.DurationFrames}f"
            };

            expander.Expanded += (_, _) => _transitionExpanded.Add(index);
            expander.Collapsed += (_, _) => _transitionExpanded.Remove(index);

            var stack = new StackPanel { Margin = new Thickness(0, 6, 0, 0) };
            var kind = new ComboBox { Margin = new Thickness(0, 0, 0, 4) };
            kind.Items.Add(new ComboBoxItem { Content = "Cut", Tag = MixerNative.TransitionCut });
            kind.Items.Add(new ComboBoxItem { Content = "Fade", Tag = MixerNative.TransitionFade });
            kind.Items.Add(new ComboBoxItem { Content = "Dip", Tag = MixerNative.TransitionDip });
            kind.SelectedIndex = preset.Kind switch { 0 => 0, 2 => 2, _ => 1 };
            kind.SelectionChanged += (_, _) =>
            {
                if (kind.SelectedItem is ComboBoxItem item && item.Tag is uint value)
                    preset.Kind = value;
                RebuildTransitions();
            };
            var duration = new TextBox { Text = preset.DurationFrames.ToString(), Margin = new Thickness(0, 0, 0, 4) };
            duration.TextChanged += (_, _) =>
            {
                if (uint.TryParse(duration.Text, out var frames) && frames > 0)
                    preset.DurationFrames = frames;
            };
            duration.LostFocus += (_, _) => RebuildTransitions();
            var swap = new CheckBox { Content = "Swap", IsChecked = preset.Swap, Foreground = System.Windows.Media.Brushes.White, Margin = new Thickness(0, 0, 0, 4) };
            swap.Checked += (_, _) => preset.Swap = true;
            swap.Unchecked += (_, _) => preset.Swap = false;
            var tbar = new Button { Content = "Use for T-bar", Height = 24 };
            tbar.Click += (_, _) =>
            {
                _tbarPresetIndex = index;
                RebuildTransitions();
            };
            var remove = new Button { Content = "−", Height = 22, Margin = new Thickness(0, 4, 0, 0) };
            remove.Click += (_, _) =>
            {
                unit.Transitions.RemoveAt(index);
                _transitionExpanded.Remove(index);
                _tbarPresetIndex = Math.Clamp(_tbarPresetIndex, 0, Math.Max(0, unit.Transitions.Count - 1));
                RebuildTransitions();
            };
            stack.Children.Add(kind);
            stack.Children.Add(new TextBlock { Text = "Duration (frames)", FontSize = 11, Foreground = System.Windows.Media.Brushes.Silver });
            stack.Children.Add(duration);
            stack.Children.Add(swap);
            stack.Children.Add(tbar);
            stack.Children.Add(remove);
            expander.Content = stack;
            expander.MouseLeftButtonDown += (_, _) =>
            {
                if (_tbarPresetIndex == index)
                    return;
                _tbarPresetIndex = index;
                RebuildTransitions();
            };
            row.Children.Add(expander);
            TransitionPanel.Children.Add(row);
        }
    }

    internal void RebuildOverlayToggles()
    {
        OverlayTogglePanel.Children.Clear();
        var unit = SelectedUnit;
        if (unit.Overlays.Count == 0)
            return;
        OverlayTogglePanel.Children.Add(new Border
        {
            Width = 1,
            Margin = new Thickness(4, 4, 12, 4),
            Background = new System.Windows.Media.SolidColorBrush(System.Windows.Media.Color.FromRgb(0x33, 0x33, 0x33))
        });
        for (var i = 0; i < unit.Overlays.Count; i++)
        {
            var slot = unit.Overlays[i];
            var name = _session.Scenes.FirstOrDefault(item => item.GpuId == slot.SceneGpuId)?.Name ?? $"{i + 1}";
            OverlayTogglePanel.Children.Add(new OverlayStrip(name, slot.Enabled, enabled =>
            {
                slot.Enabled = enabled;
                PushAuxFor(unit);
            }));
        }
    }

    internal void PushAuxFor(MixingUnitEntry unit) =>
        Commands.TryEnqueue(new PatchAuxCommand(unit.Id, unit));

    private void OpenOverlay_Click(object sender, RoutedEventArgs e)
    {
        if (_overlay is not null)
        {
            _overlay.Reload(SelectedUnit);
            _overlay.Activate();
            return;
        }
        _overlay = new OverlayWindow(_session, SelectedUnit) { Owner = this };
        _overlay.Closed += (_, _) =>
        {
            _overlay = null;
            RebuildOverlayToggles();
        };
        _overlay.Show();
    }

    private void OpenMultiview_Click(object sender, RoutedEventArgs e) =>
        OpenNewMultiview(_session.Settings.DefaultMultiviewUnitId);

    internal void OpenNewMultiview(ulong unitId)
    {
        var unit = _session.Units.FirstOrDefault(item => item.Id == unitId) ?? SelectedUnit;
        var layout = _session.AddMultiview();
        layout.PreviewUnitId = unit.Id;
        layout.ProgramUnitId = unit.Id;
        layout.EnsureTiles();
        Commands.PushMultiviewNow(layout, unit.Width, unit.Height);
        OpenMultiviewWindow(layout);
    }

    internal void OpenMultiviewWindow(MultiviewLayout layout)
    {
        var existing = _multiviews.FirstOrDefault(item => item.LayoutId == layout.Id);
        if (existing is not null)
        {
            existing.Activate();
            return;
        }
        var unit = SelectedUnit;
        Commands.PushMultiviewNow(layout, unit.Width, unit.Height);
        var window = new MultiviewWindow(_session, layout) { Owner = this };
        window.Closed += (_, _) => _multiviews.Remove(window);
        _multiviews.Add(window);
        window.Show();
    }

    internal void OpenMultiviewFor(ulong unitId) => OpenNewMultiview(unitId);

    internal void CloseMultiview(ulong layoutId)
    {
        foreach (var window in _multiviews.Where(item => item.LayoutId == layoutId).ToArray())
            window.Close();
    }

    private void RebuildMeters()
    {
        MeterPanel.Children.Clear();
        _meters.Clear();
        AddMeter(0, "Master");
        foreach (var input in _session.Inputs)
            AddMeter(input.Id, input.Name);
    }

    private void AddMeter(ulong id, string name)
    {
        var strip = new MeterStrip(name);
        _meters[id] = strip;
        MeterPanel.Children.Add(strip);
    }

    private void TickMeters()
    {
        var peaks = new Dictionary<ulong, (float L, float R)>();
        var buffer = new AudioPeak[32];
        unsafe
        {
            fixed (AudioPeak* ptr = buffer)
            {
                var n = MixerNative.CopyAudioPeaks(ptr, (uint)buffer.Length);
                for (var i = 0; i < n && i < buffer.Length; i++)
                    peaks[buffer[i].SourceId] = (buffer[i].Left, buffer[i].Right);
            }
        }
        foreach (var (id, strip) in _meters)
        {
            if (id == 0)
            {
                float left = 0, right = 0;
                foreach (var peak in peaks.Values)
                {
                    left = Math.Max(left, peak.L);
                    right = Math.Max(right, peak.R);
                }
                strip.SetLevels(left, right);
                continue;
            }
            if (peaks.TryGetValue(id, out var pair))
                strip.SetLevels(pair.L, pair.R);
            else
                strip.Decay();
        }
        _resources.Sample();
        ResourceText.Text = _resources.Line();
        WarnText.Text = _resources.Warning() ?? "";
        TickVideo();
    }

    private void TickVideo()
    {
        if (InputList.SelectedItem is not InputEntry { Kind: InputKind.Video } input)
        {
            VideoBar.Visibility = Visibility.Collapsed;
            return;
        }
        if (!TryVideoInfo(input.Id, out var info) || info.IsFile == 0)
        {
            VideoBar.Visibility = Visibility.Collapsed;
            return;
        }
        VideoBar.Visibility = Visibility.Visible;
        VideoTitle.Text = input.Name;
        var duration = info.DurationHns;
        var position = Math.Max(0, info.PositionHns);
        if (duration > 0)
        {
            position = Math.Min(position, duration);
            VideoSeek.IsEnabled = true;
            VideoTimeText.Text = $"{FormatHns(position)} / {FormatHns(duration - position)} / {FormatHns(duration)}";
            if (!_videoSeeking)
            {
                _videoSeekSuppress = true;
                VideoSeek.Value = position / (double)duration;
                _videoSeekSuppress = false;
            }
        }
        else
        {
            VideoSeek.IsEnabled = false;
            VideoTimeText.Text = $"{FormatHns(position)} / -- / --";
            if (!_videoSeeking)
            {
                _videoSeekSuppress = true;
                VideoSeek.Value = 0;
                _videoSeekSuppress = false;
            }
        }
        VideoPlay.Content = info.Playing != 0 ? "❚❚" : "▶";
    }

    private static bool TryVideoInfo(ulong id, out MixerVideoInfo info) =>
        MixerNative.TryCopyVideoInfo(id, out info);

    private ulong? SelectedVideoId() =>
        InputList.SelectedItem is InputEntry { Kind: InputKind.Video } input
            ? input.Id
            : null;

    private void InputList_SelectionChanged(object sender, SelectionChangedEventArgs e) => TickVideo();

    private static string FormatHns(long hns)
    {
        var time = TimeSpan.FromTicks(Math.Max(0, hns));
        return time.ToString(time.TotalHours >= 1 ? @"h\:mm\:ss" : @"mm\:ss");
    }

    private void VideoPlay_Click(object sender, RoutedEventArgs e)
    {
        if (SelectedVideoId() is not ulong id || !TryVideoInfo(id, out var info))
            return;
        MixerNative.VideoSetPlaying(id, info.Playing == 0 ? 1u : 0u);
        TickVideo();
    }

    private void VideoRestart_Click(object sender, RoutedEventArgs e)
    {
        if (SelectedVideoId() is not ulong id)
            return;
        MixerNative.VideoSeek(id, 0);
        MixerNative.VideoSetPlaying(id, 1);
        TickVideo();
    }

    private void VideoSeek_DragStarted(object sender, DragStartedEventArgs e) => _videoSeeking = true;

    private void VideoSeek_DragCompleted(object sender, DragCompletedEventArgs e)
    {
        SeekFromSlider();
        _videoSeeking = false;
    }

    private void VideoSeek_ClickSeek(object sender, MouseButtonEventArgs e)
    {
        if (_videoSeeking)
            return;
        SeekFromSlider();
    }

    private void VideoSeek_ValueChanged(object sender, RoutedPropertyChangedEventArgs<double> e)
    {
        if (_videoSeekSuppress)
            return;
        if (SelectedVideoId() is not ulong id || !TryVideoInfo(id, out var info) || info.DurationHns <= 0)
            return;
        var duration = info.DurationHns;
        VideoTimeText.Text = $"{FormatHns((long)(e.NewValue * duration))} / {FormatHns((long)((1 - e.NewValue) * duration))} / {FormatHns(duration)}";
        if (!_videoSeeking)
            MixerNative.VideoSeek(id, (long)(Math.Clamp(e.NewValue, 0, 1) * duration));
    }

    private void SeekFromSlider()
    {
        if (SelectedVideoId() is ulong id && TryVideoInfo(id, out var info) && info.DurationHns > 0)
            MixerNative.VideoSeek(id, (long)(Math.Clamp(VideoSeek.Value, 0, 1) * info.DurationHns));
    }

    private void ApplyAspect()
    {
        var unit = SelectedUnit;
        PreviewAspect.RatioWidth = unit.Width;
        PreviewAspect.RatioHeight = unit.Height;
        ProgramAspect.RatioWidth = unit.Width;
        ProgramAspect.RatioHeight = unit.Height;
    }

    private void Resources_Click(object sender, RoutedEventArgs e) => OpenResources();

    private void ResourceHud_MouseUp(object sender, MouseButtonEventArgs e) => OpenResources();

    private void OpenResources()
    {
        if (_resourcesWindow is not null)
        {
            _resourcesWindow.Activate();
            return;
        }
        _resourcesWindow = new ResourceMonitorWindow { Owner = this };
        _resourcesWindow.Closed += (_, _) => _resourcesWindow = null;
        _resourcesWindow.Show();
    }

    private void AddInput_Click(object sender, RoutedEventArgs e)
    {
        var dialog = new AddInputWindow { Owner = this };
        if (dialog.ShowDialog() != true)
            return;
        if (dialog.Kind is not (InputKind.Color or InputKind.Bars) && dialog.ResultPath is null)
            return;
        var id = _session.NextInputId++;
        var name = dialog.ResultName ?? $"Input {id}";
        var input = new InputEntry
        {
            Id = id,
            Name = name,
            Kind = dialog.Kind,
            PathOrAddress = dialog.ResultPath,
            ColorR = dialog.ColorR,
            ColorG = dialog.ColorG,
            ColorB = dialog.ColorB,
            Scroll = dialog.Scroll
        };
        try
        {
            switch (dialog.Kind)
            {
                case InputKind.Color:
                case InputKind.Bars:
                    Commands.TryEnqueue(new DefineGeneratorCommand(
                        id,
                        dialog.Kind == InputKind.Bars ? MixerNative.GenBars : MixerNative.GenSolid,
                        dialog.ColorR,
                        dialog.ColorG,
                        dialog.ColorB,
                        dialog.Scroll));
                    break;
                case InputKind.Still:
                    Commands.TryEnqueue(new LoadStillCommand(id, dialog.ResultPath!));
                    break;
                case InputKind.Video:
                    Commands.TryEnqueue(new StartVideoCommand(id, dialog.ResultPath!));
                    break;
                case InputKind.Omt:
                    Commands.TryEnqueue(new ConnectOmtCommand(id, dialog.ResultPath!));
                    break;
                case InputKind.Uvc:
                    Commands.TryEnqueue(new StartUvcCommand(id, dialog.ResultPath!));
                    break;
                default:
                    throw new InvalidOperationException($"{dialog.Kind} is not available.");
            }
        }
        catch (Exception ex)
        {
            MessageBox.Show(this, ex.Message, "Add Input");
            return;
        }
        _session.Inputs.Add(input);
        InputList.Items.Refresh();
        RebuildMeters();
    }

    private void RemoveInput_Click(object sender, RoutedEventArgs e)
    {
        if (InputList.SelectedItem is not InputEntry input)
        {
            MessageBox.Show(this, "Select an Input to delete.");
            return;
        }
        if (input.Id is MixerNative.Color or MixerNative.Bars or MixerNative.Black or MixerNative.Blue)
        {
            MessageBox.Show(this, "Built-in generators cannot be deleted.");
            return;
        }
        Commands.TryEnqueue(new DropSourceCommand(input.Id));
        WaveOutMonitor.Remove(input.Id);
        foreach (var scene in _session.Scenes)
            scene.Layers.RemoveAll(layer => layer.InputId == input.Id);
        foreach (var unit in _session.Units)
        {
            foreach (var tile in unit.MultiviewTiles)
            {
                if (tile.Kind == MvSlotKind.Input && tile.SourceId == input.Id)
                {
                    tile.Kind = MvSlotKind.None;
                    tile.SourceId = 0;
                }
            }
            Commands.TryEnqueue(new PatchAuxCommand(unit.Id, unit));
        }
        foreach (var scene in _session.Scenes)
            Commands.TryEnqueue(new DefineSceneCommand(scene, SceneWidth, SceneHeight));
        _session.Inputs.Remove(input);
        InputList.Items.Refresh();
        RebuildMeters();
        RebuildScenes();
        _overlay?.Reload(SelectedUnit);
        RebuildOverlayToggles();
        TickVideo();
    }

    private void AddScene_Click(object sender, RoutedEventArgs e)
    {
        var scene = _session.AddScene($"Scene {_session.NextSceneId}");
        Commands.TryEnqueue(new DefineSceneCommand(scene, SceneWidth, SceneHeight));
        RebuildScenes();
        SelectScene(scene);
        OpenSceneEditor(scene);
    }

    private void RemoveScene_Click(object sender, RoutedEventArgs e)
    {
        if (_selectedScene is null)
        {
            MessageBox.Show(this, "Select a Scene to delete.");
            return;
        }
        if (_session.Scenes.Count <= 1)
        {
            MessageBox.Show(this, "At least one Scene is required.");
            return;
        }
        var removed = _selectedScene;
        Commands.TryEnqueue(new DestroySceneCommand(removed.GpuId));
        _session.Scenes.Remove(removed);
        foreach (var unit in _session.Units)
        {
            unit.Overlays.RemoveAll(slot => slot.SceneGpuId == removed.GpuId);
            foreach (var tile in unit.MultiviewTiles)
            {
                if (tile.Kind == MvSlotKind.Scene && tile.SourceId == removed.GpuId)
                {
                    tile.Kind = MvSlotKind.None;
                    tile.SourceId = 0;
                }
            }
            Commands.TryEnqueue(new PatchAuxCommand(unit.Id, unit));
        }
        var fallback = _session.Scenes[0];
        unsafe
        {
            UnitState state = default;
            if (MixerNative.GetUnitState(SelectedUnit.Id, &state) == 0)
            {
                if (state.ProgramSource == removed.GpuId)
                    state.ProgramSource = fallback.GpuId;
                if (state.PreviewSource == removed.GpuId)
                    state.PreviewSource = fallback.GpuId;
                MixerNative.SetUnitState(SelectedUnit.Id, &state);
            }
        }
        RebuildScenes();
        SelectScene(fallback);
        _overlay?.Reload(SelectedUnit);
        RebuildOverlayToggles();
    }

    private void EditScene_Click(object sender, RoutedEventArgs e)
    {
        if (_selectedScene is not null)
            OpenSceneEditor(_selectedScene);
    }

    private void OpenSceneEditor(SceneEntry scene)
    {
        var monitorId = _session.NextMonitorId++;
        var dialog = new SceneEditorWindow(scene, _session, SceneWidth, SceneHeight, monitorId) { Owner = this };
        dialog.ShowDialog();
        RebuildScenes();
        SelectScene(scene);
    }

    private void UnitBox_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (_suppressUnitChange || UnitBox.SelectedItem is not MixingUnitEntry unit)
            return;
        _session.SelectedUnitId = unit.Id;
        PreviewHost.RetargetUnit(unit.Id, MixerNative.OutputPreview);
        ProgramHost.RetargetUnit(unit.Id, MixerNative.OutputProgram);
        ApplyAspect();
        _overlay?.Reload(unit);
        _tbarPresetIndex = 0;
        RebuildTransitions();
        RebuildOverlayToggles();
        UpdateStatus();
    }

    private void AddUnit_Click(object sender, RoutedEventArgs e)
    {
        var draft = new MixingUnitEntry
        {
            Id = _session.NextUnitId,
            Name = $"Mixing Unit {_session.NextUnitId}",
            Width = _session.Settings.DefaultWidth,
            Height = _session.Settings.DefaultHeight,
            FpsNum = _session.Settings.MasterFpsNum,
            FpsDen = _session.Settings.MasterFpsDen
        };
        draft.EnsureDefaultTransitions();
        var dialog = new MixingUnitWindow(draft) { Owner = this };
        if (dialog.ShowDialog() != true)
            return;
        var unit = dialog.Result;
        unit.Id = _session.NextUnitId++;
        unit.EnsureDefaultTransitions();
        unit.EnsureDefaultTiles();
        MixerNative.ThrowIfFailed(MixerNative.CreateUnit(unit.Id, unit.Width, unit.Height), "Create Mixing Unit");
        MixerNative.ThrowIfFailed(
            MixerNative.ConfigureUnit(unit.Id, unit.Width, unit.Height, unit.FpsNum, unit.FpsDen),
            "Configure Mixing Unit");
        var preview = _session.Scenes.Count > 0 ? _session.Scenes[0].GpuId : MixerNative.Bars;
        var program = _session.Scenes.Count > 1 ? _session.Scenes[1].GpuId : preview;
        Commands.PushUnitStateNow(unit.Id, CommandQueue.BuildState(unit, program, preview, 0, MixerNative.TransitionFade));
        _session.Units.Add(unit);
        UnitBox.Items.Refresh();
        UnitBox.SelectedItem = unit;
    }

    private void EditUnit_Click(object sender, RoutedEventArgs e)
    {
        var unit = SelectedUnit;
        var dialog = new MixingUnitWindow(unit) { Owner = this };
        if (dialog.ShowDialog() != true)
            return;
        unit.Name = dialog.Result.Name;
        unit.Width = dialog.Result.Width;
        unit.Height = dialog.Result.Height;
        unit.FpsNum = dialog.Result.FpsNum;
        unit.FpsDen = dialog.Result.FpsDen;
        MixerNative.ThrowIfFailed(
            MixerNative.ConfigureUnit(unit.Id, unit.Width, unit.Height, unit.FpsNum, unit.FpsDen),
            "Configure Mixing Unit");
        foreach (var scene in _session.Scenes)
            Commands.TryEnqueue(new DefineSceneCommand(scene, unit.Width, unit.Height));
        foreach (var layout in _session.Multiviews)
            Commands.PushMultiviewNow(layout, unit.Width, unit.Height);
        UnitBox.Items.Refresh();
        ApplyAspect();
        UpdateStatus();
    }

    private void DeleteUnit_Click(object sender, RoutedEventArgs e)
    {
        if (_session.Units.Count <= 1)
        {
            MessageBox.Show(this, "At least one Mixing Unit is required.");
            return;
        }
        var unit = SelectedUnit;
        foreach (var output in _session.Outputs.Where(item => item.UnitId == unit.Id).ToArray())
        {
            Commands.TryEnqueue(new RemoveOutputCommand(output.Id));
            _session.Outputs.Remove(output);
        }
        MixerNative.ThrowIfFailed(MixerNative.DestroyUnit(unit.Id), "Delete Mixing Unit");
        _session.Units.Remove(unit);
        UnitBox.Items.Refresh();
        UnitBox.SelectedIndex = 0;
    }

    private void SaveSession_Click(object sender, RoutedEventArgs e)
    {
        var dialog = new Microsoft.Win32.SaveFileDialog
        {
            Filter = "eiviz session|*.eiviz.json|JSON|*.json",
            FileName = string.IsNullOrEmpty(_session.Settings.LastSessionPath)
                ? "session.eiviz.json"
                : System.IO.Path.GetFileName(_session.Settings.LastSessionPath)
        };
        if (dialog.ShowDialog(this) != true)
            return;
        SessionStore.Save(_session, dialog.FileName);
        UpdateStatus();
    }

    private void LoadSession_Click(object sender, RoutedEventArgs e)
    {
        var dialog = new Microsoft.Win32.OpenFileDialog { Filter = "eiviz session|*.eiviz.json;*.json|JSON|*.json" };
        if (dialog.ShowDialog(this) != true)
            return;
        try
        {
            _overlay?.Close();
            foreach (var window in _multiviews.ToArray())
                window.Close();
            PreviewHost.ReleaseNative();
            ProgramHost.ReleaseNative();
            ScenePanel.Children.Clear();
            ((App)Application.Current).ReplaceSession(SessionStore.Load(dialog.FileName));
            InputList.ItemsSource = _session.Inputs;
            UnitBox.ItemsSource = _session.Units;
            _suppressUnitChange = true;
            UnitBox.SelectedIndex = 0;
            _suppressUnitChange = false;
            RebuildScenes();
            RebuildTransitions();
            RebuildOverlayToggles();
            RebuildMeters();
            if (_session.Scenes.Count > 0)
                SelectScene(_session.Scenes[0]);
            PreviewHost.RetargetUnit(SelectedUnit.Id, MixerNative.OutputPreview);
            ProgramHost.RetargetUnit(SelectedUnit.Id, MixerNative.OutputProgram);
            ApplyAspect();
            UpdateStatus();
        }
        catch (Exception ex)
        {
            MessageBox.Show(this, ex.Message, "Load session");
        }
    }

    private void Settings_Click(object sender, RoutedEventArgs e)
    {
        var dialog = new SettingsWindow(_session) { Owner = this };
        if (dialog.ShowDialog() != true)
            return;
        _session.Settings.MasterFpsNum = dialog.Settings.MasterFpsNum;
        _session.Settings.MasterFpsDen = dialog.Settings.MasterFpsDen;
        _session.Settings.DefaultWidth = dialog.Settings.DefaultWidth;
        _session.Settings.DefaultHeight = dialog.Settings.DefaultHeight;
        _session.Settings.DefaultMultiviewUnitId = dialog.Settings.DefaultMultiviewUnitId;
        _session.Settings.FrameBufferFrames = dialog.Settings.FrameBufferFrames;
        _session.Settings.InternalColorFormat = dialog.Settings.InternalColorFormat;
        MixerNative.ThrowIfFailed(
            MixerNative.SetFrameBuffer(_session.Settings.FrameBufferFrames),
            "Set frame buffer");
        RestartMediaPumps();
        ApplyOutputs(dialog.Outputs);
        UpdateStatus();
    }

    private void RestartMediaPumps()
    {
        MixerNative.VideoFormat = _session.Settings.InternalColorFormat == InternalColorFormat.Bgra
            ? MixerNative.FormatBgra
            : MixerNative.FormatUyvy;
        foreach (var input in _session.Inputs)
        {
            if (string.IsNullOrWhiteSpace(input.PathOrAddress))
                continue;
            if (input.Kind == InputKind.Video)
                Commands.TryEnqueue(new StartVideoCommand(input.Id, input.PathOrAddress));
            else if (input.Kind == InputKind.Uvc)
                Commands.TryEnqueue(new StartUvcCommand(input.Id, input.PathOrAddress));
        }
    }

    private void ApplyOutputs(IReadOnlyList<OutputEntry> outputs)
    {
        var next = outputs.ToList();
        var previous = _session.Outputs.ToList();
        var nextIds = next.Select(item => item.Id).ToHashSet();
        foreach (var existing in previous.Where(item => !nextIds.Contains(item.Id)))
            Commands.TryEnqueue(new RemoveOutputCommand(existing.Id));
        foreach (var output in next)
        {
            var prior = previous.FirstOrDefault(item => item.Id == output.Id);
            if (prior is not null && SameOutput(prior, output))
                continue;
            if (prior is not null)
                Commands.TryEnqueue(new RemoveOutputCommand(output.Id));
            if (output.Transport == OutputTransport.Omt)
            {
                Commands.TryEnqueue(new AddOutputCommand(output));
                continue;
            }
            try
            {
                Commands.AddOutputNow(output);
            }
            catch (Exception ex)
            {
                MessageBox.Show(this, ex.Message, "Output");
            }
        }
        _session.Outputs.Clear();
        foreach (var output in next)
            _session.Outputs.Add(output);
    }

    private static bool SameOutput(OutputEntry left, OutputEntry right) =>
        left.Id == right.Id
        && left.Name == right.Name
        && left.Transport == right.Transport
        && left.SourceKind == right.SourceKind
        && left.SourceId == right.SourceId
        && left.UnitId == right.UnitId;

    private void UpdateStatus()
    {
        var unit = SelectedUnit;
        StatusText.Text = $"{unit.Width}x{unit.Height} {unit.FormatFps()}   Mixing Unit {unit.Id}   Inputs are a list only — TAKE/Preview uses Scenes.";
    }
}
