using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
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
            WindowStartupLocation = WindowStartupLocation.Manual,
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
        toast.Loaded += (_, _) => Place(toast, owner);
        var timer = new DispatcherTimer { Interval = TimeSpan.FromMilliseconds(1600) };
        timer.Tick += (_, _) =>
        {
            timer.Stop();
            toast.Close();
        };
        toast.Show();
        timer.Start();
    }

    private static void Place(Window toast, Window? owner)
    {
        if (owner is { IsVisible: true })
        {
            toast.Left = owner.Left + (owner.ActualWidth - toast.ActualWidth) / 2;
            toast.Top = owner.Top + owner.ActualHeight - toast.ActualHeight - 64;
            return;
        }
        toast.WindowStartupLocation = WindowStartupLocation.CenterScreen;
    }
}
