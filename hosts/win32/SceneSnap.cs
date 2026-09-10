using System.Windows;
using System.Windows.Media;

namespace Eiviz.Host;

internal static class SceneSnap
{
    public const double EngagePixels = 6;
    public const double ReleasePixels = 12;

    public readonly record struct Box(float X, float Y, float Width, float Height, bool Hidden, bool Self);

    public static bool TrySnapMoveAxis(
        float start,
        float size,
        IReadOnlyList<Box> boxes,
        bool horizontal,
        float threshold,
        out float snapped)
    {
        var delta = BestDelta(
            [start, start + size * 0.5f, start + size],
            Axis(boxes, horizontal),
            threshold,
            out var found);
        snapped = start + delta;
        return found;
    }

    public static float LatchMoveAxis(
        float raw,
        float size,
        IReadOnlyList<Box> boxes,
        bool horizontal,
        double renderedLength,
        ref float? latched)
    {
        var pixels = Math.Max(renderedLength, 1);
        if (latched is float target)
        {
            if (Math.Abs(raw - target) * pixels <= ReleasePixels)
                return target;
            latched = null;
        }

        var threshold = (float)(EngagePixels / pixels);
        if (TrySnapMoveAxis(raw, size, boxes, horizontal, threshold, out var snapped))
        {
            latched = snapped;
            return snapped;
        }
        return raw;
    }

    public static Size RenderedSize(Visual ancestor, FrameworkElement canvas)
    {
        var transform = canvas.TransformToAncestor(ancestor);
        var topLeft = transform.Transform(new Point(0, 0));
        var bottomRight = transform.Transform(new Point(canvas.Width, canvas.Height));
        return new Size(
            Math.Abs(bottomRight.X - topLeft.X),
            Math.Abs(bottomRight.Y - topLeft.Y));
    }

    public static void SnapResize(ref float width, ref float height, float x, float y, bool sizeLinked, IReadOnlyList<Box> boxes, float xThreshold, float yThreshold)
    {
        var ratio = height / Math.Max(width, 0.0001f);
        var xs = Axis(boxes, horizontal: true);
        var right = SnapPoint(x + width, xs, xThreshold);
        width = Math.Max(0.02f, right - x);
        if (sizeLinked)
        {
            height = Math.Max(0.02f, width * ratio);
            return;
        }
        var ys = Axis(boxes, horizontal: false);
        var bottom = SnapPoint(y + height, ys, yThreshold);
        height = Math.Max(0.02f, bottom - y);
    }

    private static float SnapPoint(float value, List<float> guides, float threshold)
    {
        return value + BestDelta([value], guides, threshold, out _);
    }

    private static float BestDelta(float[] points, List<float> guides, float threshold, out bool found)
    {
        var best = 0f;
        var bestAbs = threshold;
        found = false;
        foreach (var point in points)
        {
            foreach (var guide in guides)
            {
                var delta = guide - point;
                var abs = Math.Abs(delta);
                if (abs <= bestAbs)
                {
                    bestAbs = abs;
                    best = delta;
                    found = true;
                }
            }
        }
        return found ? best : 0f;
    }

    private static List<float> Axis(IReadOnlyList<Box> boxes, bool horizontal)
    {
        var values = new List<float> { 0f, 0.5f, 1f };
        foreach (var box in boxes)
        {
            if (box.Hidden || box.Self)
                continue;
            if (horizontal)
            {
                values.Add(box.X);
                values.Add(box.X + box.Width * 0.5f);
                values.Add(box.X + box.Width);
            }
            else
            {
                values.Add(box.Y);
                values.Add(box.Y + box.Height * 0.5f);
                values.Add(box.Y + box.Height);
            }
        }
        return values;
    }
}
