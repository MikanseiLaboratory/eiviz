using System.Windows;

namespace Eiviz.Host.Dialogs;

public partial class TextPromptWindow : Window
{
    public TextPromptWindow(string title, string prompt, string initial)
    {
        InitializeComponent();
        Title = title;
        PromptText.Text = prompt;
        ValueBox.Text = initial;
        Loaded += (_, _) =>
        {
            ValueBox.Focus();
            ValueBox.SelectAll();
        };
    }

    public string Result => ValueBox.Text;

    public static bool TryPrompt(Window owner, string title, string prompt, string initial, out string value)
    {
        var dialog = new TextPromptWindow(title, prompt, initial) { Owner = owner };
        if (dialog.ShowDialog() == true && TagCatalog.Normalize(dialog.Result) is { } name)
        {
            value = name;
            return true;
        }
        value = "";
        return false;
    }

    private void Ok_Click(object sender, RoutedEventArgs e)
    {
        if (TagCatalog.Normalize(ValueBox.Text) is null)
            return;
        DialogResult = true;
    }
}
