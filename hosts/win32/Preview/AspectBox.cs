using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;

namespace Eiviz.Host.Preview;

internal sealed class AspectBox : Decorator
{
    public AspectBox()
    {
        UseLayoutRounding = true;
        SnapsToDevicePixels = true;
    }

    public static readonly DependencyProperty RatioWidthProperty = DependencyProperty.Register(
        nameof(RatioWidth), typeof(double), typeof(AspectBox),
        new FrameworkPropertyMetadata(16.0, FrameworkPropertyMetadataOptions.AffectsMeasure));

    public static readonly DependencyProperty RatioHeightProperty = DependencyProperty.Register(
        nameof(RatioHeight), typeof(double), typeof(AspectBox),
        new FrameworkPropertyMetadata(9.0, FrameworkPropertyMetadataOptions.AffectsMeasure));

    public static readonly DependencyProperty HeaderHeightProperty = DependencyProperty.Register(
        nameof(HeaderHeight), typeof(double), typeof(AspectBox),
        new FrameworkPropertyMetadata(0.0, FrameworkPropertyMetadataOptions.AffectsMeasure));

    public double RatioWidth
    {
        get => (double)GetValue(RatioWidthProperty);
        set => SetValue(RatioWidthProperty, value);
    }

    public double RatioHeight
    {
        get => (double)GetValue(RatioHeightProperty);
        set => SetValue(RatioHeightProperty, value);
    }

    public double HeaderHeight
    {
        get => (double)GetValue(HeaderHeightProperty);
        set => SetValue(HeaderHeightProperty, value);
    }

    protected override Size MeasureOverride(Size constraint)
    {
        var size = Snap(Fit(constraint));
        Child?.Measure(size);
        return size;
    }

    protected override Size ArrangeOverride(Size arrangeSize)
    {
        var size = Snap(Fit(arrangeSize));
        var x = Math.Max(0, Math.Floor((arrangeSize.Width - size.Width) / 2));
        var y = Math.Max(0, Math.Floor((arrangeSize.Height - size.Height) / 2));
        Child?.Arrange(new Rect(x, y, size.Width, size.Height));
        return arrangeSize;
    }

    private Size Fit(Size limit)
    {
        var header = Math.Max(0, HeaderHeight);
        var rw = Math.Max(1, RatioWidth);
        var rh = Math.Max(1, RatioHeight);
        var aspect = rw / rh;
        var width = double.IsInfinity(limit.Width) ? rw : Math.Max(2, limit.Width);
        var height = double.IsInfinity(limit.Height) ? rh + header : Math.Max(2 + header, limit.Height);
        var videoHeight = Math.Max(2, height - header);
        if (width / videoHeight > aspect)
            return new Size(videoHeight * aspect, videoHeight + header);
        return new Size(width, width / aspect + header);
    }

    private Size Snap(Size size)
    {
        var dpi = VisualTreeHelper.GetDpi(this);
        var scaleX = Math.Max(1e-6, dpi.DpiScaleX);
        var scaleY = Math.Max(1e-6, dpi.DpiScaleY);
        return new Size(
            Math.Max(2, Math.Floor(size.Width * scaleX) / scaleX),
            Math.Max(2, Math.Floor(size.Height * scaleY) / scaleY));
    }
}
