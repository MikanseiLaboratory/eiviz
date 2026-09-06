using System.Windows;
using System.Windows.Controls;

namespace Eiviz.Host.Preview;

internal sealed class AspectBox : Decorator
{
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
        var size = Fit(constraint);
        Child?.Measure(size);
        return size;
    }

    protected override Size ArrangeOverride(Size arrangeSize)
    {
        var size = Fit(arrangeSize);
        Child?.Arrange(new Rect(0, 0, size.Width, size.Height));
        return size;
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
}
