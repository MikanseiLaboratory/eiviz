using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using Eiviz.Host;
using Eiviz.Host.I18n;
using Eiviz.Host.Interop;
using Eiviz.Host.Preview;

namespace Eiviz.Host.Dialogs;

public partial class SwitcherWindow : Window
{
    private readonly MixingUnitEntry _unit;
    private int _tbarPresetIndex;
    private bool _tbarLatching;
    private bool _tbarLocked;
    private readonly Dictionary<ulong, SceneThumb> _thumbs = [];
    private ListFilter _sceneFilter = ListFilter.All;

    public SwitcherWindow(MixingUnitEntry unit)
    {
        InitializeComponent();
        _unit = unit;
        Title = unit.Name;
        TitleText.Text = unit.Name;
        PreviewAspect.RatioWidth = unit.Width;
        PreviewAspect.RatioHeight = unit.Height;
        ProgramAspect.RatioWidth = unit.Width;
        ProgramAspect.RatioHeight = unit.Height;
        RebuildSceneTabs();
        RebuildScenes();
        RebuildTransitions();
        RebuildOverlays();
        SceneScroll.ScrollChanged += (_, _) => ApplyThumbSubscriptions();
        OnTopBox.IsChecked = unit.AlwaysOnTop;
        ApplyTopmost();
        Loaded += (_, _) =>
        {
            ApplyBusColors();
            RefreshBusTitles();
            RefreshSceneThumbs();
            PreviewHost.RetargetUnit(unit.Id, MixerNative.OutputPreview);
            ProgramHost.RetargetUnit(unit.Id, MixerNative.OutputProgram);
        };
        Closed += (_, _) =>
        {
            PreviewHost.ReleaseNative();
            ProgramHost.ReleaseNative();
            foreach (var thumb in _thumbs.Values)
                thumb.Host.SetWanted(false);
            _thumbs.Clear();
        };
    }

    public ulong UnitId => _unit.Id;

    private void OnTop_Click(object sender, RoutedEventArgs e)
    {
        _unit.AlwaysOnTop = OnTopBox.IsChecked == true;
        ApplyTopmost();
    }

    internal void ApplyTopmost()
    {
        Topmost = _unit.AlwaysOnTop;
        Owner = _unit.AlwaysOnTop ? Application.Current.MainWindow : null;
    }

    private Session Session => ((App)Application.Current).Session;

    private CommandQueue Commands => ((App)Application.Current).Commands;

    internal void ApplyMixerMix() =>
        MainWindow.ApplyTBarFromMixer(_unit.Id, TBar, ref _tbarLatching, ref _tbarLocked);

    internal void SyncFromUnit()
    {
        Title = _unit.Name;
        TitleText.Text = _unit.Name;
        PreviewAspect.RatioWidth = _unit.Width;
        PreviewAspect.RatioHeight = _unit.Height;
        ProgramAspect.RatioWidth = _unit.Width;
        ProgramAspect.RatioHeight = _unit.Height;
        RebuildSceneTabs();
        RebuildScenes();
        RebuildTransitions();
        RebuildOverlays();
        ApplyBusColors();
        RefreshBusTitles();
        RefreshSceneThumbs();
    }

    internal void ApplyBusColors()
    {
        BusTheme.Apply(Session.Settings, PreviewFrame, PreviewHeader, PreviewHeaderText, preview: true);
        BusTheme.Apply(Session.Settings, ProgramFrame, ProgramHeader, ProgramHeaderText, preview: false);
        RebuildTransitions();
        RebuildSceneTabs();
        RefreshSceneThumbs();
    }

    internal void RebuildOverlays()
    {
        OverlayTogglePanel.Children.Clear();
        if (_unit.Overlays.Count == 0)
            return;
        var main = Application.Current.MainWindow as MainWindow;
        for (var i = 0; i < _unit.Overlays.Count; i++)
        {
            var slot = _unit.Overlays[i];
            var name = slot.DisplayName(Session);
            OverlayTogglePanel.Children.Add(new OverlayStrip(name, slot.Enabled, enabled =>
            {
                main?.ToggleOverlay(_unit, slot, enabled);
            }));
        }
    }

    private IEnumerable<SceneEntry> VisibleScenes()
    {
        if (_sceneFilter.Mode == ListFilterMode.Tag
            && (_sceneFilter.Tag is not { } tag || !Session.SceneTags.Contains(tag, StringComparer.Ordinal)))
            _sceneFilter = ListFilter.All;
        return Session.Scenes.Where(scene => _unit.ShowsOnSwitcher(scene) && _sceneFilter.MatchesScene(scene));
    }

    private void RebuildSceneTabs()
    {
        SceneTabBar.Children.Clear();
        AddSceneTab(Loc.T("tag.all"), _sceneFilter.SameAs(ListFilter.All), ListFilter.All);
        foreach (var tag in Session.SceneTags)
        {
            var filter = ListFilter.ForTag(tag);
            AddSceneTab(tag, _sceneFilter.SameAs(filter), filter);
        }
    }

    private void AddSceneTab(string label, bool on, ListFilter filter)
    {
        var button = new Button
        {
            Content = label,
            Height = 20,
            FontSize = 11,
            FontWeight = on ? FontWeights.SemiBold : FontWeights.Normal,
            Margin = new Thickness(1, 0, 1, 2),
            Padding = new Thickness(2, 0, 2, 0),
            Foreground = on ? Brushes.White : Brushes.Silver,
            Background = Brushes.Transparent,
            BorderBrush = on
                ? BusTheme.PreviewBrush(Session.Settings)
                : Brushes.Transparent,
            BorderThickness = new Thickness(0, 0, 0, 2),
            Cursor = Cursors.Hand
        };
        button.Click += (_, _) =>
        {
            _sceneFilter = filter;
            RebuildSceneTabs();
            RebuildScenes();
        };
        SceneTabBar.Children.Add(button);
    }

    private void ManageScenes_Click(object sender, RoutedEventArgs e)
    {
        var dialog = new SwitcherScenesWindow(_unit, Session) { Owner = this };
        if (dialog.ShowDialog() == true)
            RebuildScenes();
    }

    private void OpenOverlay_Click(object sender, RoutedEventArgs e)
    {
        if (Application.Current.MainWindow is MainWindow main)
            main.OpenOverlayFor(_unit);
    }

    private void RebuildScenes()
    {
        var visible = VisibleScenes().ToList();
        var keep = visible.Select(scene => scene.Id).ToHashSet();
        foreach (var id in _thumbs.Keys.Where(id => !keep.Contains(id)).ToList())
        {
            _thumbs[id].Host.SetWanted(false);
            SceneStrip.Children.Remove(_thumbs[id].Chrome);
            _thumbs.Remove(id);
        }

        var interval = Session.Settings.ResolvedPresentInterval();
        var index = 0;
        foreach (var scene in visible)
        {
            if (_thumbs.TryGetValue(scene.Id, out var thumb))
            {
                thumb.Label.Text = scene.Name;
                thumb.CollapsedLabel.Text = scene.Name;
                thumb.Host.Bind(scene.GpuId, 148, 80, interval);
                ApplyCollapsed(thumb, scene);
            }
            else
            {
                thumb = CreateThumb(scene);
                _thumbs[scene.Id] = thumb;
                SceneStrip.Children.Add(thumb.Chrome);
            }
            var current = SceneStrip.Children.IndexOf(thumb.Chrome);
            if (current != index)
            {
                SceneStrip.Children.Remove(thumb.Chrome);
                SceneStrip.Children.Insert(index, thumb.Chrome);
            }
            thumb.Host.SetPresentInterval(interval);
            index++;
        }
        RefreshSceneThumbs();
    }

    private SceneThumb CreateThumb(SceneEntry scene)
    {
        var host = new ThumbView { Height = 80 };
        host.Bind(scene.GpuId, 148, 80, Session.Settings.ResolvedPresentInterval());
        var label = new TextBlock
        {
            Text = scene.Name,
            Foreground = Brushes.White,
            FontSize = 11,
            TextTrimming = TextTrimming.CharacterEllipsis,
            VerticalAlignment = VerticalAlignment.Center
        };
        var titleBar = new Border
        {
            Background = new SolidColorBrush(Color.FromRgb(0x33, 0x33, 0x33)),
            Padding = new Thickness(6, 3, 6, 3)
        };
        titleBar.Child = label;
        var chrome = new Border
        {
            Width = 148,
            Height = 104,
            Margin = new Thickness(0, 0, 8, 8),
            BorderThickness = new Thickness(2),
            BorderBrush = new SolidColorBrush(Color.FromRgb(0x55, 0x55, 0x55)),
            Background = new SolidColorBrush(Color.FromRgb(0x11, 0x11, 0x11)),
            Cursor = Cursors.Hand,
            HorizontalAlignment = HorizontalAlignment.Left,
            VerticalAlignment = VerticalAlignment.Top
        };
        var expanded = new StackPanel();
        expanded.Children.Add(titleBar);
        expanded.Children.Add(host);
        var collapsedLabel = new TextBlock
        {
            Text = scene.Name,
            Foreground = Brushes.White,
            FontSize = 12,
            LayoutTransform = new RotateTransform(-90)
        };
        var collapsed = new Border
        {
            Background = new SolidColorBrush(Color.FromRgb(0x33, 0x33, 0x33)),
            Visibility = Visibility.Collapsed,
            Child = new Viewbox
            {
                Stretch = Stretch.Uniform,
                Margin = new Thickness(4, 8, 4, 8),
                Child = collapsedLabel
            }
        };
        var root = new Grid();
        root.Children.Add(expanded);
        root.Children.Add(collapsed);
        chrome.Child = root;
        chrome.PreviewMouseRightButtonDown += (_, e) =>
        {
            scene.PreviewCollapsed = !scene.PreviewCollapsed;
            if (_thumbs.TryGetValue(scene.Id, out var live))
                ApplyCollapsed(live, scene);
            ApplyThumbSubscriptions();
            if (Application.Current.MainWindow is MainWindow main)
                main.NotifySceneTilesFromSwitcher();
            e.Handled = true;
        };
        chrome.MouseLeftButtonUp += (_, _) => PreviewScene(scene.Id);
        titleBar.MouseLeftButtonDown += (_, e) =>
        {
            if (e.ClickCount >= 2)
            {
                if (Application.Current.MainWindow is MainWindow main)
                    main.EditSceneFromSwitcher(scene);
                e.Handled = true;
            }
        };
        chrome.ContextMenu = ThumbMenu(scene);
        var thumb = new SceneThumb
        {
            SceneId = scene.Id,
            Chrome = chrome,
            Host = host,
            Label = label,
            Expanded = expanded,
            Collapsed = collapsed,
            CollapsedLabel = collapsedLabel
        };
        ApplyCollapsed(thumb, scene);
        return thumb;
    }

    private ContextMenu ThumbMenu(SceneEntry scene)
    {
        var menu = new ContextMenu();
        var hide = new MenuItem { Header = Loc.T("switcher.hideHere") };
        hide.Click += (_, _) =>
        {
            if (_unit.SwitcherSceneFilter == SwitcherSceneFilter.All)
            {
                _unit.SwitcherSceneFilter = SwitcherSceneFilter.Exclude;
                _unit.SwitcherSceneIds.Clear();
            }
            if (_unit.SwitcherSceneFilter == SwitcherSceneFilter.Exclude
                && !_unit.SwitcherSceneIds.Contains(scene.Id))
                _unit.SwitcherSceneIds.Add(scene.Id);
            if (_unit.SwitcherSceneFilter == SwitcherSceneFilter.Include)
                _unit.SwitcherSceneIds.Remove(scene.Id);
            RebuildScenes();
        };
        menu.Items.Add(hide);
        return menu;
    }

    internal void ApplySceneCollapsed()
    {
        foreach (var scene in Session.Scenes)
        {
            if (!_thumbs.TryGetValue(scene.Id, out var thumb))
                continue;
            ApplyCollapsed(thumb, scene);
        }
        ApplyThumbSubscriptions();
    }

    private static void ApplyCollapsed(SceneThumb thumb, SceneEntry scene)
    {
        var collapsed = scene.PreviewCollapsed;
        thumb.Chrome.Width = collapsed ? 32 : 148;
        thumb.Chrome.Height = 104;
        thumb.Expanded.Visibility = collapsed ? Visibility.Collapsed : Visibility.Visible;
        thumb.Collapsed.Visibility = collapsed ? Visibility.Visible : Visibility.Collapsed;
        if (collapsed)
            thumb.Host.SetWanted(false);
    }

    private void PreviewScene(ulong sceneId)
    {
        var scene = Session.Scenes.FirstOrDefault(item => item.Id == sceneId);
        if (scene is null)
            return;
        Commands.TryEnqueue(new PreviewSceneCommand(_unit.Id, scene.GpuId));
        RefreshBusTitles();
        RefreshSceneThumbs();
    }

    private void RefreshSceneThumbs()
    {
        ReadBusSources(out var previewId, out var programId);
        var preview = BusTheme.PreviewBrush(Session.Settings);
        var program = BusTheme.ProgramBrush(Session.Settings);
        var inactive = BusTheme.InactiveBrush(Session.Settings);
        foreach (var scene in Session.Scenes)
        {
            if (!_thumbs.TryGetValue(scene.Id, out var thumb))
                continue;
            thumb.Chrome.BorderBrush = scene.GpuId == programId
                ? program
                : scene.GpuId == previewId
                    ? preview
                    : inactive;
        }
        ApplyThumbSubscriptions();
    }

    private void ApplyThumbSubscriptions()
    {
        ReadBusSources(out var previewId, out var programId);
        foreach (var scene in Session.Scenes)
        {
            if (!_thumbs.TryGetValue(scene.Id, out var thumb))
                continue;
            if (scene.PreviewCollapsed)
            {
                thumb.Host.SetWanted(false);
                continue;
            }
            var pinned = scene.GpuId == programId || scene.GpuId == previewId;
            thumb.Host.SetWanted(pinned || ThumbViewport.Intersects(thumb.Chrome, SceneScroll));
        }
    }

    private void ReadBusSources(out ulong previewId, out ulong programId)
    {
        previewId = 0;
        programId = 0;
        unsafe
        {
            UnitState state = default;
            if (MixerNative.GetUnitState(_unit.Id, &state) == 0)
            {
                previewId = state.PreviewSource;
                programId = state.ProgramSource;
            }
        }
    }

    private void RebuildTransitions()
    {
        TransitionPanel.Children.Clear();
        for (var i = 0; i < _unit.Transitions.Count; i++)
        {
            var index = i;
            var preset = _unit.Transitions[i];
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
            var label = new Border
            {
                BorderBrush = selected
                    ? BusTheme.PreviewBrush(Session.Settings)
                    : new SolidColorBrush(Color.FromRgb(0x44, 0x44, 0x44)),
                BorderThickness = new Thickness(1),
                Background = new SolidColorBrush(Color.FromRgb(0x22, 0x22, 0x22)),
                Padding = new Thickness(6, 4, 6, 4),
                Child = new TextBlock
                {
                    Text = $"{preset.Label}  {preset.DurationValue}{(preset.DurationUnit == MixerNative.DurationMs ? "ms" : "f")}",
                    Foreground = Brushes.White
                }
            };
            label.MouseLeftButtonDown += (_, _) =>
            {
                _tbarPresetIndex = index;
                RebuildTransitions();
            };
            row.Children.Add(label);
            TransitionPanel.Children.Add(row);
        }
    }

    private void FirePreset(TransitionPreset preset)
    {
        if (preset.Kind == MixerNative.TransitionCut || preset.DurationValue <= 1)
            Commands.TryEnqueue(new CutCommand(_unit.Id, preset.Swap));
        else
            Commands.TryEnqueue(preset.ToAuto(_unit.Id, _unit));
        RefreshBusTitles();
        RefreshSceneThumbs();
    }

    private TransitionPreset TbarPreset()
    {
        if (_unit.Transitions.Count == 0)
            return new TransitionPreset { Kind = MixerNative.TransitionCut, Swap = true };
        var index = Math.Clamp(_tbarPresetIndex, 0, _unit.Transitions.Count - 1);
        return _unit.Transitions[index];
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
            Commands.TryEnqueue(new CutCommand(_unit.Id, TbarPreset().Swap));
            return;
        }
        Commands.TryEnqueue(new SetMixCommand(_unit.Id, mix, TbarPreset()));
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

    private void RefreshBusTitles()
    {
        ReadBusSources(out var previewId, out var programId);
        var previewName = Session.Scenes.FirstOrDefault(item => item.GpuId == previewId)?.Name;
        var programName = Session.Scenes.FirstOrDefault(item => item.GpuId == programId)?.Name;
        PreviewHeaderText.Text = string.IsNullOrEmpty(previewName) ? "PREVIEW" : $"PREVIEW — {previewName}";
        ProgramHeaderText.Text = string.IsNullOrEmpty(programName) ? "PROGRAM" : $"PROGRAM — {programName}";
    }

    private sealed class SceneThumb
    {
        public ulong SceneId;
        public Border Chrome = null!;
        public ThumbView Host = null!;
        public TextBlock Label = null!;
        public StackPanel Expanded = null!;
        public Border Collapsed = null!;
        public TextBlock CollapsedLabel = null!;
    }
}
