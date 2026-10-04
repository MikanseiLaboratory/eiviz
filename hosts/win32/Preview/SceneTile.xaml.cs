using System;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Threading;
using Eiviz.Host.I18n;
using Eiviz.Host.Interop;

namespace Eiviz.Host.Preview;

public partial class SceneTile : UserControl
{
    private const double AnimPanelWidth = 130;
    private static readonly Brush AnimLive = new SolidColorBrush(Color.FromRgb(0x1E, 0x6B, 0x3A));
    private readonly DispatcherTimer _animTally = new() { Interval = TimeSpan.FromMilliseconds(200) };
    private bool _animOpen;
    private string _animShape = "";

    public SceneTile()
    {
        InitializeComponent();
        AnimExpandedButton.ToolTip = Loc.T("anim.panel");
        AnimCollapsedButton.ToolTip = Loc.T("anim.panel");
        _animTally.Tick += (_, _) => PaintAnim();
        Unloaded += (_, _) => _animTally.Stop();
        Loaded += (_, _) =>
        {
            if (_animOpen)
                _animTally.Start();
        };
        MouseLeftButtonUp += (_, _) => Select();
        MouseDoubleClick += (_, e) =>
        {
            if (FindAncestor<Button>(e.OriginalSource as DependencyObject) is not null)
                return;
            Raise(SceneEditRequested);
            e.Handled = true;
        };
    }

    public SceneEntry? Scene { get; private set; }
    public event EventHandler<SceneEntry>? SceneSelected;
    public event EventHandler<SceneEntry>? SceneEditRequested;
    public event EventHandler<SceneEntry>? SceneCutRequested;
    public event EventHandler<SceneEntry>? SceneLoopRequested;
    public event EventHandler<SceneEntry>? ScenePlayRequested;
    public event EventHandler<SceneEntry>? SceneAudioRequested;
    public event EventHandler<SceneEntry>? ScenePreviewRequested;
    public event EventHandler<SceneEntry>? SceneCloseRequested;
    public event EventHandler<SceneEntry>? SceneCollapseToggled;
    public event EventHandler<SceneEntry>? SceneSnapshotRequested;

    public void Bind(SceneEntry scene, int number, bool selected, uint presentInterval = 3, Color? previewColor = null, Color? inactiveColor = null)
    {
        Scene = scene;
        Title.Text = scene.Name;
        CollapsedTitle.Text = scene.Name;
        Number.Text = number.ToString();
        CollapsedNumber.Text = number.ToString();
        Monitor.SetWanted(false);
        ApplyCollapsed();
        if (_animOpen)
            BuildAnim();
        if (HostRole.IsRemote || Application.Current is App { Backend.CanShowSceneThumbs: false })
            return;
        Monitor.Bind(scene.GpuId, 170, 90, presentInterval);
    }

    public void SetPresentInterval(uint presentInterval) =>
        Monitor.SetPresentInterval(presentInterval);

    public void SetThumbWanted(bool wanted) => Monitor.SetWanted(wanted);

    public void SetCloseEnabled(bool enabled)
    {
        CloseExpandedButton.IsEnabled = enabled;
        CloseCollapsedButton.IsEnabled = enabled;
    }

    public void SetSelected(bool selected, Color? previewColor = null, Color? inactiveColor = null) =>
        SetBusRoles(selected, false, previewColor, null, inactiveColor);

    private bool _preview;
    private bool _program;
    private Color _borderColor;

    public void SetBusRoles(bool preview, bool program, Color? previewColor = null, Color? programColor = null, Color? inactiveColor = null)
    {
        var idle = inactiveColor ?? Color.FromRgb(64, 64, 64);
        var color = program
            ? programColor ?? Color.FromRgb(255, 0, 0)
            : preview
                ? previewColor ?? Color.FromRgb(0, 255, 0)
                : idle;
        if (_preview == preview && _program == program && _borderColor == color)
            return;
        _preview = preview;
        _program = program;
        _borderColor = color;
        Chrome.BorderBrush = new SolidColorBrush(color);
        Chrome.BorderThickness = new Thickness(3);
        var fill = program || preview
            ? Color.FromArgb(80, color.R, color.G, color.B)
            : Color.FromRgb(0x33, 0x33, 0x33);
        CollapsedBody.Background = new SolidColorBrush(fill);
    }

    public void SetTransport(bool hasVideo, bool loop, bool playing, bool muted)
    {
        LoopButton.IsEnabled = hasVideo;
        PlayButton.IsEnabled = hasVideo;
        PlayButton.Content = playing ? "❚❚" : "▶";
        OnOffLook.Apply(LoopButton, hasVideo && loop);
        OnOffLook.Apply(AudioButton, !muted);
    }

    public void ApplyCollapsed()
    {
        var collapsed = HostRole.IsRemote || Scene?.PreviewCollapsed == true;
        Width = (collapsed ? 40 : 176) + (_animOpen ? AnimPanelWidth : 0);
        AnimPanel.Visibility = _animOpen ? Visibility.Visible : Visibility.Collapsed;
        AnimExpandedButton.Content = _animOpen ? "◂" : "▸";
        AnimCollapsedButton.Content = _animOpen ? "◂" : "▸";
        Height = 140;
        ExpandedBody.Visibility = collapsed ? Visibility.Collapsed : Visibility.Visible;
        CollapsedBody.Visibility = collapsed ? Visibility.Visible : Visibility.Collapsed;
        if (collapsed)
            SetThumbWanted(false);
        InvalidateMeasure();
        InvalidateArrange();
    }

    private void Chrome_RightClick(object sender, MouseButtonEventArgs e)
    {
        if (HostRole.IsRemote)
            return;
        if (FindAncestor<Button>(e.OriginalSource as DependencyObject) is not null)
            return;
        if (Scene is not { } scene)
            return;
        scene.PreviewCollapsed = !scene.PreviewCollapsed;
        ApplyCollapsed();
        SceneCollapseToggled?.Invoke(this, scene);
        e.Handled = true;
    }

    private static T? FindAncestor<T>(DependencyObject? current) where T : DependencyObject
    {
        while (current is not null)
        {
            if (current is T match)
                return match;
            current = VisualTreeHelper.GetParent(current);
        }
        return null;
    }

    private void Select()
    {
        if (Scene is { } scene)
            SceneSelected?.Invoke(this, scene);
    }

    private void Cut_Click(object sender, RoutedEventArgs e) => Raise(SceneCutRequested);

    private void Loop_Click(object sender, RoutedEventArgs e) => Raise(SceneLoopRequested);

    private void Play_Click(object sender, RoutedEventArgs e) => Raise(ScenePlayRequested);

    private void Audio_Click(object sender, RoutedEventArgs e) => Raise(SceneAudioRequested);

    private void Preview_Click(object sender, RoutedEventArgs e) => Raise(ScenePreviewRequested);

    private void Settings_Click(object sender, RoutedEventArgs e) => Raise(SceneEditRequested);

    private void Settings_RightClick(object sender, MouseButtonEventArgs e)
    {
        if (HostRole.IsRemote)
            return;
        Raise(SceneSnapshotRequested);
        e.Handled = true;
    }

    private void Anim_Click(object sender, RoutedEventArgs e)
    {
        _animOpen = !_animOpen;
        if (_animOpen)
        {
            BuildAnim();
            _animTally.Start();
        }
        else
        {
            _animTally.Stop();
        }
        ApplyCollapsed();
    }

    private static string AnimShape(SceneEntry scene) =>
        string.Join("|", scene.States.Select(state => $"{state.Id}:{state.Name}"))
        + "#" + string.Join("|", scene.Sequences.Select(sequence => $"{sequence.Id}:{sequence.Name}:{sequence.Steps.Count}"));

    private void BuildAnim()
    {
        AnimButtons.Children.Clear();
        if (Scene is not { } scene)
            return;
        _animShape = AnimShape(scene);
        if (scene.States.Count == 0)
        {
            AnimButtons.Children.Add(new TextBlock
            {
                Text = Loc.T("anim.empty"),
                Foreground = Brushes.Silver,
                FontSize = 10,
                TextWrapping = TextWrapping.Wrap
            });
            return;
        }
        AnimButtons.Children.Add(AnimHeading(Loc.T("anim.states")));
        foreach (var state in scene.States)
        {
            var id = state.Id;
            AnimButtons.Children.Add(AnimButton(LabelOf(state.Name, id), ("state", id), () => SceneAnimPlayback.GoTo(scene, id)));
        }
        var saved = AnimButton(Loc.T("anim.saved"), ("state", 0UL), () => SceneAnimPlayback.GoTo(scene, 0));
        saved.ToolTip = Loc.T("anim.savedHelp");
        saved.Foreground = Brushes.Silver;
        AnimButtons.Children.Add(saved);
        if (scene.Sequences.Count == 0)
        {
            PaintAnim();
            return;
        }
        AnimButtons.Children.Add(new Border
        {
            Height = 1,
            Background = new SolidColorBrush(Color.FromRgb(0x55, 0x55, 0x55)),
            Margin = new Thickness(0, 4, 0, 2)
        });
        AnimButtons.Children.Add(AnimHeading(Loc.T("anim.sequences")));
        foreach (var sequence in scene.Sequences)
        {
            var id = sequence.Id;
            var row = new DockPanel { Margin = new Thickness(0, 0, 0, 2) };
            var stop = AnimButton("■", null, () => SceneAnimPlayback.Sequence(scene, id, MixerNative.SceneSeqStop));
            stop.Width = 22;
            stop.Margin = new Thickness(2, 0, 0, 0);
            stop.ToolTip = Loc.T("anim.stop");
            DockPanel.SetDock(stop, Dock.Right);
            row.Children.Add(stop);
            var play = AnimButton("▶ " + LabelOf(sequence.Name, id), ("sequence", id), () => SceneAnimPlayback.Sequence(scene, id, MixerNative.SceneSeqPlay));
            play.Margin = new Thickness(0);
            row.Children.Add(play);
            AnimButtons.Children.Add(row);
        }
        PaintAnim();
    }

    private static TextBlock AnimHeading(string text) => new()
    {
        Text = text,
        Foreground = Brushes.Silver,
        FontSize = 10,
        Margin = new Thickness(0, 2, 0, 2)
    };

    private static Button AnimButton(string label, (string Kind, ulong Id)? tag, Action click)
    {
        var button = new Button
        {
            Content = new TextBlock { Text = label, TextTrimming = TextTrimming.CharacterEllipsis },
            Height = 22,
            FontSize = 11,
            Padding = new Thickness(4, 0, 4, 0),
            Margin = new Thickness(0, 0, 0, 2),
            HorizontalContentAlignment = HorizontalAlignment.Left,
            Tag = tag,
            ToolTip = label
        };
        button.Click += (_, _) => click();
        return button;
    }

    private void PaintAnim()
    {
        if (!_animOpen || Scene is not { } scene)
            return;
        if (AnimShape(scene) != _animShape)
        {
            BuildAnim();
            return;
        }
        var live = SceneAnimPlayback.Read(scene);
        var lit = live.MovingState ?? live.ShownState;
        foreach (var button in AnimButtonsIn(AnimButtons))
        {
            var on = button.Tag switch
            {
                ("state", ulong id) => lit == id,
                ("sequence", ulong id) => live.SequenceId == id,
                _ => false
            };
            if (on)
                button.Background = AnimLive;
            else
                button.ClearValue(BackgroundProperty);
        }
    }

    private static IEnumerable<Button> AnimButtonsIn(Panel panel)
    {
        foreach (var child in panel.Children)
        {
            if (child is Button button)
                yield return button;
            else if (child is Panel nested)
            {
                foreach (var inner in AnimButtonsIn(nested))
                    yield return inner;
            }
        }
    }

    private static string LabelOf(string name, ulong id) =>
        string.IsNullOrWhiteSpace(name) ? id.ToString() : name;

    private void Close_Click(object sender, RoutedEventArgs e)
    {
        if (OnAirLock.Active)
            return;
        Raise(SceneCloseRequested);
    }

    private void Raise(EventHandler<SceneEntry>? handler)
    {
        if (Scene is { } scene)
            handler?.Invoke(this, scene);
    }
}
