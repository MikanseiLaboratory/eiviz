using System.Windows;
using System.Windows.Input;
using System.Windows.Media;
using Eiviz.Host.Interop;

namespace Eiviz.Host.Dialogs;

/// <summary>
/// Draws the recorded waveform of one angle, centred on where the channel is now:
/// everything left of the middle already happened, everything right of it is still
/// ahead. Each column is the min/max of the audio buckets it covers, which is how a
/// replay operator spots a quiet gap to cut on.
/// </summary>
internal sealed class ReplayWaveform : FrameworkElement
{
    private float[] _mins = [];
    private float[] _maxes = [];
    private uint _buckets;
    /// Buckets the mixer had to evict, so bucket 0 here is `_dropped` in absolute
    /// numbering.
    private uint _dropped;
    private uint _rate = 20;
    private double _fps = 60.0;
    private ulong _inFrame;
    private ulong _outFrame;
    private ulong _positionFrame;
    private bool _hasIn;
    private bool _hasOut;

    private static readonly Brush Background = new SolidColorBrush(Color.FromRgb(0x18, 0x18, 0x1C));
    private static readonly Pen Wave = new(new SolidColorBrush(Color.FromRgb(0x6C, 0xD4, 0x8C)), 1);
    private static readonly Pen Mark = new(new SolidColorBrush(Color.FromRgb(0xFF, 0xD1, 0x54)), 1);
    private static readonly Pen Playhead = new(new SolidColorBrush(Color.FromRgb(0xE0, 0x60, 0x60)), 2);
    /// <summary>The band between the marks: the clip being cut, at a glance.</summary>
    private static readonly Brush Range = new SolidColorBrush(Color.FromRgb(0x33, 0x2C, 0x12));

    /// <summary>Raised while the operator drags across the waveform.</summary>
    public event Action<ulong>? SeekRequested;

    /// <summary>Shift-click: mark In on the frame under the pointer.</summary>
    public event Action<ulong>? InRequested;

    /// <summary>Ctrl-click: mark Out on the frame under the pointer.</summary>
    public event Action<ulong>? OutRequested;

    public ReplayWaveform()
    {
        ClipToBounds = true;
        MinHeight = 56;
        // The waveform is the seek bar: an operator who wants a different frame points
        // at it rather than nudging with a jog wheel.
        Cursor = Cursors.Hand;
        Focusable = true;
    }

    /// <summary>
    /// Seconds drawn on each side of the playhead. Ten seconds reads like a jog wheel
    /// and still shows enough of the recording to find a cut.
    /// </summary>
    public double WindowSeconds { get; set; } = 10.0;

    public ulong InFrame
    {
        get => _inFrame;
        set { _inFrame = value; InvalidateVisual(); }
    }

    public ulong OutFrame
    {
        get => _outFrame;
        set { _outFrame = value; InvalidateVisual(); }
    }

    public ulong PositionFrame
    {
        get => _positionFrame;
        set { _positionFrame = value; InvalidateVisual(); }
    }

    public bool HasIn
    {
        get => _hasIn;
        set { _hasIn = value; InvalidateVisual(); }
    }

    public bool HasOut
    {
        get => _hasOut;
        set { _hasOut = value; InvalidateVisual(); }
    }

    /// Pulls the current waveform for an angle and repaints. Returns the number of
    /// buckets the mixer holds for it.
    public uint Refresh(uint angleId, double fps)
    {
        _fps = fps <= 0 ? 60.0 : fps;
        _rate = MixerNative.ReplayPeakRate();
        uint dropped = 0;
        uint count;
        unsafe
        {
            count = MixerNative.ReplayPeakCount(angleId, &dropped);
            if (_mins.Length < count)
            {
                _mins = new float[count];
                _maxes = new float[count];
            }
            if (count > 0)
            {
                fixed (float* mins = _mins)
                fixed (float* maxes = _maxes)
                {
                    var copied = MixerNative.ReplayCopyPeaks(angleId, 0, mins, maxes, count);
                    count = copied > 0 ? (uint)copied : 0;
                }
            }
        }
        _dropped = dropped;
        _buckets = count;
        InvalidateVisual();
        return _buckets;
    }

    /// The newest frame the waveform knows about.
    private double LatestFrame => _rate == 0 ? 0.0 : (_dropped + _buckets) * _fps / _rate;

    /// The frame drawn at the middle of the control: where the channel is, or the live
    /// edge when nothing is playing.
    private double CenterFrame =>
        _positionFrame != 0 ? Math.Min(_positionFrame, Math.Max(0.0, LatestFrame)) : LatestFrame;

    /// The frame under an x coordinate. Left of the middle is the past, right is what
    /// has not played yet.
    public ulong FrameAt(double x) =>
        FrameAtWindow(
            ActualWidth <= 0 ? 0.5 : Math.Clamp(x / ActualWidth, 0.0, 1.0),
            CenterFrame,
            Math.Max(1.0, WindowSeconds * _fps),
            LatestFrame);

    /// Maps a position across the drawn window to a recording frame: `ratio` 0 is the
    /// left edge (past), 1 the right edge (future). Clamped to what has been recorded.
    internal static ulong FrameAtWindow(double ratio, double center, double half, double latest)
    {
        var clamped = Math.Clamp(ratio, 0.0, 1.0);
        var frame = center - half + clamped * half * 2.0;
        var bounded = Math.Clamp(frame, 0.0, Math.Max(0.0, latest));
        return (ulong)Math.Max(0, bounded);
    }

    /// <summary>How close to a mark a press has to be, in pixels, to grab it.</summary>
    private const double GrabRadius = 6.0;

    /// <summary>Which mark the pointer is dragging, if any.</summary>
    private int _dragging;

    /// <summary>Pixel position of a frame in the window the waveform is showing.</summary>
    private double PixelOf(ulong frame)
    {
        var center = CenterFrame;
        var half = Math.Max(1.0, WindowSeconds * _fps);
        var framesPerColumn = (2 * half) / Math.Max(1.0, ActualWidth);
        return (frame - (center - half)) / framesPerColumn;
    }

    /// <summary>The mark under a horizontal position, or 0 for none.</summary>
    private int MarkAt(double x)
    {
        if (_hasIn && Math.Abs(PixelOf(_inFrame) - x) <= GrabRadius)
            return 1;
        if (_hasOut && Math.Abs(PixelOf(_outFrame) - x) <= GrabRadius)
            return 2;
        return 0;
    }

    protected override void OnMouseLeftButtonDown(MouseButtonEventArgs e)
    {
        base.OnMouseLeftButtonDown(e);
        var position = e.GetPosition(this);
        // A press on a mark grabs it, so the range can be trimmed where it is visible. A press
        // on the mark also seeks there, which is how an operator checks a cut point.
        var grabbed = MarkAt(position.X);
        if (grabbed != 0 && (Keyboard.Modifiers & ModifierKeys.Shift) == 0)
        {
            _dragging = grabbed;
            CaptureMouse();
            Focus();
            SeekRequested?.Invoke(grabbed == 1 ? _inFrame : _outFrame);
            e.Handled = true;
            return;
        }
        var frame = FrameAt(position.X);
        // A plain click moves the playhead, which is how an operator finds a quiet
        // moment. Shift marks In there and Ctrl marks Out there, so a cut can be struck
        // straight off the waveform: the waveform is where the quiet moment is visible,
        // and the frame under the pointer is the one being looked at. Without this an
        // operator had to seek, look away to the buttons, and press one; the playhead
        // kept running in between, so the cut landed somewhere other than the gap.
        if ((Keyboard.Modifiers & ModifierKeys.Shift) != 0)
        {
            InRequested?.Invoke(frame);
            e.Handled = true;
            return;
        }
        if ((Keyboard.Modifiers & ModifierKeys.Control) != 0)
        {
            OutRequested?.Invoke(frame);
            e.Handled = true;
            return;
        }
        CaptureMouse();
        Focus();
        SeekRequested?.Invoke(frame);
        e.Handled = true;
    }

    protected override void OnMouseMove(MouseEventArgs e)
    {
        base.OnMouseMove(e);
        if (!IsMouseCaptured)
        {
            // The pointer over a mark is a handle, so it says so.
            Cursor = MarkAt(e.GetPosition(this).X) != 0 ? Cursors.SizeWE : Cursors.Arrow;
            return;
        }
        var frame = FrameAt(e.GetPosition(this).X);
        if (_dragging == 1)
        {
            // In cannot pass Out: a clip with a negative length is not a clip.
            if (!_hasOut || frame < _outFrame)
                InRequested?.Invoke(frame);
            e.Handled = true;
            return;
        }
        if (_dragging == 2)
        {
            if (!_hasIn || frame > _inFrame)
                OutRequested?.Invoke(frame);
            e.Handled = true;
            return;
        }
        SeekRequested?.Invoke(frame);
        e.Handled = true;
    }

    protected override void OnMouseLeftButtonUp(MouseButtonEventArgs e)
    {
        base.OnMouseLeftButtonUp(e);
        _dragging = 0;
        if (IsMouseCaptured)
            ReleaseMouseCapture();
        e.Handled = true;
    }

    /// Bucket index of a video frame, in absolute numbering.
    public long BucketOf(ulong frame) =>
        _rate == 0 ? 0 : (long)(frame * _rate / _fps);

    protected override void OnRender(DrawingContext dc)
    {
        var width = ActualWidth;
        var height = ActualHeight;
        if (width < 2 || height < 4)
            return;
        dc.DrawRectangle(Background, null, new System.Windows.Rect(0, 0, width, height));
        var middle = height / 2;
        var columns = (int)width;

        if (_buckets > 0)
        {
            var center = CenterFrame;
            var half = Math.Max(1.0, WindowSeconds * _fps);
            var from = center - half;
            var to = center + half;
            var framesPerColumn = (to - from) / columns;
            var first = (long)_dropped;
            var last = first + _buckets;
            for (var column = 0; column < columns; column++)
            {
                var frameFrom = from + column * framesPerColumn;
                var frameTo = frameFrom + framesPerColumn;
                var bucketFrom = Math.Max(first, BucketOfFrame(frameFrom));
                var bucketTo = Math.Min(last, BucketOfFrame(frameTo) + 1);
                if (bucketTo <= bucketFrom)
                    continue;
                var low = float.MaxValue;
                var high = float.MinValue;
                for (var bucket = bucketFrom; bucket < bucketTo; bucket++)
                {
                    var index = (int)(bucket - first);
                    if (index < 0 || index >= _buckets)
                        continue;
                    low = Math.Min(low, _mins[index]);
                    high = Math.Max(high, _maxes[index]);
                }
                if (low > high)
                    continue;
                var scale = middle - 2;
                var y1 = middle - high * scale;
                var y2 = middle - low * scale;
                dc.DrawLine(Wave, new Point(column + 0.5, y1), new Point(column + 0.5, y2));
            }
            // The clip being cut, shaded. The marks alone are two hairlines; the band is what
            // says "this stretch is my highlight" without the operator tracing either one.
            if (_hasIn && _hasOut && _outFrame > _inFrame)
            {
                var left = Math.Max(from, (double)_inFrame);
                var right = Math.Min(to, (double)_outFrame);
                if (right > left)
                {
                    var x0 = (left - from) / framesPerColumn;
                    var x1 = (right - from) / framesPerColumn;
                    dc.DrawRectangle(Range, null, new System.Windows.Rect(x0, 0, x1 - x0, height));
                }
            }
            DrawMarker(dc, Mark, _inFrame, from, to, height, _hasIn);
            DrawMarker(dc, Mark, _outFrame, from, to, height, _hasOut);
            DrawMarker(dc, Playhead, _positionFrame, from, to, height, _positionFrame != 0);
        }

        dc.DrawLine(new Pen(new SolidColorBrush(Color.FromRgb(0x40, 0x40, 0x48)), 1),
            new Point(0, middle), new Point(width, middle));
    }

    private long BucketOfFrame(double frame) =>
        _rate == 0 ? 0 : (long)(frame * _rate / Math.Max(1.0, _fps));

    private void DrawMarker(
        DrawingContext dc,
        Pen pen,
        ulong frame,
        double from,
        double to,
        double height,
        bool visible)
    {
        if (!visible || to <= from || frame < from || frame > to)
            return;
        var x = (frame - from) * ActualWidth / (to - from);
        dc.DrawLine(pen, new Point(x, 0), new Point(x, height));
    }
}
