using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;

namespace Eiviz.Host.Preview;

public partial class SceneTile : UserControl
{
    public SceneTile()
    {
        InitializeComponent();
        MouseLeftButtonUp += (_, _) => Select();
        MouseDoubleClick += (_, _) => Edit();
        Monitor.SurfaceClicked += (_, _) => Select();
        Monitor.SurfaceDoubleClicked += (_, _) => Edit();
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

    public void Bind(SceneEntry scene, int number, bool selected)
    {
        Scene = scene;
        Title.Text = scene.Name;
        Number.Text = number.ToString();
        Monitor.RetargetMonitor(scene.MonitorId, scene.GpuId);
        SetSelected(selected);
    }

    public void SetSelected(bool selected)
    {
        Chrome.BorderBrush = selected
            ? new SolidColorBrush(Color.FromRgb(0xE8, 0x77, 0x22))
            : new SolidColorBrush(Color.FromRgb(0x55, 0x55, 0x55));
    }

    public void SetTransport(bool hasVideo, bool loop, bool playing, bool muted)
    {
        LoopButton.IsEnabled = hasVideo;
        PlayButton.IsEnabled = hasVideo;
        LoopButton.Opacity = loop ? 1 : 0.55;
        PlayButton.Content = playing ? "❚❚" : "▶";
        AudioButton.Opacity = muted ? 0.45 : 1;
    }

    private void Select()
    {
        if (Scene is { } scene)
            SceneSelected?.Invoke(this, scene);
    }

    private void Edit()
    {
        if (Scene is { } scene)
            SceneEditRequested?.Invoke(this, scene);
    }

    private void Cut_Click(object sender, RoutedEventArgs e) => Raise(SceneCutRequested);

    private void Loop_Click(object sender, RoutedEventArgs e) => Raise(SceneLoopRequested);

    private void Play_Click(object sender, RoutedEventArgs e) => Raise(ScenePlayRequested);

    private void Audio_Click(object sender, RoutedEventArgs e) => Raise(SceneAudioRequested);

    private void Preview_Click(object sender, RoutedEventArgs e) => Raise(ScenePreviewRequested);

    private void Settings_Click(object sender, RoutedEventArgs e) => Raise(SceneEditRequested);

    private void Close_Click(object sender, RoutedEventArgs e) => Raise(SceneCloseRequested);

    private void Raise(EventHandler<SceneEntry>? handler)
    {
        if (Scene is { } scene)
            handler?.Invoke(this, scene);
    }
}
