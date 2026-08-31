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
    private bool _lockKind;

    public AddInputWindow()
    {
        InitializeComponent();
        foreach (var kind in new[] { InputKind.Color, InputKind.Still, InputKind.Video, InputKind.Omt, InputKind.Ndi, InputKind.Uvc })
        {
            var button = new Button
            {
                Content = InputKindNames.Category(kind),
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
        RefreshNdi();
        RefreshUvc();
    }

    public InputKind Kind => _kind;
    public string? ResultPath { get; private set; }
    public string? ResultName { get; private set; }
    public float ColorR { get; private set; } = 1;
    public float ColorG { get; private set; }
    public float ColorB { get; private set; }
    public bool Scroll { get; private set; }
    public float ResultToneHz { get; private set; }
    public float ResultToneLevelDbfs { get; private set; } = -20;
    public bool ResultUseGpu { get; private set; } = true;
    public uint ResultFrameBufferFrames { get; private set; } = 1;
    public BandwidthSave ResultSaveMode { get; private set; } = BandwidthSave.NotOnPreviewOrProgram;
    public bool ResultKeepFullOnMultiview { get; private set; }
    public OmtQuality ResultOmtQuality { get; private set; } = OmtQuality.Default;
    public NdiBandwidth ResultNdiBandwidth { get; private set; } = NdiBandwidth.Highest;
    public bool ResultVideoLoop { get; private set; } = true;
    public VideoPlayWhen ResultVideoPlayWhen { get; private set; } = VideoPlayWhen.Never;
    public VideoTriggerWhen ResultVideoRestartWhen { get; private set; } = VideoTriggerWhen.Never;
    public VideoTriggerWhen ResultVideoPauseWhen { get; private set; } = VideoTriggerWhen.Never;

    public void Load(InputEntry input)
    {
        Title = "Input Properties";
        NameBox.Text = input.Name;
        _lockKind = input.IsBuiltin;
        _kind = input.Kind is InputKind.Bars or InputKind.Black ? InputKind.Color : input.Kind;
        if (input.Kind == InputKind.Bars)
            BarsRadio.IsChecked = true;
        else
            SolidRadio.IsChecked = true;
        RSlider.Value = Math.Clamp(input.ColorR * 255.0, 0, 255);
        GSlider.Value = Math.Clamp(input.ColorG * 255.0, 0, 255);
        BSlider.Value = Math.Clamp(input.ColorB * 255.0, 0, 255);
        ScrollBox.IsChecked = input.Scroll;
        SelectTag(ToneBox, ToneTag(input.ToneHz));
        StillPath.Text = input.Kind == InputKind.Still ? input.PathOrAddress ?? "" : "";
        VideoPath.Text = input.Kind == InputKind.Video ? input.PathOrAddress ?? "" : "";
        VideoLoopBox.IsChecked = input.VideoLoop;
        SelectTag(VideoPlayBox, ((int)input.VideoPlayWhen).ToString());
        SelectTag(VideoRestartBox, ((int)input.VideoRestartWhen).ToString());
        SelectTag(VideoPauseBox, ((int)input.VideoPauseWhen).ToString());
        OmtAddress.Text = input.Kind == InputKind.Omt ? input.PathOrAddress ?? "" : "";
        NdiAddress.Text = input.Kind == InputKind.Ndi ? input.PathOrAddress ?? "" : "";
        SelectTag(OmtPathBox, input.UseGpu ? "gpu" : "cpu");
        SelectTag(OmtQualityBox, ((int)input.OmtQuality).ToString());
        SelectTag(OmtBufferBox, Math.Clamp(input.FrameBufferFrames == 0 ? 1 : input.FrameBufferFrames, 1u, 8u).ToString());
        SelectTag(NdiBufferBox, Math.Clamp(input.FrameBufferFrames == 0 ? 1 : input.FrameBufferFrames, 1u, 8u).ToString());
        SelectTag(NdiBandwidthBox, ((int)input.NdiBandwidth).ToString());
        SelectTag(OmtSaveBox, ((int)input.BandwidthSave).ToString());
        OmtMvBox.IsChecked = input.KeepFullOnMultiview;
        if (input.Kind == InputKind.Uvc && !string.IsNullOrWhiteSpace(input.PathOrAddress))
        {
            foreach (var item in UvcList.Items)
            {
                if (item is CameraItem camera && camera.Link == input.PathOrAddress)
                {
                    UvcList.SelectedItem = item;
                    break;
                }
            }
        }
        Highlight();
        foreach (Button button in CategoryPanel.Children)
        {
            if (button.Tag is not InputKind kind)
                continue;
            button.IsEnabled = !_lockKind || SameCategory(kind, input.Kind);
        }
    }

    private void Category_Click(object sender, RoutedEventArgs e)
    {
        if (sender is not Button { Tag: InputKind kind } || (_lockKind && !SameCategory(kind, _kind)))
            return;
        _kind = kind;
        Highlight();
    }

    private void Highlight()
    {
        ColourPanel.Visibility = VisibleIf(InputKind.Color);
        StillPanel.Visibility = VisibleIf(InputKind.Still);
        VideoPanel.Visibility = VisibleIf(InputKind.Video);
        OmtPanel.Visibility = VisibleIf(InputKind.Omt);
        NdiPanel.Visibility = VisibleIf(InputKind.Ndi);
        UvcPanel.Visibility = VisibleIf(InputKind.Uvc);
        foreach (Button button in CategoryPanel.Children)
            button.Background = Equals(button.Tag, _kind)
                ? new System.Windows.Media.SolidColorBrush(System.Windows.Media.Color.FromRgb(0x2E, 0x6B, 0x3C))
                : new System.Windows.Media.SolidColorBrush(System.Windows.Media.Color.FromRgb(0x33, 0x33, 0x33));
    }

    private Visibility VisibleIf(InputKind kind) => _kind == kind ? Visibility.Visible : Visibility.Collapsed;

    private static bool SameCategory(InputKind left, InputKind right) =>
        IsColour(left) && IsColour(right) || left == right;

    private static bool IsColour(InputKind kind) =>
        kind is InputKind.Color or InputKind.Bars or InputKind.Black;

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

    private void RefreshNdi_Click(object sender, RoutedEventArgs e) => RefreshNdi();

    private async void RefreshNdi()
    {
        if (NdiStatus is not null)
            NdiStatus.Text = "Discovering…";
        var text = await Task.Run(MixerNative.DiscoverNdiText);
        if (!Dispatcher.CheckAccess())
            return;
        NdiList.ItemsSource = string.IsNullOrWhiteSpace(text)
            ? Array.Empty<string>()
            : text.Split('\n', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries);
        if (NdiStatus is null)
            return;
        NdiStatus.Text = NdiList.Items.Count == 0 ? MixerNative.LastErrorText() : "";
    }

    private void NdiList_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (NdiList.SelectedItem is string address)
            NdiAddress.Text = address;
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
                ResultToneHz = ReadToneHz(ToneBox);
                ResultToneLevelDbfs = ResultToneHz > 0 ? -20 : 0;
                if (BarsRadio.IsChecked == true)
                    _kind = InputKind.Bars;
                ResultName = _kind == InputKind.Bars
                    ? (Scroll ? "SMPTE HD Bars (scroll)" : "SMPTE HD Bars")
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
                ResultVideoLoop = VideoLoopBox.IsChecked == true;
                ResultVideoPlayWhen = ReadVideoPlayWhen(VideoPlayBox);
                ResultVideoRestartWhen = ReadVideoTriggerWhen(VideoRestartBox);
                ResultVideoPauseWhen = ReadVideoTriggerWhen(VideoPauseBox);
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
                ResultSaveMode = ReadSaveMode(OmtSaveBox);
                ResultKeepFullOnMultiview = OmtMvBox.IsChecked == true;
                ResultOmtQuality = ReadOmtQuality(OmtQualityBox);
                break;
            case InputKind.Ndi:
                if (string.IsNullOrWhiteSpace(NdiAddress.Text))
                    return;
                ResultPath = NdiAddress.Text.Trim();
                ResultName = ResultPath;
                ResultUseGpu = false;
                ResultFrameBufferFrames = 1;
                if (NdiBufferBox.SelectedItem is ComboBoxItem ndiBuffer && ndiBuffer.Tag is string ndiTag
                    && uint.TryParse(ndiTag, out var ndiFrames))
                    ResultFrameBufferFrames = Math.Clamp(ndiFrames, 1u, 8u);
                ResultNdiBandwidth = ReadNdiBandwidth(NdiBandwidthBox);
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
        if (!string.IsNullOrWhiteSpace(NameBox.Text))
            ResultName = NameBox.Text.Trim();
        DialogResult = true;
    }

    private static NdiBandwidth ReadNdiBandwidth(ComboBox box)
    {
        if (box.SelectedItem is ComboBoxItem item && item.Tag is string tag && uint.TryParse(tag, out var value))
            return value == 1 ? NdiBandwidth.Lowest : NdiBandwidth.Highest;
        return NdiBandwidth.Highest;
    }

    private static OmtQuality ReadOmtQuality(ComboBox box)
    {
        if (box.SelectedItem is ComboBoxItem item && item.Tag is string tag && uint.TryParse(tag, out var value))
        {
            return value switch
            {
                1 => OmtQuality.Low,
                50 => OmtQuality.Medium,
                100 => OmtQuality.High,
                _ => OmtQuality.Default
            };
        }
        return OmtQuality.Default;
    }

    private static float ReadToneHz(ComboBox box)
    {
        if (box.SelectedItem is ComboBoxItem item && item.Tag is string tag && float.TryParse(tag, out var hz))
            return hz;
        return 0;
    }

    private static string ToneTag(float hz) =>
        hz >= 1500 ? "2000" : hz >= 700 ? "1000" : hz >= 200 ? "440" : "0";

    private static VideoPlayWhen ReadVideoPlayWhen(ComboBox box)
    {
        if (box.SelectedItem is ComboBoxItem item && item.Tag is string tag && uint.TryParse(tag, out var value))
            return (VideoPlayWhen)Math.Clamp(value, 0u, 3u);
        return VideoPlayWhen.Never;
    }

    private static VideoTriggerWhen ReadVideoTriggerWhen(ComboBox box)
    {
        if (box.SelectedItem is ComboBoxItem item && item.Tag is string tag && uint.TryParse(tag, out var value))
            return (VideoTriggerWhen)Math.Clamp(value, 0u, 3u);
        return VideoTriggerWhen.Never;
    }

    private static BandwidthSave ReadSaveMode(ComboBox box)
    {
        if (box.SelectedItem is ComboBoxItem item && item.Tag is string tag && uint.TryParse(tag, out var mode))
            return (BandwidthSave)Math.Clamp(mode, 0u, 3u);
        return BandwidthSave.NotOnPreviewOrProgram;
    }

    private static void SelectTag(ComboBox box, string tag)
    {
        foreach (ComboBoxItem item in box.Items)
        {
            if (Equals(item.Tag, tag))
            {
                box.SelectedItem = item;
                return;
            }
        }
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
