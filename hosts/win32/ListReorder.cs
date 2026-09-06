using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;

namespace Eiviz.Host;

internal static class ListReorder
{
    public static void Attach(ListBox list, Func<int, int, bool> move)
    {
        list.AllowDrop = true;
        var start = new Point();
        var from = -1;
        var dragging = false;
        list.PreviewMouseLeftButtonDown += (_, e) =>
        {
            if (FindAncestor<Button>(e.OriginalSource as DependencyObject) is not null)
            {
                from = -1;
                return;
            }
            start = e.GetPosition(list);
            from = IndexAt(list, start);
            dragging = false;
        };
        list.PreviewMouseMove += (_, e) =>
        {
            if (from < 0 || e.LeftButton != MouseButtonState.Pressed || dragging)
                return;
            var pos = e.GetPosition(list);
            if (Math.Abs(pos.Y - start.Y) < 6)
                return;
            dragging = true;
            DragDrop.DoDragDrop(list, from, DragDropEffects.Move);
            dragging = false;
            from = -1;
        };
        list.DragOver += (_, e) =>
        {
            e.Effects = DragDropEffects.Move;
            e.Handled = true;
        };
        list.Drop += (_, e) =>
        {
            if (e.Data.GetData(typeof(int)) is not int source)
                return;
            var dest = IndexAt(list, e.GetPosition(list));
            if (dest < 0)
                dest = list.Items.Count;
            move(source, dest);
        };
    }

    private static T? FindAncestor<T>(DependencyObject? node) where T : DependencyObject
    {
        while (node is not null)
        {
            if (node is T match)
                return match;
            node = VisualTreeHelper.GetParent(node);
        }
        return null;
    }

    private static int IndexAt(ListBox list, Point pos)
    {
        var hit = list.InputHitTest(pos) as DependencyObject;
        while (hit is not null && hit is not ListBoxItem)
            hit = VisualTreeHelper.GetParent(hit);
        return hit is ListBoxItem item
            ? list.ItemContainerGenerator.IndexFromContainer(item)
            : -1;
    }
}
