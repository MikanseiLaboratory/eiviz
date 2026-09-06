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
    private bool _loggedAttachFailure;
    private uint _sizedWidth;
    private uint _sizedHeight;
    private bool _syncing;
    private bool _applying;

    public ulong UnitId { get; set; } = 1;
    public uint OutputKind { get; set; } = MixerNative.OutputProgram;
    public ulong MonitorId { get; set; }
    public ulong SourceId { get; set; }
    public bool IsMonitor { get; set; }
    public bool AutoAttach { get; set; } = true;
    public uint PresentInterval { get; set; } = 1;

    public event EventHandler? SurfaceClicked;
    public event EventHandler? SurfaceDoubleClicked;

    public SwapchainHost()
    {
        UseLayoutRounding = true;
        SnapsToDevicePixels = true;
        SizeChanged += (_, _) => SyncNative();
        Loaded += (_, _) => Dispatcher.BeginInvoke(SyncNative, System.Windows.Threading.DispatcherPriority.Loaded);
    }

    protected override HandleRef BuildWindowCore(HandleRef hwndParent)
    {
        var instance = EnsurePreviewClass();
        _hwnd = CreateWindowEx(
            WsExNoRedirectionBitmap,
            PreviewClassName,
            null,
            WsChild | WsVisible | WsClipSiblings | WsClipChildren,
            0, 0, 2, 2,
            hwndParent.Handle,
            nint.Zero,
            instance,
            nint.Zero);
        if (_hwnd == nint.Zero)
            throw new InvalidOperationException($"Could not create preview HWND: {Marshal.GetLastWin32Error()}");
        return new HandleRef(this, _hwnd);
    }

    protected override nint WndProc(nint hwnd, int msg, nint wParam, nint lParam, ref bool handled)
    {
        if (msg == WmEraseBkgnd)
        {
            handled = true;
            return 1;
        }
        if (msg == WmSize && _attached && !_applying)
            ApplySize();
        if (msg == WmLButtonUp)
            SurfaceClicked?.Invoke(this, EventArgs.Empty);
        else if (msg == WmLButtonDblClk)
            SurfaceDoubleClicked?.Invoke(this, EventArgs.Empty);
        return base.WndProc(hwnd, msg, wParam, lParam, ref handled);
    }

    protected override void OnWindowPositionChanged(System.Windows.Rect rcBoundingBox)
    {
        base.OnWindowPositionChanged(rcBoundingBox);
        ApplySize();
    }

    private void SyncNative()
    {
        if (_hwnd == nint.Zero || _syncing)
            return;
        _syncing = true;
        try
        {
            UpdateWindowPos();
            ApplySize();
        }
        finally
        {
            _syncing = false;
        }
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

    public bool HasMonitor(ulong monitorId, ulong sourceId) =>
        _attached && IsMonitor && MonitorId == monitorId && SourceId == sourceId;

    public void RetargetMonitor(ulong monitorId, ulong sourceId)
    {
        if (HasMonitor(monitorId, sourceId))
            return;
        DetachNative();
        IsMonitor = true;
        MonitorId = monitorId;
        SourceId = sourceId;
        ApplySize(forceAttach: true);
    }

    public void UpdateMonitorSource(ulong sourceId)
    {
        SourceId = sourceId;
        if (_attached && IsMonitor)
            MixerNative.ThrowIfFailed(MixerNative.SetMonitorSource(MonitorId, sourceId), "Update monitor source");
    }

    public void RefreshSize() => ApplySize();

    private void ApplySize() => ApplySize(forceAttach: false);

    public void ApplyPresentInterval()
    {
        if (!IsMonitor || MonitorId == 0)
            return;
        MixerNative.SetMonitorPresentInterval(MonitorId, Math.Clamp(PresentInterval, 1u, 8u));
    }

    private void ApplySize(bool forceAttach)
    {
        if (_hwnd == nint.Zero || _applying)
            return;
        if (ActualWidth < 8 || ActualHeight < 8)
            return;
        var (layoutWidth, layoutHeight) = PixelSize();
        _applying = true;
        try
        {
            ApplySizeCore(forceAttach, layoutWidth, layoutHeight);
        }
        finally
        {
            _applying = false;
        }
    }

    private void ApplySizeCore(bool forceAttach, uint layoutWidth, uint layoutHeight)
    {
        // Always pin the HWND to the layout pixel size. Skipping this left the
        // native window at the old extent while WPF shrank/grew, so Vulkan WSI
        // scaled the swapchain into the HWND and the preview looked stretched.
        MoveWindow(_hwnd, 0, 0, (int)layoutWidth, (int)layoutHeight, false);
        var (width, height) = ReadClientSize() ?? (layoutWidth, layoutHeight);
        if (!_attached)
        {
            if (!forceAttach && !AutoAttach)
                return;
            if (!FlipBudget.TryBegin(this))
                return;
            try
            {
                if (IsMonitor)
                {
                    if (MonitorId == 0 || SourceId == 0)
                    {
                        FlipBudget.Cancel(this);
                        return;
                    }
                    MixerNative.ThrowIfFailed(
                        MixerNative.AttachMonitor(MonitorId, SourceId, _hwnd, width, height),
                        "Attach source monitor");
                    ApplyPresentInterval();
                }
                else
                    MixerNative.ThrowIfFailed(
                        MixerNative.AttachOutput(UnitId, _hwnd, width, height, OutputKind),
                        "Attach preview surface");
                _attached = true;
                _loggedAttachFailure = false;
                _sizedWidth = width;
                _sizedHeight = height;
            }
            catch (Exception ex)
            {
                FlipBudget.Cancel(this);
                if (!_loggedAttachFailure)
                {
                    _loggedAttachFailure = true;
                    HostLog.WriteException(ex);
                }
            }
            return;
        }

        if (!forceAttach && width == _sizedWidth && height == _sizedHeight)
            return;
        _sizedWidth = width;
        _sizedHeight = height;
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
        _sizedWidth = 0;
        _sizedHeight = 0;
        FlipBudget.End(this);
    }

    private (uint Width, uint Height) PixelSize()
    {
        var dpi = VisualTreeHelper.GetDpi(this);
        var width = (uint)Math.Max(2, Math.Floor(Math.Max(ActualWidth, 2) * dpi.DpiScaleX));
        var height = (uint)Math.Max(2, Math.Floor(Math.Max(ActualHeight, 2) * dpi.DpiScaleY));
        return (width, height);
    }

    private (uint Width, uint Height)? ReadClientSize()
    {
        if (!GetClientRect(_hwnd, out var rect))
            return null;
        var width = (uint)Math.Max(0, rect.Right - rect.Left);
        var height = (uint)Math.Max(0, rect.Bottom - rect.Top);
        if (width < 8 || height < 8)
            return null;
        return (width, height);
    }

    private const string PreviewClassName = "EivizPreview";
    private const int WsChild = 0x40000000;
    private const int WsVisible = 0x10000000;
    private const int WsClipSiblings = 0x04000000;
    private const int WsClipChildren = 0x02000000;
    private const int WsExNoRedirectionBitmap = 0x00200000;
    private const int CsOwndc = 0x0020;
    private const int CsDblClks = 0x0008;
    private const int BlackBrush = 4;
    private const int ErrorClassAlreadyExists = 1410;
    private const int WmEraseBkgnd = 0x0014;
    private const int WmSize = 0x0005;
    private const int WmLButtonUp = 0x0202;
    private const int WmLButtonDblClk = 0x0203;

    private static readonly object ClassLock = new();
    private static nint _previewInstance;

    private static nint EnsurePreviewClass()
    {
        lock (ClassLock)
        {
            if (_previewInstance != nint.Zero)
                return _previewInstance;
            var instance = GetModuleHandle(null);
            var user32 = GetModuleHandle("user32.dll");
            var defProc = GetProcAddress(user32, "DefWindowProcW");
            if (defProc == nint.Zero)
                throw new InvalidOperationException("Could not resolve DefWindowProcW");
            var className = Marshal.StringToHGlobalUni(PreviewClassName);
            try
            {
                var wndClass = new WndClassEx
                {
                    cbSize = (uint)Marshal.SizeOf<WndClassEx>(),
                    style = CsOwndc | CsDblClks,
                    lpfnWndProc = defProc,
                    hInstance = instance,
                    hbrBackground = GetStockObject(BlackBrush),
                    lpszClassName = className
                };
                if (RegisterClassEx(in wndClass) == 0)
                {
                    var err = Marshal.GetLastWin32Error();
                    if (err != ErrorClassAlreadyExists)
                        throw new InvalidOperationException($"Could not register preview HWND class: {err}");
                }
            }
            finally
            {
                Marshal.FreeHGlobal(className);
            }
            _previewInstance = instance;
            return instance;
        }
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct WndClassEx
    {
        public uint cbSize;
        public uint style;
        public nint lpfnWndProc;
        public int cbClsExtra;
        public int cbWndExtra;
        public nint hInstance;
        public nint hIcon;
        public nint hCursor;
        public nint hbrBackground;
        public nint lpszMenuName;
        public nint lpszClassName;
        public nint hIconSm;
    }

    [LibraryImport("user32.dll", EntryPoint = "CreateWindowExW", SetLastError = true, StringMarshalling = StringMarshalling.Utf16)]
    private static partial nint CreateWindowEx(
        int extendedStyle, string className, string? windowName, int style,
        int x, int y, int width, int height, nint parent, nint menu,
        nint instance, nint parameter);

    [LibraryImport("user32.dll", EntryPoint = "RegisterClassExW", SetLastError = true)]
    private static partial ushort RegisterClassEx(in WndClassEx wndClass);

    [LibraryImport("kernel32.dll", EntryPoint = "GetModuleHandleW", SetLastError = true, StringMarshalling = StringMarshalling.Utf16)]
    private static partial nint GetModuleHandle(string? moduleName);

    [LibraryImport("kernel32.dll", EntryPoint = "GetProcAddress", StringMarshalling = StringMarshalling.Utf8)]
    private static partial nint GetProcAddress(nint module, string procName);

    [LibraryImport("gdi32.dll")]
    private static partial nint GetStockObject(int index);

    [StructLayout(LayoutKind.Sequential)]
    private struct WinRect
    {
        public int Left;
        public int Top;
        public int Right;
        public int Bottom;
    }

    [LibraryImport("user32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static partial bool GetClientRect(nint hwnd, out WinRect rect);

    [LibraryImport("user32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static partial bool MoveWindow(nint hwnd, int x, int y, int width, int height, [MarshalAs(UnmanagedType.Bool)] bool repaint);

    [LibraryImport("user32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static partial bool DestroyWindow(nint hwnd);
}
