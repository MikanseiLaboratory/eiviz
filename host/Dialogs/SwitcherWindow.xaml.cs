using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using Eiviz.Host.Interop;

namespace Eiviz.Host.Dialogs;

public partial class SwitcherWindow : Window
{
    private readonly MixingUnitEntry _unit;
    private int _tbarPresetIndex;
    private bool _tbarLatching;
    private bool _tbarLocked;
    private bool _suppressScene;

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
        RebuildScenes();
        RebuildTransitions();
        Loaded += (_, _) =>
        {
            ApplyBusColors();
            PreviewHost.RetargetUnit(unit.Id, MixerNative.OutputPreview);
            ProgramHost.RetargetUnit(unit.Id, MixerNative.OutputProgram);
        };
        Closed += (_, _) =>
        {
            PreviewHost.ReleaseNative();
            ProgramHost.ReleaseNative();
        };
    }

    public ulong UnitId => _unit.Id;

    private Session Session => ((App)Application.Current).Session;

    private CommandQueue Commands => ((App)Application.Current).Commands;

    internal void SyncFromUnit()
    {
        Title = _unit.Name;
        TitleText.Text = _unit.Name;
        PreviewAspect.RatioWidth = _unit.Width;
        PreviewAspect.RatioHeight = _unit.Height;
        ProgramAspect.RatioWidth = _unit.Width;
        ProgramAspect.RatioHeight = _unit.Height;
        RebuildScenes();
        RebuildTransitions();
        ApplyBusColors();
    }

    internal void ApplyBusColors()
    {
        BusTheme.Apply(Session.Settings, PreviewFrame, PreviewHeader, PreviewHeaderText, preview: true);
        BusTheme.Apply(Session.Settings, ProgramFrame, ProgramHeader, ProgramHeaderText, preview: false);
        RebuildTransitions();
    }

    private void RebuildScenes()
    {
        _suppressScene = true;
        SceneList.ItemsSource = Session.Scenes.ToList();
        SceneList.DisplayMemberPath = nameof(SceneEntry.Name);
        _suppressScene = false;
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
                    : new System.Windows.Media.SolidColorBrush(System.Windows.Media.Color.FromRgb(0x44, 0x44, 0x44)),
                BorderThickness = new Thickness(1),
                Background = new System.Windows.Media.SolidColorBrush(System.Windows.Media.Color.FromRgb(0x22, 0x22, 0x22)),
                Padding = new Thickness(6, 4, 6, 4),
                Child = new TextBlock
                {
                    Text = $"{preset.Label}  {preset.DurationValue}{(preset.DurationUnit == MixerNative.DurationMs ? "ms" : "f")}",
                    Foreground = System.Windows.Media.Brushes.White
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
        Commands.TryEnqueue(new SetMixCommand(_unit.Id, mix));
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

    private void SceneList_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (_suppressScene)
            return;
        if (SceneList.SelectedItem is SceneEntry scene)
            Commands.TryEnqueue(new PreviewSceneCommand(_unit.Id, scene.GpuId));
    }

    private void SceneList_Activate(object sender, MouseButtonEventArgs e)
    {
        if (SceneList.SelectedItem is SceneEntry scene)
            Commands.TryEnqueue(new PreviewSceneCommand(_unit.Id, scene.GpuId));
    }
}
