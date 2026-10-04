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
        var left = x;
        var top = y;
        SnapResize(ref left, ref top, ref width, ref height, moveLeft: false, moveTop: false, sizeLinked, boxes, xThreshold, yThreshold);
    }

    /// <summary>
    /// Snaps the edges that are moving. A size can match the frame (1) or another
    /// box even when that edge sits outside the frame.
    /// </summary>
    public static void SnapResize(
        ref float x,
        ref float y,
        ref float width,
        ref float height,
        bool moveLeft,
        bool moveTop,
        bool sizeLinked,
        IReadOnlyList<Box> boxes,
        float xThreshold,
        float yThreshold)
    {
        var ratio = height / Math.Max(width, 0.0001f);
        var right = x + width;
        var bottom = y + height;
        if (moveLeft)
            x = ChooseEdge(x, right, Sizes(boxes, horizontal: true), Axis(boxes, horizontal: true), xThreshold, anchorIsEnd: true);
        else
            right = ChooseEdge(right, x, Sizes(boxes, horizontal: true), Axis(boxes, horizontal: true), xThreshold, anchorIsEnd: false);
        width = Math.Max(0.02f, right - x);
        if (sizeLinked)
        {
            var nextHeight = Math.Max(0.02f, width * ratio);
            if (moveTop)
                y = bottom - nextHeight;
            height = nextHeight;
            return;
        }
        if (moveTop)
            y = ChooseEdge(y, bottom, Sizes(boxes, horizontal: false), Axis(boxes, horizontal: false), yThreshold, anchorIsEnd: true);
        else
            bottom = ChooseEdge(bottom, y, Sizes(boxes, horizontal: false), Axis(boxes, horizontal: false), yThreshold, anchorIsEnd: false);
        height = Math.Max(0.02f, bottom - y);
    }

    private static float ChooseEdge(float moving, float anchor, List<float> sizes, List<float> guides, float threshold, bool anchorIsEnd)
    {
        var best = moving;
        var bestAbs = threshold;
        void Take(float candidate)
        {
            var abs = Math.Abs(candidate - moving);
            if (abs <= bestAbs)
            {
                bestAbs = abs;
                best = candidate;
            }
        }
        foreach (var guide in guides)
            Take(guide);
        foreach (var size in sizes)
            Take(anchorIsEnd ? anchor - size : anchor + size);
        return best;
    }

    private static List<float> Sizes(IReadOnlyList<Box> boxes, bool horizontal)
    {
        var values = new List<float> { 1f };
        foreach (var box in boxes)
        {
            if (box.Hidden || box.Self)
                continue;
            var size = horizontal ? box.Width : box.Height;
            if (size > 0.02f)
                values.Add(size);
        }
        return values;
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
