using System.Runtime.InteropServices;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Interop;
using System.Windows.Media;
using System.Windows.Threading;

namespace Eiviz.Host.Dialogs;

public partial class MultiviewWindow : Window
{
    private readonly Session _session;
    private readonly MultiviewLayout _layout;
    private bool _fullscreen;
    private bool _suppressPresent;
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
        Topmost = layout.AlwaysOnTop;
        OnTopBox.IsChecked = layout.AlwaysOnTop;
        SyncPresentInterval();
        SyncLabelAnchor();
        SourceInitialized += (_, _) =>
        {
            if (PresentationSource.FromVisual(this) is HwndSource source)
                source.AddHook(WndProc);
        };
        Loaded += (_, _) =>
        {
            SnapWindowToAspect();
            MultiviewHost.RetargetMonitor(layout.MonitorId, layout.GpuId);
            _layout.PushPresentInterval(_session.Settings);
        };
    }

    public ulong LayoutId => _layout.Id;

    internal void SyncPresentInterval()
    {
        _suppressPresent = true;
        var tag = _layout.PresentInterval == 0 ? "0" : MultiviewLayout.ClampPresentInterval(_layout.PresentInterval).ToString();
        foreach (ComboBoxItem item in PresentBox.Items)
        {
            if (Equals(item.Tag, tag))
            {
                PresentBox.SelectedItem = item;
                break;
            }
        }
        _suppressPresent = false;
    }

    internal void SyncLabelAnchor()
    {
        _suppressPresent = true;
        var tag = _layout.ResolvedLabelAnchor(_session.Settings) == MvLabelAnchor.Top ? "Top" : "Bottom";
        foreach (ComboBoxItem item in AnchorBox.Items)
        {
            if (Equals(item.Tag, tag))
            {
                AnchorBox.SelectedItem = item;
                break;
            }
        }
        _suppressPresent = false;
    }

    private void OnTop_Click(object sender, RoutedEventArgs e)
    {
        _layout.AlwaysOnTop = OnTopBox.IsChecked == true;
        Topmost = _layout.AlwaysOnTop;
        Owner = _layout.AlwaysOnTop ? Application.Current.MainWindow : null;
    }

    private void AnchorBox_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (_suppressPresent)
            return;
        if (AnchorBox.SelectedItem is ComboBoxItem item && item.Tag is string tag)
        {
            _layout.LabelAnchor = tag == "Top" ? MvLabelAnchor.Top : MvLabelAnchor.Bottom;
            _layout.PushLabelStyle(_session.Settings);
        }
    }

    private void PresentBox_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (_suppressPresent)
            return;
        if (PresentBox.SelectedItem is ComboBoxItem item && item.Tag is string tag
            && uint.TryParse(tag, out var interval))
        {
            _layout.PresentInterval = interval == 0 ? 0 : MultiviewLayout.ClampPresentInterval(interval);
            _layout.PushPresentInterval(_session.Settings);
        }
    }

    private void Tiles_Click(object sender, RoutedEventArgs e)
    {
        var dialog = new MultiviewSlotsWindow(_session, _layout) { Owner = this };
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
            Dispatcher.BeginInvoke(SnapWindowToAspect, DispatcherPriority.Loaded);
        }
        Dispatcher.BeginInvoke(MultiviewHost.RefreshSize, DispatcherPriority.Loaded);
    }

    private void SnapWindowToAspect()
    {
        if (_fullscreen)
            return;
        var nc = Math.Max(0, Height - ActualHeight);
        var chrome = Chrome.Visibility == Visibility.Visible ? Chrome.ActualHeight : 0;
        Height = chrome + Math.Max(2, ActualWidth) * 9.0 / 16.0 + nc;
    }

    private nint WndProc(nint hwnd, int msg, nint wParam, nint lParam, ref bool handled)
    {
        if (msg != WmSizing || _fullscreen)
            return 0;
        var rect = Marshal.PtrToStructure<WinRect>(lParam);
        ConstrainRect(hwnd, wParam.ToInt32(), ref rect);
        Marshal.StructureToPtr(rect, lParam, true);
        handled = true;
        return 0;
    }

    private void ConstrainRect(nint hwnd, int edge, ref WinRect rect)
    {
        if (!GetWindowRect(hwnd, out var window) || !GetClientRect(hwnd, out var client))
            return;
        var frameW = (window.Right - window.Left) - (client.Right - client.Left);
        var frameH = (window.Bottom - window.Top) - (client.Bottom - client.Top);
        var dpi = VisualTreeHelper.GetDpi(this);
        var chromePx = Chrome.Visibility == Visibility.Visible
            ? (int)Math.Round(Chrome.ActualHeight * dpi.DpiScaleY)
            : 0;
        var width = rect.Right - rect.Left;
        var height = rect.Bottom - rect.Top;
        if (edge is WmszTop or WmszBottom)
        {
            var contentH = Math.Max(2, height - frameH - chromePx);
            var newW = (int)Math.Round(contentH * 16.0 / 9.0) + frameW;
            if (edge == WmszTop)
                rect.Top = rect.Bottom - (contentH + chromePx + frameH);
            else
                rect.Bottom = rect.Top + contentH + chromePx + frameH;
            rect.Right = rect.Left + newW;
            return;
        }
        var contentW = Math.Max(2, width - frameW);
        var newH = (int)Math.Round(contentW * 9.0 / 16.0) + chromePx + frameH;
        if (edge is WmszTopLeft or WmszTopRight)
            rect.Top = rect.Bottom - newH;
        else
            rect.Bottom = rect.Top + newH;
    }

    private const int WmSizing = 0x0214;
    private const int WmszTop = 3;
    private const int WmszTopLeft = 4;
    private const int WmszTopRight = 5;
    private const int WmszBottom = 6;

    [StructLayout(LayoutKind.Sequential)]
    private struct WinRect
    {
        public int Left;
        public int Top;
        public int Right;
        public int Bottom;
    }

    [LibraryImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static partial bool GetWindowRect(nint hwnd, out WinRect rect);

    [LibraryImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static partial bool GetClientRect(nint hwnd, out WinRect rect);
}
