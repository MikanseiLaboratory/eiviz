using System.Diagnostics;
using System.IO;
using System.Net;
using System.Net.Sockets;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
using System.Windows;
using System.Windows.Controls;
using Eiviz.Host.I18n;
using Eiviz.Host.Interop;
using Eiviz.Host.Media;
using Eiviz.Host.Preview;

namespace Eiviz.Host.Dialogs;

public partial class SettingsWindow : Window
{
    private Session _session => ((App)Application.Current).Session;
    private ulong _nextOutputId;
    private HttpListener? _licenseReturn;
    private CancellationTokenSource? _licenseReturnCancel;

    public SettingsWindow(Session session, int category = 0)
    {
        InitializeComponent();
        Closed += (_, _) => StopLicenseReturn();
        if (category > 0 && category < CategoryList.Items.Count)
            CategoryList.SelectedIndex = category;
        if (App.IsRemote && OpenMvButton is not null)
            OpenMvButton.Visibility = Visibility.Collapsed;
        _nextOutputId = session.NextOutputId;
        OnAirLock.Changed += ApplyOnAirLock;
        Closed += (_, _) => OnAirLock.Changed -= ApplyOnAirLock;
        Settings = new SessionSettings
        {
            MasterFpsNum = session.Settings.MasterFpsNum,
            MasterFpsDen = session.Settings.MasterFpsDen,
            DefaultWidth = session.Settings.DefaultWidth,
            DefaultHeight = session.Settings.DefaultHeight,
            Theme = session.Settings.Theme,
            DefaultMultiviewUnitId = session.Settings.DefaultMultiviewUnitId,
            FrameBufferFrames = session.Settings.FrameBufferFrames,
            DefaultPresentInterval = session.Settings.DefaultPresentInterval,
            FlipSwapchainLimit = session.Settings.FlipSwapchainLimit,
            InternalColorFormat = session.Settings.InternalColorFormat,
            RebarOptimization = session.Settings.RebarOptimizationEnabled,
            NdiGpuUpload = session.Settings.NdiGpuUploadEnabled,
            PreviewColor = RgbColor.FromOrDefault(session.Settings.PreviewColor, RgbColor.PreviewDefault),
            ProgramColor = RgbColor.FromOrDefault(session.Settings.ProgramColor, RgbColor.ProgramDefault),
            InactiveColor = RgbColor.FromOrDefault(session.Settings.InactiveColor, RgbColor.InactiveDefault),
            MultiviewLabelSize = session.Settings.MultiviewLabelSize,
            MultiviewLabelUnit = session.Settings.MultiviewLabelUnit,
            MultiviewLabelAnchor = session.Settings.MultiviewLabelAnchor,
            VmixApiEnabled = session.Settings.VmixApiEnabledValue,
            VmixApiPort = session.Settings.VmixApiPort == 0 ? 8088 : session.Settings.VmixApiPort,
            VmixApiUser = session.Settings.VmixApiUser ?? "",
            VmixApiPassword = session.Settings.VmixApiPassword ?? "",
            VmixTcpEnabled = session.Settings.VmixTcpEnabledValue,
            NativeApiEnabled = session.Settings.NativeApiEnabledValue,
            NativeApiPort = session.Settings.NativeApiPort == 0 ? 9400 : session.Settings.NativeApiPort
        };
        foreach (var output in session.Outputs)
        {
            Outputs.Add(Clone(output));
            _nextOutputId = Math.Max(_nextOutputId, output.Id + 1);
        }
        _nextStreamEncoderId = session.NextStreamEncoderId == 0 ? 1 : session.NextStreamEncoderId;
        foreach (var encoder in session.StreamEncoders)
        {
            StreamEncoders.Add(encoder.Clone());
            _nextStreamEncoderId = Math.Max(_nextStreamEncoderId, encoder.Id + 1);
        }
        SelectTag(FpsBox, $"{Settings.MasterFpsNum}/{Settings.MasterFpsDen}");
        SelectTag(SizeBox, $"{Settings.DefaultWidth}x{Settings.DefaultHeight}");
        SelectTag(BufferBox, Settings.FrameBufferFrames.ToString());
        SelectTag(ColorFormatBox, Settings.InternalColorFormat == InternalColorFormat.Bgra ? "bgra" : "uyvy");
        SelectTag(MvPresentBox, MultiviewLayout.ClampPresentInterval(Settings.DefaultPresentInterval == 0 ? 3 : Settings.DefaultPresentInterval).ToString());
        SelectTag(FlipBudgetBox, Settings.FlipSwapchainLimit.ToString());
        MvUnitBox.ItemsSource = session.Units;
        MvUnitBox.SelectedItem = session.Units.FirstOrDefault(item => item.Id == Settings.DefaultMultiviewUnitId)
            ?? session.Units.FirstOrDefault();
        Headphone = session.Headphone.Clone();
        HeadphoneCopyMonitor = session.HeadphoneCopyMonitor;
        _devices = AudioGraphSync.EnumerateDevices(0)
            .Where(device => device.Direction != 1)
            .Select(device => (device.Kind, device.Channels, device.Id, device.Name))
            .ToList();
        RebuildOutputs();
        RebuildEncodes();
        RebuildStreams();
        RebuildLayouts();
        RebuildBuses();
        ApplyOnAirLock();
        FillRebar();
        PaintBusColors();
        WebApiEnabledBox.IsChecked = Settings.VmixApiEnabledValue;
        WebApiTcpEnabledBox.IsChecked = Settings.VmixTcpEnabledValue;
        WebApiWsEnabledBox.IsChecked = Settings.NativeApiEnabledValue;
        WebApiPortBox.Text = Settings.VmixApiPort.ToString();
        WebApiWsPortBox.Text = Settings.NativeApiPort.ToString();
        WebApiUserBox.Text = Settings.VmixApiUser;
        WebApiPasswordBox.Password = Settings.VmixApiPassword;
        RefreshFlipBudgetUsage();
    }

    public SessionSettings Settings { get; }
    public List<OutputEntry> Outputs { get; } = [];
    public List<StreamEncoderEntry> StreamEncoders { get; } = [];
    public HeadphoneEntry Headphone { get; private set; } = new();
    public bool HeadphoneCopyMonitor { get; private set; }

    private void ApplyOnAirLock()
    {
        var locked = OnAirLock.Active;
        if (AddOutputButton is not null)
            AddOutputButton.IsEnabled = !locked;
        if (AddEncodeButton is not null)
            AddEncodeButton.IsEnabled = !locked;
        if (AddStreamButton is not null)
            AddStreamButton.IsEnabled = !locked;
        if (FpsBox is not null)
            FpsBox.IsEnabled = !locked;
        if (SizeBox is not null)
            SizeBox.IsEnabled = !locked;
        if (OutputRows is not null)
            RebuildOutputs();
        if (EncodeRows is not null)
            RebuildEncodes();
        if (StreamRows is not null)
            RebuildStreams();
    }
    public ulong NextOutputId => _nextOutputId;
    public ulong NextStreamEncoderId => _nextStreamEncoderId;
    private ulong _nextStreamEncoderId = 1;
    private bool _suppressOutputs;
    private bool _suppressCategory;
    private bool _fillingEncode;
    private int _categoryIndex;
    private List<DeckLinkDeviceInfo>? _decklinkDevices;
    private string _decklinkDeviceError = "";
    private readonly Dictionary<string, (List<DeckLinkModeInfo> Modes, string Error)> _decklinkModes = new(StringComparer.Ordinal);
    private bool _rebarAvailable;
    private List<(uint Kind, uint Channels, string Id, string Name)> _devices = [];

    private void CategoryList_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (_suppressCategory)
            return;
        if (DisplayPanel is null || PerformancePanel is null || MultiviewPanel is null || AudioBusPanel is null || AdvancedPanel is null || WebApiPanel is null || LicensePanel is null || StreamPanel is null)
            return;
        var index = CategoryList.SelectedIndex;
        if (index == 3 && !PlanGate.EncodeSettingsOpen)
        {
            if (IsVisible)
                PlanGate.Tip(this, "msg.planEncodeSettings");
            _suppressCategory = true;
            CategoryList.SelectedIndex = _categoryIndex;
            _suppressCategory = false;
            return;
        }
        _categoryIndex = index;
        DisplayPanel.Visibility = index == 0 ? Visibility.Visible : Visibility.Collapsed;
        PerformancePanel.Visibility = index == 1 ? Visibility.Visible : Visibility.Collapsed;
        OutputPanel.Visibility = index == 2 ? Visibility.Visible : Visibility.Collapsed;
        StreamPanel.Visibility = index == 3 ? Visibility.Visible : Visibility.Collapsed;
        MultiviewPanel.Visibility = index == 4 ? Visibility.Visible : Visibility.Collapsed;
        AudioBusPanel.Visibility = index == 5 ? Visibility.Visible : Visibility.Collapsed;
        WebApiPanel.Visibility = index == 6 ? Visibility.Visible : Visibility.Collapsed;
        AdvancedPanel.Visibility = index == 7 ? Visibility.Visible : Visibility.Collapsed;
        LicensePanel.Visibility = index == 8 ? Visibility.Visible : Visibility.Collapsed;
        if (index == 7)
            RefreshFlipBudgetUsage();
        if (index == 8)
            RefreshLicense();
    }

    private void RefreshFlipBudgetUsage()
    {
        FlipBudgetUsage.Text = Loc.Format("settings.flipBudgetUsage", FlipBudget.Used, FlipBudget.Limit);
    }

    private void LicenseInstall_Click(object sender, RoutedEventArgs e)
    {
        var code = MixerNative.LicenseInstall(LicenseTicketBox.Text.Trim());
        RefreshLicense();
        if (code == 0)
            RebuildOutputs();
    }

    private void LicenseClear_Click(object sender, RoutedEventArgs e)
    {
        MixerNative.LicenseClear();
        LicenseTicketBox.Text = "";
        RefreshLicense();
        RebuildOutputs();
    }

    private void LicenseCopy_Click(object sender, RoutedEventArgs e)
    {
        if (!string.IsNullOrEmpty(LicenseFingerprintBox.Text))
            Clipboard.SetText(LicenseFingerprintBox.Text);
    }

    private void LicenseOpen_Click(object sender, RoutedEventArgs e)
    {
        var url = Loc.T("settings.licenseUrl");
        var query = new System.Collections.Generic.List<string>();
        var fingerprints = LicenseFingerprintBox.Text.Trim();
        if (!string.IsNullOrEmpty(fingerprints))
            query.Add("fingerprints=" + Uri.EscapeDataString(fingerprints));
        var callback = StartLicenseReturn();
        if (callback is not null)
            query.Add("return=" + Uri.EscapeDataString(callback));
        if (query.Count > 0)
            url += "?" + string.Join("&", query);
        Process.Start(new ProcessStartInfo(url) { UseShellExecute = true });
    }

    private string? StartLicenseReturn()
    {
        StopLicenseReturn();
        var probe = new TcpListener(IPAddress.Loopback, 0);
        probe.Start();
        var port = ((IPEndPoint)probe.LocalEndpoint).Port;
        probe.Stop();
        var listener = new HttpListener();
        listener.Prefixes.Add($"http://127.0.0.1:{port}/");
        try
        {
            listener.Start();
        }
        catch (HttpListenerException)
        {
            return null;
        }
        var cancel = new CancellationTokenSource();
        _licenseReturn = listener;
        _licenseReturnCancel = cancel;
        _ = Task.Run(() => AcceptLicenseReturn(listener, cancel.Token));
        return $"http://127.0.0.1:{port}/callback";
    }

    private void StopLicenseReturn()
    {
        _licenseReturnCancel?.Cancel();
        _licenseReturn?.Close();
        _licenseReturn = null;
        _licenseReturnCancel = null;
    }

    private void AcceptLicenseReturn(HttpListener listener, CancellationToken cancel)
    {
        try
        {
            while (!cancel.IsCancellationRequested)
            {
                var context = listener.GetContext();
                var ticket = context.Request.QueryString["ticket"] ?? "";
                var code = -1;
                if (!string.IsNullOrWhiteSpace(ticket))
                {
                    Dispatcher.Invoke(() =>
                    {
                        code = MixerNative.LicenseInstall(ticket.Trim());
                        LicenseTicketBox.Text = ticket.Trim();
                        RefreshLicense();
                        if (code == 0)
                            RebuildOutputs();
                    });
                }
                var message = code == 0
                    ? "eivizにライセンスを登録しました。このタブを閉じてください。"
                    : "eivizへの登録に失敗しました。このタブを閉じて、表示されたチケットをライセンス入力欄へ貼り付けてください。";
                var body = Encoding.UTF8.GetBytes(
                    $"<!DOCTYPE html><html lang=\"ja\"><meta charset=\"utf-8\"><title>eiviz</title><p>{message}</p></html>");
                context.Response.ContentType = "text/html; charset=utf-8";
                context.Response.ContentLength64 = body.Length;
                context.Response.OutputStream.Write(body, 0, body.Length);
                context.Response.OutputStream.Close();
            }
        }
        catch (Exception ex) when (ex is HttpListenerException or ObjectDisposedException or IOException)
        {
        }
    }

    private void RefreshLicense()
    {
        LicenseEditionText.Text = MainWindow.CurrentEdition() ?? "-";
        if (Owner is MainWindow owner)
            owner.ApplyEditionTitle();
        var status = MixerNative.QueryLicenseStatus(out var ticketId);
        if (status is null)
        {
            LicenseStatusText.Text = Loc.T("settings.licenseUnavailable");
            LicenseAccountText.Text = "-";
            LicenseExpiryText.Text = "-";
            LicenseFingerprintBox.Text = "";
            return;
        }
        var state = status.Value.State switch
        {
            1 => Loc.T("settings.licenseValid"),
            2 => Loc.T("settings.licenseExpired"),
            3 => Loc.T("settings.licenseFingerprintMismatch"),
            4 => Loc.T("settings.licenseBadSignature"),
            _ => Loc.T("settings.licenseUnregistered"),
        };
        var email = MixerNative.QueryLicenseEmail();
        LicenseAccountText.Text = string.IsNullOrEmpty(email) ? "-" : email;
        LicenseExpiryText.Text = status.Value.ExpiresAt > 0
            ? DateTimeOffset.FromUnixTimeSeconds(status.Value.ExpiresAt).ToLocalTime().ToString("yyyy-MM-dd HH:mm")
            : "-";
        LicenseStatusText.Text = string.IsNullOrEmpty(ticketId) ? state : $"{state}  {ticketId}";
        LicenseFingerprintBox.Text = MixerNative.QueryFingerprint();
    }

    private void Default_Click(object sender, RoutedEventArgs e)
    {
        if (!OnAirLock.Active)
        {
            SelectTag(FpsBox, "60000/1001");
            SelectTag(SizeBox, "1920x1080");
        }
        SelectTag(BufferBox, "3");
        SelectTag(ColorFormatBox, "uyvy");
        SelectTag(MvPresentBox, "3");
        SelectTag(FlipBudgetBox, "0");
        RebarOptBox.IsChecked = _rebarAvailable;
        NdiGpuBox.IsChecked = true;
        Settings.ResetBusColors();
        PaintBusColors();
        WebApiEnabledBox.IsChecked = true;
        WebApiTcpEnabledBox.IsChecked = true;
        WebApiWsEnabledBox.IsChecked = true;
        WebApiPortBox.Text = "8088";
        WebApiWsPortBox.Text = "9400";
        WebApiUserBox.Text = "";
        WebApiPasswordBox.Password = "";
    }

    private void PickPreviewColor_Click(object sender, RoutedEventArgs e)
    {
        if (PickColor("Preview color", Settings.PreviewColor) is { } color)
        {
            Settings.PreviewColor = color;
            PaintBusColors();
        }
    }

    private void PickProgramColor_Click(object sender, RoutedEventArgs e)
    {
        if (PickColor("Program color", Settings.ProgramColor) is { } color)
        {
            Settings.ProgramColor = color;
            PaintBusColors();
        }
    }

    private void PickInactiveColor_Click(object sender, RoutedEventArgs e)
    {
        if (PickColor("Inactive color", Settings.InactiveColor) is { } color)
        {
            Settings.InactiveColor = color;
            PaintBusColors();
        }
    }

    private RgbColor? PickColor(string title, RgbColor current)
    {
        var dialog = new ColorPickWindow(title, current) { Owner = this };
        return dialog.ShowDialog() == true ? dialog.Result : null;
    }

    private void PaintBusColors()
    {
        if (PreviewColorSwatch is null || ProgramColorSwatch is null || InactiveColorSwatch is null)
            return;
        PreviewColorSwatch.Background = BusTheme.PreviewBrush(Settings);
        ProgramColorSwatch.Background = BusTheme.ProgramBrush(Settings);
        InactiveColorSwatch.Background = BusTheme.InactiveBrush(Settings);
    }

    private void FillRebar()
    {
        unsafe
        {
            var info = new MixerRebarInfo();
            if (MixerNative.CopyRebarInfo(&info) != 0)
            {
                AdapterName.Text = Loc.T("rebar.mixerDown");
                RebarStatus.Text = Loc.T("rebar.unknown");
                RebarMemory.Text = "—";
                _rebarAvailable = false;
                RebarOptBox.IsEnabled = false;
                RebarOptBox.IsChecked = false;
                NdiGpuBox.IsChecked = Settings.NdiGpuUploadEnabled;
                return;
            }
            AdapterName.Text = ReadZ(info.Adapter, 128);
            var vulkan = MixerNative.Backend() == 2;
            if (vulkan)
            {
                RebarStatus.Text = info.Available != 0
                    ? Loc.T("rebar.hostVisible")
                    : Loc.T("rebar.hostVisibleOff");
            }
            else if (info.Uma != 0)
                RebarStatus.Text = Loc.T("rebar.na");
            else if (info.Available != 0)
                RebarStatus.Text = Loc.T("rebar.enabled");
            else
                RebarStatus.Text = Loc.T("rebar.disabled");
            var bar = FormatMib(info.BarBytes);
            var vram = FormatMib(info.VramBytes);
            var heaps = info.GpuUploadHeaps != 0 ? "Yes" : "No";
            RebarMemory.Text = vulkan
                ? $"{bar} host-visible  /  {vram} VRAM"
                : $"{bar} BAR  /  {vram} VRAM  ·  GPU upload heaps: {heaps}";
            _rebarAvailable = info.Available != 0;
            RebarOptBox.IsEnabled = _rebarAvailable;
            RebarOptBox.IsChecked = _rebarAvailable && Settings.RebarOptimizationEnabled;
            NdiGpuBox.IsChecked = Settings.NdiGpuUploadEnabled;
        }
    }

    private static unsafe string ReadZ(byte* ptr, int cap)
    {
        var n = 0;
        while (n < cap && ptr[n] != 0)
            n++;
        return n == 0 ? "—" : System.Text.Encoding.UTF8.GetString(new ReadOnlySpan<byte>(ptr, n));
    }

    private static string FormatMib(ulong bytes)
    {
        if (bytes == 0)
            return "—";
        return $"{bytes / (1024.0 * 1024.0):0} MiB";
    }

    private void RebuildLayouts()
    {
        MvList.ItemsSource = null;
        MvList.ItemsSource = _session.Multiviews;
        if (_session.Multiviews.Count > 0 && MvList.SelectedIndex < 0)
            MvList.SelectedIndex = 0;
    }

    private void RebuildBuses()
    {
        if (BusRows is null)
            return;
        BusRows.Children.Clear();
        var bus = Headphone;
        var box = new Border
        {
            BorderBrush = new System.Windows.Media.SolidColorBrush(System.Windows.Media.Color.FromRgb(0x44, 0x44, 0x44)),
            BorderThickness = new Thickness(1),
            Padding = new Thickness(8),
            Margin = new Thickness(0, 0, 0, 8)
        };
        var grid = new Grid();
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(72) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(72) });
        grid.RowDefinitions.Add(new RowDefinition());
        grid.RowDefinitions.Add(new RowDefinition());
        var kind = new ComboBox { Margin = new Thickness(0, 0, 8, 6) };
        kind.Items.Add(new ComboBoxItem { Content = "None", Tag = AudioDeviceKind.None });
        kind.Items.Add(new ComboBoxItem { Content = "WASAPI", Tag = AudioDeviceKind.Wasapi });
        kind.Items.Add(new ComboBoxItem { Content = "ASIO", Tag = AudioDeviceKind.Asio });
        kind.Items.Add(new ComboBoxItem { Content = "Core Audio", Tag = AudioDeviceKind.CoreAudio });
        kind.SelectedIndex = (int)bus.DeviceKind;
        kind.SelectionChanged += (_, _) =>
        {
            if (kind.SelectedItem is ComboBoxItem item && item.Tag is AudioDeviceKind value)
            {
                bus.DeviceKind = value;
                if (value == AudioDeviceKind.None)
                    bus.DeviceId = "";
                RebuildBuses();
            }
        };
        var device = new ComboBox { Margin = new Thickness(0, 0, 8, 6) };
        var left = new ComboBox { Margin = new Thickness(0, 0, 8, 6) };
        var right = new ComboBox { Margin = new Thickness(0, 0, 8, 6) };
        FillDeviceBox(device, bus.DeviceKind, bus.DeviceId);
        if (device.SelectedItem is ComboBoxItem selected && selected.Tag is string selectedId)
            bus.DeviceId = selectedId;
        FillMapBoxes(left, right, bus);
        device.Visibility = bus.DeviceKind == AudioDeviceKind.None ? Visibility.Collapsed : Visibility.Visible;
        device.SelectionChanged += (_, _) =>
        {
            if (device.SelectedItem is ComboBoxItem item && item.Tag is string id)
            {
                bus.DeviceId = id;
                FillMapBoxes(left, right, bus);
            }
        };
        left.SelectionChanged += (_, _) =>
        {
            if (left.SelectedItem is ComboBoxItem { Tag: int value })
                bus.MapLeft = value;
        };
        right.SelectionChanged += (_, _) =>
        {
            if (right.SelectedItem is ComboBoxItem { Tag: int value })
                bus.MapRight = value;
        };
        var mapVisible = bus.DeviceKind == AudioDeviceKind.None ? Visibility.Collapsed : Visibility.Visible;
        var leftLabel = new TextBlock { Text = "L ch", Foreground = System.Windows.Media.Brushes.Silver, Margin = new Thickness(0, 0, 8, 2), Visibility = mapVisible };
        var rightLabel = new TextBlock { Text = "R ch", Foreground = System.Windows.Media.Brushes.Silver, Margin = new Thickness(0, 0, 8, 2), Visibility = mapVisible };
        left.Visibility = mapVisible;
        right.Visibility = mapVisible;
        Grid.SetRow(kind, 0);
        Grid.SetRow(device, 0);
        Grid.SetColumn(device, 1);
        Grid.SetColumnSpan(device, 3);
        Grid.SetRow(leftLabel, 1);
        Grid.SetRow(left, 1);
        Grid.SetColumn(left, 2);
        Grid.SetRow(rightLabel, 1);
        Grid.SetColumn(rightLabel, 1);
        Grid.SetRow(right, 1);
        Grid.SetColumn(right, 3);
        grid.Children.Add(kind);
        grid.Children.Add(device);
        grid.Children.Add(leftLabel);
        grid.Children.Add(left);
        grid.Children.Add(rightLabel);
        grid.Children.Add(right);
        box.Child = grid;
        BusRows.Children.Add(box);
    }

    private void FillDeviceBox(ComboBox box, AudioDeviceKind kind, string deviceId)
    {
        box.Items.Clear();
        box.Items.Add(new ComboBoxItem { Content = kind is AudioDeviceKind.Wasapi or AudioDeviceKind.CoreAudio ? "Default" : "(none)", Tag = "" });
        foreach (var device in _devices.Where(item => item.Kind == (uint)kind
            || (kind == AudioDeviceKind.CoreAudio && item.Kind == (uint)AudioDeviceKind.Wasapi)
            || (kind == AudioDeviceKind.Wasapi && item.Kind == (uint)AudioDeviceKind.CoreAudio)))
        {
            var label = string.IsNullOrWhiteSpace(device.Name) ? device.Id : device.Name;
            box.Items.Add(new ComboBoxItem { Content = label, Tag = device.Id });
        }
        box.SelectedIndex = 0;
        for (var i = 0; i < box.Items.Count; i++)
        {
            if (box.Items[i] is ComboBoxItem item && Equals(item.Tag, deviceId ?? ""))
            {
                box.SelectedIndex = i;
                break;
            }
        }
    }

    private void FillMapBoxes(ComboBox left, ComboBox right, HeadphoneEntry bus)
    {
        var channels = OutputChannels(bus.DeviceKind, bus.DeviceId ?? "");
        FillMapBox(left, channels, bus.MapLeft);
        FillMapBox(right, channels, bus.MapRight);
        if (left.SelectedItem is ComboBoxItem { Tag: int leftIndex })
            bus.MapLeft = leftIndex;
        if (right.SelectedItem is ComboBoxItem { Tag: int rightIndex })
            bus.MapRight = rightIndex;
    }

    private static void FillMapBox(ComboBox box, int channels, int selected)
    {
        box.Items.Clear();
        for (var index = 0; index < channels; index++)
            box.Items.Add(new ComboBoxItem { Content = (index + 1).ToString(), Tag = index });
        if (box.Items.Count == 0)
            return;
        foreach (ComboBoxItem item in box.Items)
        {
            if (item.Tag is int value && value == selected)
            {
                box.SelectedItem = item;
                return;
            }
        }
        box.SelectedIndex = 0;
    }

    private static int OutputChannels(AudioDeviceKind kind, string deviceId)
    {
        if (kind is AudioDeviceKind.None)
            return 0;
        MixerNative.AudioDeviceIoChannels((uint)kind, deviceId ?? "", out _, out var outputs);
        return outputs;
    }

    private void AddMv_Click(object sender, RoutedEventArgs e)
    {
        if (Owner is MainWindow main)
            main.OpenNewMultiview(Settings.DefaultMultiviewUnitId);
        RebuildLayouts();
        RebuildOutputs();
    }

    private void OpenMv_Click(object sender, RoutedEventArgs e)
    {
        if (MvList.SelectedItem is not MultiviewLayout layout)
        {
            if (Owner is MainWindow mainNew)
                mainNew.OpenNewMultiview(Settings.DefaultMultiviewUnitId);
            RebuildLayouts();
            return;
        }
        if (Owner is MainWindow main)
            main.OpenMultiviewWindow(layout);
    }

    private void EditLayoutTiles_Click(object sender, RoutedEventArgs e)
    {
        if (MvList.SelectedItem is not MultiviewLayout layout)
            return;
        var dialog = new MultiviewSlotsWindow(_session, layout) { Owner = this };
        if (dialog.ShowDialog() != true)
            return;
        var unit = MvUnitBox.SelectedItem as MixingUnitEntry ?? _session.Units[0];
        if (App.IsRemote)
        {
            if (Owner is MainWindow main)
                main.RemoteMutate(MutationJson.UpsertMultiview(layout), Loc.T("chrome.multiview"));
            return;
        }
        MixerApply.PushMultiview(layout, unit.Width, unit.Height);
    }

    private void DeleteMv_Click(object sender, RoutedEventArgs e)
    {
        if (MvList.SelectedItem is not MultiviewLayout layout)
            return;
        if (Owner is MainWindow main)
            main.CloseMultiview(layout.Id);
        if (App.IsRemote)
        {
            if (Owner is MainWindow remote)
                remote.RemoteMutate(MutationJson.DeleteMultiview(layout.Id), Loc.T("chrome.delete"));
            RebuildLayouts();
            return;
        }
        MixerNative.DestroyScene(layout.GpuId);
        _session.Multiviews.Remove(layout);
        RebuildLayouts();
    }

    private void EditTiles_Click(object sender, RoutedEventArgs e)
    {
    }

    private void AddOutput_Click(object sender, RoutedEventArgs e)
    {
        if (OnAirLock.Active)
            return;
        if (_nextOutputId < 100)
            _nextOutputId = 100;
        Outputs.Add(new OutputEntry
        {
            Id = _nextOutputId++,
            Name = NextOutputName(),
            Transport = OutputTransport.Omt,
            SourceKind = OutputSourceKind.MuProgram,
            UnitId = _session.Units.Count > 0 ? _session.Units[0].Id : 1,
            UseGpu = false,
            AudioUnitId = _session.Units.Count > 0 ? _session.Units[0].Id : 1,
            SkipEncodeWhenNoReceivers = true,
            Width = Settings.DefaultWidth,
            Height = Settings.DefaultHeight,
            FpsNum = Settings.MasterFpsNum,
            FpsDen = Settings.MasterFpsDen
        });
        RebuildOutputs();
    }

    private void AddEncode_Click(object sender, RoutedEventArgs e)
    {
        if (!PlanGate.EncodeSettingsOpen)
        {
            PlanGate.Tip(this, "msg.planEncodeSettings");
            return;
        }
        if (OnAirLock.Active)
            return;
        StreamEncoders.Add(NewEncoder());
        RebuildEncodes();
        RebuildStreams();
    }

    private void AddStream_Click(object sender, RoutedEventArgs e)
    {
        if (OnAirLock.Active)
            return;
        if (_nextOutputId < 100)
            _nextOutputId = 100;
        if (StreamEncoders.Count == 0)
        {
            StreamEncoders.Add(NewEncoder());
            RebuildEncodes();
        }
        var unit = _session.Units.Count > 0 ? _session.Units[0].Id : 1;
        Outputs.Add(new OutputEntry
        {
            Id = _nextOutputId++,
            Name = NextStreamName(),
            Transport = OutputTransport.Rtmp,
            SourceKind = OutputSourceKind.MuProgram,
            UnitId = unit,
            AudioUnitId = unit,
            SkipEncodeWhenNoReceivers = false,
            Enabled = false,
            StreamEncoderId = StreamEncoders[0].Id
        });
        RebuildStreams();
    }

    private StreamEncoderEntry NewEncoder()
    {
        if (_nextStreamEncoderId == 0)
            _nextStreamEncoderId = 1;
        var unit = _session.Units.Count > 0 ? _session.Units[0].Id : 1UL;
        return new StreamEncoderEntry
        {
            Id = _nextStreamEncoderId++,
            Name = NextEncodeName(),
            SourceKind = OutputSourceKind.MuProgram,
            UnitId = unit,
            AudioUnitId = unit,
            Width = 1920,
            Height = 1080,
            FpsNum = 60_000,
            FpsDen = 1_001,
            VideoBitrate = 6_000_000,
            AudioBitrate = 128_000,
            Keyint = 120,
            EncoderThreads = 0,
            Encoder = "OpenH264",
            RefFrames = 1,
            Deblock = true
        };
    }

    private string NextEncodeName()
    {
        var prefix = Loc.T("settings.encodeName");
        var gap = Loc.Language == AppLanguage.Ja ? "" : " ";
        for (var n = 1; n < 100; n++)
        {
            var name = prefix + gap + n;
            if (StreamEncoders.TrueForAll(item => item.Name != name))
                return name;
        }
        return prefix;
    }

    private string NextStreamName()
    {
        const string prefix = "Stream";
        if (Outputs.TrueForAll(item => item.Name != prefix))
            return prefix;
        for (var n = 2; n < 100; n++)
        {
            var name = prefix + " " + n;
            if (Outputs.TrueForAll(item => item.Name != name))
                return name;
        }
        return prefix;
    }

    private void RebuildEncodes()
    {
        EncodeRows.Children.Clear();
        for (var i = 0; i < StreamEncoders.Count; i++)
            EncodeRows.Children.Add(BuildEncodeRow(StreamEncoders[i], i));
    }

    private Border BuildEncodeRow(StreamEncoderEntry encoder, int index)
    {
        var locked = OnAirLock.Active;
        var box = StreamCard();
        var grid = new Grid();
        for (var col = 0; col < 3; col++)
            grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(28) });
        for (var row = 0; row < 8; row++)
            grid.RowDefinitions.Add(new RowDefinition());
        var meters = new EncodeMeters();
        void Paint() => PaintEncodeLoad(encoder, meters);

        var name = new TextBox { Text = encoder.Name, Margin = new Thickness(0, 0, 8, 6), IsEnabled = !locked };
        name.TextChanged += (_, _) => encoder.Name = name.Text.Trim();
        var remove = new Button { Content = "−", Width = 28, IsEnabled = !locked };
        remove.Click += (_, _) =>
        {
            if (OnAirLock.Active)
                return;
            StreamEncoders.RemoveAt(index);
            RebuildEncodes();
            RebuildStreams();
        };

        var videoKbps = encoder.VideoBitrate == 0 ? 6000u : encoder.VideoBitrate / 1000;
        var audioKbps = encoder.AudioBitrate == 0 ? 128u : encoder.AudioBitrate / 1000;
        var video = Metered(Loc.T("settings.videoBitrate"), Loc.T("settings.videoBitrateHint"), meters.Video,
            NumberBox(videoKbps.ToString(), !locked, value =>
            {
                if (uint.TryParse(value, out var kbps) && kbps > 0)
                    encoder.VideoBitrate = kbps * 1000;
                Paint();
            }));
        var audio = Metered(Loc.T("settings.audioBitrate"), Loc.T("settings.audioBitrateHint"), meters.Audio,
            NumberBox(audioKbps.ToString(), !locked, value =>
            {
                if (uint.TryParse(value, out var kbps) && kbps > 0)
                    encoder.AudioBitrate = kbps * 1000;
                Paint();
            }));
        var keyint = Metered(Loc.T("settings.keyframeInterval"), Loc.T("settings.keyframeIntervalHint"), meters.Keyint,
            NumberBox((encoder.Keyint == 0 ? 120 : encoder.Keyint).ToString(), !locked, value =>
            {
                if (uint.TryParse(value, out var frames) && frames > 0)
                    encoder.Keyint = frames;
                Paint();
            }));
        var buffer = Metered(Loc.T("settings.sendBuffer"), Loc.T("settings.sendBufferHint"), meters.Buffer,
            NumberBox(encoder.BufferMs.ToString(), !locked, value =>
            {
                if (uint.TryParse(value, out var ms))
                    encoder.BufferMs = Math.Min(ms, 10_000);
                Paint();
            }));

        var size = new ComboBox { IsEnabled = !locked };
        FillEncodeSize(size, encoder);
        var sizeTag = encoder.Width == 0 || encoder.Height == 0 ? "0x0" : $"{encoder.Width}x{encoder.Height}";
        size.SelectionChanged += (_, _) =>
        {
            if (_fillingEncode)
                return;
            if (size.SelectedItem is ComboBoxItem picked && picked.Tag is string tag)
            {
                var pixels = EncodePixels(tag);
                if (!PlanGate.AllowsEncodeSize(pixels.Width, pixels.Height))
                {
                    PlanGate.Tip(this, "msg.planEncodeSize");
                    _fillingEncode = true;
                    SelectTag(size, sizeTag);
                    _fillingEncode = false;
                    return;
                }
                sizeTag = tag;
            }
            if (!OnAirLock.Active)
                ApplyEncodeSize(size, encoder);
            Paint();
        };
        var fps = new ComboBox { IsEnabled = !locked };
        FillEncodeFps(fps, encoder);
        var fpsTag = encoder.FpsNum == 0 || encoder.FpsDen == 0 ? "0/0" : $"{encoder.FpsNum}/{encoder.FpsDen}";
        fps.SelectionChanged += (_, _) =>
        {
            if (_fillingEncode)
                return;
            if (fps.SelectedItem is ComboBoxItem picked && picked.Tag is string tag)
            {
                var rate = EncodeRate(tag);
                if (!PlanGate.AllowsEncodeFps(rate.Num, rate.Den))
                {
                    PlanGate.Tip(this, "msg.planEncodeFps");
                    _fillingEncode = true;
                    SelectTag(fps, fpsTag);
                    _fillingEncode = false;
                    return;
                }
                fpsTag = tag;
            }
            if (!OnAirLock.Active)
                ApplyEncodeFps(fps, encoder);
            Paint();
        };
        var threads = new ComboBox { IsEnabled = !locked };
        FillThreads(threads, encoder);
        threads.SelectionChanged += (_, _) =>
        {
            if (!OnAirLock.Active && threads.SelectedItem is ComboBoxItem item && item.Tag is string tag
                && uint.TryParse(tag, out var count))
                encoder.EncoderThreads = count;
            Paint();
        };
        var encoderBox = new ComboBox { IsEnabled = !locked };
        encoderBox.Items.Add(new ComboBoxItem { Content = "OpenH264", Tag = "OpenH264" });
        if (!string.IsNullOrEmpty(encoder.Encoder) && encoder.Encoder != "OpenH264")
            encoderBox.Items.Add(new ComboBoxItem { Content = encoder.Encoder, Tag = encoder.Encoder });
        SelectStringTag(encoderBox, string.IsNullOrEmpty(encoder.Encoder) ? "OpenH264" : encoder.Encoder);
        encoderBox.SelectionChanged += (_, _) =>
        {
            if (!OnAirLock.Active && encoderBox.SelectedItem is ComboBoxItem item && item.Tag is string tag)
                encoder.Encoder = tag;
            Paint();
        };
        var complexity = new ComboBox { IsEnabled = !locked };
        complexity.Items.Add(new ComboBoxItem { Content = Loc.T("settings.complexityLow"), Tag = "0" });
        complexity.Items.Add(new ComboBoxItem { Content = Loc.T("settings.complexityMedium"), Tag = "1" });
        complexity.Items.Add(new ComboBoxItem { Content = Loc.T("settings.complexityHigh"), Tag = "2" });
        SelectStringTag(complexity, encoder.Complexity > 2 ? "2" : encoder.Complexity.ToString());
        complexity.SelectionChanged += (_, _) =>
        {
            if (!OnAirLock.Active && complexity.SelectedItem is ComboBoxItem item && item.Tag is string tag
                && uint.TryParse(tag, out var level))
                encoder.Complexity = level;
            Paint();
        };
        var refs = new ComboBox { IsEnabled = !locked };
        for (var n = 1; n <= 4; n++)
            refs.Items.Add(new ComboBoxItem { Content = n.ToString(), Tag = n.ToString() });
        SelectStringTag(refs, encoder.RefFrames is >= 1 and <= 4 ? encoder.RefFrames.ToString() : "1");
        refs.SelectionChanged += (_, _) =>
        {
            if (!OnAirLock.Active && refs.SelectedItem is ComboBoxItem item && item.Tag is string tag
                && uint.TryParse(tag, out var count))
                encoder.RefFrames = count;
            Paint();
        };
        var delay = Metered(Loc.T("settings.audioDelay"), Loc.T("settings.audioDelayHint"), meters.Delay,
            NumberBox(encoder.AudioDelayMs.ToString(), !locked, value =>
            {
                if (uint.TryParse(value, out var ms))
                    encoder.AudioDelayMs = Math.Min(ms, 1_000);
                Paint();
            }));
        var entropy = new ComboBox { IsEnabled = !locked };
        entropy.Items.Add(new ComboBoxItem { Content = "CAVLC", Tag = "0" });
        entropy.Items.Add(new ComboBoxItem { Content = "CABAC", Tag = "1" });
        SelectStringTag(entropy, encoder.Cabac ? "1" : "0");
        entropy.SelectionChanged += (_, _) =>
        {
            if (!OnAirLock.Active && entropy.SelectedItem is ComboBoxItem item && item.Tag is string tag)
                encoder.Cabac = tag == "1";
            Paint();
        };
        var toggles = new StackPanel { Margin = new Thickness(0, 4, 0, 0) };
        toggles.Children.Add(MeteredToggle(Loc.T("settings.deblock"), Loc.T("settings.deblockHint"), encoder.Deblock, !locked, meters.Deblock, value =>
        {
            encoder.Deblock = value;
            Paint();
        }));
        toggles.Children.Add(MeteredToggle(Loc.T("settings.backgroundDetect"), Loc.T("settings.backgroundDetectHint"), encoder.BackgroundDetection, !locked, meters.Background, value =>
        {
            encoder.BackgroundDetection = value;
            Paint();
        }));
        toggles.Children.Add(MeteredToggle(Loc.T("settings.adaptiveQuant"), Loc.T("settings.adaptiveQuantHint"), encoder.AdaptiveQuant, !locked, meters.Quant, value =>
        {
            encoder.AdaptiveQuant = value;
            Paint();
        }));
        toggles.Children.Add(MeteredToggle(Loc.T("settings.sceneChange"), Loc.T("settings.sceneChangeHint"), encoder.SceneChange, !locked, meters.Scene, value =>
        {
            encoder.SceneChange = value;
            Paint();
        }));
        var load = LoadSummary(Loc.T("settings.encodeLoad"), Loc.T("settings.encodeLoadHint"), meters.Total);
        var sizePanel = Metered(Loc.T("settings.encodeSize"), Loc.T("settings.encodeSizeHint"), meters.Size, size);
        var fpsPanel = Metered(Loc.T("settings.encodeFps"), Loc.T("settings.encodeFpsHint"), meters.Fps, fps);
        var threadPanel = Metered(Loc.T("settings.threads"), Loc.T("settings.threadsHint"), meters.Threads, threads);
        var encoderPanel = Metered(Loc.T("settings.encoder"), Loc.T("settings.encoderHint"), meters.Encoder, encoderBox);
        var complexityPanel = Metered(Loc.T("settings.complexity"), Loc.T("settings.complexityHint"), meters.Complexity, complexity);
        var refsPanel = Metered(Loc.T("settings.refFrames"), Loc.T("settings.refFramesHint"), meters.Refs, refs);
        var entropyPanel = Metered(Loc.T("settings.entropy"), Loc.T("settings.entropyHint"), meters.Entropy, entropy);

        Grid.SetColumnSpan(name, 3);
        Grid.SetColumn(remove, 3);
        Grid.SetRow(load, 1);
        Grid.SetColumnSpan(load, 3);
        Grid.SetRow(video, 2);
        Grid.SetRow(audio, 2);
        Grid.SetColumn(audio, 1);
        Grid.SetRow(keyint, 2);
        Grid.SetColumn(keyint, 2);
        Grid.SetRow(sizePanel, 3);
        Grid.SetRow(fpsPanel, 3);
        Grid.SetColumn(fpsPanel, 1);
        Grid.SetRow(threadPanel, 4);
        Grid.SetRow(buffer, 4);
        Grid.SetColumn(buffer, 1);
        Grid.SetRow(encoderPanel, 5);
        Grid.SetRow(complexityPanel, 5);
        Grid.SetColumn(complexityPanel, 1);
        Grid.SetRow(refsPanel, 5);
        Grid.SetColumn(refsPanel, 2);
        Grid.SetRow(delay, 6);
        Grid.SetRow(entropyPanel, 6);
        Grid.SetColumn(entropyPanel, 1);
        Grid.SetRow(toggles, 7);
        Grid.SetColumnSpan(toggles, 3);
        grid.Children.Add(name);
        grid.Children.Add(remove);
        grid.Children.Add(load);
        grid.Children.Add(video);
        grid.Children.Add(audio);
        grid.Children.Add(keyint);
        grid.Children.Add(sizePanel);
        grid.Children.Add(fpsPanel);
        grid.Children.Add(threadPanel);
        grid.Children.Add(buffer);
        grid.Children.Add(encoderPanel);
        grid.Children.Add(complexityPanel);
        grid.Children.Add(refsPanel);
        grid.Children.Add(delay);
        grid.Children.Add(entropyPanel);
        grid.Children.Add(toggles);
        Paint();
        box.Child = grid;
        return box;
    }

    private static StackPanel LoadSummary(string label, string hint, ImpactGauge gauge)
    {
        var panel = new StackPanel { Margin = new Thickness(0, 0, 8, 8) };
        var head = new DockPanel { LastChildFill = true };
        DockPanel.SetDock(gauge, Dock.Right);
        head.Children.Add(gauge);
        head.Children.Add(new TextBlock { Text = label, VerticalAlignment = VerticalAlignment.Center });
        panel.Children.Add(head);
        panel.Children.Add(EncodeHint(hint));
        return panel;
    }

    private static TextBox NumberBox(string value, bool enabled, Action<string> changed)
    {
        var box = new TextBox { Text = value, IsEnabled = enabled };
        box.TextChanged += (_, _) => changed(box.Text);
        return box;
    }

    private static StackPanel Metered(string label, string hint, ImpactGauge gauge, UIElement control)
    {
        var panel = new StackPanel { Margin = new Thickness(0, 0, 8, 8) };
        var head = new DockPanel { LastChildFill = true, Margin = new Thickness(0, 0, 0, 2) };
        DockPanel.SetDock(gauge, Dock.Right);
        head.Children.Add(gauge);
        head.Children.Add(new TextBlock { Text = label, VerticalAlignment = VerticalAlignment.Center });
        panel.Children.Add(head);
        panel.Children.Add(control);
        panel.Children.Add(EncodeHint(hint));
        return panel;
    }

    private static StackPanel MeteredToggle(string label, string hint, bool value, bool enabled, ImpactGauge gauge, Action<bool> changed)
    {
        var box = new CheckBox
        {
            Content = label,
            IsChecked = value,
            IsEnabled = enabled,
            VerticalAlignment = VerticalAlignment.Center
        };
        box.Checked += (_, _) => changed(true);
        box.Unchecked += (_, _) => changed(false);
        var panel = new StackPanel { Margin = new Thickness(0, 0, 0, 8) };
        var head = new DockPanel { LastChildFill = true };
        DockPanel.SetDock(gauge, Dock.Right);
        head.Children.Add(gauge);
        head.Children.Add(box);
        panel.Children.Add(head);
        panel.Children.Add(EncodeHint(hint));
        return panel;
    }

    private static TextBlock EncodeHint(string text) => new()
    {
        Text = text,
        TextWrapping = TextWrapping.Wrap,
        Foreground = System.Windows.Media.Brushes.Gray,
        Margin = new Thickness(0, 2, 0, 0)
    };

    private static void PaintEncodeLoad(StreamEncoderEntry encoder, EncodeMeters meters)
    {
        var videoKbps = encoder.VideoBitrate == 0 ? 6000u : encoder.VideoBitrate / 1000;
        meters.Video.Set(videoKbps switch { <= 2500 => 1, <= 4500 => 2, <= 8000 => 3, <= 12000 => 4, _ => 5 });
        meters.Audio.Set(1);
        var key = encoder.Keyint == 0 ? 120 : encoder.Keyint;
        meters.Keyint.Set(key switch { < 30 => 5, < 60 => 4, < 90 => 3, < 180 => 2, _ => 1 });
        var height = encoder.Height == 0 ? 1080 : encoder.Height;
        meters.Size.Set(height switch { >= 2160 => 5, >= 1440 => 4, >= 1080 => 3, >= 720 => 2, _ => 1 });
        var fps = encoder.FpsDen == 0 || encoder.FpsNum == 0 ? 60d : encoder.FpsNum / (double)encoder.FpsDen;
        meters.Fps.Set(fps >= 59 ? 5 : fps >= 50 ? 4 : fps >= 30 ? 2 : 1);
        meters.Threads.Set(encoder.EncoderThreads switch { 1 => 5, 2 => 4, 3 => 3, _ => 1 });
        meters.Buffer.Set(0);
        meters.Encoder.Set(4);
        meters.Complexity.Set(encoder.Complexity switch { 0 => 1, 1 => 3, _ => 5 });
        var refs = encoder.RefFrames is >= 1 and <= 4 ? (int)encoder.RefFrames : 1;
        meters.Refs.Set(refs);
        meters.Delay.Set(0);
        meters.Entropy.Set(encoder.Cabac ? 4 : 1);
        meters.Deblock.Set(encoder.Deblock ? 2 : 1);
        meters.Background.Set(encoder.BackgroundDetection ? 1 : 3);
        meters.Quant.Set(encoder.AdaptiveQuant ? 3 : 1);
        meters.Scene.Set(encoder.SceneChange ? 3 : 1);
        meters.Total.Set(EncodeLoadPips(encoder));
    }

    private static int EncodeLoadPips(StreamEncoderEntry encoder)
    {
        var score = encoder.Complexity switch { 0 => 8, 1 => 18, _ => 30 };
        var refs = encoder.RefFrames is >= 1 and <= 4 ? (int)encoder.RefFrames : 1;
        score += refs * 4;
        if (encoder.Cabac)
            score += 10;
        if (encoder.Deblock)
            score += 6;
        score += encoder.BackgroundDetection ? 2 : 6;
        if (encoder.AdaptiveQuant)
            score += 6;
        if (encoder.SceneChange)
            score += 6;
        score += encoder.EncoderThreads switch { 1 => 16, 2 => 8, 3 => 4, _ => 0 };
        var pixels = encoder.Width == 0 || encoder.Height == 0
            ? 1920L * 1080
            : (long)encoder.Width * encoder.Height;
        score += pixels > 1920L * 1080 ? 28 : pixels >= 1920L * 1080 ? 12 : 6;
        var fpsMilli = encoder.FpsDen == 0 || encoder.FpsNum == 0
            ? 60_000
            : (int)((long)encoder.FpsNum * 1000 / encoder.FpsDen);
        score += fpsMilli >= 50_000 ? 10 : 4;
        var kbps = encoder.VideoBitrate == 0 ? 6000 : (int)(encoder.VideoBitrate / 1000);
        score += Math.Min(kbps / 2000, 8);
        if (encoder.Keyint is > 0 and < 60)
            score += 4;
        score += 8;
        return Math.Clamp((int)Math.Round((score - 45) * 10d / 90d), 1, 10);
    }

    private sealed class EncodeMeters
    {
        public ImpactGauge Total { get; } = new(10);
        public ImpactGauge Video { get; } = new(5);
        public ImpactGauge Audio { get; } = new(5);
        public ImpactGauge Keyint { get; } = new(5);
        public ImpactGauge Size { get; } = new(5);
        public ImpactGauge Fps { get; } = new(5);
        public ImpactGauge Threads { get; } = new(5);
        public ImpactGauge Buffer { get; } = new(5);
        public ImpactGauge Encoder { get; } = new(5);
        public ImpactGauge Complexity { get; } = new(5);
        public ImpactGauge Refs { get; } = new(5);
        public ImpactGauge Delay { get; } = new(5);
        public ImpactGauge Entropy { get; } = new(5);
        public ImpactGauge Deblock { get; } = new(5);
        public ImpactGauge Background { get; } = new(5);
        public ImpactGauge Quant { get; } = new(5);
        public ImpactGauge Scene { get; } = new(5);
    }

    private sealed class ImpactGauge : StackPanel
    {
        private readonly Border[] _pips;
        private static readonly System.Windows.Media.SolidColorBrush Empty = Frozen(0x33, 0x33, 0x33);
        private static readonly System.Windows.Media.SolidColorBrush Light = Frozen(0x7D, 0xC8, 0x7D);
        private static readonly System.Windows.Media.SolidColorBrush Mid = Frozen(0xE0, 0xC3, 0x6A);
        private static readonly System.Windows.Media.SolidColorBrush Heavy = Frozen(0xE0, 0x78, 0x78);

        public ImpactGauge(int count)
        {
            Orientation = Orientation.Horizontal;
            VerticalAlignment = VerticalAlignment.Center;
            Margin = new Thickness(8, 0, 0, 0);
            _pips = new Border[count];
            var wide = count > 5;
            for (var i = 0; i < count; i++)
            {
                var pip = new Border
                {
                    Width = wide ? 7 : 10,
                    Height = 8,
                    Margin = new Thickness(1, 0, 1, 0),
                    CornerRadius = new CornerRadius(1),
                    Background = Empty
                };
                _pips[i] = pip;
                Children.Add(pip);
            }
        }

        public void Set(int level)
        {
            level = Math.Clamp(level, 0, _pips.Length);
            var ratio = _pips.Length == 0 ? 0d : (double)level / _pips.Length;
            var fill = ratio >= 0.75 ? Heavy : ratio >= 0.45 ? Mid : Light;
            for (var i = 0; i < _pips.Length; i++)
                _pips[i].Background = i < level ? fill : Empty;
        }

        private static System.Windows.Media.SolidColorBrush Frozen(byte r, byte g, byte b)
        {
            var brush = new System.Windows.Media.SolidColorBrush(System.Windows.Media.Color.FromRgb(r, g, b));
            brush.Freeze();
            return brush;
        }
    }

    private static void SelectStringTag(ComboBox box, string tag)
    {
        foreach (var item in box.Items)
        {
            if (item is ComboBoxItem choice && choice.Tag is string value && value == tag)
            {
                box.SelectedItem = choice;
                return;
            }
        }
        if (box.Items.Count > 0)
            box.SelectedIndex = 0;
    }

    private void RebuildStreams()
    {
        StreamRows.Children.Clear();
        for (var i = 0; i < Outputs.Count; i++)
        {
            var index = i;
            var output = Outputs[i];
            if (output.Transport != OutputTransport.Rtmp)
                continue;
            StreamRows.Children.Add(BuildStreamRow(output, index));
        }
    }

    private Border BuildStreamRow(OutputEntry output, int index)
    {
        var locked = OnAirLock.Active;
        var box = StreamCard();
        var grid = new Grid();
        for (var col = 0; col < 3; col++)
            grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(28) });
        for (var row = 0; row < 6; row++)
            grid.RowDefinitions.Add(new RowDefinition());

        var name = new TextBox { Text = output.Name, Margin = new Thickness(0, 0, 8, 6), IsEnabled = !locked };
        name.TextChanged += (_, _) => output.Name = name.Text.Trim();
        var remove = new Button { Content = "−", Width = 28, IsEnabled = !locked };
        remove.Click += (_, _) =>
        {
            if (OnAirLock.Active)
                return;
            Outputs.RemoveAt(index);
            RebuildStreams();
        };

        var encode = new ComboBox { Margin = new Thickness(0, 0, 8, 6), IsEnabled = !locked };
        foreach (var encoder in StreamEncoders)
            encode.Items.Add(new ComboBoxItem { Content = encoder.Name, Tag = encoder.Id });
        SelectUlongTag(encode, output.StreamEncoderId);
        encode.SelectionChanged += (_, _) =>
        {
            if (encode.SelectedItem is ComboBoxItem item && item.Tag is ulong id)
                output.StreamEncoderId = id;
        };
        var encodeLabel = new TextBlock { Text = Loc.T("settings.useEncode"), Margin = new Thickness(0, 0, 8, 2) };

        var server = Labeled(Loc.T("settings.streamServer"), output.RtmpUrl, !locked, value => output.RtmpUrl = value.Trim());
        var key = new PasswordBox { Margin = new Thickness(0, 0, 8, 6), IsEnabled = !locked };
        if (!string.IsNullOrEmpty(output.RtmpSecretRef))
            key.Password = CredentialStore.Load("rtmp/" + output.RtmpSecretRef);
        key.PasswordChanged += (_, _) =>
        {
            if (key.Password.Length == 0)
                return;
            if (string.IsNullOrEmpty(output.RtmpSecretRef))
                output.RtmpSecretRef = "stream-" + output.Id;
            CredentialStore.Save("rtmp/" + output.RtmpSecretRef, key.Password);
        };
        var keyLabel = new TextBlock { Text = Loc.T("settings.streamKey"), Margin = new Thickness(0, 0, 8, 2) };

        Grid.SetColumnSpan(name, 3);
        Grid.SetColumn(remove, 3);
        Grid.SetRow(encodeLabel, 1);
        Grid.SetColumnSpan(encodeLabel, 4);
        Grid.SetRow(encode, 2);
        Grid.SetColumnSpan(encode, 3);
        Grid.SetRow(server, 3);
        Grid.SetColumnSpan(server, 4);
        Grid.SetRow(keyLabel, 4);
        Grid.SetColumnSpan(keyLabel, 4);
        Grid.SetRow(key, 5);
        Grid.SetColumnSpan(key, 4);
        grid.Children.Add(name);
        grid.Children.Add(remove);
        grid.Children.Add(encodeLabel);
        grid.Children.Add(encode);
        grid.Children.Add(server);
        grid.Children.Add(keyLabel);
        grid.Children.Add(key);
        box.Child = grid;
        return box;
    }

    private static Border StreamCard() => new()
    {
        BorderBrush = new System.Windows.Media.SolidColorBrush(System.Windows.Media.Color.FromRgb(0x44, 0x44, 0x44)),
        BorderThickness = new Thickness(1),
        Padding = new Thickness(8),
        Margin = new Thickness(0, 0, 0, 8)
    };

    private static StackPanel Labeled(string label, string value, bool enabled, Action<string> changed)
    {
        var panel = new StackPanel { Margin = new Thickness(0, 0, 8, 6) };
        panel.Children.Add(new TextBlock { Text = label, Margin = new Thickness(0, 0, 0, 2) });
        var box = new TextBox { Text = value, IsEnabled = enabled };
        box.TextChanged += (_, _) => changed(box.Text);
        panel.Children.Add(box);
        return panel;
    }

    private void RebuildOutputs()
    {
        OutputRows.Children.Clear();
        _suppressOutputs = true;
        for (var i = 0; i < Outputs.Count; i++)
        {
            var index = i;
            var output = Outputs[i];
            if (output.Transport == OutputTransport.Rtmp)
                continue;
            var box = new Border
            {
                BorderBrush = new System.Windows.Media.SolidColorBrush(System.Windows.Media.Color.FromRgb(0x44, 0x44, 0x44)),
                BorderThickness = new Thickness(1),
                Padding = new Thickness(8),
                Margin = new Thickness(0, 0, 0, 8)
            };
            var grid = new Grid();
            grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(28) });
            grid.RowDefinitions.Add(new RowDefinition());
            grid.RowDefinitions.Add(new RowDefinition());
            grid.RowDefinitions.Add(new RowDefinition());
            grid.RowDefinitions.Add(new RowDefinition());
            grid.RowDefinitions.Add(new RowDefinition());
            grid.RowDefinitions.Add(new RowDefinition());
            grid.RowDefinitions.Add(new RowDefinition());
            grid.RowDefinitions.Add(new RowDefinition());

            var name = new TextBox { Text = output.Name, Margin = new Thickness(0, 0, 8, 6) };
            name.TextChanged += (_, _) => output.Name = name.Text.Trim();
            var transport = new ComboBox { Margin = new Thickness(0, 0, 8, 6), IsEnabled = !OnAirLock.Active };
            transport.Items.Add(new ComboBoxItem { Content = "OMT", Tag = OutputTransport.Omt });
            transport.Items.Add(new ComboBoxItem { Content = "NDI", Tag = OutputTransport.Ndi });
            var deckLink = new ComboBoxItem { Content = "DeckLink", Tag = OutputTransport.DeckLink };
            if (output.Transport != OutputTransport.DeckLink && !PlanGate.DeckLinkOutput)
                PlanGate.BlockComboItem(deckLink, "msg.planDecklinkOutput");
            transport.Items.Add(deckLink);
            foreach (ComboBoxItem item in transport.Items)
            {
                if (item.Tag is OutputTransport tagged && tagged == output.Transport)
                    transport.SelectedItem = item;
            }
            transport.SelectionChanged += (_, _) =>
            {
                if (_suppressOutputs)
                    return;
                if (transport.SelectedItem is ComboBoxItem item && item.Tag is OutputTransport value)
                {
                    if (value == OutputTransport.DeckLink
                        && output.Transport != OutputTransport.DeckLink
                        && !PlanGate.DeckLinkOutput)
                    {
                        _suppressOutputs = true;
                        foreach (ComboBoxItem existing in transport.Items)
                        {
                            if (existing.Tag is OutputTransport tagged && tagged == output.Transport)
                                transport.SelectedItem = existing;
                        }
                        _suppressOutputs = false;
                        return;
                    }
                    output.Transport = value;
                    if (value != OutputTransport.Omt)
                        output.UseGpu = false;
                }
                RebuildOutputs();
            };

            var locked = OnAirLock.Active;
            var path = new ComboBox { Margin = new Thickness(0, 0, 8, 6), IsEnabled = !locked && output.Transport == OutputTransport.Omt };
            path.Items.Add(new ComboBoxItem { Content = I18n.Loc.T("settings.omtCpu"), Tag = false });
            path.Items.Add(new ComboBoxItem { Content = I18n.Loc.T("settings.omtGpu"), Tag = true });
            path.SelectedIndex = output.UseGpu ? 1 : 0;
            path.SelectionChanged += (_, _) =>
            {
                if (path.SelectedItem is ComboBoxItem item && item.Tag is bool value)
                    output.UseGpu = value;
            };

            var kinds = new WrapPanel { Margin = new Thickness(0, 0, 0, 6) };
            AddKind(kinds, output, OutputSourceKind.Input, "Input", index);
            AddKind(kinds, output, OutputSourceKind.Scene, "Scene", index);
            AddKind(kinds, output, OutputSourceKind.MuPreview, "MU PRV", index);
            AddKind(kinds, output, OutputSourceKind.MuProgram, "MU PGM", index);
            AddKind(kinds, output, OutputSourceKind.Multiview, "Multiview", index);

            var pick = new ComboBox { Margin = new Thickness(0, 0, 8, 6), IsEnabled = !locked };
            FillOutputPick(pick, output);
            pick.SelectionChanged += (_, _) => ApplyOutputPick(pick, output);

            var audio = new ComboBox { Margin = new Thickness(0, 0, 8, 6) };
            FillOutputAudio(audio, output);
            audio.SelectionChanged += (_, _) =>
            {
                if (audio.SelectedItem is AudioChoice choice)
                    output.AudioUnitId = choice.Id;
            };

            var planBlocked = output.Transport == OutputTransport.DeckLink && !PlanGate.DeckLinkOutput;
            var enabled = new CheckBox
            {
                Content = "Enabled",
                IsChecked = output.Enabled && !planBlocked,
                IsEnabled = !locked && !planBlocked,
                Foreground = System.Windows.Media.Brushes.White,
                VerticalAlignment = VerticalAlignment.Center,
                Margin = new Thickness(0, 0, 8, 6)
            };
            if (planBlocked)
            {
                enabled.ToolTip = Loc.T("msg.planDecklinkOutput");
                ToolTipService.SetShowOnDisabled(enabled, true);
            }
            else
            {
                enabled.Checked += (_, _) => output.Enabled = true;
                enabled.Unchecked += (_, _) => output.Enabled = false;
            }

            var skipIdle = new CheckBox
            {
                Content = Loc.T("settings.skipEncodeWhenNoReceivers"),
                IsChecked = output.SkipEncodeWhenNoReceivers,
                IsEnabled = output.Transport == OutputTransport.Omt,
                Foreground = System.Windows.Media.Brushes.White,
                VerticalAlignment = VerticalAlignment.Center,
                Margin = new Thickness(0, 0, 8, 6)
            };
            skipIdle.Checked += (_, _) => output.SkipEncodeWhenNoReceivers = true;
            skipIdle.Unchecked += (_, _) => output.SkipEncodeWhenNoReceivers = false;

            var size = new ComboBox { Margin = new Thickness(0, 0, 8, 6), IsEnabled = !locked };
            FillOutputSize(size, output);
            size.SelectionChanged += (_, _) =>
            {
                if (!OnAirLock.Active)
                    ApplyOutputSize(size, output);
            };

            var fps = new ComboBox { Margin = new Thickness(0, 0, 8, 6), IsEnabled = !locked };
            FillOutputFps(fps, output);
            fps.SelectionChanged += (_, _) =>
            {
                if (!OnAirLock.Active)
                    ApplyOutputFps(fps, output);
            };

            FrameworkElement? endpoint = null;
            if (output.Transport == OutputTransport.Rtmp)
            {
                var url = new TextBox
                {
                    Text = output.RtmpUrl,
                    ToolTip = Loc.T("settings.rtmpUrl"),
                    Margin = new Thickness(0, 0, 8, 6),
                    IsEnabled = !locked
                };
                url.TextChanged += (_, _) => output.RtmpUrl = url.Text.Trim();
                endpoint = url;
            }
            else if (output.Transport == OutputTransport.DeckLink)
            {
                endpoint = DeckLinkOutputFields(output, locked);
            }

            var remove = new Button { Content = "−", Width = 28, IsEnabled = !locked };
            remove.Click += (_, _) =>
            {
                if (OnAirLock.Active)
                    return;
                Outputs.RemoveAt(index);
                RebuildOutputs();
            };

            Grid.SetRow(name, 0);
            Grid.SetColumnSpan(name, 3);
            Grid.SetRow(remove, 0);
            Grid.SetColumn(remove, 3);
            Grid.SetRow(transport, 1);
            Grid.SetRow(path, 1);
            Grid.SetColumn(path, 1);
            Grid.SetRow(enabled, 1);
            Grid.SetColumn(enabled, 2);
            Grid.SetRow(kinds, 2);
            Grid.SetColumnSpan(kinds, 4);
            Grid.SetRow(pick, 3);
            Grid.SetColumnSpan(pick, 3);
            Grid.SetRow(audio, 4);
            Grid.SetColumnSpan(audio, 3);
            Grid.SetRow(size, 5);
            Grid.SetRow(fps, 5);
            Grid.SetColumn(fps, 1);
            Grid.SetRow(skipIdle, 6);
            Grid.SetColumnSpan(skipIdle, 4);
            if (endpoint is not null)
            {
                Grid.SetRow(endpoint, 7);
                Grid.SetColumnSpan(endpoint, 3);
            }
            grid.Children.Add(name);
            grid.Children.Add(remove);
            grid.Children.Add(transport);
            grid.Children.Add(path);
            grid.Children.Add(enabled);
            grid.Children.Add(kinds);
            grid.Children.Add(pick);
            grid.Children.Add(audio);
            grid.Children.Add(size);
            grid.Children.Add(fps);
            grid.Children.Add(skipIdle);
            if (endpoint is not null)
                grid.Children.Add(endpoint);
            box.Child = grid;
            OutputRows.Children.Add(box);
        }
        _suppressOutputs = false;
    }

    private List<DeckLinkDeviceInfo> DeckLinkDevices()
    {
        if (_decklinkDevices is not null)
            return _decklinkDevices;
        _decklinkDevices = MixerNative.QueryDeckLinkDevices(out _decklinkDeviceError);
        return _decklinkDevices;
    }

    private (List<DeckLinkModeInfo> Modes, string Error) DeckLinkModes(string deviceId)
    {
        if (string.IsNullOrWhiteSpace(deviceId))
            return ([], "");
        if (_decklinkModes.TryGetValue(deviceId, out var cached))
            return cached;
        var modes = MixerNative.QueryDeckLinkModes(deviceId, out var error);
        cached = (modes, error);
        _decklinkModes[deviceId] = cached;
        return cached;
    }

    private FrameworkElement DeckLinkOutputFields(OutputEntry output, bool locked)
    {
        var stack = new StackPanel { Margin = new Thickness(0, 0, 8, 6) };
        stack.Children.Add(new TextBlock
        {
            Text = Loc.T("settings.decklinkDevice"),
            Margin = new Thickness(0, 0, 0, 2)
        });
        var devices = DeckLinkDevices()
            .Where(item => item.Playback || item.Id == output.DecklinkDevice)
            .ToList();
        if (!string.IsNullOrWhiteSpace(output.DecklinkDevice) && devices.TrueForAll(item => item.Id != output.DecklinkDevice))
        {
            devices.Insert(0, new DeckLinkDeviceInfo
            {
                Id = output.DecklinkDevice,
                Name = output.DecklinkDevice,
                Playback = true
            });
        }
        var device = new ComboBox
        {
            IsEnabled = !locked,
            DisplayMemberPath = "Label",
            Margin = new Thickness(0, 0, 0, 6)
        };
        foreach (var item in devices)
            device.Items.Add(item);
        foreach (DeckLinkDeviceInfo item in device.Items)
        {
            if (item.Id == output.DecklinkDevice)
            {
                device.SelectedItem = item;
                break;
            }
        }
        device.SelectionChanged += (_, _) =>
        {
            if (_suppressOutputs || device.SelectedItem is not DeckLinkDeviceInfo info)
                return;
            if (info.Id == output.DecklinkDevice)
                return;
            output.DecklinkDevice = info.Id;
            output.DecklinkMode = "";
            RebuildOutputs();
        };
        stack.Children.Add(device);

        stack.Children.Add(new TextBlock
        {
            Text = Loc.T("settings.decklinkMode"),
            Margin = new Thickness(0, 0, 0, 2)
        });
        var listed = DeckLinkModes(output.DecklinkDevice);
        var modes = listed.Modes.ToList();
        if (!string.IsNullOrWhiteSpace(output.DecklinkMode) && modes.TrueForAll(item => item.Id != output.DecklinkMode))
            modes.Insert(0, new DeckLinkModeInfo { Id = output.DecklinkMode, Name = output.DecklinkMode });
        var mode = new ComboBox
        {
            IsEnabled = !locked,
            DisplayMemberPath = "Label",
            Margin = new Thickness(0, 0, 0, 6)
        };
        foreach (var item in modes)
            mode.Items.Add(item);
        foreach (DeckLinkModeInfo item in mode.Items)
        {
            if (item.Id == output.DecklinkMode)
            {
                mode.SelectedItem = item;
                break;
            }
        }
        if (mode.SelectedItem is null && mode.Items.Count > 0)
        {
            mode.SelectedIndex = 0;
            if (mode.SelectedItem is DeckLinkModeInfo picked)
                output.DecklinkMode = picked.Id;
        }
        mode.SelectionChanged += (_, _) =>
        {
            if (_suppressOutputs || mode.SelectedItem is not DeckLinkModeInfo info)
                return;
            output.DecklinkMode = info.Id;
        };
        stack.Children.Add(mode);

        var key = new CheckBox
        {
            Content = Loc.T("settings.decklinkExternalKey"),
            IsChecked = output.DecklinkExternalKey,
            IsEnabled = !locked,
            Foreground = System.Windows.Media.Brushes.White,
            Margin = new Thickness(0, 0, 0, 6)
        };
        key.Checked += (_, _) => output.DecklinkExternalKey = true;
        key.Unchecked += (_, _) => output.DecklinkExternalKey = false;
        stack.Children.Add(key);

        var notice = !string.IsNullOrWhiteSpace(_decklinkDeviceError)
            ? _decklinkDeviceError
            : listed.Error;
        if (string.IsNullOrWhiteSpace(notice) && devices.Count == 0)
            notice = Loc.T("settings.decklinkNone");
        if (!string.IsNullOrWhiteSpace(notice))
        {
            stack.Children.Add(new TextBlock
            {
                Text = notice,
                TextWrapping = TextWrapping.Wrap,
                Foreground = new System.Windows.Media.SolidColorBrush(System.Windows.Media.Color.FromRgb(0xCC, 0x88, 0x88))
            });
        }
        return stack;
    }

    private void Ok_Click(object sender, RoutedEventArgs e)
    {
        if (!OnAirLock.Active
            && FpsBox.SelectedItem is ComboBoxItem fps && fps.Tag is string fpsTag)
        {
            var parts = fpsTag.Split('/');
            Settings.MasterFpsNum = uint.Parse(parts[0]);
            Settings.MasterFpsDen = uint.Parse(parts[1]);
        }
        if (!OnAirLock.Active
            && SizeBox.SelectedItem is ComboBoxItem size && size.Tag is string sizeTag)
        {
            var parts = sizeTag.Split('x');
            Settings.DefaultWidth = uint.Parse(parts[0]);
            Settings.DefaultHeight = uint.Parse(parts[1]);
        }
        _session.NextOutputId = _nextOutputId;
        _session.NextStreamEncoderId = _nextStreamEncoderId;
        if (MvUnitBox.SelectedItem is MixingUnitEntry unit)
            Settings.DefaultMultiviewUnitId = unit.Id;
        if (BufferBox.SelectedItem is ComboBoxItem buffer && buffer.Tag is string bufferTag
            && uint.TryParse(bufferTag, out var frames))
            Settings.FrameBufferFrames = Math.Clamp(frames, 1u, 8u);
        if (ColorFormatBox.SelectedItem is ComboBoxItem color && color.Tag is string colorTag)
            Settings.InternalColorFormat = colorTag == "bgra" ? InternalColorFormat.Bgra : InternalColorFormat.Uyvy;
        if (MvPresentBox.SelectedItem is ComboBoxItem present && present.Tag is string presentTag
            && uint.TryParse(presentTag, out var interval))
            Settings.DefaultPresentInterval = MultiviewLayout.ClampPresentInterval(interval);
        if (FlipBudgetBox.SelectedItem is ComboBoxItem flip && flip.Tag is string flipTag
            && uint.TryParse(flipTag, out var flipLimit))
            Settings.FlipSwapchainLimit = flipLimit is 0 or 4 or 6 or 8 or 10 or 12 or 16 ? flipLimit : 0;
        Settings.RebarOptimization = _rebarAvailable && RebarOptBox.IsChecked == true;
        Settings.NdiGpuUpload = NdiGpuBox.IsChecked == true;
        Settings.VmixApiEnabled = WebApiEnabledBox.IsChecked == true;
        Settings.VmixTcpEnabled = WebApiTcpEnabledBox.IsChecked == true;
        Settings.NativeApiEnabled = WebApiWsEnabledBox.IsChecked == true;
        if (uint.TryParse(WebApiPortBox.Text.Trim(), out var apiPort) && apiPort is > 0 and <= 65535)
            Settings.VmixApiPort = apiPort;
        if (uint.TryParse(WebApiWsPortBox.Text.Trim(), out var wsPort) && wsPort is > 0 and <= 65535)
            Settings.NativeApiPort = wsPort;
        Settings.VmixApiUser = WebApiUserBox.Text ?? "";
        Settings.VmixApiPassword = WebApiPasswordBox.Password ?? "";
        DialogResult = true;
    }

    private void AddKind(WrapPanel panel, OutputEntry output, OutputSourceKind kind, string label, int index)
    {
        var radio = new RadioButton
        {
            Content = label,
            GroupName = $"out-{index}",
            IsChecked = output.SourceKind == kind,
            IsEnabled = !OnAirLock.Active,
            Foreground = System.Windows.Media.Brushes.White,
            Margin = new Thickness(0, 0, 12, 0)
        };
        radio.Checked += (_, _) =>
        {
            if (_suppressOutputs)
                return;
            var wasMultiview = output.SourceKind == OutputSourceKind.Multiview;
            output.SourceKind = kind;
            if (kind == OutputSourceKind.Multiview)
                output.AudioUnitId = 0;
            else if (wasMultiview)
                output.AudioUnitId = output.UnitId == 0 ? 1 : output.UnitId;
            RebuildOutputs();
        };
        panel.Children.Add(radio);
    }

    private void FillOutputPick(ComboBox box, OutputEntry output)
    {
        switch (output.SourceKind)
        {
            case OutputSourceKind.Input:
                box.ItemsSource = _session.Inputs;
                box.DisplayMemberPath = "ListLabel";
                box.SelectedValuePath = "Id";
                box.SelectedValue = output.SourceId;
                if (box.SelectedItem is InputEntry input)
                    output.SourceId = input.Id;
                else if (_session.Inputs.Count > 0)
                {
                    box.SelectedIndex = 0;
                    output.SourceId = _session.Inputs[0].Id;
                }
                break;
            case OutputSourceKind.Scene:
                if (output.SourceId != 0 && output.SourceId < MixerNative.SceneBase)
                    output.SourceId = MixerNative.SceneGpuId(output.SourceId);
                box.ItemsSource = _session.Scenes;
                box.DisplayMemberPath = "Name";
                box.SelectedValuePath = "GpuId";
                box.SelectedValue = output.SourceId;
                if (box.SelectedItem is SceneEntry scene)
                    output.SourceId = scene.GpuId;
                else if (_session.Scenes.Count > 0)
                {
                    box.SelectedIndex = 0;
                    output.SourceId = _session.Scenes[0].GpuId;
                }
                break;
            case OutputSourceKind.Multiview:
                if (output.SourceId != 0 && output.SourceId < MixerNative.MultiviewBase)
                    output.SourceId = MixerNative.MultiviewGpuId(output.SourceId);
                box.ItemsSource = _session.Multiviews;
                box.DisplayMemberPath = "Name";
                box.SelectedValuePath = "GpuId";
                box.SelectedValue = output.SourceId;
                if (box.SelectedItem is MultiviewLayout layout)
                    output.SourceId = layout.GpuId;
                else if (_session.Multiviews.Count > 0)
                {
                    box.SelectedIndex = 0;
                    output.SourceId = _session.Multiviews[0].GpuId;
                }
                break;
            default:
                box.ItemsSource = _session.Units;
                box.DisplayMemberPath = "Name";
                box.SelectedValuePath = "Id";
                box.SelectedValue = output.UnitId;
                if (box.SelectedItem is MixingUnitEntry unit)
                    output.UnitId = unit.Id;
                break;
        }
    }

    private sealed class AudioChoice
    {
        public ulong Id { get; init; }
        public required string Name { get; init; }
    }

    private void FillOutputAudio(ComboBox box, OutputEntry output)
    {
        var items = new List<AudioChoice> { new() { Id = 0, Name = "None" } };
        items.AddRange(_session.Units.Select(unit => new AudioChoice { Id = unit.Id, Name = unit.Name }));
        box.ItemsSource = items;
        box.DisplayMemberPath = "Name";
        if (output.SourceKind == OutputSourceKind.Multiview)
        {
            output.AudioUnitId = 0;
            box.IsEnabled = false;
        }
        else
            box.IsEnabled = true;
        box.SelectedItem = items.FirstOrDefault(item => item.Id == output.AudioUnitId) ?? items[0];
        if (box.SelectedItem is AudioChoice choice)
            output.AudioUnitId = choice.Id;
    }

    private static void ApplyOutputPick(ComboBox box, OutputEntry output)
    {
        switch (output.SourceKind)
        {
            case OutputSourceKind.Input:
                if (box.SelectedItem is InputEntry input)
                    output.SourceId = input.Id;
                break;
            case OutputSourceKind.Scene:
                if (box.SelectedItem is SceneEntry scene)
                    output.SourceId = scene.GpuId;
                break;
            case OutputSourceKind.Multiview:
                if (box.SelectedItem is MultiviewLayout layout)
                    output.SourceId = layout.GpuId;
                break;
            default:
                if (box.SelectedItem is MixingUnitEntry unit)
                    output.UnitId = unit.Id;
                break;
        }
    }

    private (uint Width, uint Height) EncodePixels(string tag)
    {
        if (tag == "0x0")
            return (Settings.DefaultWidth, Settings.DefaultHeight);
        var parts = tag.Split('x');
        if (parts.Length == 2
            && uint.TryParse(parts[0], out var width)
            && uint.TryParse(parts[1], out var height))
            return (width, height);
        return (0, 0);
    }

    private (uint Num, uint Den) EncodeRate(string tag)
    {
        if (tag == "0/0")
            return (Settings.MasterFpsNum, Settings.MasterFpsDen);
        var parts = tag.Split('/');
        if (parts.Length == 2
            && uint.TryParse(parts[0], out var num)
            && uint.TryParse(parts[1], out var den))
            return (num, den);
        return (0, 0);
    }

    private static void FillEncodeSize(ComboBox box, StreamEncoderEntry encoder)
    {
        box.Items.Add(new ComboBoxItem { Content = Loc.T("settings.followSessionSettings"), Tag = "0x0" });
        box.Items.Add(new ComboBoxItem { Content = "1920x1080", Tag = "1920x1080" });
        box.Items.Add(new ComboBoxItem { Content = "1280x720", Tag = "1280x720" });
        box.Items.Add(new ComboBoxItem { Content = "3840x2160", Tag = "3840x2160" });
        var tag = encoder.Width == 0 || encoder.Height == 0 ? "0x0" : $"{encoder.Width}x{encoder.Height}";
        if (box.Items.Cast<ComboBoxItem>().All(item => !Equals(item.Tag, tag)))
            box.Items.Add(new ComboBoxItem { Content = tag, Tag = tag });
        SelectTag(box, tag);
    }

    private static void ApplyEncodeSize(ComboBox box, StreamEncoderEntry encoder)
    {
        if (box.SelectedItem is not ComboBoxItem item || item.Tag is not string tag)
            return;
        if (tag == "0x0")
        {
            encoder.Width = 0;
            encoder.Height = 0;
            return;
        }
        var parts = tag.Split('x');
        if (parts.Length == 2
            && uint.TryParse(parts[0], out var width)
            && uint.TryParse(parts[1], out var height))
        {
            encoder.Width = width;
            encoder.Height = height;
        }
    }

    private static void FillEncodeFps(ComboBox box, StreamEncoderEntry encoder)
    {
        box.Items.Add(new ComboBoxItem { Content = Loc.T("settings.followSessionSettings"), Tag = "0/0" });
        box.Items.Add(new ComboBoxItem { Content = "23.976p", Tag = "24000/1001" });
        box.Items.Add(new ComboBoxItem { Content = "24p", Tag = "24/1" });
        box.Items.Add(new ComboBoxItem { Content = "25p", Tag = "25/1" });
        box.Items.Add(new ComboBoxItem { Content = "29.97p", Tag = "30000/1001" });
        box.Items.Add(new ComboBoxItem { Content = "30p", Tag = "30/1" });
        box.Items.Add(new ComboBoxItem { Content = "50p", Tag = "50/1" });
        box.Items.Add(new ComboBoxItem { Content = "NTSC 59.94p", Tag = "60000/1001" });
        box.Items.Add(new ComboBoxItem { Content = "60p", Tag = "60/1" });
        var tag = encoder.FpsNum == 0 || encoder.FpsDen == 0 ? "0/0" : $"{encoder.FpsNum}/{encoder.FpsDen}";
        if (box.Items.Cast<ComboBoxItem>().All(item => !Equals(item.Tag, tag)))
            box.Items.Add(new ComboBoxItem { Content = tag, Tag = tag });
        SelectTag(box, tag);
    }

    private static void ApplyEncodeFps(ComboBox box, StreamEncoderEntry encoder)
    {
        if (box.SelectedItem is not ComboBoxItem item || item.Tag is not string tag)
            return;
        var parts = tag.Split('/');
        if (parts.Length == 2
            && uint.TryParse(parts[0], out var num)
            && uint.TryParse(parts[1], out var den))
        {
            encoder.FpsNum = num;
            encoder.FpsDen = den;
        }
    }

    private static void FillThreads(ComboBox box, StreamEncoderEntry encoder)
    {
        box.Items.Add(new ComboBoxItem { Content = Loc.T("settings.threadsAuto"), Tag = "0" });
        for (var n = 1; n <= 4; n++)
            box.Items.Add(new ComboBoxItem { Content = n.ToString(), Tag = n.ToString() });
        var tag = encoder.EncoderThreads is >= 1 and <= 4 ? encoder.EncoderThreads.ToString() : "0";
        SelectTag(box, tag);
    }

    private static void SelectUlongTag(ComboBox box, ulong tag)
    {
        foreach (ComboBoxItem item in box.Items)
        {
            if (item.Tag is ulong value && value == tag)
            {
                box.SelectedItem = item;
                return;
            }
        }
    }

    private static void FillOutputSize(ComboBox box, OutputEntry output)
    {
        box.Items.Add(new ComboBoxItem { Content = Loc.T("settings.followSessionSettings"), Tag = "0x0" });
        box.Items.Add(new ComboBoxItem { Content = "1920x1080", Tag = "1920x1080" });
        box.Items.Add(new ComboBoxItem { Content = "1280x720", Tag = "1280x720" });
        box.Items.Add(new ComboBoxItem { Content = "3840x2160", Tag = "3840x2160" });
        var tag = output.Width == 0 || output.Height == 0 ? "0x0" : $"{output.Width}x{output.Height}";
        if (box.Items.Cast<ComboBoxItem>().All(item => !Equals(item.Tag, tag)))
            box.Items.Add(new ComboBoxItem { Content = tag, Tag = tag });
        SelectTag(box, tag);
    }

    private static void ApplyOutputSize(ComboBox box, OutputEntry output)
    {
        if (box.SelectedItem is not ComboBoxItem item || item.Tag is not string tag)
            return;
        if (tag == "0x0")
        {
            output.Width = 0;
            output.Height = 0;
            return;
        }
        var parts = tag.Split('x');
        if (parts.Length == 2
            && uint.TryParse(parts[0], out var width)
            && uint.TryParse(parts[1], out var height))
        {
            output.Width = width;
            output.Height = height;
        }
    }

    private static void FillOutputFps(ComboBox box, OutputEntry output)
    {
        box.Items.Add(new ComboBoxItem { Content = Loc.T("settings.followSessionSettings"), Tag = "0/0" });
        box.Items.Add(new ComboBoxItem { Content = "23.976p", Tag = "24000/1001" });
        box.Items.Add(new ComboBoxItem { Content = "24p", Tag = "24/1" });
        box.Items.Add(new ComboBoxItem { Content = "25p", Tag = "25/1" });
        box.Items.Add(new ComboBoxItem { Content = "29.97p", Tag = "30000/1001" });
        box.Items.Add(new ComboBoxItem { Content = "30p", Tag = "30/1" });
        box.Items.Add(new ComboBoxItem { Content = "50p", Tag = "50/1" });
        box.Items.Add(new ComboBoxItem { Content = "NTSC 59.94p", Tag = "60000/1001" });
        box.Items.Add(new ComboBoxItem { Content = "60p", Tag = "60/1" });
        box.Items.Add(new ComboBoxItem { Content = "119.88p", Tag = "120000/1001" });
        box.Items.Add(new ComboBoxItem { Content = "120p", Tag = "120/1" });
        var tag = output.FpsNum == 0 || output.FpsDen == 0 ? "0/0" : $"{output.FpsNum}/{output.FpsDen}";
        if (box.Items.Cast<ComboBoxItem>().All(item => !Equals(item.Tag, tag)))
            box.Items.Add(new ComboBoxItem { Content = tag, Tag = tag });
        SelectTag(box, tag);
    }

    private static void ApplyOutputFps(ComboBox box, OutputEntry output)
    {
        if (box.SelectedItem is not ComboBoxItem item || item.Tag is not string tag)
            return;
        var parts = tag.Split('/');
        if (parts.Length == 2
            && uint.TryParse(parts[0], out var num)
            && uint.TryParse(parts[1], out var den))
        {
            output.FpsNum = num;
            output.FpsDen = den;
        }
    }

    private string NextOutputName()
    {
        const string prefix = "eiviz-out";
        if (Outputs.TrueForAll(item => item.Name != prefix)
            && _session.Outputs.TrueForAll(item => item.Name != prefix))
            return prefix;
        for (var i = 2; ; i++)
        {
            var name = $"{prefix}-{i}";
            if (Outputs.TrueForAll(item => item.Name != name)
                && _session.Outputs.TrueForAll(item => item.Name != name))
                return name;
        }
    }

    private static OutputEntry Clone(OutputEntry output) => output.Clone();

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
}
