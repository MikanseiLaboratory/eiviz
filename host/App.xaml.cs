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
    internal CommandQueue Commands { get; private set; } = null!;
    internal Session Session { get; private set; } = null!;

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
            BootMixer();
        }
        catch (Exception ex)
        {
            HostLog.WriteCrash(ex);
            throw;
        }
    }

    private void ReplaceSession(Session session)
    {
        var previous = Commands;
        Commands = null!;
        previous.DisposeAsync().AsTask().ConfigureAwait(false).GetAwaiter().GetResult();
        foreach (var unit in Session.Units.ToArray())
            MixerNative.DestroyUnit(unit.Id);
        MixerNative.Destroy();
        Session = session;
        BootMixer();
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
            MixerNative.Create(0, Session.Settings.MasterFpsNum, Session.Settings.MasterFpsDen),
            "DX12 mixer initialization");
        Commands = new CommandQueue();
        foreach (var unit in Session.Units)
        {
            unit.EnsureDefaultTransitions();
            MixerNative.ThrowIfFailed(MixerNative.CreateUnit(unit.Id, unit.Width, unit.Height), "Create Mixing Unit");
            MixerNative.ThrowIfFailed(
                MixerNative.ConfigureUnit(unit.Id, unit.Width, unit.Height, unit.FpsNum, unit.FpsDen),
                "Configure Mixing Unit");
        }
        FlipBudget.Configure(Session.Settings.FlipSwapchainLimit);
        MixerNative.ThrowIfFailed(
            MixerNative.SetFrameBuffer(Math.Clamp(Session.Settings.FrameBufferFrames, 1u, 8u)),
            "Set frame buffer");
        BusTheme.PushMultiviewLabels(Session);
        MixerNative.ThrowIfFailed(
            MixerNative.SetRebarOptimization(Session.Settings.RebarOptimizationEnabled ? 1u : 0u),
            "Set ReBAR optimization");
        MixerNative.ThrowIfFailed(
            MixerNative.SetNdiGpuUpload(Session.Settings.NdiGpuUploadEnabled ? 1u : 0u),
            "Set NDI GPU upload");
        MixerNative.VideoFormat = Session.Settings.InternalColorFormat == InternalColorFormat.Bgra
            ? MixerNative.FormatBgra
            : MixerNative.FormatUyvy;
        var primary = Session.Units[0];
        foreach (var scene in Session.Scenes)
            Commands.DefineSceneNow(scene, primary.Width, primary.Height);
        foreach (var layout in Session.Multiviews)
            Commands.PushMultiviewNow(layout, primary.Width, primary.Height);
        foreach (var unit in Session.Units)
        {
            var preview = Session.Scenes[0].GpuId;
            var program = Session.Scenes.Count > 1 ? Session.Scenes[1].GpuId : preview;
            Commands.PushUnitStateNow(unit.Id, CommandQueue.BuildState(unit, program, preview, 0, MixerNative.TransitionFade));
        }
        AttachInputs();
        AudioGraphSync.Push(Session);
        foreach (var output in Session.Outputs)
        {
            if (!output.Enabled)
                continue;
            if (output.Transport is not (OutputTransport.Omt or OutputTransport.Ndi))
                continue;
            Commands.TryEnqueue(new AddOutputCommand(output));
        }
        ApplyVmixApi();
        SessionStore.Publish(Session);
    }

    internal static void ApplyVmixApi()
    {
        var settings = ((App)Current).Session.Settings;
        MixerNative.ThrowIfFailed(
            MixerNative.ApiConfigure(
                settings.VmixApiEnabledValue ? 1u : 0u,
                settings.VmixApiPort == 0 ? 8088 : settings.VmixApiPort,
                settings.VmixApiUser ?? "",
                settings.VmixApiPassword ?? ""),
            "Configure vMix HTTP API");
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
                    Commands.TryEnqueue(new LoadStillCommand(input.Id, input.PathOrAddress));
                    break;
                case InputKind.Video when !network && !string.IsNullOrWhiteSpace(input.PathOrAddress):
                    Commands.TryEnqueue(new StartVideoCommand(
                        input.Id,
                        input.PathOrAddress,
                        input.VideoLoop,
                        input.VideoStartsPlaying,
                        input.FrameBufferFrames));
                    break;
                case InputKind.Uvc when !network && !string.IsNullOrWhiteSpace(input.PathOrAddress):
                    Commands.TryEnqueue(new StartUvcCommand(input.Id, input.PathOrAddress, input.CaptureWidth, input.CaptureHeight, input.CaptureFpsNum, input.CaptureFpsDen, input.FrameBufferFrames));
                    break;
                case InputKind.Omt when network && !string.IsNullOrWhiteSpace(input.PathOrAddress):
                    Commands.TryEnqueue(new ConnectOmtCommand(
                        input.Id,
                        input.PathOrAddress,
                        input.UseGpu,
                        input.FrameBufferFrames == 0 ? 1 : Math.Clamp(input.FrameBufferFrames, 1u, 8u),
                        input.BandwidthSave,
                        input.KeepFullOnMultiview,
                        input.OmtQuality));
                    break;
                case InputKind.Ndi when network && !string.IsNullOrWhiteSpace(input.PathOrAddress):
                    Commands.TryEnqueue(new ConnectNdiCommand(
                        input.Id,
                        input.PathOrAddress,
                        input.FrameBufferFrames == 0 ? 1 : Math.Clamp(input.FrameBufferFrames, 1u, 8u),
                        input.NdiBandwidth));
                    break;
                case InputKind.Mix when !network && input.MixTargetId != 0:
                    Commands.TryEnqueue(new DefineMixInputCommand(
                        input.Id,
                        input.MixTargetId,
                        InputKindNames.MixSourceKind(input.MixSource),
                        input.FrameBufferFrames == 0 ? 1 : Math.Clamp(input.FrameBufferFrames, 1u, 8u),
                        input.MixAudioBusId));
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

    protected override async void OnExit(ExitEventArgs e)
    {
        if (Commands is not null)
            await Commands.DisposeAsync();
        foreach (var unit in Session.Units.ToArray())
            MixerNative.DestroyUnit(unit.Id);
        MixerNative.Destroy();
        base.OnExit(e);
    }
}
