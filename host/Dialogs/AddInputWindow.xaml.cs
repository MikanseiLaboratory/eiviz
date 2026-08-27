using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using Eiviz.Host.Interop;
using Eiviz.Host.Media;
using Microsoft.Win32;

namespace Eiviz.Host.Dialogs;

public partial class AddInputWindow : Window
{
    private static readonly List<string> StillHistory = [];
    private static readonly List<string> VideoHistory = [];
    private InputKind _kind = InputKind.Still;

    public AddInputWindow()
    {
        InitializeComponent();
        foreach (var kind in new[] { InputKind.Color, InputKind.Still, InputKind.Video, InputKind.Omt, InputKind.Ndi, InputKind.Uvc })
        {
            var button = new Button
            {
                Content = kind == InputKind.Color ? "Colours" : kind.ToString(),
                Height = 36,
                Margin = new Thickness(8, 2, 8, 2),
                Tag = kind,
                HorizontalContentAlignment = HorizontalAlignment.Left
            };
            button.Click += Category_Click;
            CategoryPanel.Children.Add(button);
        }
        StillRecent.ItemsSource = StillHistory.ToArray();
        VideoRecent.ItemsSource = VideoHistory.ToArray();
        Highlight();
        RefreshOmt();
        RefreshUvc();
    }

    public InputKind Kind => _kind;
    public string? ResultPath { get; private set; }
    public string? ResultName { get; private set; }
    public float ColorR { get; private set; } = 1;
    public float ColorG { get; private set; }
    public float ColorB { get; private set; }
    public bool Scroll { get; private set; }
    public bool ResultUseGpu { get; private set; } = true;
    public uint ResultFrameBufferFrames { get; private set; } = 1;

    private void Category_Click(object sender, RoutedEventArgs e)
    {
        if (sender is Button { Tag: InputKind kind })
        {
            _kind = kind;
            Highlight();
        }
    }

    private void Highlight()
    {
        ColourPanel.Visibility = VisibleIf(InputKind.Color);
        StillPanel.Visibility = VisibleIf(InputKind.Still);
        VideoPanel.Visibility = VisibleIf(InputKind.Video);
        OmtPanel.Visibility = VisibleIf(InputKind.Omt);
        NdiPanel.Visibility = VisibleIf(InputKind.Ndi);
        UvcPanel.Visibility = VisibleIf(InputKind.Uvc);
        OkButton.IsEnabled = _kind != InputKind.Ndi;
        foreach (Button button in CategoryPanel.Children)
            button.Background = Equals(button.Tag, _kind)
                ? new System.Windows.Media.SolidColorBrush(System.Windows.Media.Color.FromRgb(0x2E, 0x6B, 0x3C))
                : new System.Windows.Media.SolidColorBrush(System.Windows.Media.Color.FromRgb(0x33, 0x33, 0x33));
    }

    private Visibility VisibleIf(InputKind kind) => _kind == kind ? Visibility.Visible : Visibility.Collapsed;

    private void Colour_Changed(object sender, RoutedPropertyChangedEventArgs<double> e)
    {
        if (ColourPreview is null)
            return;
        var r = (byte)RSlider.Value;
        var g = (byte)GSlider.Value;
        var b = (byte)BSlider.Value;
        ColourPreview.Background = new System.Windows.Media.SolidColorBrush(System.Windows.Media.Color.FromRgb(r, g, b));
    }

    private void BrowseStill_Click(object sender, RoutedEventArgs e)
    {
        var dialog = new OpenFileDialog { Filter = "Images|*.png;*.jpg;*.jpeg;*.bmp;*.tif;*.tiff" };
        if (dialog.ShowDialog(this) == true)
            StillPath.Text = dialog.FileName;
    }

    private void BrowseVideo_Click(object sender, RoutedEventArgs e)
    {
        var dialog = new OpenFileDialog { Filter = "Video|*.mp4;*.mov;*.mkv;*.avi;*.wmv;*.mxf|All|*.*" };
        if (dialog.ShowDialog(this) == true)
            VideoPath.Text = dialog.FileName;
    }

    private void StillRecent_DoubleClick(object sender, MouseButtonEventArgs e)
    {
        if (StillRecent.SelectedItem is string path)
            StillPath.Text = path;
    }

    private void VideoRecent_DoubleClick(object sender, MouseButtonEventArgs e)
    {
        if (VideoRecent.SelectedItem is string path)
            VideoPath.Text = path;
    }

    private void RefreshOmt_Click(object sender, RoutedEventArgs e) => RefreshOmt();

    private void RefreshOmt()
    {
        var text = MixerNative.DiscoverText();
        OmtList.ItemsSource = string.IsNullOrWhiteSpace(text)
            ? Array.Empty<string>()
            : text.Split('\n', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries);
    }

    private void OmtList_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (OmtList.SelectedItem is string address)
            OmtAddress.Text = address;
    }

    private void RefreshUvc_Click(object sender, RoutedEventArgs e) => RefreshUvc();

    private void RefreshUvc()
    {
        try
        {
            UvcList.ItemsSource = MfFramePump.EnumerateCameras()
                .Select(item => new CameraItem(item.Name, item.Link))
                .ToArray();
        }
        catch (Exception ex)
        {
            UvcList.ItemsSource = new[] { new CameraItem($"Capture enum failed: {ex.Message}", "") };
        }
    }

    private void Ok_Click(object sender, RoutedEventArgs e)
    {
        switch (_kind)
        {
            case InputKind.Color:
                ColorR = (float)(RSlider.Value / 255.0);
                ColorG = (float)(GSlider.Value / 255.0);
                ColorB = (float)(BSlider.Value / 255.0);
                Scroll = ScrollBox.IsChecked == true;
                if (BarsRadio.IsChecked == true)
                    _kind = InputKind.Bars;
                ResultName = _kind == InputKind.Bars
                    ? (Scroll ? "SMPTE Bars (scroll)" : "SMPTE Bars")
                    : $"Colour {((byte)RSlider.Value):X2}{((byte)GSlider.Value):X2}{((byte)BSlider.Value):X2}";
                ResultPath = "";
                break;
            case InputKind.Still:
                if (string.IsNullOrWhiteSpace(StillPath.Text))
                    return;
                ResultPath = StillPath.Text.Trim();
                ResultName = System.IO.Path.GetFileName(ResultPath);
                Remember(StillHistory, ResultPath);
                break;
            case InputKind.Video:
                if (string.IsNullOrWhiteSpace(VideoPath.Text))
                    return;
                ResultPath = VideoPath.Text.Trim();
                ResultName = System.IO.Path.GetFileName(ResultPath);
                Remember(VideoHistory, ResultPath);
                break;
            case InputKind.Omt:
                if (string.IsNullOrWhiteSpace(OmtAddress.Text))
                    return;
                ResultPath = OmtAddress.Text.Trim();
                ResultName = ResultPath;
                ResultUseGpu = OmtPathBox.SelectedItem is ComboBoxItem { Tag: "gpu" };
                ResultFrameBufferFrames = 1;
                if (OmtBufferBox.SelectedItem is ComboBoxItem buffer && buffer.Tag is string tag
                    && uint.TryParse(tag, out var frames))
                    ResultFrameBufferFrames = Math.Clamp(frames, 1u, 8u);
                break;
            case InputKind.Uvc:
                if (UvcList.SelectedItem is not CameraItem camera || string.IsNullOrEmpty(camera.Link))
                    return;
                ResultPath = camera.Link;
                ResultName = camera.Name;
                break;
            default:
                return;
        }
        DialogResult = true;
    }

    private static void Remember(List<string> history, string path)
    {
        history.Remove(path);
        history.Insert(0, path);
        if (history.Count > 24)
            history.RemoveRange(24, history.Count - 24);
    }

    private sealed record CameraItem(string Name, string Link)
    {
        public override string ToString() => Name;
    }
}
