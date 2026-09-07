using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Data;
using System.Windows.Threading;
using Eiviz.Host.I18n;

namespace Eiviz.Host.Controls;

public partial class TokenSecretField : UserControl
{
    private static readonly DependencyPropertyDescriptor PressedDescriptor =
        DependencyPropertyDescriptor.FromProperty(Button.IsPressedProperty, typeof(Button));

    private readonly DispatcherTimer _copiedTimer = new() { Interval = TimeSpan.FromSeconds(1.5) };
    private bool _revealing;

    public TokenSecretField()
    {
        InitializeComponent();
        _copiedTimer.Tick += CopiedTimer_Tick;
        Loaded += OnLoaded;
        Unloaded += OnUnloaded;
    }

    public string Password
    {
        get => _revealing ? PlainBox.Text ?? "" : SecretBox.Password ?? "";
        set
        {
            var text = value ?? "";
            SecretBox.Password = text;
            PlainBox.Text = text;
        }
    }

    private void OnLoaded(object sender, RoutedEventArgs e)
    {
        PressedDescriptor.RemoveValueChanged(RevealButton, OnRevealPressedChanged);
        PressedDescriptor.AddValueChanged(RevealButton, OnRevealPressedChanged);
    }

    private void OnUnloaded(object sender, RoutedEventArgs e)
    {
        PressedDescriptor.RemoveValueChanged(RevealButton, OnRevealPressedChanged);
        _copiedTimer.Stop();
        SetRevealed(false);
    }

    private void OnRevealPressedChanged(object? sender, EventArgs e) =>
        SetRevealed(RevealButton.IsPressed);

    private void SetRevealed(bool reveal)
    {
        if (_revealing == reveal)
            return;
        if (reveal)
        {
            PlainBox.Text = SecretBox.Password;
            PlainBox.Visibility = Visibility.Visible;
            SecretBox.Visibility = Visibility.Collapsed;
        }
        else
        {
            SecretBox.Password = PlainBox.Text ?? "";
            PlainBox.Visibility = Visibility.Collapsed;
            SecretBox.Visibility = Visibility.Visible;
        }
        _revealing = reveal;
    }

    private void Copy_Click(object sender, RoutedEventArgs e)
    {
        var value = Password;
        try
        {
            if (value.Length == 0)
                Clipboard.Clear();
            else
                Clipboard.SetText(value);
        }
        catch (ExternalException)
        {
            return;
        }
        CopyButton.Content = Loc.T("token.copied");
        _copiedTimer.Stop();
        _copiedTimer.Start();
    }

    private void CopiedTimer_Tick(object? sender, EventArgs e)
    {
        _copiedTimer.Stop();
        CopyButton.SetBinding(Button.ContentProperty, new Binding("[token.copy]") { Source = Loc.Source });
    }
}
