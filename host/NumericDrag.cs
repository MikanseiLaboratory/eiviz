using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;

namespace Eiviz.Host;

internal static class NumericDrag
{
    public static void Attach(FrameworkElement handle, TextBox box, float pixelsPerUnit, Action apply, Action? commit = null, string format = "0.#")
    {
        Point? start = null;
        var origin = 0f;
        handle.Cursor = Cursors.SizeNS;
        handle.PreviewMouseLeftButtonDown += (_, e) =>
        {
            if (!float.TryParse(box.Text, out origin))
                return;
            start = e.GetPosition(handle);
            handle.CaptureMouse();
            e.Handled = true;
        };
        handle.PreviewMouseMove += (_, e) =>
        {
            if (start is not { } originPoint || e.LeftButton != MouseButtonState.Pressed)
                return;
            var dy = (float)(originPoint.Y - e.GetPosition(handle).Y);
            box.Text = (origin + dy / pixelsPerUnit).ToString(format);
            apply();
        };
        handle.PreviewMouseLeftButtonUp += (_, _) =>
        {
            if (start is null)
                return;
            start = null;
            handle.ReleaseMouseCapture();
            (commit ?? apply)();
        };
    }

    public static void AttachBox(TextBox box, float pixelsPerUnit, Action apply, Action? commit = null, string format = "0.#")
    {
        Point? start = null;
        var origin = 0f;
        var dragging = false;
        box.PreviewMouseLeftButtonDown += (_, e) =>
        {
            if (!float.TryParse(box.Text, out origin))
                return;
            start = e.GetPosition(box);
            dragging = false;
        };
        box.PreviewMouseMove += (_, e) =>
        {
            if (start is not { } originPoint || e.LeftButton != MouseButtonState.Pressed)
                return;
            var delta = e.GetPosition(box) - originPoint;
            if (!dragging && Math.Abs(delta.Y) < 3)
                return;
            if (!dragging)
            {
                dragging = true;
                box.CaptureMouse();
            }
            box.Text = (origin + (float)(-delta.Y) / pixelsPerUnit).ToString(format);
            apply();
            e.Handled = true;
        };
        box.PreviewMouseLeftButtonUp += (_, _) =>
        {
            if (!dragging)
            {
                start = null;
                return;
            }
            dragging = false;
            start = null;
            box.ReleaseMouseCapture();
            (commit ?? apply)();
        };
    }
}
