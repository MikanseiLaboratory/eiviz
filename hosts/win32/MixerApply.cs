using System.IO;
using System.Linq;
using System.Runtime.InteropServices;
using System.Windows;
using Eiviz.Host.Dialogs;
using Eiviz.Host.I18n;
using Eiviz.Host.Interop;
using Rect = Eiviz.Host.Interop.Rect;

namespace Eiviz.Host;

internal static class MixerApply
{
    public static void DefineScene(SceneEntry scene, uint width, uint height)
    {
        if (Application.Current is App { Backend.IsRemote: true })
            return;
        PushScene(scene, width, height);
    }

    public static void PushMultiview(MultiviewLayout layout, uint width, uint height)
    {
        if (Application.Current is App { Backend.IsRemote: true })
            return;
        var session = Application.Current is App app ? app.Session : null;
        PushLayout(layout, width, height, session);
    }

    public static void PushUnitState(ulong unitId, UnitState state)
    {
        unsafe
        {
            MixerNative.ThrowIfFailed(MixerNative.SetUnitState(unitId, &state), "Set unit state");
        }
    }

    public static void AddOutput(OutputEntry output)
    {
        NormalizeOutputSource(output);
        var audioBusId = output.SourceKind == OutputSourceKind.Multiview ? 0uL : output.AudioBusId;
        var code = MixerNative.OutputAdd(
            output.Id,
            (uint)output.Transport,
            output.Name,
            (uint)output.SourceKind,
            output.SourceId,
            output.UnitId,
            output.UseGpu ? 1u : 0u,
            audioBusId,
            output.SkipEncodeWhenNoReceivers ? 1u : 0u);
        if (code != 0)
            MixerNative.ThrowIfFailed(code, "Add output");
    }

    public static bool TryAddOutput(OutputEntry output) => Try(() => AddOutput(output));

    public static bool Cut(ulong unitId, bool swap) =>
        Application.Current is App app ? app.Backend.Cut(unitId, swap) : CutLocal(unitId, swap);

    internal static bool CutLocal(ulong unitId, bool swap) => Try(() =>
    {
        MixerNative.ThrowIfFailed(MixerNative.Cut(unitId, swap ? 1u : 0u, MixerNative.IncomingPreview), "CUT");
        if (Application.Current is App app)
            CaptureSceneBuses(app.Session);
    });

    public static bool Auto(ulong unitId, MixingUnitEntry unit, TransitionPreset preset) =>
        Application.Current is App app ? app.Backend.Auto(unitId, unit, preset) : AutoLocal(unitId, unit, preset);

    internal static bool AutoLocal(ulong unitId, MixingUnitEntry unit, TransitionPreset preset)
    {
        return Try(() => Auto(unitId, preset.Kind, preset.DurationMsFor(unit), preset.Swap, preset.KeepPreview,
            preset.Easing, preset.Direction, preset.DipR, preset.DipG, preset.DipB, preset.DipA,
            preset.CustomWgsl, preset.Softness, preset.Param));
    }

    public static void Auto(
        ulong unitId,
        uint kind,
        uint durationMs,
        bool swap,
        bool keepPreview,
        uint easing,
        uint direction,
        float dipR,
        float dipG,
        float dipB,
        float dipA,
        string? customWgsl,
        float softness = 0.02f,
        float param = 0f)
    {
        unsafe
        {
            UnitState current = default;
            if (MixerNative.GetUnitState(unitId, &current) == 0)
            {
                current.TransitionKind = kind;
                current.TransitionEasing = easing;
                current.TransitionDirection = direction;
                current.KeepPreview = keepPreview ? 1u : 0u;
                current.DipR = dipR;
                current.DipG = dipG;
                current.DipB = dipB;
                current.DipA = dipA <= 0 ? 1 : dipA;
                current.Softness = softness;
                current.Param = param;
                MixerNative.SetUnitState(unitId, &current);
            }
        }
        MixerNative.SetCustomWgsl(unitId, ResolveCustomWgsl(kind, customWgsl));
        MixerNative.ThrowIfFailed(
            MixerNative.Auto(
                unitId,
                kind,
                durationMs,
                swap ? 1u : 0u,
                keepPreview ? 1u : 0u,
                easing,
                direction,
                dipR,
                dipG,
                dipB,
                dipA <= 0 ? 1 : dipA,
                MixerNative.IncomingPreview,
                softness,
                param),
            "AUTO");
    }

    public static bool PreviewScene(ulong unitId, ulong sceneGpuId) =>
        Application.Current is App app ? app.Backend.Preview(unitId, sceneGpuId) : PreviewLocal(unitId, sceneGpuId);

    internal static bool PreviewLocal(ulong unitId, ulong sceneGpuId) => Try(() =>
    {
        unsafe
        {
            UnitState current = default;
            MixerNative.ThrowIfFailed(MixerNative.GetUnitState(unitId, &current), "Get unit");
            current.PreviewSource = sceneGpuId;
            MixerNative.ThrowIfFailed(MixerNative.SetUnitState(unitId, &current), "Preview scene");
        }
        if (Application.Current is App app)
            CaptureSceneBuses(app.Session);
    });

    public static void CaptureSceneBuses(Session session)
    {
        if (HostRole.IsRemote)
            return;
        if (Application.Current is not App app)
            return;
        foreach (var unit in session.Units)
        {
            app.Backend.BusSources(unit.Id, out var previewGpu, out var programGpu);
            if (SceneId(session, previewGpu) is ulong preview)
                unit.PreviewSceneId = preview;
            if (SceneId(session, programGpu) is ulong program)
                unit.ProgramSceneId = program;
        }
    }

    public static void ApplySceneBuses(Session session)
    {
        if (HostRole.IsRemote)
            return;
        foreach (var unit in session.Units)
        {
            var preview = SceneGpu(session, unit.PreviewSceneId)
                ?? session.Scenes.FirstOrDefault()?.GpuId
                ?? 0;
            var program = SceneGpu(session, unit.ProgramSceneId)
                ?? session.Scenes.ElementAtOrDefault(1)?.GpuId
                ?? preview;
            if (preview == 0 && program == 0)
                continue;
            Try(() =>
            {
                unsafe
                {
                    UnitState current = default;
                    MixerNative.ThrowIfFailed(MixerNative.GetUnitState(unit.Id, &current), "Get unit");
                    current.PreviewSource = preview;
                    current.ProgramSource = program;
                    MixerNative.ThrowIfFailed(MixerNative.SetUnitState(unit.Id, &current), "Restore buses");
                }
            });
        }
    }

    private static ulong? SceneId(Session session, ulong gpuId) =>
        session.Scenes.FirstOrDefault(scene => scene.GpuId == gpuId)?.Id;

    private static ulong? SceneGpu(Session session, ulong sceneId) =>
        sceneId == 0 ? null : session.Scenes.FirstOrDefault(scene => scene.Id == sceneId)?.GpuId;

    public static bool SetMix(ulong unitId, float mix, TransitionPreset? preset = null) =>
        Application.Current is App app ? app.Backend.SetMix(unitId, mix, preset) : SetMixLocal(unitId, mix, preset);

    internal static bool SetMixLocal(ulong unitId, float mix, TransitionPreset? preset = null) => Try(() =>
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
    });

    public static bool PatchAux(ulong unitId, MixingUnitEntry unit)
    {
        if (Application.Current is App { Backend.IsRemote: true })
            return true;
        return Try(() =>
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
    });
    }

    public static bool TryDefineScene(SceneEntry scene, uint width, uint height) =>
        Try(() => PushScene(scene, width, height));

    public static bool DestroyScene(ulong gpuId) => Try(() => MixerNative.DestroyScene(gpuId));

    public static bool ConnectOmt(
        ulong sourceId,
        string address,
        bool useGpu,
        uint frameBufferFrames,
        BandwidthSave saveMode,
        bool keepFullOnMultiview,
        OmtQuality quality) => Try(() =>
    {
        MixerNative.ThrowIfFailed(
            MixerNative.ConnectOmt(
                sourceId,
                address,
                useGpu ? 1u : 0u,
                Math.Clamp(frameBufferFrames, 1u, 8u),
                (uint)quality),
            "OMT connect");
        MixerNative.ThrowIfFailed(
            MixerNative.SetLiveSave(
                sourceId,
                (uint)saveMode,
                keepFullOnMultiview ? MixerNative.SaveFlagMultiview : 0u),
            "OMT bandwidth save");
    });

    public static bool ConnectNdi(ulong sourceId, string address, uint frameBufferFrames, NdiBandwidth bandwidth) =>
        Try(() => MixerNative.ThrowIfFailed(
            MixerNative.ConnectNdi(
                sourceId,
                address,
                Math.Clamp(frameBufferFrames, 1u, 8u),
                bandwidth == NdiBandwidth.Lowest ? 1u : 0u),
            "NDI connect"));

    public static bool LiveSave(ulong sourceId, BandwidthSave saveMode, bool keepFullOnMultiview, OmtQuality? omtQuality = null) =>
        Try(() =>
        {
            MixerNative.ThrowIfFailed(
                MixerNative.SetLiveSave(
                    sourceId,
                    (uint)saveMode,
                    keepFullOnMultiview ? MixerNative.SaveFlagMultiview : 0u),
                "Bandwidth save");
            if (omtQuality is { } quality)
            {
                MixerNative.ThrowIfFailed(
                    MixerNative.SetOmtQuality(sourceId, (uint)quality),
                    "OMT quality");
            }
        });

    public static bool LoadStill(ulong sourceId, string path) => Try(() =>
    {
        if (!File.Exists(path))
            throw new InvalidOperationException(Loc.MissingFile("Still load"));
        MixerNative.ThrowIfFailed(MixerNative.LoadStill(sourceId, path), "Still load");
    }, reportLoadError: true);

    public static bool StartVideo(
        ulong sourceId,
        string path,
        bool loop = true,
        bool playing = true,
        uint frameBufferFrames = 3,
        long positionHns = 0) => Try(() =>
    {
        if (!File.Exists(path))
            throw new InvalidOperationException(Loc.MissingFile("Video start"));
        MixerNative.ThrowIfFailed(
            MixerNative.VideoStart(
                sourceId,
                path,
                0,
                MixerNative.VideoFormat,
                0,
                0,
                0,
                0,
                Math.Clamp(frameBufferFrames == 0 ? 3u : frameBufferFrames, 1u, 8u)),
            "Video start");
        MixerNative.VideoSetLoop(sourceId, loop ? 1u : 0u);
        MixerNative.VideoSetPlaying(sourceId, playing ? 1u : 0u);
        if (positionHns > 0)
            MixerNative.VideoSeek(sourceId, positionHns);
    }, reportLoadError: true);

    public static bool StartUvc(
        ulong sourceId,
        string symbolicLink,
        uint width,
        uint height,
        uint fpsNum,
        uint fpsDen,
        uint frameBufferFrames = 3) => Try(() =>
        MixerNative.ThrowIfFailed(
            MixerNative.VideoStart(
                sourceId,
                symbolicLink,
                1,
                MixerNative.VideoFormat,
                width,
                height,
                fpsNum,
                fpsDen,
                Math.Clamp(frameBufferFrames == 0 ? 3u : frameBufferFrames, 1u, 8u)),
            "UVC start"));

    public static bool RemoveOutput(ulong outputId) => Try(() => MixerNative.OutputRemove(outputId));

    public static bool DefineGenerator(
        ulong sourceId,
        uint kind,
        float r,
        float g,
        float b,
        bool scroll,
        float toneHz = 0,
        float toneLevelDbfs = -20) => Try(() =>
    {
        MixerNative.ThrowIfFailed(
            MixerNative.DefineGenerator(sourceId, kind, r, g, b, 1, scroll ? 1u : 0u),
            "Define colour generator");
        MixerNative.GeneratorSetTone(sourceId, toneHz, toneLevelDbfs);
    });

    public static bool DefineMixInput(ulong sourceId, ulong targetId, uint sourceKind, uint delay, ulong audioBusId) =>
        Try(() => MixerNative.ThrowIfFailed(
            MixerNative.DefineMixInput(sourceId, targetId, sourceKind, delay, audioBusId),
            "Define Mix Input"));

    public static bool StartAudioCapture(InputEntry input) =>
        Try(() => MixerNative.ThrowIfFailed(
            MixerNative.AudioCaptureStart(
                input.Id,
                input.AudioDeviceKind switch
                {
                    AudioDeviceKind.Wasapi => 1u,
                    AudioDeviceKind.Asio => 2u,
                    AudioDeviceKind.CoreAudio => 3u,
                    _ => 0u
                },
                input.AudioDeviceId,
                input.AudioCaptureMode switch
                {
                    AudioCaptureMode.EndpointLoopback => 1u,
                    AudioCaptureMode.ProcessLoopback => 2u,
                    _ => 0u
                },
                input.AudioMapLeft,
                input.AudioMapRight,
                input.AudioProcessExe,
                input.AudioProcessAumid),
            "Audio capture start"));

    public static bool DropSource(ulong sourceId) => Try(() => MixerNative.DestroySource(sourceId));

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

    internal static void NormalizeOutputSource(OutputEntry output)
    {
        if (output.SourceKind == OutputSourceKind.Multiview
            && output.SourceId != 0
            && output.SourceId < MixerNative.MultiviewBase)
            output.SourceId = MixerNative.MultiviewGpuId(output.SourceId);
        else if (output.SourceKind == OutputSourceKind.Scene
            && output.SourceId != 0
            && output.SourceId < MixerNative.SceneBase)
            output.SourceId = MixerNative.SceneGpuId(output.SourceId);
    }

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

    private static string ResolveCustomWgsl(uint kind, string? wgsl) =>
        kind == MixerNative.TransitionCustom && string.IsNullOrWhiteSpace(wgsl)
            ? CustomWgslWindow.WgslTemplate
            : wgsl ?? "";

    private static bool Try(Action action, bool reportLoadError = false)
    {
        try
        {
            action();
            return true;
        }
        catch (Exception ex)
        {
            HostLog.WriteException(ex);
            if (reportLoadError)
                ReportUserError(ex.Message, Loc.T("msg.addInput"));
            return false;
        }
    }

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
}
