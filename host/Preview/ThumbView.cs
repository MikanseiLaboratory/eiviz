using System.Runtime.InteropServices;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using Eiviz.Host.Interop;

namespace Eiviz.Host.Preview;

public sealed class ThumbView : UserControl
{
    private readonly Image _image = new()
    {
        Stretch = Stretch.Uniform,
        HorizontalAlignment = HorizontalAlignment.Stretch,
        VerticalAlignment = VerticalAlignment.Stretch
    };
    private WriteableBitmap? _bitmap;
    private byte[] _scratch = [];
    private ulong _sourceId;
    private uint _width = 170;
    private uint _height = 90;
    private uint _interval = 3;
    private bool _wanted;
    private bool _subscribed;
    private int _lastBytes;
    private uint _lastW;
    private uint _lastH;
    private ulong _lastHash;

    public ThumbView()
    {
        Background = Brushes.Black;
        Content = _image;
        Loaded += (_, _) =>
        {
            if (_wanted)
                Subscribe();
        };
        Unloaded += (_, _) => Unsubscribe();
    }

    public ulong SourceId => _sourceId;

    public void Bind(ulong sourceId, uint width, uint height, uint interval)
    {
        var w = Math.Clamp(width, 2u, 960u);
        var h = Math.Clamp(height, 2u, 540u);
        var frames = Math.Clamp(interval, 1u, 8u);
        if (_sourceId == sourceId && _width == w && _height == h && _interval == frames && _subscribed)
            return;
        var previous = _sourceId;
        _sourceId = sourceId;
        _width = w;
        _height = h;
        _interval = frames;
        if (_wanted)
            Subscribe();
        else if (previous != 0 && previous != sourceId)
            ThumbSubscriptions.Release(this, previous);
    }

    public void SetPresentInterval(uint interval)
    {
        var frames = Math.Clamp(interval, 1u, 8u);
        if (_interval == frames)
            return;
        _interval = frames;
        if (_wanted)
            Subscribe();
    }

    public void SetWanted(bool wanted)
    {
        if (_wanted == wanted && _subscribed == wanted)
            return;
        _wanted = wanted;
        if (wanted)
            Subscribe();
        else
            Unsubscribe();
    }

    public void RefreshSize(uint width, uint height)
    {
        Bind(_sourceId, width, height, _interval);
    }

    internal void Poll()
    {
        if (!_subscribed || _sourceId == 0)
            return;
        const int maxBytes = 960 * 540 * 4;
        if (_scratch.Length < maxBytes)
            _scratch = new byte[maxBytes];
        uint w = 0, h = 0, stride = 0;
        int n;
        unsafe
        {
            fixed (byte* ptr = _scratch)
                n = MixerNative.ThumbRead(_sourceId, ptr, (nuint)_scratch.Length, &w, &h, &stride);
        }
        if (n <= 0 || w == 0 || h == 0)
            return;
        var hash = HashPixels(_scratch, n);
        if (_bitmap is not null && n == _lastBytes && w == _lastW && h == _lastH && hash == _lastHash)
            return;
        _lastBytes = n;
        _lastW = w;
        _lastH = h;
        _lastHash = hash;
        if (_bitmap is null || _bitmap.PixelWidth != w || _bitmap.PixelHeight != h)
        {
            _bitmap = new WriteableBitmap((int)w, (int)h, 96, 96, PixelFormats.Bgra32, null);
            _image.Source = _bitmap;
        }
        _bitmap.Lock();
        try
        {
            var destStride = _bitmap.BackBufferStride;
            var rowBytes = (int)Math.Min(stride, (uint)destStride);
            for (var y = 0; y < h; y++)
                Marshal.Copy(_scratch, y * (int)stride, _bitmap.BackBuffer + y * destStride, rowBytes);
            _bitmap.AddDirtyRect(new Int32Rect(0, 0, (int)w, (int)h));
        }
        finally
        {
            _bitmap.Unlock();
        }
    }

    private static ulong HashPixels(byte[] data, int length)
    {
        ulong hash = 14695981039346656037UL;
        var step = Math.Max(1, length / 64);
        for (var i = 0; i < length; i += step)
            hash = (hash ^ data[i]) * 1099511628211UL;
        if (length > 0)
            hash ^= data[length - 1];
        return hash ^ (uint)length;
    }

    private void Subscribe()
    {
        if (_sourceId == 0 || _width == 0 || _height == 0)
            return;
        ThumbSubscriptions.Retain(this, _sourceId, _width, _height, _interval);
        _subscribed = true;
        ThumbPump.Register(this);
    }

    public void Unsubscribe()
    {
        ThumbPump.Unregister(this);
        if (_sourceId != 0)
            ThumbSubscriptions.Release(this, _sourceId);
        _subscribed = false;
    }
}

internal static class ThumbSubscriptions
{
    private readonly record struct Sub(ThumbView View, uint Width, uint Height, uint Interval);

    private static readonly Dictionary<ulong, List<Sub>> Live = [];

    public static void Retain(ThumbView view, ulong sourceId, uint width, uint height, uint interval)
    {
        var stale = new List<ulong>();
        foreach (var (id, list) in Live)
        {
            if (id == sourceId)
                continue;
            if (list.RemoveAll(item => ReferenceEquals(item.View, view)) > 0)
                stale.Add(id);
        }
        foreach (var id in stale)
            Push(id);
        if (!Live.TryGetValue(sourceId, out var current))
        {
            current = [];
            Live[sourceId] = current;
        }
        var same = current.Exists(item =>
            ReferenceEquals(item.View, view)
            && item.Width == width
            && item.Height == height
            && item.Interval == interval);
        current.RemoveAll(item => ReferenceEquals(item.View, view));
        current.Add(new Sub(view, width, height, interval));
        if (!same)
            Push(sourceId);
    }

    public static void Release(ThumbView view, ulong sourceId)
    {
        if (!Live.TryGetValue(sourceId, out var current))
            return;
        current.RemoveAll(item => ReferenceEquals(item.View, view));
        Push(sourceId);
        if (current.Count == 0)
            Live.Remove(sourceId);
    }

    private static void Push(ulong sourceId)
    {
        if (!Live.TryGetValue(sourceId, out var current) || current.Count == 0)
        {
            MixerNative.ThumbSet(sourceId, 0, 0, 0);
            return;
        }
        uint width = 2, height = 2, interval = 8;
        foreach (var item in current)
        {
            width = Math.Max(width, item.Width);
            height = Math.Max(height, item.Height);
            interval = Math.Min(interval, item.Interval);
        }
        MixerNative.ThumbSet(sourceId, width, height, interval);
    }
}

internal static class ThumbPump
{
    private static readonly List<WeakReference<ThumbView>> Views = [];
    private static bool _hooked;

    public static void Register(ThumbView view)
    {
        Unregister(view);
        Views.Add(new WeakReference<ThumbView>(view));
        if (_hooked)
            return;
        CompositionTarget.Rendering += OnRender;
        _hooked = true;
    }

    public static void Unregister(ThumbView view)
    {
        Views.RemoveAll(item => !item.TryGetTarget(out var live) || ReferenceEquals(live, view));
        if (Views.Count == 0 && _hooked)
        {
            CompositionTarget.Rendering -= OnRender;
            _hooked = false;
        }
    }

    private static void OnRender(object? sender, EventArgs e)
    {
        for (var i = Views.Count - 1; i >= 0; i--)
        {
            if (!Views[i].TryGetTarget(out var view))
            {
                Views.RemoveAt(i);
                continue;
            }
            view.Poll();
        }
    }
}

internal static class ThumbViewport
{
    public static bool Intersects(FrameworkElement element, ScrollViewer? scroll)
    {
        if (scroll is null || !element.IsVisible || element.ActualWidth <= 0 || element.ActualHeight <= 0)
            return false;
        if (PresentationSource.FromVisual(element) is null)
            return false;
        try
        {
            var bounds = element.TransformToAncestor(scroll)
                .TransformBounds(new System.Windows.Rect(0, 0, element.ActualWidth, element.ActualHeight));
            var view = new System.Windows.Rect(0, 0, scroll.ViewportWidth, scroll.ViewportHeight);
            if (view.Width <= 0 || view.Height <= 0)
                view = new System.Windows.Rect(0, 0, Math.Max(scroll.ActualWidth, 1), Math.Max(scroll.ActualHeight, 1));
            return bounds.IntersectsWith(view);
        }
        catch (InvalidOperationException)
        {
            return false;
        }
    }
}
