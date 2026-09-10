namespace Eiviz.Host;

internal static class SceneSnap
{
    public const double PixelThreshold = 8;

    public readonly record struct Box(float X, float Y, float Width, float Height, bool Hidden, bool Self);

    public static void SnapMove(ref float x, ref float y, float width, float height, IReadOnlyList<Box> boxes, float threshold)
    {
        var xs = Axis(boxes, horizontal: true);
        var ys = Axis(boxes, horizontal: false);
        x = SnapValue(x, x + width * 0.5f, x + width, xs, threshold);
        y = SnapValue(y, y + height * 0.5f, y + height, ys, threshold);
    }

    public static void SnapResize(ref float width, ref float height, float x, float y, bool sizeLinked, IReadOnlyList<Box> boxes, float threshold)
    {
        var ratio = height / Math.Max(width, 0.0001f);
        var xs = Axis(boxes, horizontal: true);
        var right = SnapPoint(x + width, xs, threshold);
        width = Math.Max(0.02f, right - x);
        if (sizeLinked)
        {
            height = Math.Max(0.02f, width * ratio);
            return;
        }
        var ys = Axis(boxes, horizontal: false);
        var bottom = SnapPoint(y + height, ys, threshold);
        height = Math.Max(0.02f, bottom - y);
    }

    private static float SnapValue(float start, float mid, float end, List<float> guides, float threshold)
    {
        var delta = BestDelta([start, mid, end], guides, threshold);
        return start + delta;
    }

    private static float SnapPoint(float value, List<float> guides, float threshold)
    {
        return value + BestDelta([value], guides, threshold);
    }

    private static float BestDelta(float[] points, List<float> guides, float threshold)
    {
        var best = 0f;
        var bestAbs = threshold;
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
                }
            }
        }
        return bestAbs <= threshold ? best : 0f;
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
