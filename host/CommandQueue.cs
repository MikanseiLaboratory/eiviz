using System.IO;
using System.Threading.Channels;
using Eiviz.Host.Interop;
using Eiviz.Host.Media;

namespace Eiviz.Host;

internal abstract record MixerCommand;
internal sealed record SetMixCommand(ulong UnitId, float Value) : MixerCommand;
internal sealed record CutCommand(ulong UnitId, bool Swap) : MixerCommand;
internal sealed record AutoCommand(ulong UnitId, uint Kind, uint DurationMs, bool Swap) : MixerCommand;
internal sealed record PreviewSceneCommand(ulong UnitId, ulong SceneGpuId) : MixerCommand;
internal sealed record PushUnitStateCommand(ulong UnitId, UnitState State) : MixerCommand;
internal sealed record DefineSceneCommand(SceneEntry Scene, uint Width, uint Height) : MixerCommand;
internal sealed record DestroySceneCommand(ulong GpuId) : MixerCommand;
internal sealed record ConnectOmtCommand(ulong SourceId, string Address) : MixerCommand;
internal sealed record LoadStillCommand(ulong SourceId, string Path) : MixerCommand;
internal sealed record StartVideoCommand(ulong SourceId, string Path) : MixerCommand;
internal sealed record StartUvcCommand(ulong SourceId, string SymbolicLink) : MixerCommand;
internal sealed record PatchAuxCommand(ulong UnitId, MixingUnitEntry Unit) : MixerCommand;
internal sealed record AddOutputCommand(OutputEntry Output) : MixerCommand;
internal sealed record RemoveOutputCommand(ulong OutputId) : MixerCommand;
internal sealed record DefineGeneratorCommand(ulong SourceId, uint Kind, float R, float G, float B, bool Scroll) : MixerCommand;

internal sealed class CommandQueue : IAsyncDisposable
{
    private readonly Channel<MixerCommand> _commands = Channel.CreateUnbounded<MixerCommand>(
        new UnboundedChannelOptions { SingleReader = true, SingleWriter = false });
    private readonly CancellationTokenSource _shutdown = new();
    private readonly Task _consumer;
    private readonly Dictionary<ulong, IDisposable> _pumps = [];

    public CommandQueue()
    {
        _consumer = ConsumeAsync(_shutdown.Token);
    }

    public bool TryEnqueue(MixerCommand command) => _commands.Writer.TryWrite(command);

    public void DefineSceneNow(SceneEntry scene, uint width, uint height)
    {
        PushScene(scene, width, height);
    }

    public void PushMultiviewNow(MultiviewLayout layout, uint width, uint height)
    {
        PushLayout(layout, width, height);
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
            output.UnitId);
        if (output.Transport == OutputTransport.Omt)
            MixerNative.ThrowIfFailed(code, "Add output");
        else if (code != 0)
            throw new InvalidOperationException(MixerNative.LastErrorText());
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
        unit.EnsureDefaultTiles();
        var tiles = unit.MultiviewTiles.Take(16).ToList();
        state.MvSlotCount = (uint)tiles.Count;
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
                    Z = slot.Z
                };
            SetOverlay(ref state, i, desc);
        }
        for (var i = 0; i < 16; i++)
            SetMv(ref state, i, i < tiles.Count ? EncodeSlot(tiles[i]) : 0);
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

    private static void SetMv(ref UnitState state, int index, ulong id)
    {
        switch (index)
        {
            case 0: state.Mv0 = id; break;
            case 1: state.Mv1 = id; break;
            case 2: state.Mv2 = id; break;
            case 3: state.Mv3 = id; break;
            case 4: state.Mv4 = id; break;
            case 5: state.Mv5 = id; break;
            case 6: state.Mv6 = id; break;
            case 7: state.Mv7 = id; break;
            case 8: state.Mv8 = id; break;
            case 9: state.Mv9 = id; break;
            case 10: state.Mv10 = id; break;
            case 11: state.Mv11 = id; break;
            case 12: state.Mv12 = id; break;
            case 13: state.Mv13 = id; break;
            case 14: state.Mv14 = id; break;
            default: state.Mv15 = id; break;
        }
    }

    private static void PushScene(SceneEntry scene, uint width, uint height)
    {
        PushLayers(scene.GpuId, width, height, scene.Layers.Select(layer => new OverlayDesc
        {
            SourceId = layer.InputId,
            Rect = new Rect { X = layer.X, Y = layer.Y, Width = layer.Width, Height = layer.Height },
            Opacity = layer.Opacity,
            Z = layer.Z
        }).ToArray());
    }

    internal static void PushLayout(MultiviewLayout layout, uint width, uint height)
    {
        var tiles = layout.Tiles;
        if (tiles.Count == 0)
        {
            PushLayers(layout.GpuId, width, height, []);
            return;
        }
        var (cols, rows) = TileGrid(tiles.Count);
        var layers = new OverlayDesc[tiles.Count];
        for (var i = 0; i < tiles.Count; i++)
        {
            var col = i % cols;
            var row = i / cols;
            layers[i] = new OverlayDesc
            {
                SourceId = EncodeSlot(tiles[i]),
                Rect = new Rect
                {
                    X = col / (float)cols,
                    Y = row / (float)rows,
                    Width = 1f / cols,
                    Height = 1f / rows
                },
                Opacity = 1,
                Z = i
            };
        }
        PushLayers(layout.GpuId, width, height, layers);
    }

    internal static (int Cols, int Rows) TileGrid(int count)
    {
        if (count <= 1) return (1, 1);
        if (count == 2) return (2, 1);
        if (count <= 4) return (2, 2);
        if (count <= 6) return (3, 2);
        if (count <= 8) return (4, 2);
        if (count <= 12) return (4, 3);
        if (count <= 16) return (4, 4);
        var cols = (int)Math.Ceiling(Math.Sqrt(count));
        var rows = (int)Math.Ceiling(count / (double)cols);
        return (cols, rows);
    }

    private static void PushLayers(ulong gpuId, uint width, uint height, OverlayDesc[] layers)
    {
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

    private async Task ConsumeAsync(CancellationToken token)
    {
        await foreach (var command in _commands.Reader.ReadAllAsync(token))
        {
            try
            {
                switch (command)
                {
                    case SetMixCommand setMix:
                        ApplyMix(setMix.UnitId, setMix.Value);
                        break;
                    case CutCommand cut:
                        MixerNative.ThrowIfFailed(MixerNative.Cut(cut.UnitId, cut.Swap ? 1u : 0u), "CUT");
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
                        MixerNative.ThrowIfFailed(MixerNative.ConnectOmt(connect.SourceId, connect.Address), "OMT connect");
                        break;
                    case LoadStillCommand still:
                        MixerNative.ThrowIfFailed(MixerNative.LoadStill(still.SourceId, still.Path), "Still load");
                        break;
                    case StartVideoCommand video:
                        ReplacePump(video.SourceId, MfFramePump.StartFile(video.SourceId, video.Path));
                        break;
                    case StartUvcCommand uvc:
                        ReplacePump(uvc.SourceId, MfFramePump.StartCapture(uvc.SourceId, uvc.SymbolicLink));
                        break;
                    case AddOutputCommand add:
                        MixerNative.ThrowIfFailed(
                            MixerNative.OutputAdd(
                                add.Output.Id,
                                (uint)add.Output.Transport,
                                add.Output.Name,
                                (uint)add.Output.SourceKind,
                                add.Output.SourceId,
                                add.Output.UnitId),
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
                        break;
                    default:
                        throw new InvalidOperationException("Unknown command.");
                }
            }
            catch (Exception ex)
            {
                File.WriteAllText(Path.Combine(AppContext.BaseDirectory, "host-error.txt"), ex.ToString());
            }
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
                MixerNative.SetUnitState(auto.UnitId, &current);
            }
        }
        MixerNative.ThrowIfFailed(MixerNative.Auto(auto.UnitId, auto.DurationMs, auto.Swap ? 1u : 0u), "AUTO");
    }

    private static void ApplyPreview(ulong unitId, ulong sceneGpuId)
    {
        unsafe
        {
            UnitState current = default;
            MixerNative.ThrowIfFailed(MixerNative.GetUnitState(unitId, &current), "Get unit");
            current.PreviewSource = sceneGpuId;
            current.Mix = 0;
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
            MixerNative.SetUnitState(unitId, &state);
        }
    }

    private static void ApplyMix(ulong unitId, float mix)
    {
        unsafe
        {
            UnitState current = default;
            if (MixerNative.GetUnitState(unitId, &current) != 0)
                return;
            current.Mix = Math.Clamp(mix, 0f, 1f);
            MixerNative.SetUnitState(unitId, &current);
        }
    }

    private void ReplacePump(ulong id, IDisposable pump)
    {
        if (_pumps.Remove(id, out var previous))
            previous.Dispose();
        _pumps[id] = pump;
    }

    public async ValueTask DisposeAsync()
    {
        foreach (var pump in _pumps.Values)
            pump.Dispose();
        _pumps.Clear();
        _commands.Writer.TryComplete();
        _shutdown.Cancel();
        try { await _consumer; }
        catch (OperationCanceledException) { }
        _shutdown.Dispose();
    }
}
