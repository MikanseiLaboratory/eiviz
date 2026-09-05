using System;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using Eiviz.Host.I18n;

namespace Eiviz.Host.Preview;

public partial class SceneTile : UserControl
{
    public SceneTile()
    {
        InitializeComponent();
        MouseLeftButtonUp += (_, _) => Select();
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
        if (scene.PreviewCollapsed)
            Monitor.SetWanted(false);
        Monitor.Bind(scene.GpuId, 170, 90, presentInterval);
        ApplyCollapsed();
    }

    public void SetPresentInterval(uint presentInterval) =>
        Monitor.SetPresentInterval(presentInterval);

    public void SetThumbWanted(bool wanted) => Monitor.SetWanted(wanted);

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
    }

    public void SetTransport(bool hasVideo, bool loop, bool playing, bool muted)
    {
        LoopButton.IsEnabled = hasVideo;
        PlayButton.IsEnabled = hasVideo;
        LoopButton.Opacity = loop ? 1 : 0.55;
        PlayButton.Content = playing ? "❚❚" : "▶";
        AudioButton.Opacity = muted ? 0.45 : 1;
    }

    public void ApplyCollapsed()
    {
        var collapsed = Scene?.PreviewCollapsed == true;
        Width = collapsed ? 40 : 176;
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
        if (FindAncestor<Button>(e.OriginalSource as DependencyObject) is not null)
            return;
        OpenSceneMenu(Chrome, includeEdit: false);
        e.Handled = true;
    }

    private void OpenSceneMenu(FrameworkElement target, bool includeEdit)
    {
        if (Scene is not { } scene)
            return;
        var menu = new ContextMenu();
        if (includeEdit)
        {
            var edit = new MenuItem { Header = Loc.T("chrome.edit") };
            edit.Click += (_, _) => Raise(SceneEditRequested);
            menu.Items.Add(edit);
        }
        var snap = new MenuItem { Header = Loc.T("action.Snapshot") };
        snap.Click += (_, _) => Raise(SceneSnapshotRequested);
        menu.Items.Add(snap);
        if (!includeEdit)
        {
            var collapse = new MenuItem
            {
                Header = Loc.T(scene.PreviewCollapsed ? "scene.expand" : "scene.collapse")
            };
            collapse.Click += (_, _) =>
            {
                scene.PreviewCollapsed = !scene.PreviewCollapsed;
                ApplyCollapsed();
                SceneCollapseToggled?.Invoke(this, scene);
            };
            menu.Items.Add(collapse);
        }
        menu.PlacementTarget = target;
        menu.IsOpen = true;
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

    private void TitleBar_MouseLeftButtonDown(object sender, MouseButtonEventArgs e)
    {
        if (e.ClickCount >= 2)
        {
            Raise(SceneEditRequested);
            e.Handled = true;
        }
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

    private void Settings_Click(object sender, RoutedEventArgs e)
    {
        if (sender is FrameworkElement target)
            OpenSceneMenu(target, includeEdit: true);
    }

    private void Close_Click(object sender, RoutedEventArgs e) => Raise(SceneCloseRequested);

    private void Raise(EventHandler<SceneEntry>? handler)
    {
        if (Scene is { } scene)
            handler?.Invoke(this, scene);
    }
}
