using System.Windows;
using System.Windows.Input;
using Eiviz.Host.Interop;

namespace Eiviz.Host.Dialogs;

public partial class MultiviewWindow : Window
{
    private readonly Session _session;
    private readonly MultiviewLayout _layout;
    private bool _fullscreen;
    private WindowStyle _savedStyle;
    private WindowState _savedState;
    private ResizeMode _savedResize;

    public MultiviewWindow(Session session, MultiviewLayout layout)
    {
        InitializeComponent();
        _session = session;
        _layout = layout;
        Title = layout.Name;
        TitleText.Text = layout.Name;
        Loaded += (_, _) =>
        {
            MultiviewHost.RetargetMonitor(layout.MonitorId, layout.GpuId);
        };
    }

    public ulong LayoutId => _layout.Id;

    private void Tiles_Click(object sender, RoutedEventArgs e)
    {
        var dialog = new MultiviewSlotsWindow(_session, _layout.Tiles, 32) { Owner = this };
        if (dialog.ShowDialog() != true)
            return;
        var unit = _session.Units.FirstOrDefault(item => item.Id == _session.Settings.DefaultMultiviewUnitId)
            ?? _session.Units[0];
        ((App)Application.Current).Commands.PushMultiviewNow(_layout, unit.Width, unit.Height);
        MultiviewHost.UpdateMonitorSource(_layout.GpuId);
    }

    private void Fullscreen_Click(object sender, RoutedEventArgs e) => ToggleFullscreen();

    private void Window_KeyDown(object sender, KeyEventArgs e)
    {
        if (e.Key == Key.F11)
        {
            ToggleFullscreen();
            e.Handled = true;
        }
        else if (e.Key == Key.Escape && _fullscreen)
        {
            ToggleFullscreen();
            e.Handled = true;
        }
    }

    private void ToggleFullscreen()
    {
        if (!_fullscreen)
        {
            _savedStyle = WindowStyle;
            _savedState = WindowState;
            _savedResize = ResizeMode;
            Chrome.Visibility = Visibility.Collapsed;
            WindowStyle = WindowStyle.None;
            ResizeMode = ResizeMode.NoResize;
            WindowState = WindowState.Normal;
            WindowState = WindowState.Maximized;
            _fullscreen = true;
        }
        else
        {
            Chrome.Visibility = Visibility.Visible;
            WindowStyle = _savedStyle;
            ResizeMode = _savedResize;
            WindowState = _savedState;
            _fullscreen = false;
        }
    }
}
