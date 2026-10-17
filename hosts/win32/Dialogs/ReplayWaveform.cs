using System.Windows;
using System.Windows.Media;
using Eiviz.Host.Interop;

namespace Eiviz.Host.Dialogs;

/// <summary>
/// Draws the recorded waveform of one angle. Each column is the min/max of the
/// audio buckets it covers, which is how a replay operator spots a quiet gap to
/// cut on.
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

    public ReplayWaveform()
    {
        ClipToBounds = true;
        MinHeight = 56;
    }

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
            var first = (long)_dropped;
            var last = first + _buckets;
            for (var column = 0; column < columns; column++)
            {
                var from = first + (last - first) * column / columns;
                var to = first + (last - first) * (column + 1) / columns;
                if (to <= from)
                    to = from + 1;
                var low = float.MaxValue;
                var high = float.MinValue;
                for (var bucket = from; bucket < to; bucket++)
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
            DrawMarker(dc, Mark, _inFrame, first, last, height, _hasIn);
            DrawMarker(dc, Mark, _outFrame, first, last, height, _hasOut);
            DrawMarker(dc, Playhead, _positionFrame, first, last, height, _positionFrame != 0);
        }

        dc.DrawLine(new Pen(new SolidColorBrush(Color.FromRgb(0x40, 0x40, 0x48)), 1),
            new Point(0, middle), new Point(width, middle));
    }

    private void DrawMarker(
        DrawingContext dc,
        Pen pen,
        ulong frame,
        long first,
        long last,
        double height,
        bool visible)
    {
        if (!visible || last <= first)
            return;
        var bucket = BucketOf(frame);
        if (bucket < first || bucket > last)
            return;
        var x = (bucket - first) * ActualWidth / (last - first);
        dc.DrawLine(pen, new Point(x, 0), new Point(x, height));
    }
}
