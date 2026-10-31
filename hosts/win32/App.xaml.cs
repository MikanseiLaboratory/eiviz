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
            {
                // The mixer is either missing or from another build. Both happen in
                // development (a release `dotnet build` overwrites the debug cdylibs),
                // so the message names the file and what to do.
                throw new InvalidOperationException(Loc.T("msg.mixerMismatch"));
            }
            LoadRequiredProModule(e.Args);
            Session = Session.Default();
            if (HostRole.IsRemote)
                BootRemoteMixer();
            else
            {
                if (!TryLoadStartupSession(e.Args))
                {
                    Shutdown();
                    return;
                }
                Backend = new LocalEivizBackend();
                BootMixer();
            }
            _started = true;
        }
        catch (Exception ex)
        {
            HostLog.WriteCrash(ex);
            // A startup failure must never look like a crash. The operator gets the
            // reason, what to do about it, and where the log is, and the process exits
            // with a code that says it failed.
            MessageBox.Show(
                $"{ex.Message}\n\n{Loc.T("msg.startupFailedHint")}\n{HostLog.DirectoryPath}",
                Loc.T("msg.startupFailed"),
                MessageBoxButton.OK,
                MessageBoxImage.Error);
            Shutdown(1);
            return;
        }
    }

    private bool TryLoadStartupSession(string[] args)
    {
        var path = args.Select(item => item.Trim('"'))
            .FirstOrDefault(item =>
                item.EndsWith(".eivz", StringComparison.OrdinalIgnoreCase)
                || item.EndsWith(".eivzx", StringComparison.OrdinalIgnoreCase));
        if (string.IsNullOrWhiteSpace(path))
            return true;
        try
        {
            if (SessionStore.FileHasAssets(path))
            {
                var prompt = new ImportExportDialog(path);
                if (prompt.ShowDialog() != true)
                    return true;
                Session = SessionStore.Import(path, prompt.SessionPath, prompt.MediaDirectory);
                AppPrefs.Current.RememberSession(prompt.SessionPath);
                return true;
            }
            Session = SessionStore.Load(path);
            AppPrefs.Current.RememberSession(path);
            return true;
        }
        catch (Exception ex)
        {
            HostLog.WriteException(ex);
            MessageBox.Show(ex.Message, Loc.T("msg.loadSession"));
            return false;
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
        using var allowClose = OnAirLock.AllowClose();
        var previous = MainWindow as MainWindow;
        previous?.CloseOwnedSurfaces();
        FlipBudget.Reset();
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

    private static void LoadRequiredProModule(string[] args)
    {
        string? path = null;
        for (var i = 0; i < args.Length - 1; i++)
        {
            if (args[i] == "--pro-module")
                path = args[i + 1];
        }
        if (MixerNative.ProPrepare(path) == 0)
            return;
        var reason = MixerNative.QueryProError();
        // The reason on its own ("no Pro module public key") does not tell an operator
        // what to do, so the path and the two real causes travel with it.
        throw new InvalidOperationException(
            string.IsNullOrWhiteSpace(path)
                ? Loc.Format("msg.proModuleMissing", reason)
                : Loc.Format("msg.proModuleRejected", path, reason));
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
        // The mixer starts with no Replay at all, so a project that already has one needs
        // its settings and clip list pushed once. Without this the clips of an earlier
        // run have nothing to play from, even though the host shows them.
        if (!string.IsNullOrWhiteSpace(Session.ReplayJson))
        {
            MixerNative.ThrowIfFailed(
                MixerNative.ReplayConfigure(Session.ReplayJson),
                "Replay configure");
        }
        MixerApply.ApplySceneBuses(Session);
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

    internal async Task<(bool Ok, string Error)> TryConnectRemoteAsync(string url, string token)
    {
        var endpoint = url.Trim();
        try
        {
            var next = await RemoteEivizBackend.OpenAsync(endpoint, token ?? "");
            Backend?.Dispose();
            Backend = next;
            CredentialStore.Save(endpoint, token ?? "");
            AppPrefs.Current.RemoteUrl = endpoint;
            AppPrefs.Current.RememberRemote(endpoint);
            return (true, "");
        }
        catch (Exception ex)
        {
            HostLog.Write("ERROR", ex.Message);
            Backend ??= new DisconnectedRemoteBackend();
            return (false, ex.Message);
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
        ApplyTcpApi(settings);
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

        var owner = ListenOwner(MixerNative.ApiListenOwnerText());
        if (port < 65535)
        {
            var next = port + 1;
            var retry = MixerNative.ApiConfigure(1, next, user, password);
            if (retry == 0)
            {
                settings.VmixApiPort = next;
                PersistHttpPort(app);
                HostLog.Write(
                    "WARN",
                    owner is null
                        ? $"vMix HTTP API listen failed on port {port}; listening on {next}"
                        : $"vMix HTTP API listen failed on port {port}; in use by {owner}; listening on {next}");
                QueueApiWarning(() => ShowHttpListenMoved(port, next, owner));
                return;
            }
            if (retry != 5)
                MixerNative.ThrowIfFailed(retry, "Configure vMix HTTP API");
            owner = ListenOwner(MixerNative.ApiListenOwnerText());
            port = next;
        }

        settings.VmixApiEnabled = false;
        var disable = MixerNative.ApiConfigure(0, port, user, password);
        if (disable != 0)
            HostLog.Write("WARN", $"disable vMix HTTP API after listen failure: {disable}");

        HostLog.Write(
            "WARN",
            owner is null
                ? $"vMix HTTP API listen failed on port {port}; disabled"
                : $"vMix HTTP API listen failed on port {port}; in use by {owner}; disabled");
        QueueApiWarning(() => ShowHttpListenFailed(port, owner));
    }

    private static string? ListenOwner(string ownerName) =>
        string.IsNullOrEmpty(ownerName) ? null : ownerName;

    private static void PersistHttpPort(App app)
    {
        var path = SessionStore.CurrentPath();
        if (!SessionStore.CanOverwrite(path))
            return;
        try
        {
            SessionStore.Save(app.Session, path!);
        }
        catch (Exception ex)
        {
            HostLog.Write("WARN", $"save HTTP port: {ex.Message}");
        }
    }

    private static void ApplyTcpApi(SessionSettings settings)
    {
        var enabled = settings.VmixTcpEnabledValue;
        var code = MixerNative.TcpConfigure(enabled ? 1u : 0u);
        if (code == 0)
            return;
        if (!enabled || code != 5)
            MixerNative.ThrowIfFailed(code, "Configure vMix TCP API");

        var owner = ListenOwner(MixerNative.TcpListenOwnerText());
        settings.VmixTcpEnabled = false;
        var disable = MixerNative.TcpConfigure(0);
        if (disable != 0)
            HostLog.Write("WARN", $"disable vMix TCP API after listen failure: {disable}");
        HostLog.Write(
            "WARN",
            owner is null
                ? "vMix TCP API listen failed on port 8099; disabled"
                : $"vMix TCP API listen failed on port 8099; in use by {owner}; disabled");
        QueueApiWarning(() => ShowTcpListenFailed(owner));
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

        var owner = ListenOwner(MixerNative.WsListenOwnerText());
        prefs.NativeApiEnabled = false;
        prefs.Save();
        var disable = MixerNative.WsConfigureOwned(0, bind, port, "", "", "");
        if (disable != 0)
            HostLog.Write("WARN", $"disable Protobuf WebSocket API after listen failure: {disable}");
        HostLog.Write(
            "WARN",
            owner is null
                ? $"Protobuf WebSocket API listen failed on port {port}; disabled"
                : $"Protobuf WebSocket API listen failed on port {port}; in use by {owner}; disabled");
        QueueApiWarning(() => ShowWsListenFailed(port, owner));
    }

    private static void ShowHttpListenMoved(uint port, uint next, string? owner)
    {
        // A moved port is not a problem to solve: the server is running, the port is
        // remembered, and the log already says so. A dialog here would greet every
        // start on a machine where vMix owns 8088.
        var text = string.IsNullOrEmpty(owner)
            ? Loc.Format("msg.httpListenMoved", port, next)
            : Loc.Format("msg.httpListenMovedOwner", port, owner, next);
        HostLog.Write("WARN", text);
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
        // Another application owning 8099 (vMix) is a normal setup, not something the
        // operator can fix by reading a dialog: the TCP API is off and the log says
        // why. Only an unclaimed port that still refuses to open is worth a dialog.
        if (!string.IsNullOrEmpty(owner))
        {
            HostLog.Write("WARN", text);
            return;
        }
        ShowApiWarning(text);
    }

    private static void ShowWsListenFailed(uint port, string? owner)
    {
        var text = string.IsNullOrEmpty(owner)
            ? Loc.Format("msg.wsListenFailed", port)
            : Loc.Format("msg.wsListenFailedOwner", port, owner);
        ShowApiWarning(text);
    }

    private static void QueueApiWarning(Action show)
    {
        // ApplicationIdle never runs while the 16ms UI timer is queued, so the
        // warning dialog was logged and then never shown.
        var app = (App)Current;
        app.Dispatcher.BeginInvoke(show, DispatcherPriority.Normal);
    }

    private static void ShowApiWarning(string text)
    {
        var title = Loc.T("settings.webApi");
        var window = Current?.MainWindow;
        if (window is not { IsVisible: true })
        {
            if (window is not null)
            {
                void OnShown(object? sender, EventArgs e)
                {
                    window.ContentRendered -= OnShown;
                    ShowApiWarning(text);
                }
                window.ContentRendered += OnShown;
                return;
            }
            MessageBox.Show(text, title, MessageBoxButton.OK, MessageBoxImage.Warning);
            return;
        }
        try
        {
            MessageBox.Show(window, text, title, MessageBoxButton.OK, MessageBoxImage.Warning);
        }
        catch (InvalidOperationException)
        {
            MessageBox.Show(text, title, MessageBoxButton.OK, MessageBoxImage.Warning);
        }
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
                case InputKind.DeckLink when network && !string.IsNullOrWhiteSpace(input.PathOrAddress):
                    MixerNative.ThrowIfFailed(
                        MixerNative.ConnectDeckLink(
                            input.Id,
                            input.PathOrAddress,
                            input.DecklinkMode,
                            input.FrameBufferFrames == 0 ? 1 : Math.Clamp(input.FrameBufferFrames, 1u, 8u)),
                        "DeckLink connect");
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
                        input.FrameBufferFrames == 0 ? 1 : Math.Clamp(input.FrameBufferFrames, 1u, 8u));
                    break;
                case InputKind.Audio when !network:
                    MixerApply.StartAudioCapture(input);
                    break;
                case InputKind.Replay:
                    break;
            }
        }
        catch (Exception ex)
        {
            HostLog.WriteException(ex);
        }
    }

    private bool _started;
    private DateTime _lastErrorToastUtc = DateTime.MinValue;

    private void App_DispatcherUnhandledException(object sender, DispatcherUnhandledExceptionEventArgs e)
    {
        HostLog.WriteCrash(e.Exception);
        // A failed startup leaves no usable session, so only a running switcher keeps going.
        if (!_started)
            return;
        e.Handled = true;
        var now = DateTime.UtcNow;
        if (now - _lastErrorToastUtc < TimeSpan.FromSeconds(3))
            return;
        _lastErrorToastUtc = now;
        try
        {
            StatusToast.Show(MainWindow, $"{Loc.T("msg.unexpectedError")}: {e.Exception.Message}");
        }
        catch (Exception toastError)
        {
            HostLog.WriteException(toastError);
        }
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
