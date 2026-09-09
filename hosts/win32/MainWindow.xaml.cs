using System.ComponentModel;
using System.Diagnostics;
using System.IO;
using System.Text.Json;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Data;
using System.Windows.Input;
using System.Windows.Threading;
using Eiviz.Host.Dialogs;
using Eiviz.Host.I18n;
using Eiviz.Host.Interop;
using Eiviz.Host.Media;
using Eiviz.Host.Preview;

namespace Eiviz.Host;

public partial class MainWindow : Window
{
    private SceneEntry? _selectedScene;
    private int _tbarPresetIndex;
    private bool _tbarLatching;
    private bool _tbarLocked;
    private bool _suppressUnitChange;
    private OverlayWindow? _overlay;
    private ResourceMonitorWindow? _resourcesWindow;
    private LogWindow? _logWindow;
    private readonly List<MultiviewWindow> _multiviews = [];
    private readonly Dictionary<ulong, InputPreviewWindow> _inputPreviews = [];
    private readonly Dictionary<ulong, AudioInputWindow> _audioInputs = [];
    private readonly Dictionary<ulong, SwitcherWindow> _switchers = [];
    private readonly HashSet<int> _transitionExpanded = [];
    private readonly Dictionary<int, TransitionGroup> _kindMenuGroup = [];
    private readonly DispatcherTimer _meterTimer = new() { Interval = TimeSpan.FromMilliseconds(50) };
    private readonly DispatcherTimer _tbarTimer = new() { Interval = TimeSpan.FromMilliseconds(16) };
    private readonly Dictionary<ulong, MeterStrip> _meters = [];
    private readonly ResourceMonitor _resources = new();
    private bool _sceneEditorOpen;
    private long _sceneEditorGuard;
    private bool _videoSeeking;
    private bool _videoSeekSuppress;
    private long _lastSeekSentMs;
    private ulong _lastProgramId;
    private ulong _shownProgramId;
    private ulong _shownPreviewId;
    private bool _fatalHandled;
    private ListFilter _inputFilter = ListFilter.All;
    private ListFilter _sceneFilter = ListFilter.All;
    private ICollectionView? _inputView;
    private Session _session => ((App)Application.Current).Session;

    public MainWindow()
    {
        InitializeComponent();
        Title = Loc.T(HostRole.IsRemote ? "app.titleRemote" : "app.title");
        ApplyRemoteChrome();
        BindInputList();
        RebuildInputTabs();
        RebuildSceneTabs();
        UnitBox.ItemsSource = _session.Units;
        _suppressUnitChange = true;
        var unit = _session.Units.FirstOrDefault(item => item.Id == _session.SelectedUnitId)
            ?? _session.Units.FirstOrDefault();
        UnitBox.SelectedItem = unit;
        if (unit is not null)
            _session.SelectedUnitId = unit.Id;
        _suppressUnitChange = false;
        RebuildScenes();
        RebuildTransitions();
        RebuildOverlayToggles();
        RebuildMeters();
        _meterTimer.Tick += (_, _) => TickMeters();
        _meterTimer.Start();
        _tbarTimer.Tick += (_, _) =>
        {
            ((App)Application.Current).Backend.Poll();
            RefreshStatusBar();
            SyncTBarsFromMixer();
        };
        _tbarTimer.Start();
        Loaded += (_, _) => PromptMissingMedia();
        Closed += (_, _) =>
        {
            _tbarTimer.Stop();
            _meterTimer.Stop();
            _resources.Dispose();
            if (HostRole.IsRemote)
                RemoteVideoCatalog.Updated -= OnRemoteVideoCatalogUpdated;
            if (!ReferenceEquals(Application.Current?.MainWindow, this))
                return;
            CloseOwnedSurfaces();
        };
        SceneScroll.ScrollChanged += (_, _) => ApplySceneTileThumbs();
        Loaded += (_, _) =>
        {
            ApplyBusColors();
            ApplyAspect();
            RefreshStatusBar();
            FillVideoSources();
            BindMainVideo();
            if (!HostRole.IsRemote)
                AudioGraphSync.Push(_session);
            SyncSelectedSceneFromMixer();
            RefreshSceneTiles();
        };
        if (HostRole.IsRemote)
            RemoteVideoCatalog.Updated += OnRemoteVideoCatalogUpdated;
    }

    internal void ReloadFromSession()
    {
        var selectedUnit = _session.SelectedUnitId;
        var selectedInput = InputList.SelectedItem is InputEntry input ? input.Id : 0UL;
        BindInputList();
        RebuildInputTabs();
        RebuildSceneTabs();
        UnitBox.ItemsSource = null;
        UnitBox.ItemsSource = _session.Units;
        _suppressUnitChange = true;
        UnitBox.SelectedItem = _session.Units.FirstOrDefault(item => item.Id == selectedUnit)
            ?? _session.Units.FirstOrDefault();
        _suppressUnitChange = false;
        RebuildScenes();
        SyncSelectedSceneFromMixer();
        RefreshSceneTiles();
        if (selectedInput != 0)
        {
            var keep = _session.Inputs.FirstOrDefault(item => item.Id == selectedInput);
            if (keep is not null)
                InputList.SelectedItem = keep;
        }
        RebuildTransitions();
        RebuildOverlayToggles();
        RebuildMeters();
        ApplyAspect();
        FillVideoSources();
        if (!HostRole.IsRemote)
            BindPreviewProgram();
        else
            BindMainVideo();
        _overlay?.Reload(SelectedUnit);
    }

    private bool _suppressVideoSource;
    private bool _suppressVideoLayout;

    private void ApplyRemoteChrome()
    {
        if (!HostRole.IsRemote)
        {
            SnapshotButton.Visibility = Visibility.Visible;
            SnapshotInputButton.Visibility = Visibility.Visible;
            return;
        }
        NewSessionButton.Visibility = Visibility.Collapsed;
        SaveSessionButton.Visibility = Visibility.Visible;
        SaveSessionButton.IsEnabled = false;
        SaveSessionMenuButton.Visibility = Visibility.Collapsed;
        LoadSessionButton.Visibility = Visibility.Collapsed;
        LoadLastSessionButton.Visibility = Visibility.Collapsed;
        ConnectButton.Visibility = Visibility.Visible;
        DisconnectButton.Visibility = Visibility.Visible;
        VideoLayoutBox.Visibility = Visibility.Visible;
        PreviewSourceBox.Visibility = Visibility.Visible;
        ProgramSourceBox.Visibility = Visibility.Visible;
        MultiviewSourceBox.Visibility = Visibility.Visible;
        PreviewInputButton.Visibility = Visibility.Collapsed;
        SnapshotButton.Visibility = Visibility.Collapsed;
        SnapshotInputButton.Visibility = Visibility.Collapsed;
        PackRemoteInputActions();
        RemoteIdleText.Text = Loc.T("msg.remoteIdle");
        FillVideoLayoutBox();
        ApplyVideoLayout();
        ApplyRemoteLiveUi(false);
    }

    private void PackRemoteInputActions()
    {
        InputActionsGrid.ColumnDefinitions.Clear();
        InputActionsGrid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        InputActionsGrid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(8) });
        InputActionsGrid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        InputActionsGrid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(8) });
        InputActionsGrid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        Grid.SetColumn(AddInputButton, 0);
        Grid.SetColumn(EditInputButton, 2);
        Grid.SetColumn(DeleteInputButton, 4);
    }

    private void FillVideoSources()
    {
        if (!HostRole.IsRemote)
            return;
        _suppressVideoSource = true;
        var previewChoice = RemoteVideoCatalog.FromPrefs(true);
        var programChoice = RemoteVideoCatalog.FromPrefs(false);
        PreviewSourceBox.ItemsSource = WithChoice(RemoteVideoCatalog.List(_session), previewChoice);
        ProgramSourceBox.ItemsSource = WithChoice(RemoteVideoCatalog.List(_session), programChoice);
        SelectChoice(PreviewSourceBox, previewChoice);
        SelectChoice(ProgramSourceBox, programChoice);
        var multiviewChoice = RemoteVideoCatalog.FromPrefsMultiview();
        MultiviewSourceBox.ItemsSource = WithChoice(RemoteVideoCatalog.List(_session), multiviewChoice);
        SelectChoice(MultiviewSourceBox, multiviewChoice);
        _suppressVideoSource = false;
    }

    private void FillVideoLayoutBox()
    {
        if (!HostRole.IsRemote)
            return;
        _suppressVideoLayout = true;
        VideoLayoutBox.ItemsSource = new[]
        {
            new VideoLayoutItem(false, Loc.T("chrome.layoutPrvPgm")),
            new VideoLayoutItem(true, Loc.T("chrome.layoutMultiview"))
        };
        VideoLayoutBox.SelectedIndex = RemoteVideoCatalog.IsMultiviewLayout() ? 1 : 0;
        _suppressVideoLayout = false;
    }

    private void OnRemoteVideoCatalogUpdated()
    {
        FillVideoSources();
        BindMainVideo();
    }

    private static List<RemoteVideoItem> WithChoice(List<RemoteVideoItem> items, RemoteVideoChoice choice)
    {
        if (RemoteVideoCatalog.CanConnect(choice) && items.All(item => item.Choice != choice))
        {
            var prefix = choice.Transport == OutputTransport.Ndi ? "NDI" : "OMT";
            items.Insert(1, new RemoteVideoItem { Choice = choice, Label = $"{prefix}  {choice.Address}" });
        }
        return items;
    }

    private static void SelectChoice(ComboBox box, RemoteVideoChoice choice)
    {
        foreach (RemoteVideoItem item in box.Items)
        {
            if (item.Choice == choice)
            {
                box.SelectedItem = item;
                return;
            }
        }
        if (box.Items.Count > 0)
            box.SelectedIndex = 0;
    }

    private void PreviewSource_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (_suppressVideoSource || PreviewSourceBox.SelectedItem is not RemoteVideoItem item)
            return;
        RemoteVideoCatalog.Save(true, item.Choice);
        BindMainVideo();
    }

    private void ProgramSource_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (_suppressVideoSource || ProgramSourceBox.SelectedItem is not RemoteVideoItem item)
            return;
        RemoteVideoCatalog.Save(false, item.Choice);
        BindMainVideo();
    }

    private void MultiviewSource_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (_suppressVideoSource || MultiviewSourceBox.SelectedItem is not RemoteVideoItem item)
            return;
        RemoteVideoCatalog.SaveMultiview(item.Choice);
        BindMainVideo();
    }

    private void VideoLayout_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (_suppressVideoLayout || VideoLayoutBox.SelectedItem is not VideoLayoutItem item)
            return;
        RemoteVideoCatalog.SaveLayout(item.Multiview);
        ApplyVideoLayout();
        BindMainVideo();
    }

    private void Connect_Click(object sender, RoutedEventArgs e)
    {
        var url = AppPrefs.Current.RemoteUrl;
        var token = CredentialStore.Load(url);
        if (!ConnectWindow.TryPrompt(this, url, token, out var nextUrl, out var nextToken))
            return;
        ConnectTo(nextUrl, nextToken);
    }

    private void Disconnect_Click(object sender, RoutedEventArgs e)
    {
        var app = (App)Application.Current;
        _overlay?.Close();
        CloseAllSwitchers();
        foreach (var window in _multiviews.ToArray())
            window.Close();
        PreviewHost.ReleaseNative();
        ProgramHost.ReleaseNative();
        MainMultiviewHost.ReleaseNative();
        app.DisconnectRemote();
        RefreshStatusBar();
    }

    private void ConnectTo(string url, string token)
    {
        var app = (App)Application.Current;
        if (!app.TryConnectRemote(url, token, out var error))
        {
            MessageBox.Show(this, error, Loc.T("msg.remoteConnectFailed"));
            RefreshStatusBar();
            return;
        }
        FillVideoSources();
        BindMainVideo();
        RefreshStatusBar();
    }

    private void ApplyVideoLayout()
    {
        var multiview = HostRole.IsRemote && RemoteVideoCatalog.IsMultiviewLayout();
        PreviewAspect.Visibility = multiview ? Visibility.Collapsed : Visibility.Visible;
        ProgramAspect.Visibility = multiview ? Visibility.Collapsed : Visibility.Visible;
        ProgramSplitter.Visibility = multiview ? Visibility.Collapsed : Visibility.Visible;
        MainMultiviewAspect.Visibility = multiview ? Visibility.Visible : Visibility.Collapsed;
        VideoCol2.Width = multiview ? new GridLength(0) : new GridLength(1, GridUnitType.Star);
        VideoCol2.MinWidth = multiview ? 0 : 160;
    }

    private void BindMainVideo()
    {
        if (HostRole.IsRemote && RemoteVideoCatalog.IsMultiviewLayout())
        {
            PreviewHost.ReleaseNative();
            ProgramHost.ReleaseNative();
            if (Application.Current is App app)
                app.Backend.BindMainMultiview(MainMultiviewHost);
        }
        else
            BindPreviewProgram();
        RefreshStatusBar();
    }

    private void BindPreviewProgram()
    {
        if (HostRole.IsRemote)
            MainMultiviewHost.ReleaseNative();
        if (Application.Current is App app)
            app.Backend.BindPreviewProgram(PreviewHost, ProgramHost, SelectedUnit.Id);
        else
        {
            PreviewHost.RetargetUnit(SelectedUnit.Id, MixerNative.OutputPreview);
            ProgramHost.RetargetUnit(SelectedUnit.Id, MixerNative.OutputProgram);
        }
        RefreshStatusBar();
    }

    internal void BindPreviewProgramSurfaces(SwapchainHost preview, SwapchainHost program, ulong unitId)
    {
        if (Application.Current is App app)
            app.Backend.BindPreviewProgram(preview, program, unitId);
    }

    private void ApplyRemoteLiveUi(bool live)
    {
        var overlay = live ? Visibility.Collapsed : Visibility.Visible;
        if (RemoteIdleOverlay.Visibility != overlay)
            RemoteIdleOverlay.Visibility = overlay;
        var mix = live ? Visibility.Visible : Visibility.Collapsed;
        if (MixUnitBar.Visibility != mix)
            MixUnitBar.Visibility = mix;
        if (SettingsButton.IsEnabled != live)
            SettingsButton.IsEnabled = live;
        if (DisconnectButton.IsEnabled != live)
            DisconnectButton.IsEnabled = live;
        if (SaveSessionButton.IsEnabled != live)
            SaveSessionButton.IsEnabled = live;
    }

    private void RefreshStatusBar()
    {
        if (Application.Current is not App app)
            return;
        if (HostRole.IsRemote)
            ApplyRemoteLiveUi(app.Backend.Connected);
        var warn = StatusWarn(app);
        if (WarnText.Text != warn)
            WarnText.Text = warn;
    }

    private string StatusWarn(App app)
    {
        if (!HostRole.IsRemote)
            return _resources.Warning() ?? "";
        if (app.Backend is not RemoteEivizBackend remote || !remote.Connected)
            return app.Backend.StatusText;
        if (!string.IsNullOrEmpty(remote.RemoteVideoWarn))
            return remote.RemoteVideoWarn;
        return _resources.Warning() ?? "";
    }


    private MixingUnitEntry SelectedUnit =>
        UnitBox.SelectedItem as MixingUnitEntry ?? _session.Units[0];

    private uint SceneWidth => SelectedUnit.Width;
    private uint SceneHeight => SelectedUnit.Height;

}

internal sealed record VideoLayoutItem(bool Multiview, string Label)
{
    public override string ToString() => Label;
}
