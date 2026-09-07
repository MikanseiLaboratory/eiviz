using System.Windows;

namespace Eiviz.Host.Dialogs;

public partial class ConnectWindow : Window
{
    public ConnectWindow(string url, string token)
    {
        InitializeComponent();
        UrlBox.Text = string.IsNullOrWhiteSpace(url) ? "ws://127.0.0.1:9400" : url;
        TokenBox.Password = token ?? "";
        Loaded += (_, _) =>
        {
            UrlBox.Focus();
            UrlBox.SelectAll();
        };
    }

    public string Url => UrlBox.Text.Trim();
    public string Token => TokenBox.Password;

    private void Ok_Click(object sender, RoutedEventArgs e)
    {
        if (string.IsNullOrWhiteSpace(UrlBox.Text))
            return;
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
