using Eiviz.Host.Interop;

namespace Eiviz.Host;

public enum InputKind
{
    Color,
    Bars,
    Black,
    Still,
    Video,
    Omt,
    Ndi,
    Uvc
}

public enum BandwidthSave
{
    AlwaysLow = 0,
    NotOnProgram = 1,
    NotOnPreviewOrProgram = 2,
    AlwaysFull = 3
}

public enum OmtQuality
{
    Default = 0,
    Low = 1,
    Medium = 50,
    High = 100
}

public enum NdiBandwidth
{
    Highest = 0,
    Lowest = 1
}

public enum OutputTransport
{
    Omt = 0,
    Ndi = 1,
    DeckLink = 2
}

public enum OutputSourceKind
{
    Scene = 0,
    MuPreview = 1,
    MuProgram = 2,
    Multiview = 3,
    Input = 4
}

public enum MvSlotKind
{
    None,
    Input,
    Scene,
    MuPreview,
    MuProgram
}

public enum MultiviewTemplate
{
    PreviewProgram8,
    PreviewProgram8Bottom,
    PreviewProgram8Left,
    PreviewProgram8Right,
    PreviewProgram2,
    Quad4TopLeft,
    Quad4TopRight,
    Quad4BottomLeft,
    Quad4BottomRight,
    Large5TopLeft,
    Large5TopRight,
    Large5BottomLeft,
    Large5BottomRight,
    Grid2x2,
    Grid3x3,
    Grid4x4
}

internal static class MultiviewGeometry
{
    public static readonly (string Title, MultiviewTemplate[] Items)[] Groups =
    [
        ("Preview + Program + 8",
        [
            MultiviewTemplate.PreviewProgram8,
            MultiviewTemplate.PreviewProgram8Bottom,
            MultiviewTemplate.PreviewProgram8Left,
            MultiviewTemplate.PreviewProgram8Right
        ]),
        ("Preview + Program + 2", [MultiviewTemplate.PreviewProgram2]),
        ("1 + 5",
        [
            MultiviewTemplate.Large5TopLeft,
            MultiviewTemplate.Large5TopRight,
            MultiviewTemplate.Large5BottomLeft,
            MultiviewTemplate.Large5BottomRight
        ]),
        ("3 + 4",
        [
            MultiviewTemplate.Quad4TopLeft,
            MultiviewTemplate.Quad4TopRight,
            MultiviewTemplate.Quad4BottomLeft,
            MultiviewTemplate.Quad4BottomRight
        ]),
        ("Grid",
        [
            MultiviewTemplate.Grid2x2,
            MultiviewTemplate.Grid3x3,
            MultiviewTemplate.Grid4x4
        ])
    ];

    public static IEnumerable<MultiviewTemplate> Choices => Groups.SelectMany(group => group.Items);

    public static int TileCount(MultiviewTemplate template) => template switch
    {
        MultiviewTemplate.PreviewProgram2 => 2,
        MultiviewTemplate.Quad4TopLeft or MultiviewTemplate.Quad4TopRight
            or MultiviewTemplate.Quad4BottomLeft or MultiviewTemplate.Quad4BottomRight => 7,
        MultiviewTemplate.Large5TopLeft or MultiviewTemplate.Large5TopRight
            or MultiviewTemplate.Large5BottomLeft or MultiviewTemplate.Large5BottomRight => 6,
        MultiviewTemplate.Grid2x2 => 4,
        MultiviewTemplate.Grid3x3 => 9,
        MultiviewTemplate.Grid4x4 => 16,
        _ => 8
    };

    public static bool HasBusPanes(MultiviewTemplate template) => template
        is MultiviewTemplate.PreviewProgram8
        or MultiviewTemplate.PreviewProgram8Bottom
        or MultiviewTemplate.PreviewProgram8Left
        or MultiviewTemplate.PreviewProgram8Right
        or MultiviewTemplate.PreviewProgram2;

    public static string Title(MultiviewTemplate template) => template switch
    {
        MultiviewTemplate.PreviewProgram8 => "Buses on top",
        MultiviewTemplate.PreviewProgram8Bottom => "Buses on bottom",
        MultiviewTemplate.PreviewProgram8Left => "Buses on left",
        MultiviewTemplate.PreviewProgram8Right => "Buses on right",
        MultiviewTemplate.PreviewProgram2 => "Buses on top",
        MultiviewTemplate.Large5TopLeft => "Large top-left",
        MultiviewTemplate.Large5TopRight => "Large top-right",
        MultiviewTemplate.Large5BottomLeft => "Large bottom-left",
        MultiviewTemplate.Large5BottomRight => "Large bottom-right",
        MultiviewTemplate.Quad4TopLeft => "Four top-left",
        MultiviewTemplate.Quad4TopRight => "Four top-right",
        MultiviewTemplate.Quad4BottomLeft => "Four bottom-left",
        MultiviewTemplate.Quad4BottomRight => "Four bottom-right",
        MultiviewTemplate.Grid2x2 => "2×2",
        MultiviewTemplate.Grid3x3 => "3×3",
        MultiviewTemplate.Grid4x4 => "4×4",
        _ => template.ToString()
    };

    public static (string Preview, string Program) BusTitles(MultiviewTemplate template) => template switch
    {
        MultiviewTemplate.PreviewProgram8 => ("PRV (top left)", "PGM (top right)"),
        MultiviewTemplate.PreviewProgram8Bottom => ("PRV (bottom left)", "PGM (bottom right)"),
        MultiviewTemplate.PreviewProgram8Left => ("PRV (bottom left)", "PGM (top left)"),
        MultiviewTemplate.PreviewProgram8Right => ("PRV (bottom right)", "PGM (top right)"),
        MultiviewTemplate.PreviewProgram2 => ("PRV (top left)", "PGM (top right)"),
        _ => ("Preview tally unit", "Program tally unit")
    };

    public static IReadOnlyList<MultiviewPane> Panes(MultiviewTemplate template)
    {
        var panes = new List<MultiviewPane>();
        switch (template)
        {
            case MultiviewTemplate.PreviewProgram2:
                panes.Add(new MultiviewPane(0f, 0f, 0.5f, 0.5f, MultiviewPaneKind.Preview));
                panes.Add(new MultiviewPane(0.5f, 0f, 0.5f, 0.5f, MultiviewPaneKind.Program));
                AddGrid(panes, 2, 1, 0f, 0.5f, 1f, 0.5f);
                break;
            case MultiviewTemplate.PreviewProgram8:
                panes.Add(new MultiviewPane(0f, 0f, 0.5f, 0.5f, MultiviewPaneKind.Preview));
                panes.Add(new MultiviewPane(0.5f, 0f, 0.5f, 0.5f, MultiviewPaneKind.Program));
                AddGrid(panes, 4, 2, 0f, 0.5f, 1f, 0.5f);
                break;
            case MultiviewTemplate.PreviewProgram8Bottom:
                AddGrid(panes, 4, 2, 0f, 0f, 1f, 0.5f);
                panes.Add(new MultiviewPane(0f, 0.5f, 0.5f, 0.5f, MultiviewPaneKind.Preview));
                panes.Add(new MultiviewPane(0.5f, 0.5f, 0.5f, 0.5f, MultiviewPaneKind.Program));
                break;
            case MultiviewTemplate.PreviewProgram8Left:
                panes.Add(new MultiviewPane(0f, 0.5f, 0.5f, 0.5f, MultiviewPaneKind.Preview));
                panes.Add(new MultiviewPane(0f, 0f, 0.5f, 0.5f, MultiviewPaneKind.Program));
                AddGrid(panes, 2, 4, 0.5f, 0f, 0.5f, 1f);
                break;
            case MultiviewTemplate.PreviewProgram8Right:
                AddGrid(panes, 2, 4, 0f, 0f, 0.5f, 1f);
                panes.Add(new MultiviewPane(0.5f, 0.5f, 0.5f, 0.5f, MultiviewPaneKind.Preview));
                panes.Add(new MultiviewPane(0.5f, 0f, 0.5f, 0.5f, MultiviewPaneKind.Program));
                break;
            case MultiviewTemplate.Quad4TopLeft:
                AddQuad4(panes, 0);
                break;
            case MultiviewTemplate.Quad4TopRight:
                AddQuad4(panes, 1);
                break;
            case MultiviewTemplate.Quad4BottomLeft:
                AddQuad4(panes, 2);
                break;
            case MultiviewTemplate.Quad4BottomRight:
                AddQuad4(panes, 3);
                break;
            case MultiviewTemplate.Large5TopLeft:
                AddLarge5(panes, 0, 0);
                break;
            case MultiviewTemplate.Large5TopRight:
                AddLarge5(panes, 1, 0);
                break;
            case MultiviewTemplate.Large5BottomLeft:
                AddLarge5(panes, 0, 1);
                break;
            case MultiviewTemplate.Large5BottomRight:
                AddLarge5(panes, 1, 1);
                break;
            case MultiviewTemplate.Grid3x3:
                AddGrid(panes, 3, 3, 0f, 0f, 1f, 1f);
                break;
            case MultiviewTemplate.Grid4x4:
                AddGrid(panes, 4, 4, 0f, 0f, 1f, 1f);
                break;
            default:
                AddGrid(panes, 2, 2, 0f, 0f, 1f, 1f);
                break;
        }
        return panes;
    }

    private static void AddGrid(List<MultiviewPane> panes, int cols, int rows, float x, float y, float w, float h)
    {
        for (var i = 0; i < cols * rows; i++)
        {
            var col = i % cols;
            var row = i / cols;
            var x0 = x + w * col / cols;
            var y0 = y + h * row / rows;
            var x1 = x + w * (col + 1) / cols;
            var y1 = y + h * (row + 1) / rows;
            panes.Add(new MultiviewPane(x0, y0, x1 - x0, y1 - y0, MultiviewPaneKind.Tile));
        }
    }

    private static void AddQuad4(List<MultiviewPane> panes, int smallQuad)
    {
        for (var quad = 0; quad < 4; quad++)
        {
            var x = quad % 2 * 0.5f;
            var y = quad / 2 * 0.5f;
            if (quad == smallQuad)
                AddGrid(panes, 2, 2, x, y, 0.5f, 0.5f);
            else
                panes.Add(new MultiviewPane(x, y, 0.5f, 0.5f, MultiviewPaneKind.Tile));
        }
    }

    private static void AddLarge5(List<MultiviewPane> panes, int largeCol, int largeRow)
    {
        var x0 = largeCol / 3f;
        var y0 = largeRow / 3f;
        var x1 = (largeCol + 2) / 3f;
        var y1 = (largeRow + 2) / 3f;
        panes.Add(new MultiviewPane(x0, y0, x1 - x0, y1 - y0, MultiviewPaneKind.Tile));
        for (var row = 0; row < 3; row++)
        {
            for (var col = 0; col < 3; col++)
            {
                if (col >= largeCol && col < largeCol + 2 && row >= largeRow && row < largeRow + 2)
                    continue;
                var sx0 = col / 3f;
                var sy0 = row / 3f;
                var sx1 = (col + 1) / 3f;
                var sy1 = (row + 1) / 3f;
                panes.Add(new MultiviewPane(sx0, sy0, sx1 - sx0, sy1 - sy0, MultiviewPaneKind.Tile));
            }
        }
    }
}

internal enum MultiviewPaneKind
{
    Preview,
    Program,
    Tile
}

internal readonly record struct MultiviewPane(float X, float Y, float Width, float Height, MultiviewPaneKind Kind);

public enum VideoPlayWhen
{
    Never = 0,
    OnActive = 1,
    OnPreview = 2,
    Always = 3
}

public enum VideoTriggerWhen
{
    Never = 0,
    OnActive = 1,
    OnDeactivated = 2,
    OnPreview = 3
}

public sealed class MvSlot
{
    public MvSlotKind Kind { get; set; }
    public ulong SourceId { get; set; }
    public bool LabelFollow { get; set; } = true;
    public string Label { get; set; } = "";
}

public sealed class InputEntry
{
    public ulong Id { get; init; }
    public required string Name { get; set; }
    public InputKind Kind { get; set; }
    public string? PathOrAddress { get; set; }
    public float ColorR { get; set; } = 1;
    public float ColorG { get; set; }
    public float ColorB { get; set; }
    public bool Scroll { get; set; }
    public float ToneHz { get; set; }
    public float ToneLevelDbfs { get; set; } = -20;
    public uint BusMask { get; set; } = 1;
    public float Gain { get; set; } = 1;
    public bool Mute { get; set; }
    public bool UseGpu { get; set; }
    public uint FrameBufferFrames { get; set; } = 1;
    public BandwidthSave BandwidthSave { get; set; } = BandwidthSave.NotOnPreviewOrProgram;
    public bool KeepFullOnMultiview { get; set; }
    public OmtQuality OmtQuality { get; set; } = OmtQuality.Default;
    public NdiBandwidth NdiBandwidth { get; set; } = NdiBandwidth.Highest;
    public bool VideoLoop { get; set; } = true;
    public VideoPlayWhen VideoPlayWhen { get; set; } = VideoPlayWhen.Never;
    public VideoTriggerWhen VideoRestartWhen { get; set; } = VideoTriggerWhen.Never;
    public VideoTriggerWhen VideoPauseWhen { get; set; } = VideoTriggerWhen.Never;
    public bool VideoStartsPlaying =>
        VideoPlayWhen is VideoPlayWhen.Never or VideoPlayWhen.Always;
    public bool IsBuiltin => Id is MixerNative.Color or MixerNative.Bars or MixerNative.Black or MixerNative.Blue;
    public override string ToString() => Name;
}

internal static class InputKindNames
{
    public static string Category(InputKind kind) => kind switch
    {
        InputKind.Color or InputKind.Bars or InputKind.Black => "Colours",
        InputKind.Still => "Still",
        InputKind.Video => "Video",
        InputKind.Omt => "OMT",
        InputKind.Ndi => "NDI®",
        InputKind.Uvc => "Video Capture",
        _ => kind.ToString()
    };
}

public sealed class SceneLayer
{
    public ulong InputId { get; set; }
    public float X { get; set; }
    public float Y { get; set; }
    public float Width { get; set; } = 1;
    public float Height { get; set; } = 1;
    public float Opacity { get; set; } = 1;
    public int Z { get; set; }
    public bool AudioFollow { get; set; } = true;
}

public sealed class SceneEntry
{
    public ulong Id { get; set; }
    public required string Name { get; set; }
    public ulong MonitorId { get; set; }
    public List<SceneLayer> Layers { get; } = [];
    public ulong GpuId => MixerNative.SceneBase | Id;
    public override string ToString() => Name;
}

public sealed class TransitionPreset
{
    public uint Kind { get; set; } = MixerNative.TransitionFade;
    public uint DurationFrames { get; set; } = 30;
    public bool Swap { get; set; } = true;
    public string Label => Kind switch
    {
        0 => "Cut",
        2 => "Dip",
        _ => "Fade"
    };
}

public sealed class OverlaySlot
{
    public ulong SceneGpuId { get; set; }
    public float X { get; set; } = 0.62f;
    public float Y { get; set; } = 0.08f;
    public float Width { get; set; } = 0.32f;
    public float Height { get; set; } = 0.32f;
    public float Opacity { get; set; } = 1;
    public int Z { get; set; }
    public bool Enabled { get; set; } = true;
}

public sealed class MultiviewLayout
{
    public ulong Id { get; set; }
    public required string Name { get; set; }
    public ulong MonitorId { get; set; }
    public ulong PreviewUnitId { get; set; } = 1;
    public ulong ProgramUnitId { get; set; } = 1;
    public uint PresentInterval { get; set; }
    public MultiviewTemplate Template { get; set; } = MultiviewTemplate.PreviewProgram8;
    public List<MvSlot> Tiles { get; } = [];
    public bool PreviewLabelFollow { get; set; } = true;
    public string PreviewLabel { get; set; } = "";
    public bool ProgramLabelFollow { get; set; } = true;
    public string ProgramLabel { get; set; } = "";
    public ulong GpuId => MixerNative.MultiviewBase | Id;
    public override string ToString() => Name;

    public static uint ClampPresentInterval(uint frames) => Math.Clamp(frames, 1u, 8u);

    public uint ResolvedPresentInterval(SessionSettings settings)
    {
        if (PresentInterval == 0)
            return ClampPresentInterval(settings.DefaultPresentInterval == 0 ? 3 : settings.DefaultPresentInterval);
        return ClampPresentInterval(PresentInterval);
    }

    public void PushPresentInterval(SessionSettings settings) =>
        MixerNative.SetMonitorPresentInterval(MonitorId, ResolvedPresentInterval(settings));

    public void EnsureTiles()
    {
        var want = MultiviewGeometry.TileCount(Template);
        while (Tiles.Count < want)
            Tiles.Add(new MvSlot());
        while (Tiles.Count > want)
            Tiles.RemoveAt(Tiles.Count - 1);
        foreach (var tile in Tiles)
        {
            if (tile.Kind is MvSlotKind.MuPreview or MvSlotKind.MuProgram)
            {
                tile.Kind = MvSlotKind.None;
                tile.SourceId = 0;
            }
        }
    }
}

public sealed class MixingUnitEntry
{
    public ulong Id { get; set; }
    public required string Name { get; set; }
    public uint Width { get; set; } = 1920;
    public uint Height { get; set; } = 1080;
    public uint FpsNum { get; set; } = 60_000;
    public uint FpsDen { get; set; } = 1_001;
    public List<TransitionPreset> Transitions { get; } = [];
    public List<OverlaySlot> Overlays { get; } = [];
    public List<MvSlot> MultiviewTiles { get; } = [];
    public ulong AudioBusId { get; set; } = 1;
    public AudioLinkMode AudioLink { get; set; } = AudioLinkMode.Follow;
    public override string ToString() => $"{Name}  {Width}x{Height} {FormatFps()}";

    public string FormatFps()
    {
        if (FpsNum == 60_000 && FpsDen == 1_001)
            return "59.94p";
        if (FpsDen == 1)
            return $"{FpsNum}p";
        return $"{FpsNum}/{FpsDen}";
    }

    public uint DurationMs(uint frames) =>
        (uint)Math.Max(1, Math.Round(frames * 1000.0 * FpsDen / FpsNum));

    public void EnsureDefaultTransitions()
    {
        if (Transitions.Count > 0)
            return;
        Transitions.Add(new TransitionPreset { Kind = MixerNative.TransitionCut, DurationFrames = 1, Swap = true });
        Transitions.Add(new TransitionPreset { Kind = MixerNative.TransitionFade, DurationFrames = 30, Swap = true });
    }

    public void EnsureDefaultTiles()
    {
        if (MultiviewTiles.Count > 0)
            return;
        for (var i = 0; i < 8; i++)
            MultiviewTiles.Add(new MvSlot());
    }
}

public sealed class OutputEntry
{
    public ulong Id { get; set; }
    public required string Name { get; set; }
    public OutputTransport Transport { get; set; } = OutputTransport.Omt;
    public OutputSourceKind SourceKind { get; set; } = OutputSourceKind.MuProgram;
    public ulong SourceId { get; set; }
    public ulong UnitId { get; set; } = 1;
    public bool UseGpu { get; set; }
    public bool Enabled { get; set; } = true;
}

public enum InternalColorFormat
{
    Uyvy = 0,
    Bgra = 1
}

public enum AudioBusRole
{
    Master = 0,
    Headphone = 1,
    Aux = 2
}

public enum AudioDeviceKind
{
    None = 0,
    Wasapi = 1,
    Asio = 2,
    CoreAudio = 3
}

public enum AudioLinkMode
{
    Follow = 0,
    Independent = 1
}

public sealed class AudioBusEntry
{
    public ulong Id { get; set; }
    public required string Name { get; set; }
    public AudioBusRole Role { get; set; }
    public AudioDeviceKind DeviceKind { get; set; }
    public string DeviceId { get; set; } = "";
    public int MapLeft { get; set; }
    public int MapRight { get; set; } = 1;
    public bool Exclusive { get; set; }
    public uint Bit { get; set; }
    public float Gain { get; set; } = 1;
    public bool Mute { get; set; }
    public override string ToString() => Name;
}

public sealed class SessionSettings
{
    public uint MasterFpsNum { get; set; } = 60_000;
    public uint MasterFpsDen { get; set; } = 1_001;
    public uint DefaultWidth { get; set; } = 1920;
    public uint DefaultHeight { get; set; } = 1080;
    public string Theme { get; set; } = "Charcoal";
    public ulong DefaultMultiviewUnitId { get; set; } = 1;
    public uint FrameBufferFrames { get; set; } = 3;
    public uint DefaultPresentInterval { get; set; } = 3;
    public InternalColorFormat InternalColorFormat { get; set; } = InternalColorFormat.Uyvy;
    public bool? RebarOptimization { get; set; } = true;
    public bool? RebarDirectSample { get; set; } = false;
    public bool? NdiGpuUpload { get; set; } = true;
    public RgbColor PreviewColor { get; set; } = RgbColor.PreviewDefault;
    public RgbColor ProgramColor { get; set; } = RgbColor.ProgramDefault;
    public RgbColor InactiveColor { get; set; } = RgbColor.InactiveDefault;

    public bool RebarOptimizationEnabled => RebarOptimization != false;
    public bool NdiGpuUploadEnabled => NdiGpuUpload != false;
    public string? LastSessionPath { get; set; }

    public void ResetBusColors()
    {
        PreviewColor = RgbColor.PreviewDefault;
        ProgramColor = RgbColor.ProgramDefault;
        InactiveColor = RgbColor.InactiveDefault;
    }

    public uint ResolvedPresentInterval() =>
        MultiviewLayout.ClampPresentInterval(DefaultPresentInterval == 0 ? 3 : DefaultPresentInterval);
}

public sealed class RgbColor
{
    public byte R { get; set; }
    public byte G { get; set; }
    public byte B { get; set; }

    public static RgbColor PreviewDefault => new() { R = 0, G = 255, B = 0 };
    public static RgbColor ProgramDefault => new() { R = 255, G = 0, B = 0 };
    public static RgbColor InactiveDefault => new() { R = 64, G = 64, B = 64 };

    public RgbColor Clone() => new() { R = R, G = G, B = B };

    public static RgbColor FromOrDefault(RgbColor? color, RgbColor fallback) =>
        color is null ? fallback.Clone() : color.Clone();
}

public sealed class Session
{
    public SessionSettings Settings { get; } = new();
    public List<InputEntry> Inputs { get; } = [];
    public List<SceneEntry> Scenes { get; } = [];
    public List<MixingUnitEntry> Units { get; } = [];
    public List<OutputEntry> Outputs { get; } = [];
    public List<MultiviewLayout> Multiviews { get; } = [];
    public List<AudioBusEntry> Buses { get; } = [];
    public ulong NextInputId { get; set; } = 10;
    public ulong NextSceneId { get; set; } = 1;
    public ulong NextUnitId { get; set; } = 1;
    public ulong NextMonitorId { get; set; } = 1000;
    public ulong NextOutputId { get; set; } = 100;
    public ulong NextMultiviewId { get; set; } = 1;
    public ulong NextBusId { get; set; } = 3;
    public ulong SelectedUnitId { get; set; } = 1;
    public bool HeadphoneCopyMaster { get; set; }

    public static Session Default()
    {
        var session = new Session();
        session.EnsureDefaultBuses();
        session.Inputs.Add(new InputEntry { Id = MixerNative.Color, Name = "Color Red", Kind = InputKind.Color, ColorR = 1 });
        session.Inputs.Add(new InputEntry { Id = MixerNative.Bars, Name = "SMPTE HD Bars", Kind = InputKind.Bars, ToneHz = 1000 });
        session.Inputs.Add(new InputEntry { Id = MixerNative.Black, Name = "Black", Kind = InputKind.Black, ColorR = 0, ColorG = 0, ColorB = 0 });
        session.Inputs.Add(new InputEntry { Id = MixerNative.Blue, Name = "Blue", Kind = InputKind.Color, ColorR = 0, ColorG = 0, ColorB = 1 });
        var unit = new MixingUnitEntry { Id = 1, Name = "Mixing Unit 1", AudioBusId = 1, AudioLink = AudioLinkMode.Follow };
        unit.Transitions.Add(new TransitionPreset { Kind = MixerNative.TransitionCut, DurationFrames = 1, Swap = true });
        unit.Transitions.Add(new TransitionPreset { Kind = MixerNative.TransitionFade, DurationFrames = 30, Swap = true });
        unit.EnsureDefaultTiles();
        session.Units.Add(unit);
        session.NextUnitId = 2;
        session.AddScene("Scene 1", MixerNative.Bars);
        session.AddScene("Scene 2", MixerNative.Color);
        session.Outputs.Add(new OutputEntry
        {
            Id = session.NextOutputId++,
            Name = "eiviz-pgm",
            Transport = OutputTransport.Omt,
            SourceKind = OutputSourceKind.MuProgram,
            UnitId = 1,
            UseGpu = true
        });
        return session;
    }

    public SceneEntry AddScene(string name, ulong? fullInput = null)
    {
        var scene = new SceneEntry
        {
            Id = NextSceneId++,
            Name = name,
            MonitorId = NextMonitorId++
        };
        if (fullInput is ulong input)
        {
            scene.Layers.Add(new SceneLayer
            {
                InputId = input,
                Width = 1,
                Height = 1,
                Opacity = 1,
                Z = 0
            });
        }
        Scenes.Add(scene);
        return scene;
    }

    public MultiviewLayout AddMultiview(string? name = null)
    {
        var layout = new MultiviewLayout
        {
            Id = NextMultiviewId++,
            Name = name ?? $"Multiview {NextMultiviewId - 1}",
            MonitorId = NextMonitorId++,
            PreviewUnitId = Settings.DefaultMultiviewUnitId,
            ProgramUnitId = Settings.DefaultMultiviewUnitId
        };
        layout.EnsureTiles();
        Multiviews.Add(layout);
        return layout;
    }

    public void EnsureDefaultBuses()
    {
        if (Buses.All(bus => bus.Role != AudioBusRole.Master))
        {
            Buses.Insert(0, new AudioBusEntry
            {
                Id = 1,
                Name = "Master",
                Role = AudioBusRole.Master,
                DeviceKind = AudioDeviceKind.Wasapi,
                MapLeft = 0,
                MapRight = 1,
                Bit = 0
            });
        }
        if (Buses.All(bus => bus.Role != AudioBusRole.Headphone))
        {
            var insert = Buses.Count > 0 && Buses[0].Role == AudioBusRole.Master ? 1 : 0;
            Buses.Insert(insert, new AudioBusEntry
            {
                Id = 2,
                Name = "Headphone",
                Role = AudioBusRole.Headphone,
                DeviceKind = AudioDeviceKind.None,
                MapLeft = 0,
                MapRight = 1,
                Bit = 1
            });
        }
        if (NextBusId < 3)
            NextBusId = 3;
        foreach (var unit in Units)
        {
            if (unit.AudioBusId == 0)
                unit.AudioBusId = 1;
        }
    }

    public string NextAuxBusName()
    {
        for (var letter = 'A'; letter <= 'H'; letter++)
        {
            var name = $"Bus {letter}";
            if (Buses.TrueForAll(bus => bus.Name != name))
                return name;
        }
        return $"Bus {NextBusId}";
    }
}
