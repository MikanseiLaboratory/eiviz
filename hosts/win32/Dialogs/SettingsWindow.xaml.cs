using System.Windows;
using System.Windows.Controls;
using Eiviz.Host.I18n;
using Eiviz.Host.Interop;
using Eiviz.Host.Media;

namespace Eiviz.Host.Dialogs;

public partial class SettingsWindow : Window
{
    private Session _session => ((App)Application.Current).Session;
    private ulong _nextOutputId;

    public SettingsWindow(Session session, int category = 0)
    {
        InitializeComponent();
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
        SelectTag(FpsBox, $"{Settings.MasterFpsNum}/{Settings.MasterFpsDen}");
        SelectTag(SizeBox, $"{Settings.DefaultWidth}x{Settings.DefaultHeight}");
        SelectTag(BufferBox, Settings.FrameBufferFrames.ToString());
        SelectTag(ColorFormatBox, Settings.InternalColorFormat == InternalColorFormat.Bgra ? "bgra" : "uyvy");
        SelectTag(MvPresentBox, MultiviewLayout.ClampPresentInterval(Settings.DefaultPresentInterval == 0 ? 3 : Settings.DefaultPresentInterval).ToString());
        SelectTag(FlipBudgetBox, Settings.FlipSwapchainLimit.ToString());
        MvUnitBox.ItemsSource = session.Units;
        MvUnitBox.SelectedItem = session.Units.FirstOrDefault(item => item.Id == Settings.DefaultMultiviewUnitId)
            ?? session.Units.FirstOrDefault();
        _nextBusId = session.NextBusId;
        session.EnsureDefaultBuses();
        foreach (var bus in session.Buses)
            Buses.Add(CloneBus(bus));
        _nextBusId = Math.Max(_nextBusId, Buses.Count == 0 ? 3 : Buses.Max(item => item.Id) + 1);
        HeadphoneCopyBox.IsChecked = session.HeadphoneCopyMaster;
        _devices = AudioGraphSync.EnumerateDevices(0)
            .Where(device => device.Direction != 1)
            .Select(device => (device.Kind, device.Channels, device.Id, device.Name))
            .ToList();
        RebuildOutputs();
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
    }

    public SessionSettings Settings { get; }
    public List<OutputEntry> Outputs { get; } = [];
    public List<AudioBusEntry> Buses { get; } = [];
    public bool HeadphoneCopyMaster { get; private set; }

    private void ApplyOnAirLock()
    {
        var locked = OnAirLock.Active;
        if (AddOutputButton is not null)
            AddOutputButton.IsEnabled = !locked;
        if (FpsBox is not null)
            FpsBox.IsEnabled = !locked;
        if (SizeBox is not null)
            SizeBox.IsEnabled = !locked;
        if (OutputRows is not null)
            RebuildOutputs();
    }
    public ulong NextOutputId => _nextOutputId;
    public ulong NextBusId => _nextBusId;
    private ulong _nextBusId;
    private bool _suppressOutputs;
    private bool _rebarAvailable;
    private List<(uint Kind, uint Channels, string Id, string Name)> _devices = [];

    private void CategoryList_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (DisplayPanel is null || PerformancePanel is null || MultiviewPanel is null || AudioBusPanel is null || AdvancedPanel is null || WebApiPanel is null)
            return;
        var index = CategoryList.SelectedIndex;
        DisplayPanel.Visibility = index == 0 ? Visibility.Visible : Visibility.Collapsed;
        PerformancePanel.Visibility = index == 1 ? Visibility.Visible : Visibility.Collapsed;
        OutputPanel.Visibility = index == 2 ? Visibility.Visible : Visibility.Collapsed;
        MultiviewPanel.Visibility = index == 3 ? Visibility.Visible : Visibility.Collapsed;
        AudioBusPanel.Visibility = index == 4 ? Visibility.Visible : Visibility.Collapsed;
        WebApiPanel.Visibility = index == 5 ? Visibility.Visible : Visibility.Collapsed;
        AdvancedPanel.Visibility = index == 6 ? Visibility.Visible : Visibility.Collapsed;
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

    private void AddBus_Click(object sender, RoutedEventArgs e)
    {
        var auxCount = Buses.Count(item => item.Role == AudioBusRole.Aux);
        if (auxCount >= 8)
            return;
        var bit = 2u;
        while (Buses.Any(item => item.Bit == bit) && bit < 31)
            bit++;
        Buses.Add(new AudioBusEntry
        {
            Id = _nextBusId++,
            Name = NextAuxName(),
            Role = AudioBusRole.Aux,
            DeviceKind = AudioDeviceKind.None,
            MapLeft = 0,
            MapRight = 1,
            Bit = bit
        });
        RebuildBuses();
    }

    private string NextAuxName()
    {
        for (var letter = 'A'; letter <= 'H'; letter++)
        {
            var name = $"Bus {letter}";
            if (Buses.TrueForAll(item => item.Name != name))
                return name;
        }
        return $"Bus {_nextBusId}";
    }

    private void RebuildBuses()
    {
        if (BusRows is null)
            return;
        BusRows.Children.Clear();
        foreach (var bus in Buses.ToArray())
        {
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
            grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(28) });
            grid.RowDefinitions.Add(new RowDefinition());
            grid.RowDefinitions.Add(new RowDefinition());
            grid.RowDefinitions.Add(new RowDefinition());

            var name = new TextBox { Text = bus.Name, Margin = new Thickness(0, 0, 8, 6), IsReadOnly = bus.Role != AudioBusRole.Aux };
            name.TextChanged += (_, _) => bus.Name = name.Text.Trim();
            var kind = new ComboBox { Margin = new Thickness(0, 0, 8, 6) };
            kind.Items.Add(new ComboBoxItem { Content = "Enabled", Tag = AudioDeviceKind.None });
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
            FillDeviceBox(device, bus);
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
            var remove = new Button { Content = "−", Width = 28, IsEnabled = bus.Role == AudioBusRole.Aux };
            remove.Click += (_, _) =>
            {
                Buses.Remove(bus);
                RebuildBuses();
            };

            var mapVisible = bus.DeviceKind == AudioDeviceKind.None ? Visibility.Collapsed : Visibility.Visible;
            var leftLabel = new TextBlock { Text = "L ch", Foreground = System.Windows.Media.Brushes.Silver, Margin = new Thickness(0, 0, 8, 2), Visibility = mapVisible };
            var rightLabel = new TextBlock { Text = "R ch", Foreground = System.Windows.Media.Brushes.Silver, Margin = new Thickness(0, 0, 8, 2), Visibility = mapVisible };
            left.Visibility = mapVisible;
            right.Visibility = mapVisible;

            Grid.SetRow(name, 0);
            Grid.SetColumnSpan(name, 4);
            Grid.SetRow(remove, 0);
            Grid.SetColumn(remove, 4);
            Grid.SetRow(kind, 1);
            Grid.SetRow(device, 1);
            Grid.SetColumn(device, 1);
            Grid.SetColumnSpan(device, 3);
            Grid.SetRow(leftLabel, 2);
            Grid.SetRow(left, 2);
            Grid.SetColumn(left, 2);
            Grid.SetRow(rightLabel, 2);
            Grid.SetColumn(rightLabel, 1);
            Grid.SetRow(right, 2);
            Grid.SetColumn(right, 3);

            grid.Children.Add(name);
            grid.Children.Add(remove);
            grid.Children.Add(kind);
            grid.Children.Add(device);
            grid.Children.Add(leftLabel);
            grid.Children.Add(left);
            grid.Children.Add(rightLabel);
            grid.Children.Add(right);
            box.Child = grid;
            BusRows.Children.Add(box);
        }
    }

    private void FillDeviceBox(ComboBox box, AudioBusEntry bus)
    {
        box.Items.Clear();
        box.Items.Add(new ComboBoxItem { Content = bus.DeviceKind is AudioDeviceKind.Wasapi or AudioDeviceKind.CoreAudio ? "Default" : "(none)", Tag = "" });
        foreach (var device in _devices.Where(item => item.Kind == (uint)bus.DeviceKind
            || (bus.DeviceKind == AudioDeviceKind.CoreAudio && item.Kind == (uint)AudioDeviceKind.Wasapi)
            || (bus.DeviceKind == AudioDeviceKind.Wasapi && item.Kind == (uint)AudioDeviceKind.CoreAudio)))
        {
            var label = string.IsNullOrWhiteSpace(device.Name) ? device.Id : device.Name;
            box.Items.Add(new ComboBoxItem { Content = label, Tag = device.Id });
        }
        box.SelectedIndex = 0;
        for (var i = 0; i < box.Items.Count; i++)
        {
            if (box.Items[i] is ComboBoxItem item && Equals(item.Tag, bus.DeviceId ?? ""))
            {
                box.SelectedIndex = i;
                break;
            }
        }
    }

    private static void FillMapBoxes(ComboBox left, ComboBox right, AudioBusEntry bus)
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

    private static AudioBusEntry CloneBus(AudioBusEntry bus) => new()
    {
        Id = bus.Id,
        Name = bus.Name,
        Role = bus.Role,
        DeviceKind = bus.DeviceKind,
        DeviceId = bus.DeviceId,
        MapLeft = bus.MapLeft,
        MapRight = bus.MapRight,
        Bit = bus.Bit,
        Gain = MixerNative.MixerGain(bus.Gain),
        Mute = bus.Mute
    };

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
            AudioBusId = 1,
            SkipEncodeWhenNoReceivers = true,
            Width = Settings.DefaultWidth,
            Height = Settings.DefaultHeight,
            FpsNum = Settings.MasterFpsNum,
            FpsDen = Settings.MasterFpsDen
        });
        RebuildOutputs();
    }

    private void RebuildOutputs()
    {
        OutputRows.Children.Clear();
        _suppressOutputs = true;
        for (var i = 0; i < Outputs.Count; i++)
        {
            var index = i;
            var output = Outputs[i];
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

            var name = new TextBox { Text = output.Name, Margin = new Thickness(0, 0, 8, 6) };
            name.TextChanged += (_, _) => output.Name = name.Text.Trim();
            var transport = new ComboBox { Margin = new Thickness(0, 0, 8, 6), IsEnabled = !OnAirLock.Active };
            transport.Items.Add(new ComboBoxItem { Content = "OMT", Tag = OutputTransport.Omt });
            transport.Items.Add(new ComboBoxItem { Content = "NDI", Tag = OutputTransport.Ndi });
            transport.Items.Add(new ComboBoxItem { Content = "DeckLink", Tag = OutputTransport.DeckLink });
            transport.SelectedIndex = (int)output.Transport;
            transport.SelectionChanged += (_, _) =>
            {
                if (transport.SelectedItem is ComboBoxItem item && item.Tag is OutputTransport value)
                {
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
                if (audio.SelectedItem is AudioBusEntry bus)
                    output.AudioBusId = bus.Id;
            };

            var enabled = new CheckBox
            {
                Content = "Enabled",
                IsChecked = output.Enabled,
                IsEnabled = !locked,
                Foreground = System.Windows.Media.Brushes.White,
                VerticalAlignment = VerticalAlignment.Center,
                Margin = new Thickness(0, 0, 8, 6)
            };
            enabled.Checked += (_, _) => output.Enabled = true;
            enabled.Unchecked += (_, _) => output.Enabled = false;

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
            box.Child = grid;
            OutputRows.Children.Add(box);
        }
        _suppressOutputs = false;
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
        HeadphoneCopyMaster = HeadphoneCopyBox.IsChecked == true;
        _session.NextBusId = _nextBusId;
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
                output.AudioBusId = 0;
            else if (wasMultiview)
                output.AudioBusId = 1;
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

    private void FillOutputAudio(ComboBox box, OutputEntry output)
    {
        var items = new List<AudioBusEntry> { new() { Id = 0, Name = "None" } };
        items.AddRange(Buses);
        box.ItemsSource = items;
        box.DisplayMemberPath = "Name";
        box.SelectedValuePath = "Id";
        // Multiview senders stay silent (NDI and OMT). A bus could be
        // attached, but mosaic encode vs PCM timing on the shared send
        // thread is too messy, so the picker is locked to None.
        if (output.SourceKind == OutputSourceKind.Multiview)
        {
            output.AudioBusId = 0;
            box.IsEnabled = false;
        }
        else
            box.IsEnabled = true;
        box.SelectedItem = items.FirstOrDefault(item => item.Id == output.AudioBusId);
        if (box.SelectedItem is AudioBusEntry bus)
            output.AudioBusId = bus.Id;
        else
        {
            box.SelectedIndex = 0;
            output.AudioBusId = 0;
        }
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

    private static OutputEntry Clone(OutputEntry output) => new()
    {
        Id = output.Id,
        Name = output.Name,
        Transport = output.Transport,
        SourceKind = output.SourceKind,
        SourceId = output.SourceId,
        UnitId = output.UnitId,
        UseGpu = output.UseGpu,
        Enabled = output.Enabled,
        AudioBusId = output.AudioBusId,
        SkipEncodeWhenNoReceivers = output.SkipEncodeWhenNoReceivers,
        Width = output.Width,
        Height = output.Height,
        FpsNum = output.FpsNum,
        FpsDen = output.FpsDen
    };

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
