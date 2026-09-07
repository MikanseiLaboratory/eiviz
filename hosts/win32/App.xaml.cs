using System.Windows;
using System.Windows.Threading;
using Eiviz.Host.I18n;
using Eiviz.Host.Interop;
using Eiviz.Host.Media;
using Eiviz.Host.Preview;
using Microsoft.Win32;

namespace Eiviz.Host;

public partial class App : Application
{
    internal Session Session { get; set; } = null!;
    internal IEivizBackend Backend { get; private set; } = null!;
    internal static bool IsRemote => HostRole.IsRemote;

    protected override void OnStartup(StartupEventArgs e)
    {
        HostLog.Install();
        GpuPresentStore.Load();
        System.Runtime.GCSettings.LatencyMode = System.Runtime.GCLatencyMode.SustainedLowLatency;
        Loc.Apply(AppPrefs.Current.Language);
        ThemeService.Apply(AppPrefs.Current.Theme);
        SystemEvents.UserPreferenceChanged += (_, args) =>
        {
            if (args.Category == UserPreferenceCategory.General)
                Dispatcher.BeginInvoke(() => ThemeService.Apply(AppPrefs.Current.Theme));
        };
        base.OnStartup(e);
        try
        {
            if (MixerNative.Ping() != 0x4549_5649)
                throw new InvalidOperationException("The Rust mixer ABI does not match this host.");
            Session = Session.Default();
            if (HostRole.IsRemote)
                BootRemoteMixer();
            else
            {
                Backend = new LocalEivizBackend();
                BootMixer();
            }
        }
        catch (Exception ex)
        {
            HostLog.WriteCrash(ex);
            throw;
        }
    }

    internal void ReplaceDocument(Session session)
    {
        Session = session;
        if (MainWindow is MainWindow window)
            window.ReloadFromSession();
    }

    private void ReplaceSession(Session session)
    {
        Backend?.Dispose();
        foreach (var unit in Session.Units.ToArray())
            MixerNative.DestroyUnit(unit.Id);
        MixerNative.Destroy();
        Session = session;
        if (HostRole.IsRemote)
            BootRemoteMixer();
        else
        {
            Backend = new LocalEivizBackend();
            BootMixer();
        }
    }

    /// Restart the mixer and replace the main window so surfaces attach on first
    /// layout, the same as a cold start. Do not reuse HWNDs across mixer lifetimes.
    internal void ReloadSession(Session session)
    {
        var previous = MainWindow as MainWindow;
        previous?.CloseOwnedSurfaces();
        ReplaceSession(session);
        var next = new MainWindow();
        if (previous is not null)
        {
            next.WindowStartupLocation = WindowStartupLocation.Manual;
            if (previous.WindowState == WindowState.Normal)
            {
                next.Left = previous.Left;
                next.Top = previous.Top;
                next.Width = previous.Width;
                next.Height = previous.Height;
            }
            else
            {
                next.Left = previous.RestoreBounds.Left;
                next.Top = previous.RestoreBounds.Top;
                next.Width = previous.RestoreBounds.Width;
                next.Height = previous.RestoreBounds.Height;
            }
            next.WindowState = previous.WindowState;
        }
        MainWindow = next;
        next.Show();
        previous?.Close();
    }

    private void BootMixer()
    {
        MixerNative.ThrowIfFailed(
            MixerNative.CreateWithBackend(
                AppPrefs.Current.CreateAbi,
                0,
                Session.Settings.MasterFpsNum,
                Session.Settings.MasterFpsDen),
            "GPU mixer initialization");
        FlipBudget.Configure(Session.Settings.FlipSwapchainLimit);
        MixerNative.VideoFormat = Session.Settings.InternalColorFormat == InternalColorFormat.Bgra
            ? MixerNative.FormatBgra
            : MixerNative.FormatUyvy;
        SessionStore.ReplaceRuntime(Session);
        ApplyVmixApi();
        SessionStore.Publish(Session);
    }

    private void BootRemoteMixer()
    {
        MixerNative.ThrowIfFailed(
            MixerNative.CreateWithBackend(
                AppPrefs.Current.CreateAbi,
                0,
                60,
                1),
            "GPU mixer initialization");
        MixerNative.DefineGenerator(MixerNative.Black, MixerNative.GenSolid, 0, 0, 0, 1, 0);
        FlipBudget.Configure(0);
        RemoteVideoCatalog.Start();
        Backend = new DisconnectedRemoteBackend();
    }

    internal bool TryConnectRemote(string url, string token, out string error)
    {
        var endpoint = url.Trim();
        try
        {
            var next = RemoteEivizBackend.Open(endpoint, token ?? "");
            Backend?.Dispose();
            Backend = next;
            CredentialStore.Save(endpoint, token ?? "");
            AppPrefs.Current.RemoteUrl = endpoint;
            AppPrefs.Current.RememberRemote(endpoint);
            error = "";
            return true;
        }
        catch (Exception ex)
        {
            HostLog.Write("ERROR", ex.Message);
            error = ex.Message;
            Backend ??= new DisconnectedRemoteBackend();
            return false;
        }
    }

    internal void DisconnectRemote()
    {
        if (!HostRole.IsRemote)
            return;
        if (Backend is DisconnectedRemoteBackend)
            return;
        Backend?.Dispose();
        Backend = new DisconnectedRemoteBackend();
    }

    internal static void ApplyVmixApi()
    {
        var app = (App)Current;
        var settings = app.Session.Settings;
        var port = settings.VmixApiPort == 0 ? 8088u : settings.VmixApiPort;
        var user = settings.VmixApiUser ?? "";
        var password = settings.VmixApiPassword ?? "";
        ApplyHttpApi(app, settings, port, user, password);
        ApplyTcpApi(app, settings);
        ApplyNativeApi(app, settings);
    }

    private static void ApplyHttpApi(App app, SessionSettings settings, uint port, string user, string password)
    {
        var enabled = settings.VmixApiEnabledValue;
        var code = MixerNative.ApiConfigure(enabled ? 1u : 0u, port, user, password);
        if (code == 0)
            return;
        if (!enabled || code != 5)
            MixerNative.ThrowIfFailed(code, "Configure vMix HTTP API");

        settings.VmixApiEnabled = false;
        var disable = MixerNative.ApiConfigure(0, port, user, password);
        if (disable != 0)
            HostLog.Write("WARN", $"disable vMix HTTP API after listen failure: {disable}");

        var ownerName = MixerNative.ApiListenOwnerText();
        var owner = string.IsNullOrEmpty(ownerName) ? null : ownerName;
        HostLog.Write(
            "WARN",
            owner is null
                ? $"vMix HTTP API listen failed on port {port}; disabled"
                : $"vMix HTTP API listen failed on port {port}; in use by {owner}; disabled");
        app.Dispatcher.BeginInvoke(
            () => ShowHttpListenFailed(port, owner),
            DispatcherPriority.ApplicationIdle);
    }

    private static void ApplyTcpApi(App app, SessionSettings settings)
    {
        var enabled = settings.VmixTcpEnabledValue;
        var code = MixerNative.TcpConfigure(enabled ? 1u : 0u);
        if (code == 0)
            return;
        if (!enabled || code != 5)
            MixerNative.ThrowIfFailed(code, "Configure vMix TCP API");

        settings.VmixTcpEnabled = false;
        var disable = MixerNative.TcpConfigure(0);
        if (disable != 0)
            HostLog.Write("WARN", $"disable vMix TCP API after listen failure: {disable}");

        var ownerName = MixerNative.TcpListenOwnerText();
        var owner = string.IsNullOrEmpty(ownerName) ? null : ownerName;
        HostLog.Write(
            "WARN",
            owner is null
                ? "vMix TCP API listen failed on port 8099; disabled"
                : $"vMix TCP API listen failed on port 8099; in use by {owner}; disabled");
        app.Dispatcher.BeginInvoke(
            () => ShowTcpListenFailed(owner),
            DispatcherPriority.ApplicationIdle);
    }

    private static void ApplyNativeApi(App app, SessionSettings settings)
    {
        if (app.Backend.IsRemote)
            return;
        var prefs = AppPrefs.Current;
        var port = prefs.NativeApiPort == 0 ? 9400u : prefs.NativeApiPort;
        var bind = string.IsNullOrWhiteSpace(prefs.NativeApiBind) ? "127.0.0.1" : prefs.NativeApiBind.Trim();
        var enabled = prefs.NativeApiEnabled;
        var token = CredentialStore.Load("listen");
        var code = MixerNative.WsConfigureOwned(
            enabled ? 1u : 0u,
            bind,
            port,
            token,
            prefs.NativeApiRole ?? "admin",
            prefs.ResolvedMediaDirectory);
        if (code == 0)
            return;
        if (!enabled || code != 5)
            MixerNative.ThrowIfFailed(code, "Configure Protobuf WebSocket API");

        prefs.NativeApiEnabled = false;
        prefs.Save();
        var disable = MixerNative.WsConfigureOwned(0, bind, port, "", "", "");
        if (disable != 0)
            HostLog.Write("WARN", $"disable Protobuf WebSocket API after listen failure: {disable}");

        var ownerName = MixerNative.WsListenOwnerText();
        var owner = string.IsNullOrEmpty(ownerName) ? null : ownerName;
        HostLog.Write(
            "WARN",
            owner is null
                ? $"Protobuf WebSocket API listen failed on port {port}; disabled"
                : $"Protobuf WebSocket API listen failed on port {port}; in use by {owner}; disabled");
        app.Dispatcher.BeginInvoke(
            () => ShowWsListenFailed(port, owner),
            DispatcherPriority.ApplicationIdle);
    }

    private static void ShowHttpListenFailed(uint port, string? owner)
    {
        var text = string.IsNullOrEmpty(owner)
            ? Loc.Format("msg.httpListenFailed", port)
            : Loc.Format("msg.httpListenFailedOwner", port, owner);
        ShowApiWarning(text);
    }

    private static void ShowTcpListenFailed(string? owner)
    {
        var text = string.IsNullOrEmpty(owner)
            ? Loc.T("msg.tcpListenFailed")
            : Loc.Format("msg.tcpListenFailedOwner", owner);
        ShowApiWarning(text);
    }

    private static void ShowWsListenFailed(uint port, string? owner)
    {
        var text = string.IsNullOrEmpty(owner)
            ? Loc.Format("msg.wsListenFailed", port)
            : Loc.Format("msg.wsListenFailedOwner", port, owner);
        ShowApiWarning(text);
    }

    private static void ShowApiWarning(string text)
    {
        var title = Loc.T("settings.webApi");
        var window = Current.MainWindow;
        if (window is null)
            MessageBox.Show(text, title, MessageBoxButton.OK, MessageBoxImage.Warning);
        else
            MessageBox.Show(window, text, title, MessageBoxButton.OK, MessageBoxImage.Warning);
    }

    private void AttachInputs()
    {
        foreach (var input in Session.Inputs)
            AttachInput(input, network: false);
        foreach (var input in Session.Inputs)
            AttachInput(input, network: true);
    }

    private void AttachInput(InputEntry input, bool network)
    {
        try
        {
            switch (input.Kind)
            {
                case InputKind.Color:
                case InputKind.Bars:
                    if (network)
                        return;
                    MixerNative.ThrowIfFailed(
                        MixerNative.DefineGenerator(
                            input.Id,
                            input.Kind == InputKind.Bars ? MixerNative.GenBars : MixerNative.GenSolid,
                            input.ColorR,
                            input.ColorG,
                            input.ColorB,
                            1,
                            input.Scroll ? 1u : 0u),
                        "Define colour generator");
                    MixerNative.GeneratorSetTone(input.Id, input.ToneHz, input.ToneLevelDbfs);
                    break;
                case InputKind.Still when !network && !string.IsNullOrWhiteSpace(input.PathOrAddress):
                    MixerApply.LoadStill(input.Id, input.PathOrAddress);
                    break;
                case InputKind.Video when !network && !string.IsNullOrWhiteSpace(input.PathOrAddress):
                    MixerApply.StartVideo(
                        input.Id,
                        input.PathOrAddress,
                        input.VideoLoop,
                        input.VideoStartsPlaying,
                        input.FrameBufferFrames);
                    break;
                case InputKind.UVC when !network && !string.IsNullOrWhiteSpace(input.PathOrAddress):
                    MixerApply.StartUvc(input.Id, input.PathOrAddress, input.CaptureWidth, input.CaptureHeight, input.CaptureFpsNum, input.CaptureFpsDen, input.FrameBufferFrames);
                    break;
                case InputKind.OMT when network && !string.IsNullOrWhiteSpace(input.PathOrAddress):
                    MixerApply.ConnectOmt(
                        input.Id,
                        input.PathOrAddress,
                        input.UseGpu,
                        input.FrameBufferFrames == 0 ? 1 : Math.Clamp(input.FrameBufferFrames, 1u, 8u),
                        input.BandwidthSave,
                        input.KeepFullOnMultiview,
                        input.OmtQuality);
                    break;
                case InputKind.NDI when network && !string.IsNullOrWhiteSpace(input.PathOrAddress):
                    MixerApply.ConnectNdi(
                        input.Id,
                        input.PathOrAddress,
                        input.FrameBufferFrames == 0 ? 1 : Math.Clamp(input.FrameBufferFrames, 1u, 8u),
                        input.NdiBandwidth);
                    break;
                case InputKind.Mix when !network && input.MixTargetId != 0:
                    MixerApply.DefineMixInput(
                        input.Id,
                        input.MixTargetId,
                        InputKindNames.MixSourceKind(input.MixSource),
                        input.FrameBufferFrames == 0 ? 1 : Math.Clamp(input.FrameBufferFrames, 1u, 8u),
                        input.MixAudioBusId);
                    break;
            }
        }
        catch (Exception ex)
        {
            HostLog.WriteException(ex);
        }
    }

    private void App_DispatcherUnhandledException(object sender, DispatcherUnhandledExceptionEventArgs e)
    {
        HostLog.WriteCrash(e.Exception);
    }

    protected override void OnExit(ExitEventArgs e)
    {
        Backend?.Dispose();
        if (!IsRemote)
        {
            foreach (var unit in Session.Units.ToArray())
                MixerNative.DestroyUnit(unit.Id);
        }
        MixerNative.Destroy();
        base.OnExit(e);
    }
}
