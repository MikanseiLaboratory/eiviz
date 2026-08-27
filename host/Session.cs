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

public sealed class MvSlot
{
    public MvSlotKind Kind { get; set; }
    public ulong SourceId { get; set; }
}

public sealed class InputEntry
{
    public ulong Id { get; init; }
    public required string Name { get; set; }
    public InputKind Kind { get; init; }
    public string? PathOrAddress { get; set; }
    public float ColorR { get; set; } = 1;
    public float ColorG { get; set; }
    public float ColorB { get; set; }
    public bool Scroll { get; set; }
    public uint BusMask { get; set; } = 1;
    public float Gain { get; set; } = 1;
    public bool Mute { get; set; }
    public bool UseGpu { get; set; }
    public uint FrameBufferFrames { get; set; } = 1;
    public override string ToString() => Name;
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
    public List<MvSlot> Tiles { get; } = [];
    public ulong GpuId => MixerNative.MultiviewBase | Id;
    public override string ToString() => Name;

    public void EnsureTiles()
    {
        while (Tiles.Count < 8)
            Tiles.Add(new MvSlot());
        while (Tiles.Count > 8)
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
    Asio = 2
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
    public InternalColorFormat InternalColorFormat { get; set; } = InternalColorFormat.Uyvy;
    public string? LastSessionPath { get; set; }
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
        session.Inputs.Add(new InputEntry { Id = MixerNative.Color, Name = "Color Red", Kind = InputKind.Color });
        session.Inputs.Add(new InputEntry { Id = MixerNative.Bars, Name = "SMPTE Bars", Kind = InputKind.Bars });
        session.Inputs.Add(new InputEntry { Id = MixerNative.Black, Name = "Black", Kind = InputKind.Black });
        session.Inputs.Add(new InputEntry { Id = MixerNative.Blue, Name = "Blue", Kind = InputKind.Color });
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
