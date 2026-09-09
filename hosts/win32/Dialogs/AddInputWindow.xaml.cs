using System.IO;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using Eiviz.Host.I18n;
using Eiviz.Host.Interop;
using Microsoft.Win32;

namespace Eiviz.Host.Dialogs;

public partial class AddInputWindow : Window
{
    private static List<string> StillHistory => AppPrefs.Current.RecentStills;
    private static List<string> VideoHistory => AppPrefs.Current.RecentVideos;
    private InputKind _kind = InputKind.Still;
    private bool _lockKind;
    private TagCheckPanel? _tags;

    public AddInputWindow()
    {
        InitializeComponent();
        foreach (var kind in new[] { InputKind.Color, InputKind.Still, InputKind.Video, InputKind.OMT, InputKind.NDI, InputKind.UVC, InputKind.Mix, InputKind.Audio })
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
        RefreshAudioDevices();
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
    public uint ResultCaptureWidth { get; private set; }
    public uint ResultCaptureHeight { get; private set; }
    public uint ResultCaptureFpsNum { get; private set; } = 60;
    public uint ResultCaptureFpsDen { get; private set; } = 1;
    public IReadOnlyList<string> ResultTags { get; private set; } = [];
    public MixSource ResultMixSource { get; private set; } = MixSource.MuProgram;
    public ulong ResultMixTargetId { get; private set; }
    public ulong ResultMixAudioBusId { get; private set; }
    public AudioCaptureMode ResultAudioCaptureMode { get; private set; } = AudioCaptureMode.Mic;
    public AudioDeviceKind ResultAudioDeviceKind { get; private set; } = AudioDeviceKind.Wasapi;
    public string? ResultAudioDeviceId { get; private set; }
    public int ResultAudioMapLeft { get; private set; }
    public int ResultAudioMapRight { get; private set; } = 1;
    public string? ResultAudioProcessExe { get; private set; }
    public string? ResultAudioProcessAumid { get; private set; }

    public void BindTags(Session session, IEnumerable<string>? selected = null)
    {
        _tags = new TagCheckPanel(TagPanel, session.InputTags, selected, this);
        BindMixTargets(session);
        BindMixAudio(session);
    }

    public void BindMixTargets(Session session)
    {
        MixTargetBox.Items.Clear();
        foreach (var unit in session.Units)
            MixTargetBox.Items.Add(new MixTargetItem(unit.Name, unit.Id, false));
        foreach (var layout in session.Multiviews)
            MixTargetBox.Items.Add(new MixTargetItem(layout.Name, layout.GpuId, true));
        if (MixTargetBox.Items.Count > 0)
            MixTargetBox.SelectedIndex = 0;
    }

    public void BindMixAudio(Session session)
    {
        MixAudioBox.Items.Clear();
        MixAudioBox.Items.Add(new ComboBoxItem { Content = "None", Tag = "0" });
        foreach (var bus in session.Buses)
            MixAudioBox.Items.Add(new ComboBoxItem { Content = bus.Name, Tag = bus.Id.ToString() });
        MixAudioBox.SelectedIndex = 0;
    }

    public void Load(InputEntry input)
    {
        Title = "Input Properties";
        NameBox.Text = input.Name;
        IdLabel.Visibility = Visibility.Visible;
        IdLabel.Text = $"ID {input.Id}   GUID {input.Guid}";
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
        var mediaBuffer = Math.Clamp(input.FrameBufferFrames == 0 ? 3 : input.FrameBufferFrames, 1u, 8u).ToString();
        SelectTag(VideoBufferBox, mediaBuffer);
        SelectTag(UvcBufferBox, mediaBuffer);
        OmtAddress.Text = input.Kind == InputKind.OMT ? input.PathOrAddress ?? "" : "";
        NdiAddress.Text = input.Kind == InputKind.NDI ? input.PathOrAddress ?? "" : "";
        SelectTag(OmtPathBox, input.UseGpu ? "gpu" : "cpu");
        SelectTag(OmtQualityBox, ((int)input.OmtQuality).ToString());
        SelectTag(OmtBufferBox, Math.Clamp(input.FrameBufferFrames == 0 ? 1 : input.FrameBufferFrames, 1u, 8u).ToString());
        SelectTag(NdiBufferBox, Math.Clamp(input.FrameBufferFrames == 0 ? 1 : input.FrameBufferFrames, 1u, 8u).ToString());
        SelectTag(NdiBandwidthBox, ((int)input.NdiBandwidth).ToString());
        SelectTag(OmtSaveBox, ((int)input.BandwidthSave).ToString());
        OmtMvBox.IsChecked = input.KeepFullOnMultiview;
        if (input.Kind == InputKind.Mix)
        {
            foreach (MixTargetItem item in MixTargetBox.Items)
            {
                if (item.Id == input.MixTargetId)
                {
                    MixTargetBox.SelectedItem = item;
                    break;
                }
            }
            SelectTag(MixBusBox, input.MixSource == MixSource.MuPreview ? "preview" : "program");
            SelectTag(MixAudioBox, input.MixAudioBusId.ToString());
            SelectTag(MixBufferBox, Math.Clamp(input.FrameBufferFrames == 0 ? 1 : input.FrameBufferFrames, 1u, 8u).ToString());
        }
        if (input.Kind == InputKind.Audio)
        {
            SelectTag(AudioModeBox, input.AudioCaptureMode switch
            {
                AudioCaptureMode.EndpointLoopback => "loopback",
                AudioCaptureMode.ProcessLoopback => "process",
                _ => "mic"
            });
            RefreshAudioDevices();
            foreach (AudioDeviceItem item in AudioDeviceBox.Items)
            {
                if (item.Id == input.AudioDeviceId)
                {
                    AudioDeviceBox.SelectedItem = item;
                    break;
                }
            }
            _ = RefreshAudioProcesses(input.AudioProcessExe, input.AudioProcessAumid);
        }
        if (input.Kind == InputKind.UVC && !string.IsNullOrWhiteSpace(input.PathOrAddress))
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
        OmtPanel.Visibility = VisibleIf(InputKind.OMT);
        NdiPanel.Visibility = VisibleIf(InputKind.NDI);
        UvcPanel.Visibility = VisibleIf(InputKind.UVC);
        MixPanel.Visibility = VisibleIf(InputKind.Mix);
        AudioPanel.Visibility = VisibleIf(InputKind.Audio);
        UpdateAudioModePanels();
        MixBusBox.IsEnabled = MixTargetBox.SelectedItem is MixTargetItem { IsMultiview: false };
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

    private static IEivizBackend? CurrentBackend =>
        Application.Current is App { Backend: { } backend } ? backend : null;

    private static string DiscoverHost(string kind, string query = "") =>
        CurrentBackend?.Discover(kind, query) ?? "";

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

    private async void RefreshOmt()
    {
        var text = await Task.Run(() => DiscoverHost("omt"));
        if (!Dispatcher.CheckAccess())
            return;
        OmtList.ItemsSource = InputHostDiscovery.Lines(text);
    }

    private void RefreshNdi_Click(object sender, RoutedEventArgs e) => RefreshNdi();

    private async void RefreshNdi()
    {
        if (NdiStatus is not null)
            NdiStatus.Text = "Discovering…";
        var text = await Task.Run(() => DiscoverHost("ndi"));
        if (!Dispatcher.CheckAccess())
            return;
        NdiList.ItemsSource = InputHostDiscovery.Lines(text);
        if (NdiStatus is null)
            return;
        NdiStatus.Text = NdiList.Items.Count == 0 && CurrentBackend is { IsRemote: false }
            ? MixerNative.LastErrorText()
            : "";
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

    private async void RefreshUvc()
    {
        try
        {
            var payload = await Task.Run(() => DiscoverHost("uvc"));
            if (!Dispatcher.CheckAccess())
                return;
            UvcList.ItemsSource = InputHostDiscovery.Captures(payload)
                .Select(item => new CameraItem(item.Name, item.Id))
                .ToArray();
            RefreshUvcModes();
        }
        catch (Exception ex)
        {
            UvcList.ItemsSource = new[] { new CameraItem($"Capture enum failed: {ex.Message}", "") };
        }
    }

    private void UvcList_SelectionChanged(object sender, SelectionChangedEventArgs e) => RefreshUvcModes();

    private async void RefreshUvcModes()
    {
        UvcModeBox.Items.Clear();
        if (UvcList.SelectedItem is not CameraItem camera || string.IsNullOrEmpty(camera.Link))
            return;
        var link = camera.Link;
        var payload = await Task.Run(() => DiscoverHost("uvcModes", link));
        if (!Dispatcher.CheckAccess())
            return;
        if (UvcList.SelectedItem is not CameraItem selected || selected.Link != link)
            return;
        UvcModeBox.Items.Clear();
        foreach (var mode in InputHostDiscovery.Modes(payload))
        {
            var fps = mode.FpsDen == 0 ? 0 : mode.FpsNum / (double)mode.FpsDen;
            UvcModeBox.Items.Add(new ComboBoxItem
            {
                Content = $"{mode.Width}x{mode.Height} {fps:0.##}fps",
                Tag = mode
            });
        }
        if (UvcModeBox.Items.Count > 0)
            UvcModeBox.SelectedIndex = 0;
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
                if (!File.Exists(ResultPath))
                {
                    MessageBox.Show(this, Loc.MissingFile("Still load"), Loc.T("msg.addInput"));
                    return;
                }
                ResultName = Path.GetFileName(ResultPath);
                Remember(StillHistory, ResultPath);
                break;
            case InputKind.Video:
                if (string.IsNullOrWhiteSpace(VideoPath.Text))
                    return;
                ResultPath = VideoPath.Text.Trim();
                if (!File.Exists(ResultPath))
                {
                    MessageBox.Show(this, Loc.MissingFile("Video start"), Loc.T("msg.addInput"));
                    return;
                }
                ResultName = System.IO.Path.GetFileName(ResultPath);
                ResultVideoLoop = VideoLoopBox.IsChecked == true;
                ResultVideoPlayWhen = ReadVideoPlayWhen(VideoPlayBox);
                ResultVideoRestartWhen = ReadVideoTriggerWhen(VideoRestartBox);
                ResultVideoPauseWhen = ReadVideoTriggerWhen(VideoPauseBox);
                ResultFrameBufferFrames = ReadBuffer(VideoBufferBox, 3);
                Remember(VideoHistory, ResultPath);
                break;
            case InputKind.OMT:
                if (string.IsNullOrWhiteSpace(OmtAddress.Text))
                    return;
                ResultPath = OmtAddress.Text.Trim();
                ResultName = ResultPath;
                ResultUseGpu = OmtPathBox.SelectedItem is ComboBoxItem { Tag: "gpu" };
                ResultFrameBufferFrames = ReadBuffer(OmtBufferBox, 1);
                ResultSaveMode = ReadSaveMode(OmtSaveBox);
                ResultKeepFullOnMultiview = OmtMvBox.IsChecked == true;
                ResultOmtQuality = ReadOmtQuality(OmtQualityBox);
                break;
            case InputKind.NDI:
                if (string.IsNullOrWhiteSpace(NdiAddress.Text))
                    return;
                ResultPath = NdiAddress.Text.Trim();
                ResultName = ResultPath;
                ResultUseGpu = false;
                ResultFrameBufferFrames = ReadBuffer(NdiBufferBox, 1);
                ResultNdiBandwidth = ReadNdiBandwidth(NdiBandwidthBox);
                break;
            case InputKind.Mix:
                if (MixTargetBox.SelectedItem is not MixTargetItem target)
                    return;
                ResultMixTargetId = target.Id;
                ResultMixSource = target.IsMultiview
                    ? MixSource.SessionMultiview
                    : MixBusBox.SelectedItem is ComboBoxItem { Tag: "preview" }
                        ? MixSource.MuPreview
                        : MixSource.MuProgram;
                ResultMixAudioBusId = ReadMixAudioBusId();
                ResultFrameBufferFrames = ReadBuffer(MixBufferBox, 1);
                ResultPath = "";
                ResultName = target.IsMultiview
                    ? $"{target.Name} MV"
                    : $"{target.Name} {(ResultMixSource == MixSource.MuPreview ? "PRV" : "PGM")}";
                break;
            case InputKind.Audio:
                ResultAudioMapLeft = 0;
                ResultAudioMapRight = 1;
                ResultAudioDeviceKind = AudioDeviceKind.Wasapi;
                if (AudioModeBox.SelectedItem is ComboBoxItem { Tag: "process" })
                {
                    if (AudioProcessBox.SelectedItem is not AudioProcessItem process
                        || (string.IsNullOrWhiteSpace(process.Exe) && string.IsNullOrWhiteSpace(process.Aumid)))
                        return;
                    ResultAudioCaptureMode = AudioCaptureMode.ProcessLoopback;
                    ResultAudioDeviceId = "";
                    ResultAudioProcessExe = process.Exe;
                    ResultAudioProcessAumid = process.Aumid;
                    ResultPath = process.Exe;
                    ResultName = process.Name;
                    break;
                }
                ResultAudioProcessExe = "";
                ResultAudioProcessAumid = "";
                ResultAudioCaptureMode = AudioModeBox.SelectedItem is ComboBoxItem { Tag: "loopback" }
                    ? AudioCaptureMode.EndpointLoopback
                    : AudioCaptureMode.Mic;
                if (AudioDeviceBox.SelectedItem is AudioDeviceItem device)
                {
                    ResultAudioDeviceKind = device.Kind == 2 ? AudioDeviceKind.Asio : AudioDeviceKind.Wasapi;
                    ResultAudioDeviceId = device.Id;
                    ResultName = device.Name;
                }
                else
                {
                    ResultAudioDeviceId = "";
                    ResultName = ResultAudioCaptureMode == AudioCaptureMode.EndpointLoopback
                        ? "Default output loopback"
                        : "Default microphone";
                }
                ResultPath = ResultAudioDeviceId;
                break;
            case InputKind.UVC:
                if (UvcList.SelectedItem is not CameraItem camera || string.IsNullOrEmpty(camera.Link))
                    return;
                if (UvcModeBox.SelectedItem is not ComboBoxItem modeItem || modeItem.Tag is not MixerVideoCaptureMode mode)
                    return;
                ResultPath = camera.Link;
                ResultName = camera.Name;
                ResultCaptureWidth = mode.Width;
                ResultCaptureHeight = mode.Height;
                ResultCaptureFpsNum = mode.FpsNum;
                ResultCaptureFpsDen = mode.FpsDen == 0 ? 1 : mode.FpsDen;
                ResultFrameBufferFrames = ReadBuffer(UvcBufferBox, 3);
                break;
            default:
                return;
        }
        if (!string.IsNullOrWhiteSpace(NameBox.Text))
            ResultName = NameBox.Text.Trim();
        ResultTags = _tags?.Selected.ToArray() ?? [];
        DialogResult = true;
    }

    private void MixTarget_Changed(object sender, SelectionChangedEventArgs e)
    {
        MixBusBox.IsEnabled = MixTargetBox.SelectedItem is MixTargetItem { IsMultiview: false };
    }

    private void AddTag_Click(object sender, RoutedEventArgs e) => _tags?.PromptAdd();

    private ulong ReadMixAudioBusId()
    {
        if (MixAudioBox.SelectedItem is ComboBoxItem item && item.Tag is string tag && ulong.TryParse(tag, out var id))
            return id;
        return 0;
    }

    private static uint ReadBuffer(ComboBox box, uint fallback)
    {
        if (box.SelectedItem is ComboBoxItem item && item.Tag is string tag && uint.TryParse(tag, out var frames))
            return Math.Clamp(frames, 1u, 8u);
        return fallback;
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

    private void Remember(List<string> history, string path)
    {
        if (ReferenceEquals(history, AppPrefs.Current.RecentStills))
            AppPrefs.Current.RememberStill(path);
        else
            AppPrefs.Current.RememberVideo(path);
    }

    private void AudioMode_Changed(object sender, SelectionChangedEventArgs e)
    {
        UpdateAudioModePanels();
        if (AudioModeBox?.SelectedItem is ComboBoxItem { Tag: "process" })
            _ = RefreshAudioProcesses();
        else
            RefreshAudioDevices();
    }

    private void UpdateAudioModePanels()
    {
        var process = AudioModeBox?.SelectedItem is ComboBoxItem { Tag: "process" };
        if (AudioDevicePanel is not null)
            AudioDevicePanel.Visibility = process ? Visibility.Collapsed : Visibility.Visible;
        if (AudioProcessPanel is not null)
            AudioProcessPanel.Visibility = process ? Visibility.Visible : Visibility.Collapsed;
    }

    private void RefreshAudioProcesses_Click(object sender, RoutedEventArgs e) =>
        _ = RefreshAudioProcesses();

    private async Task RefreshAudioProcesses(string? selectedExe = null, string? selectedAumid = null)
    {
        if (AudioProcessBox is null)
            return;
        var payload = await Task.Run(() => DiscoverHost("audio"));
        if (!Dispatcher.CheckAccess())
            return;
        AudioProcessBox.Items.Clear();
        foreach (var process in InputHostDiscovery.AudioProcesses(payload))
            AudioProcessBox.Items.Add(new AudioProcessItem(process.Name, process.Exe, process.Aumid));
        if (AudioProcessBox.Items.Count == 0)
            return;
        if (!string.IsNullOrWhiteSpace(selectedExe) || !string.IsNullOrWhiteSpace(selectedAumid))
        {
            foreach (AudioProcessItem item in AudioProcessBox.Items)
            {
                if ((!string.IsNullOrWhiteSpace(selectedAumid)
                        && string.Equals(item.Aumid, selectedAumid, StringComparison.OrdinalIgnoreCase))
                    || (!string.IsNullOrWhiteSpace(selectedExe)
                        && string.Equals(item.Exe, selectedExe, StringComparison.OrdinalIgnoreCase)))
                {
                    AudioProcessBox.SelectedItem = item;
                    return;
                }
            }
        }
        AudioProcessBox.SelectedIndex = 0;
    }

    private void RefreshAudioDevices()
    {
        if (AudioDeviceBox is null)
            return;
        var loopback = AudioModeBox?.SelectedItem is ComboBoxItem { Tag: "loopback" };
        AudioDeviceBox.Items.Clear();
        AudioDeviceBox.Items.Add(new AudioDeviceItem(
            loopback ? "Default output (follow)" : "Default microphone (follow)",
            "",
            1));
        foreach (var device in Media.AudioGraphSync.EnumerateDevices(0))
        {
            var capture = device.Direction == 1;
            var canLoop = (device.Caps & 2) != 0;
            if (loopback ? !canLoop : !capture)
                continue;
            AudioDeviceBox.Items.Add(new AudioDeviceItem(device.Name, device.Id, device.Kind));
        }
        AudioDeviceBox.SelectedIndex = 0;
    }

    private sealed record AudioDeviceItem(string Name, string Id, uint Kind)
    {
        public override string ToString() => Name;
    }

    private sealed record AudioProcessItem(string Name, string Exe, string Aumid)
    {
        public override string ToString() =>
            string.IsNullOrWhiteSpace(Aumid) ? Name : $"{Name} ({Aumid})";
    }

    private sealed record CameraItem(string Name, string Link)
    {
        public override string ToString() => Name;
    }

    private sealed record MixTargetItem(string Name, ulong Id, bool IsMultiview)
    {
        public override string ToString() => IsMultiview ? $"{Name} (Multiview)" : Name;
    }
}
