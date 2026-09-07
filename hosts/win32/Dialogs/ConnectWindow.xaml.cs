using System.Windows;

namespace Eiviz.Host.Dialogs;

public partial class ConnectWindow : Window
{
    public ConnectWindow(string url, string token)
    {
        InitializeComponent();
        var (host, port) = RemoteEndpoint.Split(url);
        HostBox.Text = host;
        PortBox.Text = port.ToString();
        TokenBox.Password = token ?? "";
        Loaded += (_, _) =>
        {
            HostBox.Focus();
            HostBox.SelectAll();
        };
    }

    public string Url { get; private set; } = "";
    public string Token => TokenBox.Password;

    private void Ok_Click(object sender, RoutedEventArgs e)
    {
        if (!RemoteEndpoint.TryCompose(HostBox.Text, PortBox.Text, out var url))
            return;
        Url = url;
        DialogResult = true;
    }

    public static bool TryPrompt(Window owner, string url, string token, out string nextUrl, out string nextToken)
    {
        var dialog = new ConnectWindow(url, token) { Owner = owner };
        if (dialog.ShowDialog() == true)
        {
            nextUrl = dialog.Url;
            nextToken = dialog.Token;
            return !string.IsNullOrWhiteSpace(nextUrl);
        }
        nextUrl = "";
        nextToken = "";
        return false;
    }
}
