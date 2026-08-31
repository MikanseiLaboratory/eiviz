using System.Linq;
using System.Text.Json;
using System.Text.Json.Serialization;
using Eiviz.Host.Interop;

namespace Eiviz.Host;

internal static class SessionStore
{
    private static readonly JsonSerializerOptions Json = new()
    {
        WriteIndented = true,
        PropertyNamingPolicy = JsonNamingPolicy.CamelCase,
        Converters = { new JsonStringEnumConverter() }
    };

    public static void Save(Session session, string path)
    {
        session.Settings.LastSessionPath = path;
        var dto = Document.From(session);
        MixerNative.SessionSaveText(path, JsonSerializer.Serialize(dto, Json));
    }

    public static Session Load(string path)
    {
        var dto = JsonSerializer.Deserialize<Document>(MixerNative.SessionLoadText(path), Json)
            ?? throw new InvalidOperationException("Session file is empty.");
        var session = dto.ToSession();
        session.Settings.LastSessionPath = path;
        return session;
    }

    private sealed class Document
    {
        public int Version { get; set; } = 1;
        public SessionSettings Settings { get; set; } = new();
        public List<InputDto> Inputs { get; set; } = [];
        public List<SceneDto> Scenes { get; set; } = [];
        public List<UnitDto> Units { get; set; } = [];
        public List<OutputEntry> Outputs { get; set; } = [];
        public List<MultiviewDto> Multiviews { get; set; } = [];
        public List<AudioBusEntry> Buses { get; set; } = [];
        public ulong NextInputId { get; set; }
        public ulong NextSceneId { get; set; }
        public ulong NextUnitId { get; set; }
        public ulong NextOutputId { get; set; }
        public ulong NextMultiviewId { get; set; }
        public ulong NextBusId { get; set; }
        public ulong SelectedUnitId { get; set; }
        public bool HeadphoneCopyMaster { get; set; }

        public static Document From(Session session) => new()
        {
            Settings = session.Settings,
            Inputs = session.Inputs.Select(InputDto.From).ToList(),
            Scenes = session.Scenes.Select(SceneDto.From).ToList(),
            Units = session.Units.Select(UnitDto.From).ToList(),
            Outputs = session.Outputs.Select(output => new OutputEntry
            {
                Id = output.Id,
                Name = output.Name,
                Transport = output.Transport,
                SourceKind = output.SourceKind,
                SourceId = output.SourceId,
                UnitId = output.UnitId,
                UseGpu = output.UseGpu,
                Enabled = output.Enabled
            }).ToList(),
            Multiviews = session.Multiviews.Select(MultiviewDto.From).ToList(),
            Buses = session.Buses.Select(CloneBus).ToList(),
            NextInputId = session.NextInputId,
            NextSceneId = session.NextSceneId,
            NextUnitId = session.NextUnitId,
            NextOutputId = session.NextOutputId,
            NextMultiviewId = session.NextMultiviewId,
            NextBusId = session.NextBusId,
            SelectedUnitId = session.SelectedUnitId,
            HeadphoneCopyMaster = session.HeadphoneCopyMaster
        };

        public Session ToSession()
        {
            var session = new Session { SelectedUnitId = SelectedUnitId, HeadphoneCopyMaster = HeadphoneCopyMaster };
            session.Settings.MasterFpsNum = Settings.MasterFpsNum;
            session.Settings.MasterFpsDen = Settings.MasterFpsDen;
            session.Settings.DefaultWidth = Settings.DefaultWidth;
            session.Settings.DefaultHeight = Settings.DefaultHeight;
            session.Settings.Theme = Settings.Theme;
            session.Settings.DefaultMultiviewUnitId = Settings.DefaultMultiviewUnitId;
            session.Settings.FrameBufferFrames = Settings.FrameBufferFrames == 0 ? 3 : Math.Clamp(Settings.FrameBufferFrames, 1u, 8u);
            session.Settings.DefaultPresentInterval = Settings.DefaultPresentInterval == 0 ? 3 : Math.Clamp(Settings.DefaultPresentInterval, 1u, 8u);
            session.Settings.InternalColorFormat = Settings.InternalColorFormat;
            session.Settings.RebarOptimization = Settings.RebarOptimization != false;
            foreach (var input in Inputs)
                session.Inputs.Add(input.ToEntry());
            foreach (var scene in Scenes)
                session.Scenes.Add(scene.ToEntry(session));
            foreach (var unit in Units)
                session.Units.Add(unit.ToEntry());
            foreach (var output in Outputs)
                session.Outputs.Add(output);
            foreach (var layout in Multiviews)
                session.Multiviews.Add(layout.ToEntry(session));
            foreach (var bus in Buses)
                session.Buses.Add(CloneBus(bus));
            session.EnsureDefaultBuses();
            session.NextInputId = Math.Max(NextInputId, session.Inputs.Count == 0 ? 10 : session.Inputs.Max(item => item.Id) + 1);
            session.NextSceneId = Math.Max(NextSceneId, session.Scenes.Count == 0 ? 1 : session.Scenes.Max(item => item.Id) + 1);
            session.NextUnitId = Math.Max(NextUnitId, session.Units.Count == 0 ? 1 : session.Units.Max(item => item.Id) + 1);
            session.NextOutputId = Math.Max(NextOutputId, session.Outputs.Count == 0 ? 100 : session.Outputs.Max(item => item.Id) + 1);
            session.NextMultiviewId = Math.Max(NextMultiviewId, session.Multiviews.Count == 0 ? 1 : session.Multiviews.Max(item => item.Id) + 1);
            session.NextBusId = Math.Max(NextBusId, session.Buses.Count == 0 ? 3 : session.Buses.Max(item => item.Id) + 1);
            if (session.Units.Count == 0)
            {
                var unit = new MixingUnitEntry { Id = 1, Name = "Mixing Unit 1" };
                unit.EnsureDefaultTransitions();
                unit.EnsureDefaultTiles();
                session.Units.Add(unit);
                session.NextUnitId = 2;
            }
            if (session.Scenes.Count == 0)
                session.AddScene("Scene 1", MixerNative.Bars);
            return session;
        }
    }

    private sealed class InputDto
    {
        public ulong Id { get; set; }
        public string Name { get; set; } = "";
        public InputKind Kind { get; set; }
        public string? PathOrAddress { get; set; }
        public float ColorR { get; set; }
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

        public static InputDto From(InputEntry input) => new()
        {
            Id = input.Id,
            Name = input.Name,
            Kind = input.Kind,
            PathOrAddress = input.PathOrAddress,
            ColorR = input.ColorR,
            ColorG = input.ColorG,
            ColorB = input.ColorB,
            Scroll = input.Scroll,
            ToneHz = input.ToneHz,
            ToneLevelDbfs = input.ToneLevelDbfs,
            BusMask = input.BusMask == 0 ? 1u : input.BusMask,
            Gain = input.Gain,
            Mute = input.Mute,
            UseGpu = input.UseGpu,
            FrameBufferFrames = input.FrameBufferFrames == 0 ? 1 : Math.Clamp(input.FrameBufferFrames, 1u, 8u),
            BandwidthSave = input.BandwidthSave,
            KeepFullOnMultiview = input.KeepFullOnMultiview,
            OmtQuality = input.OmtQuality,
            NdiBandwidth = input.NdiBandwidth,
            VideoLoop = input.VideoLoop,
            VideoPlayWhen = input.VideoPlayWhen,
            VideoRestartWhen = input.VideoRestartWhen,
            VideoPauseWhen = input.VideoPauseWhen
        };

        public InputEntry ToEntry() => new()
        {
            Id = Id,
            Name = Name,
            Kind = Kind,
            PathOrAddress = PathOrAddress,
            ColorR = ColorR,
            ColorG = ColorG,
            ColorB = ColorB,
            Scroll = Scroll,
            ToneHz = ToneHz,
            ToneLevelDbfs = ToneLevelDbfs,
            BusMask = BusMask == 0 ? 1u : BusMask,
            Gain = MixerNative.MixerGain(Gain),
            Mute = Mute,
            UseGpu = UseGpu,
            FrameBufferFrames = FrameBufferFrames == 0 ? 1 : Math.Clamp(FrameBufferFrames, 1u, 8u),
            BandwidthSave = BandwidthSave,
            KeepFullOnMultiview = KeepFullOnMultiview,
            OmtQuality = OmtQuality,
            NdiBandwidth = NdiBandwidth,
            VideoLoop = VideoLoop,
            VideoPlayWhen = VideoPlayWhen,
            VideoRestartWhen = VideoRestartWhen,
            VideoPauseWhen = VideoPauseWhen
        };
    }

    private sealed class SceneDto
    {
        public ulong Id { get; set; }
        public string Name { get; set; } = "";
        public List<SceneLayer> Layers { get; set; } = [];

        public static SceneDto From(SceneEntry scene) => new()
        {
            Id = scene.Id,
            Name = scene.Name,
            Layers = [.. scene.Layers]
        };

        public SceneEntry ToEntry(Session session)
        {
            var scene = new SceneEntry
            {
                Id = Id,
                Name = Name,
                MonitorId = session.NextMonitorId++
            };
            foreach (var layer in Layers)
                scene.Layers.Add(layer);
            return scene;
        }
    }

    private sealed class UnitDto
    {
        public ulong Id { get; set; }
        public string Name { get; set; } = "";
        public uint Width { get; set; }
        public uint Height { get; set; }
        public uint FpsNum { get; set; }
        public uint FpsDen { get; set; }
        public List<TransitionPreset> Transitions { get; set; } = [];
        public List<OverlaySlot> Overlays { get; set; } = [];
        public List<MvSlot> MultiviewTiles { get; set; } = [];
        public ulong AudioBusId { get; set; } = 1;
        public AudioLinkMode AudioLink { get; set; } = AudioLinkMode.Follow;

        public static UnitDto From(MixingUnitEntry unit) => new()
        {
            Id = unit.Id,
            Name = unit.Name,
            Width = unit.Width,
            Height = unit.Height,
            FpsNum = unit.FpsNum,
            FpsDen = unit.FpsDen,
            Transitions = [.. unit.Transitions],
            Overlays = [.. unit.Overlays],
            MultiviewTiles = [.. unit.MultiviewTiles],
            AudioBusId = unit.AudioBusId == 0 ? 1 : unit.AudioBusId,
            AudioLink = unit.AudioLink
        };

        public MixingUnitEntry ToEntry()
        {
            var unit = new MixingUnitEntry
            {
                Id = Id,
                Name = Name,
                Width = Width == 0 ? 1920 : Width,
                Height = Height == 0 ? 1080 : Height,
                FpsNum = FpsNum == 0 ? 60_000 : FpsNum,
                FpsDen = FpsDen == 0 ? 1_001 : FpsDen,
                AudioBusId = AudioBusId == 0 ? 1 : AudioBusId,
                AudioLink = AudioLink
            };
            foreach (var preset in Transitions)
                unit.Transitions.Add(preset);
            foreach (var overlay in Overlays)
                unit.Overlays.Add(overlay);
            foreach (var tile in MultiviewTiles)
                unit.MultiviewTiles.Add(tile);
            unit.EnsureDefaultTransitions();
            unit.EnsureDefaultTiles();
            return unit;
        }
    }

    private sealed class MultiviewDto
    {
        public ulong Id { get; set; }
        public string Name { get; set; } = "";
        public ulong PreviewUnitId { get; set; }
        public ulong ProgramUnitId { get; set; }
        public uint PresentInterval { get; set; }
        public List<MvSlot> Tiles { get; set; } = [];

        public static MultiviewDto From(MultiviewLayout layout) => new()
        {
            Id = layout.Id,
            Name = layout.Name,
            PreviewUnitId = layout.PreviewUnitId,
            ProgramUnitId = layout.ProgramUnitId,
            PresentInterval = layout.PresentInterval == 0 ? 0 : MultiviewLayout.ClampPresentInterval(layout.PresentInterval),
            Tiles = [.. layout.Tiles]
        };

        public MultiviewLayout ToEntry(Session session)
        {
            var layout = new MultiviewLayout
            {
                Id = Id,
                Name = string.IsNullOrWhiteSpace(Name) ? $"Multiview {Id}" : Name,
                MonitorId = session.NextMonitorId++,
                PreviewUnitId = PreviewUnitId == 0 ? session.Settings.DefaultMultiviewUnitId : PreviewUnitId,
                ProgramUnitId = ProgramUnitId == 0 ? session.Settings.DefaultMultiviewUnitId : ProgramUnitId,
                PresentInterval = PresentInterval == 0 ? 0 : MultiviewLayout.ClampPresentInterval(PresentInterval)
            };
            foreach (var tile in Tiles)
                layout.Tiles.Add(tile);
            layout.EnsureTiles();
            return layout;
        }
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
        Exclusive = bus.Exclusive,
        Bit = bus.Bit,
        Gain = MixerNative.MixerGain(bus.Gain),
        Mute = bus.Mute
    };
}
