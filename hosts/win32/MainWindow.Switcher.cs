using System.ComponentModel;
using System.Diagnostics;
using System.IO;
using System.Text.Json;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Data;
using System.Windows.Input;
using System.Windows.Threading;
using Eiviz.Host.Dialogs;
using Eiviz.Host.I18n;
using Eiviz.Host.Interop;
using Eiviz.Host.Media;
using Eiviz.Host.Preview;

namespace Eiviz.Host;

public partial class MainWindow
{
    private void BindInputList()
    {
        _inputView = CollectionViewSource.GetDefaultView(_session.Inputs);
        _inputView.Filter = item => item is InputEntry input && _inputFilter.MatchesInput(input);
        InputList.ItemsSource = _inputView;
    }

    private void RefreshInputList()
    {
        EnsureInputFilter();
        _inputView?.Refresh();
        RebuildInputTabs();
    }

    private void RefreshSceneList()
    {
        EnsureSceneFilter();
        RebuildSceneTabs();
        RebuildScenes();
    }

    private void EnsureInputFilter()
    {
        if (_inputFilter.Mode == ListFilterMode.Tag
            && (_inputFilter.Tag is not { } tag || !_session.InputTags.Contains(tag, StringComparer.Ordinal)))
            _inputFilter = ListFilter.All;
    }

    private void EnsureSceneFilter()
    {
        if (_sceneFilter.Mode == ListFilterMode.Tag
            && (_sceneFilter.Tag is not { } tag || !_session.SceneTags.Contains(tag, StringComparer.Ordinal)))
            _sceneFilter = ListFilter.All;
    }

    private void RebuildInputTabs()
    {
        InputTabBar.Children.Clear();
        AddFilterTab(InputTabBar, Loc.T("tag.all"), _inputFilter.SameAs(ListFilter.All), ListFilter.All, input: true);
        foreach (var tag in _session.InputTags)
        {
            var filter = ListFilter.ForTag(tag);
            AddFilterTab(InputTabBar, tag, _inputFilter.SameAs(filter), filter, input: true);
        }
        foreach (var kind in InputKindNames.TabKinds)
        {
            var filter = ListFilter.ForKind(kind);
            var on = _inputFilter.Mode == ListFilterMode.Kind
                && _inputFilter.Kind is { } selected
                && InputKindNames.SameCategory(selected, kind);
            AddFilterTab(InputTabBar, InputKindNames.Category(kind), on, filter, input: true);
        }
    }

    private void RebuildSceneTabs()
    {
        SceneTabBar.Children.Clear();
        AddFilterTab(SceneTabBar, Loc.T("tag.all"), _sceneFilter.SameAs(ListFilter.All), ListFilter.All, input: false);
        foreach (var tag in _session.SceneTags)
        {
            var filter = ListFilter.ForTag(tag);
            AddFilterTab(SceneTabBar, tag, _sceneFilter.SameAs(filter), filter, input: false);
        }
    }

    private void AddFilterTab(Panel bar, string label, bool on, ListFilter filter, bool input)
    {
        var button = TransitionGroupTab(label, on);
        button.Click += (_, _) => SelectListFilter(filter, input);
        button.MouseRightButtonUp += (_, e) =>
        {
            OpenTagMenu(button, filter, input);
            e.Handled = true;
        };
        bar.Children.Add(button);
    }

    private void SelectListFilter(ListFilter filter, bool input)
    {
        if (input)
        {
            _inputFilter = filter;
            RefreshInputList();
        }
        else
        {
            _sceneFilter = filter;
            RefreshSceneList();
        }
    }

    private void OpenTagMenu(FrameworkElement target, ListFilter filter, bool input)
    {
        var menu = new ContextMenu();
        var add = new MenuItem { Header = Loc.T("tag.add") };
        add.Click += (_, _) => AddCatalogTag(input);
        menu.Items.Add(add);
        if (filter.Mode == ListFilterMode.Tag && filter.Tag is { } tag)
        {
            var rename = new MenuItem { Header = Loc.T("tag.rename") };
            rename.Click += (_, _) => RenameCatalogTag(input, tag);
            var delete = new MenuItem { Header = Loc.T("tag.delete") };
            delete.Click += (_, _) => DeleteCatalogTag(input, tag);
            menu.Items.Add(rename);
            menu.Items.Add(delete);
        }
        menu.PlacementTarget = target;
        menu.IsOpen = true;
    }

    private void AddCatalogTag(bool input)
    {
        if (!TextPromptWindow.TryPrompt(this, Loc.T("tag.add"), Loc.T("tag.name"), "", out var name))
            return;
        var catalog = input ? _session.InputTags : _session.SceneTags;
        if (!TagCatalog.TryAdd(catalog, name, out _))
        {
            MessageBox.Show(this, Loc.T("tag.duplicate"), Loc.T("tag.add"));
            return;
        }
        if (input)
            RefreshInputList();
        else
            RefreshSceneList();
    }

    private void RenameCatalogTag(bool input, string current)
    {
        if (!TextPromptWindow.TryPrompt(this, Loc.T("tag.rename"), Loc.T("tag.name"), current, out var name))
            return;
        var catalog = input ? _session.InputTags : _session.SceneTags;
        var owners = input
            ? _session.Inputs.Select(item => item.Tags)
            : _session.Scenes.Select(item => item.Tags);
        if (!TagCatalog.Rename(catalog, owners, current, name))
        {
            MessageBox.Show(this, Loc.T("tag.duplicate"), Loc.T("tag.rename"));
            return;
        }
        if (input)
        {
            if (_inputFilter.Mode == ListFilterMode.Tag && _inputFilter.Tag == current)
                _inputFilter = ListFilter.ForTag(name);
            RefreshInputList();
        }
        else
        {
            if (_sceneFilter.Mode == ListFilterMode.Tag && _sceneFilter.Tag == current)
                _sceneFilter = ListFilter.ForTag(name);
            RefreshSceneList();
        }
    }

    private void DeleteCatalogTag(bool input, string name)
    {
        var confirm = MessageBox.Show(
            this,
            Loc.Format("tag.deleteConfirm", name),
            Loc.T("tag.delete"),
            MessageBoxButton.YesNo,
            MessageBoxImage.Question);
        if (confirm != MessageBoxResult.Yes)
            return;
        var catalog = input ? _session.InputTags : _session.SceneTags;
        var owners = input
            ? _session.Inputs.Select(item => item.Tags)
            : _session.Scenes.Select(item => item.Tags);
        TagCatalog.Remove(catalog, owners, name);
        if (input)
            RefreshInputList();
        else
            RefreshSceneList();
    }

    private SceneTile CreateSceneTile()
    {
        var tile = new SceneTile();
        tile.SceneSelected += (_, selected) => SelectScene(selected);
        tile.SceneEditRequested += (_, selected) => OpenSceneEditor(selected);
        tile.SceneCutRequested += (_, selected) => CutScene(selected);
        tile.SceneLoopRequested += (_, selected) => ToggleSceneLoop(selected);
        tile.ScenePlayRequested += (_, selected) => ToggleScenePlay(selected);
        tile.SceneAudioRequested += (_, selected) => ToggleSceneAudio(selected);
        tile.ScenePreviewRequested += (_, selected) => OpenSourcePreview(selected.GpuId, selected.Name);
        tile.SceneCloseRequested += (_, selected) => DeleteScene(selected);
        tile.SceneCollapseToggled += (_, _) =>
        {
            tile.ApplyCollapsed();
            ApplySceneTileThumbs();
            NotifySwitcherCollapsed();
        };
        tile.SceneSnapshotRequested += (_, selected) => SnapshotScene(selected);
        return tile;
    }

    private void RebuildScenes()
    {
        var existing = ScenePanel.Children.OfType<SceneTile>().ToList();
        var byId = existing
            .Where(tile => tile.Scene is not null)
            .ToDictionary(tile => tile.Scene!.Id);
        var visible = _session.Scenes.Where(_sceneFilter.MatchesScene).ToList();
        var keep = visible.Select(scene => scene.Id).ToHashSet();
        foreach (var tile in existing)
        {
            if (tile.Scene is not { } scene || !keep.Contains(scene.Id))
                ScenePanel.Children.Remove(tile);
        }

        var interval = _session.Settings.ResolvedPresentInterval();
        var preview = BusTheme.Preview(_session.Settings);
        var inactive = BusTheme.Inactive(_session.Settings);
        foreach (var scene in visible)
        {
            var index = _session.Scenes.IndexOf(scene) + 1;
            var selected = _selectedScene?.Id == scene.Id;
            if (byId.TryGetValue(scene.Id, out var tile) && ScenePanel.Children.Contains(tile))
                tile.Bind(scene, index, selected, interval, preview, inactive);
            else
            {
                tile = CreateSceneTile();
                tile.Bind(scene, index, selected, interval, preview, inactive);
                ScenePanel.Children.Add(tile);
            }
        }
        RefreshSceneTiles();
        NotifySwitchers();
        ApplyOnAirLock();
    }

    internal void NotifySceneTilesFromSwitcher()
    {
        foreach (SceneTile tile in ScenePanel.Children)
            tile.ApplyCollapsed();
        ApplySceneTileThumbs();
    }

    private void NotifySwitcherCollapsed()
    {
        foreach (var switcher in _switchers.Values)
            switcher.ApplySceneCollapsed();
    }

    private void NotifySwitchers()
    {
        foreach (var switcher in _switchers.Values)
            switcher.SyncFromUnit();
    }

    private void PushScenePresentIntervals()
    {
        var interval = _session.Settings.ResolvedPresentInterval();
        foreach (SceneTile tile in ScenePanel.Children)
            tile.SetPresentInterval(interval);
    }

    private void RefreshSceneTiles()
    {
        var programId = CurrentProgramSceneId();
        if (programId != 0)
            _lastProgramId = programId;
        else
            programId = _lastProgramId;
        _shownProgramId = programId;
        var previewGpuId = CurrentPreviewSceneGpuId();
        var previewScene = _session.Scenes.FirstOrDefault(item => item.GpuId == previewGpuId);
        _shownPreviewId = previewScene?.Id ?? 0;
        var programName = _session.Scenes.FirstOrDefault(item => item.GpuId == programId)?.Name;
        var previewName = previewScene?.Name;
        PreviewHeaderText.Text = string.IsNullOrEmpty(previewName) ? Loc.T("chrome.preview") : $"{Loc.T("chrome.preview")} — {previewName}";
        ProgramHeaderText.Text = string.IsNullOrEmpty(programName) ? Loc.T("chrome.program") : $"{Loc.T("chrome.program")} — {programName}";
        foreach (SceneTile tile in ScenePanel.Children)
        {
            if (tile.Scene is not { } scene)
                continue;
            RefreshSceneTransport(tile, scene);
            tile.SetBusRoles(
                scene.GpuId == previewGpuId && scene.GpuId != programId,
                scene.GpuId == programId,
                BusTheme.Preview(_session.Settings),
                BusTheme.Program(_session.Settings),
                BusTheme.Inactive(_session.Settings));
        }
        ApplySceneTileThumbs();
    }

    private void ApplySceneTileThumbs()
    {
        var tiles = ScenePanel.Children.OfType<SceneTile>().Where(tile => tile.Scene is not null);
        var selectedId = _selectedScene?.Id ?? 0;
        foreach (var tile in tiles)
        {
            var scene = tile.Scene!;
            tile.ApplyCollapsed();
            if (scene.PreviewCollapsed)
            {
                tile.SetThumbWanted(false);
                continue;
            }
            var pinned = scene.Id == _shownProgramId
                || scene.Id == _shownPreviewId
                || scene.Id == selectedId;
            tile.SetThumbWanted(pinned || ThumbViewport.Intersects(tile, SceneScroll));
        }
    }

    private void RefreshSceneTransport(SceneTile tile, SceneEntry scene)
    {
        var video = SceneVideo(scene);
        var playing = false;
        if (video is not null && TryVideoInfo(video.Id, out var info))
            playing = info.Playing != 0;
        tile.SetTransport(
            video is not null,
            video?.VideoLoop == true,
            playing,
            SceneInputs(scene).All(item => item.Mute));
    }

    private ulong CurrentProgramSceneId()
    {
        if (Application.Current is App app)
        {
            app.Backend.BusSources(SelectedUnit.Id, out _, out var program);
            return program;
        }
        return 0;
    }

    private ulong CurrentPreviewSceneGpuId()
    {
        if (Application.Current is App app)
        {
            app.Backend.BusSources(SelectedUnit.Id, out var preview, out _);
            return preview;
        }
        return 0;
    }

    private void SyncSelectedSceneFromMixer()
    {
        var previewGpuId = CurrentPreviewSceneGpuId();
        if (_session.Scenes.FirstOrDefault(item => item.GpuId == previewGpuId) is { } scene)
            _selectedScene = scene;
    }

    private InputEntry? SceneVideo(SceneEntry scene) =>
        SceneInputs(scene).FirstOrDefault(item => item.Kind == InputKind.Video);

    private IEnumerable<InputEntry> SceneInputs(SceneEntry scene) =>
        scene.Layers
            .Select(layer => _session.Inputs.FirstOrDefault(item => item.Id == layer.InputId))
            .OfType<InputEntry>();

    private void CutScene(SceneEntry scene)
    {
        SelectScene(scene);
        FirePreset(new TransitionPreset { Kind = MixerNative.TransitionCut, DurationValue = 1, Swap = true });
    }

    private void ToggleSceneLoop(SceneEntry scene)
    {
        if (SceneVideo(scene) is not { } video)
            return;
        video.VideoLoop = !video.VideoLoop;
        ((App)Application.Current).Backend.VideoLoop(video.Id, video.VideoLoop);
        RefreshSceneTiles();
    }

    private void ToggleScenePlay(SceneEntry scene)
    {
        if (SceneVideo(scene) is not { } video)
            return;
        if (App.IsRemote)
        {
            ((App)Application.Current).Backend.VideoPlay(video.Id, true);
            RefreshSceneTiles();
            return;
        }
        if (!TryVideoInfo(video.Id, out var info))
            return;
        MixerNative.VideoSetPlaying(video.Id, info.Playing == 0 ? 1u : 0u);
        RefreshSceneTiles();
    }

    private void ToggleSceneAudio(SceneEntry scene)
    {
        var inputs = SceneInputs(scene).ToList();
        if (inputs.Count == 0)
            return;
        var mute = !inputs.All(item => item.Mute);
        foreach (var input in inputs)
        {
            input.Mute = mute;
            ((App)Application.Current).Backend.SetInputGain(
                input.Id,
                input.BusMask == 0 ? 1u : input.BusMask,
                MixerNative.MixerGain(input.Gain),
                mute);
        }
        RebuildMeters();
        RefreshSceneTiles();
    }

    private void SelectScene(SceneEntry scene)
    {
        _selectedScene = scene;
        MixerApply.PreviewScene(SelectedUnit.Id, scene.GpuId);
        RefreshSceneTiles();
    }

    private void FirePreset(TransitionPreset preset)
    {
        var unit = SelectedUnit;
        if (preset.Kind == MixerNative.TransitionCut || preset.DurationValue <= 1)
            MixerApply.Cut(unit.Id, preset.Swap);
        else
            preset.ApplyAuto(unit.Id, unit);
        RefreshSceneTiles();
    }

    private TransitionPreset TbarPreset()
    {
        var list = SelectedUnit.Transitions;
        if (list.Count == 0)
            return new TransitionPreset { Kind = MixerNative.TransitionCut, Swap = true };
        var index = Math.Clamp(_tbarPresetIndex, 0, list.Count - 1);
        var preset = list[index];
        TransitionCatalog.ApplyKindDefaults(preset);
        return preset;
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
            MixerApply.Cut(SelectedUnit.Id, TbarPreset().Swap);
            return;
        }
        MixerApply.SetMix(SelectedUnit.Id, mix, TbarPreset());
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
        var expanded = _transitionExpanded.Where(i => i < unit.Transitions.Count).ToHashSet();
        _transitionExpanded.Clear();
        foreach (var i in expanded)
            _transitionExpanded.Add(i);
        for (var i = 0; i < unit.Transitions.Count; i++)
        {
            var index = i;
            var preset = unit.Transitions[i];
            TransitionCatalog.ApplyKindDefaults(preset);
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
                    ? BusTheme.PreviewBrush(_session.Settings)
                    : new System.Windows.Media.SolidColorBrush(System.Windows.Media.Color.FromRgb(0x44, 0x44, 0x44)),
                BorderThickness = new Thickness(1),
                Padding = new Thickness(2),
                Header = $"{preset.Label}  {preset.DurationValue}{(preset.DurationUnit == MixerNative.DurationMs ? "ms" : "f")}"
            };

            expander.Expanded += (_, _) => _transitionExpanded.Add(index);
            expander.Collapsed += (_, _) => _transitionExpanded.Remove(index);

            var stack = new StackPanel { Margin = new Thickness(0, 6, 0, 0) };
            if (preset.Kind == MixerNative.TransitionStinger)
                preset.Kind = MixerNative.TransitionFade;
            stack.Children.Add(BuildTransitionKindGrid(preset, unit, index));
            var duration = new TextBox { Text = preset.DurationValue.ToString(), Margin = new Thickness(0, 0, 0, 4) };
            duration.TextChanged += (_, _) =>
            {
                if (uint.TryParse(duration.Text, out var value) && value > 0)
                    preset.DurationValue = value;
            };
            duration.LostFocus += (_, _) => RebuildTransitions();
            var unitBox = new ComboBox { Margin = new Thickness(0, 0, 0, 4) };
            unitBox.Items.Add(new ComboBoxItem { Content = "Frames", Tag = MixerNative.DurationFrames });
            unitBox.Items.Add(new ComboBoxItem { Content = "Milliseconds", Tag = MixerNative.DurationMs });
            unitBox.SelectedIndex = preset.DurationUnit == MixerNative.DurationMs ? 1 : 0;
            unitBox.SelectionChanged += (_, _) =>
            {
                if (unitBox.SelectedItem is ComboBoxItem item && item.Tag is uint value)
                    preset.DurationUnit = value;
                RebuildTransitions();
            };
            var easing = new ComboBox { Margin = new Thickness(0, 0, 0, 4) };
            easing.Items.Add(new ComboBoxItem { Content = "Linear", Tag = MixerNative.EasingLinear });
            easing.Items.Add(new ComboBoxItem { Content = "EaseIn", Tag = MixerNative.EasingIn });
            easing.Items.Add(new ComboBoxItem { Content = "EaseOut", Tag = MixerNative.EasingOut });
            easing.Items.Add(new ComboBoxItem { Content = "EaseInOut", Tag = MixerNative.EasingInOut });
            easing.Items.Add(new ComboBoxItem { Content = "Smoothstep", Tag = MixerNative.EasingSmoothstep });
            easing.SelectedIndex = (int)Math.Min(preset.Easing, 4u);
            easing.SelectionChanged += (_, _) =>
            {
                if (easing.SelectedItem is ComboBoxItem item && item.Tag is uint value)
                    preset.Easing = value;
            };
            var direction = new ComboBox { Margin = new Thickness(0, 0, 0, 4) };
            direction.Items.Add(new ComboBoxItem { Content = "Left", Tag = 0u });
            direction.Items.Add(new ComboBoxItem { Content = "Right", Tag = 1u });
            direction.Items.Add(new ComboBoxItem { Content = "Up", Tag = 2u });
            direction.Items.Add(new ComboBoxItem { Content = "Down", Tag = 3u });
            direction.SelectedIndex = (int)Math.Min(preset.Direction, 3u);
            direction.SelectionChanged += (_, _) =>
            {
                if (direction.SelectedItem is ComboBoxItem item && item.Tag is uint value)
                    preset.Direction = value;
            };
            var swap = new CheckBox { Content = "Swap", IsChecked = preset.Swap, Foreground = System.Windows.Media.Brushes.White, Margin = new Thickness(0, 0, 0, 4) };
            swap.Checked += (_, _) => preset.Swap = true;
            swap.Unchecked += (_, _) => preset.Swap = false;
            var keep = new CheckBox { Content = "Keep Preview Scene", IsChecked = preset.KeepPreview, Foreground = System.Windows.Media.Brushes.White, Margin = new Thickness(0, 0, 0, 4) };
            keep.Checked += (_, _) => preset.KeepPreview = true;
            keep.Unchecked += (_, _) => preset.KeepPreview = false;
            var remove = new Button { Content = "−", Height = 22, Margin = new Thickness(0, 4, 0, 0) };
            remove.Click += (_, _) =>
            {
                unit.Transitions.RemoveAt(index);
                _transitionExpanded.Remove(index);
                _tbarPresetIndex = Math.Clamp(_tbarPresetIndex, 0, Math.Max(0, unit.Transitions.Count - 1));
                RebuildTransitions();
            };
            if (preset.HasDuration)
            {
                stack.Children.Add(new TextBlock { Text = "Duration", FontSize = 11, Foreground = System.Windows.Media.Brushes.Silver });
                stack.Children.Add(duration);
                stack.Children.Add(unitBox);
            }
            if (preset.HasEasing)
            {
                stack.Children.Add(new TextBlock { Text = "Easing", FontSize = 11, Foreground = System.Windows.Media.Brushes.Silver });
                stack.Children.Add(easing);
            }
            if (preset.HasDirection)
            {
                stack.Children.Add(new TextBlock { Text = "Direction", FontSize = 11, Foreground = System.Windows.Media.Brushes.Silver });
                stack.Children.Add(direction);
            }
            if (preset.HasSoftness)
            {
                var info = TransitionCatalog.Info(preset.Kind);
                stack.Children.Add(new TextBlock { Text = info.SoftnessLabel, FontSize = 11, Foreground = System.Windows.Media.Brushes.Silver });
                stack.Children.Add(TransitionFloatBox(() => preset.Softness, value => preset.Softness = Math.Clamp(value, 0f, 4f), "0.###"));
            }
            if (preset.HasParam)
            {
                var info = TransitionCatalog.Info(preset.Kind);
                stack.Children.Add(new TextBlock { Text = info.ParamLabel, FontSize = 11, Foreground = System.Windows.Media.Brushes.Silver });
                stack.Children.Add(TransitionFloatBox(() => preset.Param, value => preset.Param = Math.Max(0f, value), "0.##"));
            }
            stack.Children.Add(swap);
            stack.Children.Add(keep);
            if (preset.HasDipColor)
            {
                stack.Children.Add(new TextBlock { Text = preset.Kind == MixerNative.TransitionPush ? "Fill color" : "Dip color", FontSize = 11, Foreground = System.Windows.Media.Brushes.Silver });
                stack.Children.Add(ColorPick.Swatch(
                    () => (preset.DipR, preset.DipG, preset.DipB),
                    (r, g, b) =>
                    {
                        preset.DipR = r;
                        preset.DipG = g;
                        preset.DipB = b;
                        preset.DipA = 1;
                    }));
            }
            if (preset.HasCustomWgsl)
            {
                var edit = new Button { Content = string.IsNullOrWhiteSpace(preset.CustomWgsl) ? "Edit WGSL…" : "Edit WGSL (set)", Height = 26, Margin = new Thickness(0, 0, 0, 4) };
                edit.Click += (_, _) =>
                {
                    var dialog = new CustomWgslWindow(preset.CustomWgsl) { Owner = this };
                    if (dialog.ShowDialog() == true)
                    {
                        preset.CustomWgsl = dialog.Wgsl;
                        MixerNative.SetCustomWgsl(unit.Id, dialog.Wgsl);
                        RebuildTransitions();
                    }
                };
                stack.Children.Add(edit);
            }
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

    private static ControlTemplate FlatTabTemplate()
    {
        var border = new FrameworkElementFactory(typeof(Border));
        border.SetBinding(Border.BackgroundProperty, new Binding(nameof(Button.Background))
        {
            RelativeSource = new RelativeSource(RelativeSourceMode.TemplatedParent)
        });
        border.SetBinding(Border.BorderBrushProperty, new Binding(nameof(Button.BorderBrush))
        {
            RelativeSource = new RelativeSource(RelativeSourceMode.TemplatedParent)
        });
        border.SetBinding(Border.BorderThicknessProperty, new Binding(nameof(Button.BorderThickness))
        {
            RelativeSource = new RelativeSource(RelativeSourceMode.TemplatedParent)
        });
        var content = new FrameworkElementFactory(typeof(ContentPresenter));
        content.SetValue(ContentPresenter.HorizontalAlignmentProperty, HorizontalAlignment.Center);
        content.SetValue(ContentPresenter.VerticalAlignmentProperty, VerticalAlignment.Center);
        border.AppendChild(content);
        return new ControlTemplate(typeof(Button)) { VisualTree = border };
    }

    private Button TransitionGroupTab(string label, bool on)
    {
        return new Button
        {
            Content = label,
            Height = 20,
            FontSize = 11,
            FontWeight = on ? FontWeights.SemiBold : FontWeights.Normal,
            Margin = new Thickness(1, 0, 1, 2),
            Padding = new Thickness(2, 0, 2, 0),
            Foreground = on
                ? System.Windows.Media.Brushes.White
                : System.Windows.Media.Brushes.Silver,
            Background = System.Windows.Media.Brushes.Transparent,
            BorderBrush = on
                ? BusTheme.PreviewBrush(_session.Settings)
                : System.Windows.Media.Brushes.Transparent,
            BorderThickness = new Thickness(0, 0, 0, 2),
            Cursor = Cursors.Hand,
            Template = FlatTabTemplate()
        };
    }

    private Button TransitionPickerButton(string label, bool on)
    {
        return new Button
        {
            Content = label,
            Height = 22,
            FontSize = 11,
            Margin = new Thickness(1),
            Padding = new Thickness(2, 0, 2, 0),
            Foreground = System.Windows.Media.Brushes.White,
            Background = new System.Windows.Media.SolidColorBrush(
                System.Windows.Media.Color.FromRgb(0x22, 0x22, 0x22)),
            BorderBrush = on
                ? BusTheme.PreviewBrush(_session.Settings)
                : new System.Windows.Media.SolidColorBrush(System.Windows.Media.Color.FromRgb(0x44, 0x44, 0x44)),
            BorderThickness = new Thickness(1)
        };
    }

    private UIElement BuildTransitionKindGrid(TransitionPreset preset, MixingUnitEntry unit, int index)
    {
        var open = _kindMenuGroup.TryGetValue(index, out var stored)
            ? stored
            : TransitionCatalog.Info(preset.Kind).Group;
        var root = new StackPanel { Margin = new Thickness(0, 0, 0, 4) };
        var groups = new UniformGrid { Columns = 4 };
        foreach (var group in new[] { TransitionGroup.Basic, TransitionGroup.Wipe, TransitionGroup.Motion, TransitionGroup.Shader })
        {
            var captured = group;
            var button = TransitionGroupTab(TransitionCatalog.GroupName(group), open == group);
            button.Click += (_, _) =>
            {
                _kindMenuGroup[index] = captured;
                RebuildTransitions();
            };
            groups.Children.Add(button);
        }
        root.Children.Add(groups);

        var grid = new UniformGrid { Columns = 3, Margin = new Thickness(0, 2, 0, 0) };
        foreach (var item in TransitionCatalog.All.Where(item => item.Group == open))
        {
            var kind = item.Kind;
            var button = TransitionPickerButton(item.Label, preset.Kind == kind);
            button.Tag = kind;
            button.Click += (_, _) =>
            {
                preset.Kind = kind;
                preset.Softness = TransitionCatalog.DefaultSoftness(kind);
                preset.Param = TransitionCatalog.DefaultParam(kind);
                var duration = TransitionCatalog.DefaultDurationValue(kind);
                if (duration > 0)
                    preset.DurationValue = duration;
                var direction = TransitionCatalog.DefaultDirection(kind);
                if (direction.HasValue)
                    preset.Direction = direction.Value;
                _kindMenuGroup[index] = item.Group;
                if (kind == MixerNative.TransitionCustom && string.IsNullOrWhiteSpace(preset.CustomWgsl))
                {
                    preset.CustomWgsl = CustomWgslWindow.WgslTemplate;
                    MixerNative.SetCustomWgsl(unit.Id, preset.CustomWgsl);
                }
                RebuildTransitions();
            };
            grid.Children.Add(button);
        }
        root.Children.Add(grid);
        return root;
    }

    private static TextBox TransitionFloatBox(Func<float> get, Action<float> set, string format)
    {
        var box = new TextBox { Text = get().ToString(format), Margin = new Thickness(0, 0, 0, 4) };
        void Apply()
        {
            if (float.TryParse(box.Text, out var value))
                set(value);
        }
        box.TextChanged += (_, _) => Apply();
        NumericDrag.AttachBox(box, 80f, Apply, Apply, format);
        return box;
    }

    internal void RebuildOverlayToggles()
    {
        OverlayTogglePanel.Children.Clear();
        var unit = SelectedUnit;
        if (unit.Overlays.Count == 0)
            return;
        for (var i = 0; i < unit.Overlays.Count; i++)
        {
            var slot = unit.Overlays[i];
            var name = slot.DisplayName(_session);
            OverlayTogglePanel.Children.Add(new OverlayStrip(name, slot.Enabled, enabled =>
            {
                ToggleOverlay(unit, slot, enabled);
            }));
        }
    }

    internal void PushAuxFor(MixingUnitEntry unit) =>
        MixerApply.PatchAux(unit.Id, unit);

    internal void ToggleOverlay(MixingUnitEntry unit, OverlaySlot slot, bool enabled)
    {
        var index = (uint)Math.Max(0, unit.Overlays.IndexOf(slot));
        var ms = slot.DurationUnit == MixerNative.DurationMs
            ? Math.Max(1, slot.DurationValue)
            : unit.DurationMs(slot.DurationValue);
        if (Application.Current is App { Backend.IsRemote: true } remoteApp)
        {
            slot.Enabled = enabled;
            remoteApp.Backend.OverlayAuto(unit.Id, index, ms, enabled);
            NotifyOverlayUi();
            return;
        }
        var desc = new OverlayDesc
        {
            SourceId = slot.SceneGpuId,
            Rect = new Interop.Rect { X = slot.X, Y = slot.Y, Width = slot.Width, Height = slot.Height },
            Crop = new Interop.Rect { X = slot.CropX, Y = slot.CropY, Width = slot.CropWidth, Height = slot.CropHeight },
            Opacity = slot.Opacity,
            Z = slot.Z,
            AudioFollow = slot.AudioFollow ? 1u : 0u,
            Hidden = slot.Hidden ? 1u : 0u
        };
        if (slot.TransitionKind == MixerNative.TransitionCut || ms <= 1)
        {
            slot.Enabled = enabled;
            PushAuxFor(unit);
            NotifyOverlayUi();
            return;
        }
        if (enabled)
        {
            slot.Enabled = true;
            PushAuxFor(unit);
            unsafe
            {
                MixerNative.OverlayAuto(unit.Id, 1u, ms, &desc);
            }
            NotifyOverlayUi();
            return;
        }

        slot.Enabled = false;
        NotifyOverlayUi();
        slot.Enabled = true;
        PushAuxFor(unit);
        unsafe
        {
            MixerNative.OverlayAuto(unit.Id, 0u, ms, &desc);
        }
        slot.Enabled = false;
        var delay = TimeSpan.FromMilliseconds(ms);
        _ = Dispatcher.InvokeAsync(async () =>
        {
            await Task.Delay(delay);
            PushAuxFor(unit);
            NotifyOverlayUi();
        });
    }

    internal void NotifyOverlayUi()
    {
        Dispatcher.BeginInvoke(() =>
        {
            RebuildOverlayToggles();
            foreach (var switcher in _switchers.Values)
                switcher.RebuildOverlays();
            _overlay?.RefreshEnabled();
        });
    }

    private void OpenOverlay_Click(object sender, RoutedEventArgs e) =>
        OpenOverlayFor(SelectedUnit);

    internal void OpenOverlayFor(MixingUnitEntry unit)
    {
        if (_overlay is not null)
        {
            _overlay.Reload(unit);
            _overlay.Activate();
            return;
        }
        if (!HostRole.IsRemote && !FlipBudget.TryOpen(1, this))
            return;
        _overlay = new OverlayWindow(_session, unit) { Owner = this };
        _overlay.Closed += (_, _) =>
        {
            _overlay = null;
            RebuildOverlayToggles();
            foreach (var switcher in _switchers.Values)
                switcher.RebuildOverlays();
        };
        _overlay.Show();
    }

    internal void EditSceneFromSwitcher(SceneEntry scene) => OpenSceneEditor(scene);

}
