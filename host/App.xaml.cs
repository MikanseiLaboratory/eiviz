using System.IO;
using System.Windows;
using System.Windows.Threading;
using Eiviz.Host.Interop;
using Eiviz.Host.Media;

namespace Eiviz.Host;

public partial class App : Application
{
    internal CommandQueue Commands { get; private set; } = null!;
    internal Session Session { get; private set; } = null!;

    protected override void OnStartup(StartupEventArgs e)
    {
        System.Runtime.GCSettings.LatencyMode = System.Runtime.GCLatencyMode.SustainedLowLatency;
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
            File.WriteAllText(Path.Combine(AppContext.BaseDirectory, "host-error.txt"), ex.ToString());
            throw;
        }
    }

    internal void ReplaceSession(Session session)
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

    private void BootMixer()
    {
        MixerNative.ThrowIfFailed(
            MixerNative.Create(0, Session.Settings.MasterFpsNum, Session.Settings.MasterFpsDen),
            "DX12 mixer initialization");
        Commands = new CommandQueue();
        foreach (var unit in Session.Units)
        {
            unit.EnsureDefaultTransitions();
            unit.EnsureDefaultTiles();
            MixerNative.ThrowIfFailed(MixerNative.CreateUnit(unit.Id, unit.Width, unit.Height), "Create Mixing Unit");
            MixerNative.ThrowIfFailed(
                MixerNative.ConfigureUnit(unit.Id, unit.Width, unit.Height, unit.FpsNum, unit.FpsDen),
                "Configure Mixing Unit");
        }
        MixerNative.ThrowIfFailed(
            MixerNative.SetFrameBuffer(Math.Clamp(Session.Settings.FrameBufferFrames, 1u, 8u)),
            "Set frame buffer");
        MixerNative.ThrowIfFailed(
            MixerNative.SetRebarOptimization(Session.Settings.RebarOptimizationEnabled ? 1u : 0u),
            "Set ReBAR optimization");
        MixerNative.ThrowIfFailed(
            MixerNative.SetNdiGpuUpload(Session.Settings.NdiGpuUploadEnabled ? 1u : 0u),
            "Set NDI GPU upload");
        MixerNative.ThrowIfFailed(
            MixerNative.SetVideoGpuUpload(Session.Settings.VideoGpuUploadEnabled ? 1u : 0u),
            "Set video GPU upload");
        MixerNative.ThrowIfFailed(
            MixerNative.SetStillGpuUpload(Session.Settings.StillGpuUploadEnabled ? 1u : 0u),
            "Set still GPU upload");
        MixerNative.ThrowIfFailed(
            MixerNative.SetOmtCpuDecodeIngest(Session.Settings.OmtCpuDecodeIngestEnabled ? 1u : 0u),
            "Set OMT CPU decode ingest");
        MixerNative.ThrowIfFailed(
            MixerNative.SetOmtSkipJitterCopy(Session.Settings.OmtSkipJitterCopyEnabled ? 1u : 0u),
            "Set OMT skip jitter copy");
        MixerNative.ThrowIfFailed(
            MixerNative.SetReadbackOffClock(Session.Settings.ReadbackOffClockEnabled ? 1u : 0u),
            "Set readback off clock");
        MixerNative.ThrowIfFailed(
            MixerNative.SetMfImportNoWait(Session.Settings.MfImportNoWaitEnabled ? 1u : 0u),
            "Set MF import no wait");
        MixerNative.ThrowIfFailed(
            MixerNative.SetGpuQueueLockNarrow(Session.Settings.GpuQueueLockNarrowEnabled ? 1u : 0u),
            "Set GPU queue lock narrow");
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
            Commands.AddOutputNow(output);
        }
    }

    private void AttachInputs()
    {
        foreach (var input in Session.Inputs)
        {
            try
            {
                switch (input.Kind)
                {
                    case InputKind.Color:
                    case InputKind.Bars:
                        if (input.Id > MixerNative.Blue)
                        {
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
                        }
                        MixerNative.GeneratorSetTone(input.Id, input.ToneHz, input.ToneLevelDbfs);
                        break;
                    case InputKind.Still when !string.IsNullOrWhiteSpace(input.PathOrAddress):
                        MixerNative.ThrowIfFailed(MixerNative.LoadStill(input.Id, input.PathOrAddress), "Still load");
                        break;
                    case InputKind.Video when !string.IsNullOrWhiteSpace(input.PathOrAddress):
                        Commands.TryEnqueue(new StartVideoCommand(
                            input.Id,
                            input.PathOrAddress,
                            input.VideoLoop,
                            input.VideoStartsPlaying));
                        break;
                    case InputKind.Omt when !string.IsNullOrWhiteSpace(input.PathOrAddress):
                        Commands.TryEnqueue(new ConnectOmtCommand(
                            input.Id,
                            input.PathOrAddress,
                            input.UseGpu,
                            input.FrameBufferFrames == 0 ? 1 : Math.Clamp(input.FrameBufferFrames, 1u, 8u),
                            input.BandwidthSave,
                            input.KeepFullOnMultiview,
                            input.OmtQuality));
                        break;
                    case InputKind.Ndi when !string.IsNullOrWhiteSpace(input.PathOrAddress):
                        Commands.TryEnqueue(new ConnectNdiCommand(
                            input.Id,
                            input.PathOrAddress,
                            input.FrameBufferFrames == 0 ? 1 : Math.Clamp(input.FrameBufferFrames, 1u, 8u),
                            input.NdiBandwidth));
                        break;
                    case InputKind.Uvc when !string.IsNullOrWhiteSpace(input.PathOrAddress):
                        Commands.TryEnqueue(new StartUvcCommand(input.Id, input.PathOrAddress));
                        break;
                }
            }
            catch (Exception ex)
            {
                File.AppendAllText(Path.Combine(AppContext.BaseDirectory, "host-error.txt"), ex + Environment.NewLine);
            }
        }
    }

    private void App_DispatcherUnhandledException(object sender, DispatcherUnhandledExceptionEventArgs e)
    {
        File.WriteAllText(
            Path.Combine(AppContext.BaseDirectory, "host-error.txt"),
            e.Exception.ToString());
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
