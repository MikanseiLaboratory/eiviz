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
}
