using System.Runtime.InteropServices;
using System.Windows;
using System.Windows.Interop;
using System.Windows.Media;
using Eiviz.Host.Interop;

namespace Eiviz.Host.Preview;

internal sealed partial class SwapchainHost : HwndHost
{
    private nint _hwnd;
    private bool _attached;

    public ulong UnitId { get; set; } = 1;
    public uint OutputKind { get; set; } = MixerNative.OutputProgram;
    public ulong MonitorId { get; set; }
    public ulong SourceId { get; set; }
    public bool IsMonitor { get; set; }

    public event EventHandler? SurfaceClicked;
    public event EventHandler? SurfaceDoubleClicked;

    public SwapchainHost()
    {
        SizeChanged += (_, _) => ApplySize();
        Loaded += (_, _) => Dispatcher.BeginInvoke(ApplySize, System.Windows.Threading.DispatcherPriority.Loaded);
    }

    protected override HandleRef BuildWindowCore(HandleRef hwndParent)
    {
        _hwnd = CreateWindowEx(
            0,
            "STATIC",
            null,
            WsChild | WsVisible | WsClipSiblings | WsClipChildren,
            0, 0, 2, 2,
            hwndParent.Handle,
            nint.Zero,
            nint.Zero,
            nint.Zero);
        if (_hwnd == nint.Zero)
            throw new InvalidOperationException($"Could not create preview HWND: {Marshal.GetLastWin32Error()}");
        return new HandleRef(this, _hwnd);
    }

    protected override nint WndProc(nint hwnd, int msg, nint wParam, nint lParam, ref bool handled)
    {
        if (msg == WmLButtonUp)
            SurfaceClicked?.Invoke(this, EventArgs.Empty);
        else if (msg == WmLButtonDblClk)
            SurfaceDoubleClicked?.Invoke(this, EventArgs.Empty);
        return base.WndProc(hwnd, msg, wParam, lParam, ref handled);
    }

    protected override void OnRenderSizeChanged(SizeChangedInfo sizeInfo)
    {
        base.OnRenderSizeChanged(sizeInfo);
        ApplySize();
    }

    protected override void DestroyWindowCore(HandleRef hwnd)
    {
        DetachNative();
        if (!DestroyWindow(hwnd.Handle))
            throw new InvalidOperationException($"Could not destroy preview HWND: {Marshal.GetLastWin32Error()}");
        _hwnd = nint.Zero;
    }

    public void ReleaseNative() => DetachNative();

    public void RetargetUnit(ulong unitId, uint kind)
    {
        DetachNative();
        IsMonitor = false;
        UnitId = unitId;
        OutputKind = kind;
        ApplySize();
    }

    public void RetargetMonitor(ulong monitorId, ulong sourceId)
    {
        DetachNative();
        IsMonitor = true;
        MonitorId = monitorId;
        SourceId = sourceId;
        ApplySize();
    }

    public void UpdateMonitorSource(ulong sourceId)
    {
        SourceId = sourceId;
        if (_attached && IsMonitor)
            MixerNative.ThrowIfFailed(MixerNative.SetMonitorSource(MonitorId, sourceId), "Update monitor source");
    }

    public void RefreshSize() => ApplySize();

    private void ApplySize()
    {
        if (_hwnd == nint.Zero)
            return;
        var (width, height) = PixelSize();
        MoveWindow(_hwnd, 0, 0, (int)width, (int)height, true);
        if (!_attached)
        {
            if (IsMonitor)
                MixerNative.ThrowIfFailed(
                    MixerNative.AttachMonitor(MonitorId, SourceId, _hwnd, width, height),
                    "Attach source monitor");
            else
                MixerNative.ThrowIfFailed(
                    MixerNative.AttachOutput(UnitId, _hwnd, width, height, OutputKind),
                    "Attach DX12 surface");
            _attached = true;
            return;
        }

        if (IsMonitor)
            MixerNative.ResizeMonitor(MonitorId, width, height);
        else
            MixerNative.ResizeOutput(UnitId, OutputKind, _hwnd, width, height);
    }

    private void DetachNative()
    {
        if (!_attached)
            return;
        if (IsMonitor)
            MixerNative.DetachMonitor(MonitorId);
        else
            MixerNative.DetachOutput(UnitId, OutputKind, _hwnd);
        _attached = false;
    }

    private (uint Width, uint Height) PixelSize()
    {
        var dpi = VisualTreeHelper.GetDpi(this);
        var width = (uint)Math.Max(2, Math.Round(Math.Max(ActualWidth, 2) * dpi.DpiScaleX));
        var height = (uint)Math.Max(2, Math.Round(Math.Max(ActualHeight, 2) * dpi.DpiScaleY));
        return (width, height);
    }

    private const int WsChild = 0x40000000;
    private const int WsVisible = 0x10000000;
    private const int WsClipSiblings = 0x04000000;
    private const int WsClipChildren = 0x02000000;
    private const int WmLButtonUp = 0x0202;
    private const int WmLButtonDblClk = 0x0203;

    [LibraryImport("user32.dll", EntryPoint = "CreateWindowExW", SetLastError = true, StringMarshalling = StringMarshalling.Utf16)]
    private static partial nint CreateWindowEx(
        int extendedStyle, string className, string? windowName, int style,
        int x, int y, int width, int height, nint parent, nint menu,
        nint instance, nint parameter);

    [LibraryImport("user32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static partial bool MoveWindow(nint hwnd, int x, int y, int width, int height, [MarshalAs(UnmanagedType.Bool)] bool repaint);

    [LibraryImport("user32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static partial bool DestroyWindow(nint hwnd);
}
