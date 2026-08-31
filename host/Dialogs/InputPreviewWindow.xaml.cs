using System.Runtime.InteropServices;
using System.Windows;
using System.Windows.Input;
using System.Windows.Interop;
using System.Windows.Media;
using System.Windows.Threading;
using Eiviz.Host.Interop;

namespace Eiviz.Host.Dialogs;

public partial class InputPreviewWindow : Window
{
    private readonly ulong _monitorId;
    private readonly ulong _sourceId;
    private readonly double _ratioW;
    private readonly double _ratioH;
    private bool _fullscreen;
    private WindowStyle _savedStyle;
    private WindowState _savedState;
    private ResizeMode _savedResize;

    public ulong SourceId => _sourceId;

    public InputPreviewWindow(InputEntry input, ulong monitorId, uint ratioWidth, uint ratioHeight)
        : this(input.Name, input.Id, monitorId, ratioWidth, ratioHeight)
    {
    }

    public InputPreviewWindow(string name, ulong sourceId, ulong monitorId, uint ratioWidth, uint ratioHeight)
    {
        InitializeComponent();
        _monitorId = monitorId;
        _sourceId = sourceId;
        _ratioW = Math.Max(1, ratioWidth);
        _ratioH = Math.Max(1, ratioHeight);
        Title = name;
        TitleText.Text = name;
        VideoAspect.RatioWidth = _ratioW;
        VideoAspect.RatioHeight = _ratioH;
        SourceInitialized += (_, _) =>
        {
            if (PresentationSource.FromVisual(this) is HwndSource source)
                source.AddHook(WndProc);
        };
        Loaded += (_, _) =>
        {
            SnapWindowToAspect();
            PreviewHost.RetargetMonitor(_monitorId, _sourceId);
            MixerNative.SetMonitorPresentInterval(_monitorId, 1);
        };
        Closed += (_, _) => PreviewHost.ReleaseNative();
    }

    public void SetTitle(string name)
    {
        Title = name;
        TitleText.Text = name;
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
        Dispatcher.BeginInvoke(PreviewHost.RefreshSize, DispatcherPriority.Loaded);
    }

    private void SnapWindowToAspect()
    {
        if (_fullscreen)
            return;
        var nc = Math.Max(0, Height - ActualHeight);
        var chrome = Chrome.Visibility == Visibility.Visible ? Chrome.ActualHeight : 0;
        Height = chrome + Math.Max(2, ActualWidth) * (_ratioH / _ratioW) + nc;
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
        var aspect = _ratioW / _ratioH;
        if (edge is WmszTop or WmszBottom)
        {
            var contentH = Math.Max(2, height - frameH - chromePx);
            var newW = (int)Math.Round(contentH * aspect) + frameW;
            if (edge == WmszTop)
                rect.Top = rect.Bottom - (contentH + chromePx + frameH);
            else
                rect.Bottom = rect.Top + contentH + chromePx + frameH;
            rect.Right = rect.Left + newW;
            return;
        }
        var contentW = Math.Max(2, width - frameW);
        var newH = (int)Math.Round(contentW / aspect) + chromePx + frameH;
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
