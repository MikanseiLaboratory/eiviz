using System.IO;
using System.Runtime.InteropServices;
using System.Threading.Channels;
using System.Windows;
using Eiviz.Host.Dialogs;
using Eiviz.Host.I18n;
using Eiviz.Host.Interop;
using Rect = Eiviz.Host.Interop.Rect;

namespace Eiviz.Host;

internal abstract record MixerCommand;
internal sealed record SetMixCommand(ulong UnitId, float Value, TransitionPreset? Preset = null) : MixerCommand;
internal sealed record CutCommand(ulong UnitId, bool Swap) : MixerCommand;
internal sealed record AutoCommand(
    ulong UnitId,
    uint Kind,
    uint DurationMs,
    bool Swap,
    bool KeepPreview,
    uint Easing,
    uint Direction,
    float DipR,
    float DipG,
    float DipB,
    float DipA,
    string? CustomWgsl,
    float Softness = 0.02f,
    float Param = 0f) : MixerCommand;
internal sealed record PreviewSceneCommand(ulong UnitId, ulong SceneGpuId) : MixerCommand;
internal sealed record PushUnitStateCommand(ulong UnitId, UnitState State) : MixerCommand;
internal sealed record DefineSceneCommand(SceneEntry Scene, uint Width, uint Height) : MixerCommand;
internal sealed record DestroySceneCommand(ulong GpuId) : MixerCommand;
internal sealed record ConnectOmtCommand(ulong SourceId, string Address, bool UseGpu, uint FrameBufferFrames, BandwidthSave SaveMode, bool KeepFullOnMultiview, OmtQuality Quality) : MixerCommand;
internal sealed record ConnectNdiCommand(ulong SourceId, string Address, uint FrameBufferFrames, NdiBandwidth Bandwidth) : MixerCommand;
internal sealed record LiveSaveCommand(ulong SourceId, BandwidthSave SaveMode, bool KeepFullOnMultiview, OmtQuality? OmtQuality = null) : MixerCommand;
internal sealed record LoadStillCommand(ulong SourceId, string Path) : MixerCommand;
internal sealed record StartVideoCommand(ulong SourceId, string Path, bool Loop = true, bool Playing = true, uint FrameBufferFrames = 3) : MixerCommand;
internal sealed record StartUvcCommand(ulong SourceId, string SymbolicLink, uint Width, uint Height, uint FpsNum, uint FpsDen, uint FrameBufferFrames = 3) : MixerCommand;
internal sealed record PatchAuxCommand(ulong UnitId, MixingUnitEntry Unit) : MixerCommand;
internal sealed record AddOutputCommand(OutputEntry Output) : MixerCommand;
internal sealed record RemoveOutputCommand(ulong OutputId) : MixerCommand;
internal sealed record DefineGeneratorCommand(ulong SourceId, uint Kind, float R, float G, float B, bool Scroll, float ToneHz = 0, float ToneLevelDbfs = -20) : MixerCommand;
internal sealed record DefineMixInputCommand(ulong SourceId, ulong TargetId, uint SourceKind, uint Delay) : MixerCommand;
internal sealed record DropSourceCommand(ulong SourceId) : MixerCommand;

internal sealed class CommandQueue : IAsyncDisposable
{
    private readonly Channel<MixerCommand> _commands = Channel.CreateUnbounded<MixerCommand>(
        new UnboundedChannelOptions { SingleReader = true, SingleWriter = false });
    private readonly CancellationTokenSource _shutdown = new();
    private readonly Task _consumer;
    public CommandQueue()
    {
        _consumer = Task.Factory.StartNew(
            () => ConsumeAsync(_shutdown.Token).GetAwaiter().GetResult(),
            CancellationToken.None,
            TaskCreationOptions.LongRunning | TaskCreationOptions.DenyChildAttach,
            TaskScheduler.Default);
    }

    public bool TryEnqueue(MixerCommand command)
    {
        if (IsDeferredIo(command))
            return _commands.Writer.TryWrite(command);
        return ApplyHandled(command);
    }

    private static bool IsDeferredIo(MixerCommand command) =>
        command is ConnectOmtCommand or ConnectNdiCommand or LoadStillCommand or StartVideoCommand or StartUvcCommand;

    public void DefineSceneNow(SceneEntry scene, uint width, uint height)
    {
        PushScene(scene, width, height);
    }

    public void PushMultiviewNow(MultiviewLayout layout, uint width, uint height)
    {
        var session = Application.Current is App app ? app.Session : null;
        PushLayout(layout, width, height, session);
    }

    public void PushUnitStateNow(ulong unitId, UnitState state)
    {
        unsafe
        {
            MixerNative.ThrowIfFailed(MixerNative.SetUnitState(unitId, &state), "Set unit state");
        }
    }

    public void AddOutputNow(OutputEntry output)
    {
        var code = MixerNative.OutputAdd(
            output.Id,
            (uint)output.Transport,
            output.Name,
            (uint)output.SourceKind,
            output.SourceId,
            output.UnitId,
            output.UseGpu ? 1u : 0u);
        if (code != 0)
            MixerNative.ThrowIfFailed(code, "Add output");
    }

    public static UnitState BuildState(MixingUnitEntry unit, ulong program, ulong preview, float mix, uint transitionKind)
    {
        var state = new UnitState
        {
            ProgramSource = program,
            PreviewSource = preview,
            Mix = mix,
            TransitionKind = transitionKind,
            OverlayCount = 0
        };
        var enabled = unit.Overlays.Where(slot => slot.Enabled).Take(8).ToList();
        state.OverlayCount = (uint)enabled.Count;
        for (var i = 0; i < 8; i++)
        {
            var slot = i < enabled.Count ? enabled[i] : null;
            var desc = slot is null
                ? default
                : new OverlayDesc
                {
                    SourceId = slot.SceneGpuId,
                    Rect = new Rect { X = slot.X, Y = slot.Y, Width = slot.Width, Height = slot.Height },
                    Opacity = slot.Opacity,
                    Z = slot.Z,
                    AudioFollow = slot.AudioFollow ? 1u : 0u,
                    Hidden = slot.Hidden ? 1u : 0u,
                    Crop = new Rect { X = slot.CropX, Y = slot.CropY, Width = slot.CropWidth, Height = slot.CropHeight }
                };
            SetOverlay(ref state, i, desc);
        }
        return state;
    }

    internal static ulong EncodeSlot(MvSlot slot) => slot.Kind switch
    {
        MvSlotKind.Input => slot.SourceId,
        MvSlotKind.Scene => slot.SourceId,
        MvSlotKind.MuPreview => MixerNative.MuPreview(slot.SourceId),
        MvSlotKind.MuProgram => MixerNative.MuProgram(slot.SourceId),
        _ => 0
    };

    private static void SetOverlay(ref UnitState state, int index, OverlayDesc desc)
    {
        switch (index)
        {
            case 0: state.Overlay0 = desc; break;
            case 1: state.Overlay1 = desc; break;
            case 2: state.Overlay2 = desc; break;
            case 3: state.Overlay3 = desc; break;
            case 4: state.Overlay4 = desc; break;
            case 5: state.Overlay5 = desc; break;
            case 6: state.Overlay6 = desc; break;
            default: state.Overlay7 = desc; break;
        }
    }

    private static void PushScene(SceneEntry scene, uint width, uint height)
    {
        PushLayers(scene.GpuId, width, height, scene.Layers.Select(layer => new OverlayDesc
        {
            SourceId = layer.InputId,
            Rect = new Rect { X = layer.X, Y = layer.Y, Width = layer.Width, Height = layer.Height },
            Opacity = layer.Opacity,
            Z = layer.Z,
            AudioFollow = layer.AudioFollow ? 1u : 0u,
            Hidden = layer.Hidden ? 1u : 0u,
            Crop = new Rect { X = layer.CropX, Y = layer.CropY, Width = layer.CropWidth, Height = layer.CropHeight }
        }).ToArray());
    }

    internal static void PushLayout(MultiviewLayout layout, uint width, uint height, Session? session)
    {
        layout.EnsureTiles();
        var panes = MultiviewGeometry.Panes(layout.Template);
        var layers = new List<OverlayDesc>(panes.Count);
        var names = session is null ? new string[panes.Count] : SlotNames(layout, session, panes);
        for (var i = 0; i < panes.Count; i++)
        {
            var pane = panes[i];
            var source = i < layout.Tiles.Count ? EncodeSlot(layout.Tiles[i]) : 0;
            layers.Add(BusLayer(source, pane.X, pane.Y, pane.Width, pane.Height, i));
        }
        PushLayers(layout.GpuId, width, height, layers.ToArray(), names);
        var previewUnit = layout.Tiles.FirstOrDefault(tile => tile.Kind == MvSlotKind.MuPreview)?.SourceId ?? layout.PreviewUnitId;
        var programUnit = layout.Tiles.FirstOrDefault(tile => tile.Kind == MvSlotKind.MuProgram)?.SourceId ?? layout.ProgramUnitId;
        MixerNative.ThrowIfFailed(
            MixerNative.BindMultiview(layout.GpuId, previewUnit == 0 ? 1 : previewUnit, programUnit == 0 ? 1 : programUnit),
            "Bind multiview");
        if (session is not null)
            layout.PushLabelStyle(session.Settings);
    }

    private static OverlayDesc BusLayer(ulong sourceId, float x, float y, float w, float h, int z) => new()
    {
        SourceId = sourceId,
        Rect = new Rect { X = x, Y = y, Width = w, Height = h },
        Opacity = 1,
        Z = z,
        AudioFollow = 0
    };

    private static string[] SlotNames(MultiviewLayout layout, Session session, IReadOnlyList<MultiviewPane> panes)
    {
        var names = new string[panes.Count];
        for (var i = 0; i < panes.Count; i++)
            names[i] = i < layout.Tiles.Count ? TileLabel(layout.Tiles[i], session) : "";
        return names;
    }

    private static string TileLabel(MvSlot tile, Session session) =>
        tile.LabelFollow ? TileName(tile, session) : tile.Label ?? "";

    private static string TileName(MvSlot tile, Session session) => tile.Kind switch
    {
        MvSlotKind.Input => session.Inputs.FirstOrDefault(item => item.Id == tile.SourceId)?.Name ?? "",
        MvSlotKind.Scene => session.Scenes.FirstOrDefault(item => item.GpuId == tile.SourceId)?.Name ?? "",
        MvSlotKind.MuPreview => $"PRV  {session.Units.FirstOrDefault(item => item.Id == tile.SourceId)?.Name ?? tile.SourceId.ToString()}",
        MvSlotKind.MuProgram => $"PGM  {session.Units.FirstOrDefault(item => item.Id == tile.SourceId)?.Name ?? tile.SourceId.ToString()}",
        _ => ""
    };

    private static void PushLayers(ulong gpuId, uint width, uint height, OverlayDesc[] layers, string[]? labels = null)
    {
        var pins = new List<nint>();
        try
        {
            if (labels is not null)
            {
                var n = Math.Min(layers.Length, labels.Length);
                for (var i = 0; i < n; i++)
                {
                    if (string.IsNullOrEmpty(labels[i]))
                        continue;
                    var ptr = Marshal.StringToCoTaskMemUTF8(labels[i]);
                    pins.Add(ptr);
                    layers[i].Label = ptr;
                }
            }
            unsafe
            {
                if (layers.Length == 0)
                {
                    MixerNative.ThrowIfFailed(
                        MixerNative.DefineScene(gpuId, width, height, 0, null),
                        "Define scene");
                    return;
                }
                fixed (OverlayDesc* ptr = layers)
                {
                    MixerNative.ThrowIfFailed(
                        MixerNative.DefineScene(gpuId, width, height, (uint)layers.Length, ptr),
                        "Define scene");
                }
            }
        }
        finally
        {
            foreach (var pin in pins)
                Marshal.FreeCoTaskMem(pin);
        }
    }

    private async Task ConsumeAsync(CancellationToken token)
    {
        var inflight = new List<Task>();
        try
        {
            await foreach (var command in _commands.Reader.ReadAllAsync(token))
            {
                var captured = command;
                inflight.Add(Task.Run(() => ApplyHandled(captured)));
            }
        }
        catch (OperationCanceledException)
        {
        }
        if (inflight.Count > 0)
        {
            try { await Task.WhenAll(inflight).ConfigureAwait(false); }
            catch (Exception ex) { HostLog.WriteException(ex); }
        }
    }

    private bool ApplyHandled(MixerCommand command)
    {
        try
        {
            Apply(command);
            return true;
        }
        catch (Exception ex)
        {
            HostLog.WriteException(ex);
            if (command is LoadStillCommand or StartVideoCommand)
                ReportUserError(ex.Message, Loc.T("msg.addInput"));
            return false;
        }
    }

    private static void Apply(MixerCommand command)
    {
        switch (command)
        {
            case SetMixCommand setMix:
                ApplyMix(setMix.UnitId, setMix.Value, setMix.Preset);
                break;
            case CutCommand cut:
                MixerNative.ThrowIfFailed(MixerNative.Cut(cut.UnitId, cut.Swap ? 1u : 0u, MixerNative.IncomingPreview), "CUT");
                break;
            case AutoCommand auto:
                ApplyAuto(auto);
                break;
            case PreviewSceneCommand preview:
                ApplyPreview(preview.UnitId, preview.SceneGpuId);
                break;
            case PushUnitStateCommand push:
                ApplyState(push.UnitId, push.State);
                break;
            case PatchAuxCommand patch:
                PatchAux(patch.UnitId, patch.Unit);
                break;
            case DefineSceneCommand define:
                PushScene(define.Scene, define.Width, define.Height);
                break;
            case DestroySceneCommand destroy:
                MixerNative.DestroyScene(destroy.GpuId);
                break;
            case ConnectOmtCommand connect:
                MixerNative.ThrowIfFailed(
                    MixerNative.ConnectOmt(
                        connect.SourceId,
                        connect.Address,
                        connect.UseGpu ? 1u : 0u,
                        Math.Clamp(connect.FrameBufferFrames, 1u, 8u),
                        (uint)connect.Quality),
                    "OMT connect");
                MixerNative.ThrowIfFailed(
                    MixerNative.SetLiveSave(
                        connect.SourceId,
                        (uint)connect.SaveMode,
                        connect.KeepFullOnMultiview ? MixerNative.SaveFlagMultiview : 0u),
                    "OMT bandwidth save");
                break;
            case ConnectNdiCommand connect:
                MixerNative.ThrowIfFailed(
                    MixerNative.ConnectNdi(
                        connect.SourceId,
                        connect.Address,
                        Math.Clamp(connect.FrameBufferFrames, 1u, 8u),
                        connect.Bandwidth == NdiBandwidth.Lowest ? 1u : 0u),
                    "NDI connect");
                break;
            case LiveSaveCommand save:
                MixerNative.ThrowIfFailed(
                    MixerNative.SetLiveSave(
                        save.SourceId,
                        (uint)save.SaveMode,
                        save.KeepFullOnMultiview ? MixerNative.SaveFlagMultiview : 0u),
                    "Bandwidth save");
                if (save.OmtQuality is { } quality)
                {
                    MixerNative.ThrowIfFailed(
                        MixerNative.SetOmtQuality(save.SourceId, (uint)quality),
                        "OMT quality");
                }
                break;
            case LoadStillCommand still:
                if (!File.Exists(still.Path))
                    throw new InvalidOperationException(Loc.MissingFile("Still load"));
                MixerNative.ThrowIfFailed(MixerNative.LoadStill(still.SourceId, still.Path), "Still load");
                break;
            case StartVideoCommand video:
                if (!File.Exists(video.Path))
                    throw new InvalidOperationException(Loc.MissingFile("Video start"));
                MixerNative.ThrowIfFailed(
                    MixerNative.VideoStart(
                        video.SourceId,
                        video.Path,
                        0,
                        MixerNative.VideoFormat,
                        0,
                        0,
                        0,
                        0,
                        Math.Clamp(video.FrameBufferFrames == 0 ? 3u : video.FrameBufferFrames, 1u, 8u)),
                    "Video start");
                MixerNative.VideoSetLoop(video.SourceId, video.Loop ? 1u : 0u);
                MixerNative.VideoSetPlaying(video.SourceId, video.Playing ? 1u : 0u);
                break;
            case StartUvcCommand uvc:
                MixerNative.ThrowIfFailed(
                    MixerNative.VideoStart(
                        uvc.SourceId,
                        uvc.SymbolicLink,
                        1,
                        MixerNative.VideoFormat,
                        uvc.Width,
                        uvc.Height,
                        uvc.FpsNum,
                        uvc.FpsDen,
                        Math.Clamp(uvc.FrameBufferFrames == 0 ? 3u : uvc.FrameBufferFrames, 1u, 8u)),
                    "UVC start");
                break;
            case AddOutputCommand add:
                MixerNative.ThrowIfFailed(
                    MixerNative.OutputAdd(
                        add.Output.Id,
                        (uint)add.Output.Transport,
                        add.Output.Name,
                        (uint)add.Output.SourceKind,
                        add.Output.SourceId,
                        add.Output.UnitId,
                        add.Output.UseGpu ? 1u : 0u),
                    "Add output");
                break;
            case RemoveOutputCommand remove:
                MixerNative.OutputRemove(remove.OutputId);
                break;
            case DefineGeneratorCommand generator:
                MixerNative.ThrowIfFailed(
                    MixerNative.DefineGenerator(
                        generator.SourceId,
                        generator.Kind,
                        generator.R,
                        generator.G,
                        generator.B,
                        1,
                        generator.Scroll ? 1u : 0u),
                    "Define colour generator");
                MixerNative.GeneratorSetTone(generator.SourceId, generator.ToneHz, generator.ToneLevelDbfs);
                break;
            case DefineMixInputCommand mix:
                MixerNative.ThrowIfFailed(
                    MixerNative.DefineMixInput(mix.SourceId, mix.TargetId, mix.SourceKind, mix.Delay),
                    "Define Mix Input");
                break;
            case DropSourceCommand drop:
                MixerNative.DestroySource(drop.SourceId);
                break;
            default:
                throw new InvalidOperationException("Unknown command.");
        }
    }

    private static void ApplyAuto(AutoCommand auto)
    {
        unsafe
        {
            UnitState current = default;
            if (MixerNative.GetUnitState(auto.UnitId, &current) == 0)
            {
                current.TransitionKind = auto.Kind;
                current.TransitionEasing = auto.Easing;
                current.TransitionDirection = auto.Direction;
                current.KeepPreview = auto.KeepPreview ? 1u : 0u;
                current.DipR = auto.DipR;
                current.DipG = auto.DipG;
                current.DipB = auto.DipB;
                current.DipA = auto.DipA <= 0 ? 1 : auto.DipA;
                current.Softness = auto.Softness;
                current.Param = auto.Param;
                MixerNative.SetUnitState(auto.UnitId, &current);
            }
        }
        MixerNative.SetCustomWgsl(auto.UnitId, ResolveCustomWgsl(auto.Kind, auto.CustomWgsl));
        MixerNative.ThrowIfFailed(
            MixerNative.Auto(
                auto.UnitId,
                auto.Kind,
                auto.DurationMs,
                auto.Swap ? 1u : 0u,
                auto.KeepPreview ? 1u : 0u,
                auto.Easing,
                auto.Direction,
                auto.DipR,
                auto.DipG,
                auto.DipB,
                auto.DipA <= 0 ? 1 : auto.DipA,
                MixerNative.IncomingPreview,
                auto.Softness,
                auto.Param),
            "AUTO");
    }

    private static void ApplyPreview(ulong unitId, ulong sceneGpuId)
    {
        unsafe
        {
            UnitState current = default;
            MixerNative.ThrowIfFailed(MixerNative.GetUnitState(unitId, &current), "Get unit");
            current.PreviewSource = sceneGpuId;
            MixerNative.ThrowIfFailed(MixerNative.SetUnitState(unitId, &current), "Preview scene");
        }
    }

    private static void ApplyState(ulong unitId, UnitState state)
    {
        unsafe
        {
            MixerNative.ThrowIfFailed(MixerNative.SetUnitState(unitId, &state), "Set unit state");
        }
    }

    private static void PatchAux(ulong unitId, MixingUnitEntry unit)
    {
        unsafe
        {
            UnitState current = default;
            if (MixerNative.GetUnitState(unitId, &current) != 0)
                return;
            var state = BuildState(unit, current.ProgramSource, current.PreviewSource, current.Mix, current.TransitionKind);
            state.IncomingSource = current.IncomingSource;
            state.Softness = current.Softness;
            state.Param = current.Param;
            MixerNative.SetUnitState(unitId, &state);
        }
    }

    private static void ApplyMix(ulong unitId, float mix, TransitionPreset? preset)
    {
        unsafe
        {
            UnitState current = default;
            if (MixerNative.GetUnitState(unitId, &current) != 0)
                return;
            current.Mix = Math.Clamp(mix, 0f, 1f);
            if (preset is { } look)
            {
                current.TransitionKind = look.Kind;
                current.TransitionEasing = look.Easing;
                current.TransitionDirection = look.Direction;
                current.DipR = look.DipR;
                current.DipG = look.DipG;
                current.DipB = look.DipB;
                current.DipA = look.DipA <= 0 ? 1 : look.DipA;
                current.Softness = look.Softness;
                current.Param = look.Param;
                MixerNative.SetCustomWgsl(unitId, ResolveCustomWgsl(look.Kind, look.CustomWgsl));
            }
            MixerNative.SetUnitState(unitId, &current);
        }
    }

    private static string ResolveCustomWgsl(uint kind, string? wgsl) =>
        kind == MixerNative.TransitionCustom && string.IsNullOrWhiteSpace(wgsl)
            ? CustomWgslWindow.WgslTemplate
            : wgsl ?? "";

    private static void ReportUserError(string message, string title)
    {
        var app = Application.Current;
        if (app is null)
            return;
        _ = app.Dispatcher.BeginInvoke(() =>
        {
            if (app.MainWindow is Window window)
                MessageBox.Show(window, message, title);
            else
                MessageBox.Show(message, title);
        });
    }

    public async ValueTask DisposeAsync()
    {
        _commands.Writer.TryComplete();
        _shutdown.Cancel();
        try { await _consumer.ConfigureAwait(false); }
        catch (OperationCanceledException) { }
        _shutdown.Dispose();
    }
}
