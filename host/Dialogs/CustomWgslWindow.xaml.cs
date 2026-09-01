using System.IO;
using System.Windows;
using Eiviz.Host.Interop;
using Microsoft.Win32;

namespace Eiviz.Host.Dialogs;

public partial class CustomWgslWindow : Window
{
    public const string WgslTemplate =
        """
        fn user_transition(uv: vec2<f32>, t: f32) -> vec4<f32> {
            let a = textureSample(pgm_tex, src_samp, uv);
            let b = textureSample(pvw_tex, src_samp, uv);
            let w = smoothstep(t - 0.02, t + 0.02, uv.x);
            return mix(b, a, w);
        }
        """;

    public CustomWgslWindow(string? wgsl)
    {
        InitializeComponent();
        Editor.Text = string.IsNullOrWhiteSpace(wgsl) ? WgslTemplate : wgsl;
        Validate(silent: true);
    }

    public string Wgsl { get; private set; } = "";

    private void Load_Click(object sender, RoutedEventArgs e)
    {
        var dialog = new OpenFileDialog
        {
            Filter = "WGSL (*.wgsl)|*.wgsl|All files (*.*)|*.*",
            CheckFileExists = true
        };
        if (dialog.ShowDialog(this) != true)
            return;
        Editor.Text = File.ReadAllText(dialog.FileName);
        Validate(silent: false);
    }

    private void Validate_Click(object sender, RoutedEventArgs e) => Validate(silent: false);

    private void Save_Click(object sender, RoutedEventArgs e)
    {
        if (!Validate(silent: false))
            return;
        Wgsl = Editor.Text;
        DialogResult = true;
    }

    private bool Validate(bool silent)
    {
        var code = MixerNative.ValidateCustomWgsl(Editor.Text);
        if (code == 0)
        {
            StatusText.Foreground = System.Windows.Media.Brushes.LightGreen;
            StatusText.Text = "Valid WGSL. user_transition will be used.";
            return true;
        }
        StatusText.Foreground = System.Windows.Media.Brushes.Salmon;
        var error = MixerNative.LastErrorText();
        StatusText.Text = string.IsNullOrWhiteSpace(error) ? "Invalid WGSL." : error;
        if (!silent)
            return false;
        return false;
    }
}
