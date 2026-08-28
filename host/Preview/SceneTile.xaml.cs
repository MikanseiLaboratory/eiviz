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

    public void Bind(SceneEntry scene, bool selected)
    {
        Scene = scene;
        Title.Text = scene.Name;
        Monitor.RetargetMonitor(scene.MonitorId, scene.GpuId);
        SetSelected(selected);
    }

    public void SetSelected(bool selected)
    {
        Chrome.BorderBrush = selected
            ? new SolidColorBrush(Color.FromRgb(0xE8, 0x77, 0x22))
            : new SolidColorBrush(Color.FromRgb(0x55, 0x55, 0x55));
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
}
