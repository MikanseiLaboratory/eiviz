using System.Windows;
using System.Windows.Controls;
using System.Windows.Threading;

namespace Eiviz.Host;

internal static class StatusToast
{
    public static void Show(Window? owner, string message)
    {
        var toast = new Window
        {
            WindowStyle = WindowStyle.None,
            ResizeMode = ResizeMode.NoResize,
            ShowInTaskbar = false,
            Topmost = true,
            SizeToContent = SizeToContent.WidthAndHeight,
            WindowStartupLocation = WindowStartupLocation.CenterOwner,
            Owner = owner
        };
        HostDialog.Apply(toast);
        toast.Content = new TextBlock
        {
            Text = message,
            Margin = new Thickness(20, 12, 20, 12),
            TextWrapping = TextWrapping.Wrap,
            MaxWidth = 420
        };
        toast.MouseLeftButtonUp += (_, _) => toast.Close();
        var timer = new DispatcherTimer { Interval = TimeSpan.FromMilliseconds(1600) };
        timer.Tick += (_, _) =>
        {
            timer.Stop();
            toast.Close();
        };
        toast.Show();
        timer.Start();
    }

    public static void ShowTip(Window? owner, string message)
    {
        var toast = new Window
        {
            WindowStyle = WindowStyle.None,
            ResizeMode = ResizeMode.NoResize,
            ShowInTaskbar = false,
            ShowActivated = false,
            Topmost = true,
            SizeToContent = SizeToContent.WidthAndHeight,
            WindowStartupLocation = WindowStartupLocation.Manual,
            Owner = owner is { IsVisible: true } ? owner : null
        };
        HostDialog.Apply(toast);
        toast.Content = new TextBlock
        {
            Text = message,
            Margin = new Thickness(14, 8, 14, 8),
            TextWrapping = TextWrapping.Wrap,
            MaxWidth = 420
        };
        toast.MouseLeftButtonUp += (_, _) => toast.Close();
        var timer = new DispatcherTimer { Interval = TimeSpan.FromMilliseconds(2200) };
        timer.Tick += (_, _) =>
        {
            timer.Stop();
            toast.Close();
        };
        toast.Show();
        var area = SystemParameters.WorkArea;
        var width = toast.ActualWidth;
        var height = toast.ActualHeight;
        if (owner is { IsVisible: true })
        {
            toast.Left = owner.Left + Math.Max(8, (owner.ActualWidth - width) / 2);
            toast.Top = owner.Top + owner.ActualHeight - height - 28;
        }
        else
        {
            toast.Left = area.Left + Math.Max(8, (area.Width - width) / 2);
            toast.Top = area.Bottom - height - 48;
        }
        timer.Start();
    }
}
