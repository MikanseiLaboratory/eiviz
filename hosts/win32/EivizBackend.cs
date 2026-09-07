using System.IO;
using System.Text.Json;
using System.Text.Json.Serialization;
using System.Windows;
using Eiviz.Host.Interop;
using Eiviz.Host.Preview;

namespace Eiviz.Host;

internal enum HostConnectionMode
{
    Local,
    Remote
}

internal readonly record struct PublishedVideoOutput(
    ulong Id,
    string Name,
    OutputTransport Transport,
    OutputSourceKind SourceKind,
    ulong UnitId,
    ulong SourceId);

internal interface IEivizBackend
{
    bool IsRemote { get; }
    bool Connected { get; }
    bool CanPreviewInputs { get; }
    bool CanShowSceneThumbs { get; }
    ulong Revision { get; }
    string StatusText { get; }
    event Action? Changed;
    void Poll();
    bool Cut(ulong unitId, bool swap);
    bool Preview(ulong unitId, ulong sceneGpuId);
    bool Auto(ulong unitId, MixingUnitEntry unit, TransitionPreset preset);
    bool SetMix(ulong unitId, float mix, TransitionPreset? preset);
    bool OverlayAuto(ulong unitId, uint index, uint durationMs, bool toOn);
    bool VideoPlay(ulong inputId, bool playing);
    bool VideoLoop(ulong inputId, bool looping);
    bool VideoSeek(ulong inputId, long positionHns);
    bool Mutate(string json, ulong expectedRevision, out string error);
    bool UploadMedia(string path, string kind, string name, bool videoLoop, ulong expectedRevision, out string error);
    bool TryGetMix(ulong unitId, out float mix);
    void BusSources(ulong unitId, out ulong previewGpuId, out ulong programGpuId);
    IReadOnlyList<PublishedVideoOutput> PublishedOutputs();
    void BindPreviewProgram(SwapchainHost preview, SwapchainHost program, ulong unitId);
    void BindMultiview(SwapchainHost host, MultiviewLayout layout);
    void SyncPublishedVideo();
    void Dispose();
}

internal static class MutationJson
{
    internal static readonly JsonSerializerOptions Json = new()
    {
        PropertyNamingPolicy = JsonNamingPolicy.CamelCase,
        DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull,
        Converters = { new InputKindJsonConverter(), new JsonStringEnumConverter(JsonNamingPolicy.CamelCase) }
    };

    private static readonly JsonSerializerOptions DocumentJson = new()
    {
        PropertyNamingPolicy = JsonNamingPolicy.CamelCase,
        DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull,
        Converters = { new InputKindJsonConverter(), new JsonStringEnumConverter() }
    };

    public static string SetSceneLayers(ulong sceneId, IEnumerable<SceneLayer> layers) =>
        JsonSerializer.Serialize(new { kind = "setSceneLayers", sceneId, layers }, Json);

    public static string UpsertScene(SceneEntry scene) =>
        JsonSerializer.Serialize(new { kind = "upsertScene", scene = SceneDto.FromPublic(scene) }, Json);

    public static string SetOverlaySlot(ulong unitId, uint index, OverlaySlot slot) =>
        JsonSerializer.Serialize(new
        {
            kind = "setOverlaySlot",
            unitId,
            index,
            slot = new
            {
                sceneGpuId = slot.SceneGpuId,
                x = slot.X,
                y = slot.Y,
                width = slot.Width,
                height = slot.Height,
                opacity = slot.Opacity,
                z = slot.Z,
                enabled = slot.Enabled,
                transitionKind = slot.TransitionKind,
                durationValue = slot.DurationValue,
                durationUnit = slot.DurationUnit,
                audioFollow = slot.AudioFollow,
                sourceKind = slot.SourceKind == OverlaySourceKind.Input ? 1u : 0u,
                locked = slot.Locked,
                hidden = slot.Hidden
            }
        }, Json);

    public static string UpsertInput(InputEntry input) =>
        JsonSerializer.Serialize(new { kind = "upsertInput", input = InputWire.From(input) }, Json);

    public static string DeleteInput(ulong id) =>
        JsonSerializer.Serialize(new { kind = "deleteInput", id }, Json);

    public static string DeleteScene(ulong id) =>
        JsonSerializer.Serialize(new { kind = "deleteScene", id }, Json);

    public static string UpsertMultiview(MultiviewLayout layout)
    {
        layout.EnsureTiles();
        return JsonSerializer.Serialize(new
        {
            kind = "upsertMultiview",
            layout = new
            {
                id = layout.Id,
                name = layout.Name,
                previewUnitId = layout.PreviewUnitId,
                programUnitId = layout.ProgramUnitId,
                presentInterval = layout.PresentInterval,
                tiles = layout.Tiles.Select(tile => new
                {
                    kind = tile.Kind.ToString(),
                    sourceId = tile.SourceId,
                    labelFollow = tile.LabelFollow,
                    label = tile.Label ?? ""
                }),
                template = layout.Template.ToString(),
                previewLabelFollow = layout.PreviewLabelFollow,
                previewLabel = layout.PreviewLabel ?? "",
                programLabelFollow = layout.ProgramLabelFollow,
                programLabel = layout.ProgramLabel ?? "",
                labelAnchor = layout.LabelAnchor?.ToString(),
                labelSize = layout.LabelSize,
                labelUnit = layout.LabelUnit?.ToString(),
                alwaysOnTop = layout.AlwaysOnTop
            }
        }, Json);
    }

    public static string DeleteMultiview(ulong id) =>
        JsonSerializer.Serialize(new { kind = "deleteMultiview", id }, Json);

    public static string SetSettings(
        SessionSettings settings,
        IEnumerable<OutputEntry> outputs,
        IEnumerable<AudioBusEntry> buses,
        bool headphoneCopyMaster,
        ulong nextOutputId,
        ulong nextBusId) =>
        JsonSerializer.Serialize(new
        {
            kind = "setSettings",
            settings,
            outputs,
            buses,
            headphoneCopyMaster,
            nextOutputId,
            nextBusId
        }, DocumentJson);

    private sealed class InputWire
    {
        public ulong Id { get; set; }
        public string Guid { get; set; } = "";
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
        public BandwidthSave BandwidthSave { get; set; }
        public bool KeepFullOnMultiview { get; set; }
        public OmtQuality OmtQuality { get; set; }
        public NdiBandwidth NdiBandwidth { get; set; }
        public bool VideoLoop { get; set; } = true;
        public VideoPlayWhen VideoPlayWhen { get; set; }
        public VideoTriggerWhen VideoRestartWhen { get; set; }
        public VideoTriggerWhen VideoPauseWhen { get; set; }
        public uint CaptureWidth { get; set; }
        public uint CaptureHeight { get; set; }
        public uint CaptureFpsNum { get; set; }
        public uint CaptureFpsDen { get; set; }
        public List<string> Tags { get; set; } = [];
        public MixSource MixSource { get; set; }
        public ulong MixTargetId { get; set; }
        public ulong MixAudioBusId { get; set; }

        public static InputWire From(InputEntry input) => new()
        {
            Id = input.Id,
            Guid = input.Guid,
            Name = input.Name,
            Kind = input.Kind,
            PathOrAddress = input.PathOrAddress,
            ColorR = input.ColorR,
            ColorG = input.ColorG,
            ColorB = input.ColorB,
            Scroll = input.Scroll,
            ToneHz = input.ToneHz,
            ToneLevelDbfs = input.ToneLevelDbfs,
            BusMask = input.BusMask,
            Gain = input.Gain,
            Mute = input.Mute,
            UseGpu = input.UseGpu,
            FrameBufferFrames = input.FrameBufferFrames,
            BandwidthSave = input.BandwidthSave,
            KeepFullOnMultiview = input.KeepFullOnMultiview,
            OmtQuality = input.OmtQuality,
            NdiBandwidth = input.NdiBandwidth,
            VideoLoop = input.VideoLoop,
            VideoPlayWhen = input.VideoPlayWhen,
            VideoRestartWhen = input.VideoRestartWhen,
            VideoPauseWhen = input.VideoPauseWhen,
            CaptureWidth = input.CaptureWidth,
            CaptureHeight = input.CaptureHeight,
            CaptureFpsNum = input.CaptureFpsNum,
            CaptureFpsDen = input.CaptureFpsDen,
            Tags = [.. input.Tags],
            MixSource = input.MixSource,
            MixTargetId = input.MixTargetId,
            MixAudioBusId = input.MixAudioBusId
        };
    }

    private sealed class SceneDto
    {
        public ulong Id { get; set; }
        public string Name { get; set; } = "";
        public List<SceneLayer> Layers { get; set; } = [];
        public List<string> Tags { get; set; } = [];

        public static SceneDto FromPublic(SceneEntry scene) => new()
        {
            Id = scene.Id,
            Name = scene.Name,
            Layers = [.. scene.Layers],
            Tags = [.. scene.Tags]
        };
    }
}

internal sealed class LocalEivizBackend : IEivizBackend
{
    private ulong _after;
    public bool IsRemote => false;
    public bool Connected => true;
    public bool CanPreviewInputs => true;
    public bool CanShowSceneThumbs => true;
    public ulong Revision { get; private set; }
    public string StatusText => "";
    public event Action? Changed;

    public void Poll()
    {
        var text = MixerNative.PollEventsText(_after);
        if (string.IsNullOrEmpty(text))
            return;
        try
        {
            using var doc = JsonDocument.Parse(text);
            if (!doc.RootElement.TryGetProperty("events", out var events))
                return;
            var changed = false;
            foreach (var item in events.EnumerateArray())
            {
                if (item.TryGetProperty("sequence", out var seq))
                    _after = Math.Max(_after, seq.GetUInt64());
                if (item.TryGetProperty("revision", out var rev))
                    Revision = Math.Max(Revision, rev.GetUInt64());
                var kind = item.TryGetProperty("kind", out var k) ? k.GetString() : "";
                if (kind is "SessionChanged" or "Lag")
                    changed = true;
            }
            if (!changed || Application.Current is not App app)
                return;
            var json = MixerNative.SnapshotText();
            if (string.IsNullOrEmpty(json))
                return;
            var next = SessionStore.FromJson(json);
            next.SelectedUnitId = app.Session.SelectedUnitId;
            app.ReplaceDocument(next);
            Changed?.Invoke();
        }
        catch
        {
            // Keep the last good document when event JSON is truncated.
        }
    }

    public bool Cut(ulong unitId, bool swap) => MixerApply.CutLocal(unitId, swap);
    public bool Preview(ulong unitId, ulong sceneGpuId) => MixerApply.PreviewLocal(unitId, sceneGpuId);
    public bool Auto(ulong unitId, MixingUnitEntry unit, TransitionPreset preset) =>
        MixerApply.AutoLocal(unitId, unit, preset);
    public bool SetMix(ulong unitId, float mix, TransitionPreset? preset) =>
        MixerApply.SetMixLocal(unitId, mix, preset);

    public bool OverlayAuto(ulong unitId, uint index, uint durationMs, bool toOn)
    {
        if (Application.Current is not App app)
            return false;
        var unit = app.Session.Units.FirstOrDefault(item => item.Id == unitId);
        if (unit is null || index >= unit.Overlays.Count)
            return false;
        var slot = unit.Overlays[(int)index];
        unsafe
        {
            var desc = OverlayDescFrom(slot);
            return MixerNative.OverlayAuto(unitId, toOn ? 1u : 0u, durationMs, &desc) == 0;
        }
    }

    public bool VideoPlay(ulong inputId, bool playing) =>
        MixerNative.VideoSetPlaying(inputId, playing ? 1u : 0u) == 0;

    public bool VideoLoop(ulong inputId, bool looping) =>
        MixerNative.VideoSetLoop(inputId, looping ? 1u : 0u) == 0;

    public bool VideoSeek(ulong inputId, long positionHns) =>
        MixerNative.VideoSeek(inputId, positionHns) == 0;

    public bool Mutate(string json, ulong expectedRevision, out string error)
    {
        error = "";
        try
        {
            MixerNative.SessionReplaceText(json, expectedRevision);
            return true;
        }
        catch (Exception ex)
        {
            error = ex.Message;
            return false;
        }
    }

    public bool UploadMedia(string path, string kind, string name, bool videoLoop, ulong expectedRevision, out string error)
    {
        error = "";
        _ = (kind, name, videoLoop, expectedRevision);
        return File.Exists(path);
    }

    public bool TryGetMix(ulong unitId, out float mix)
    {
        unsafe
        {
            UnitState state = default;
            if (MixerNative.GetUnitState(unitId, &state) != 0)
            {
                mix = 0;
                return false;
            }
            mix = state.Mix;
            return true;
        }
    }

    public void BusSources(ulong unitId, out ulong previewGpuId, out ulong programGpuId)
    {
        unsafe
        {
            UnitState state = default;
            if (MixerNative.GetUnitState(unitId, &state) == 0)
            {
                previewGpuId = state.PreviewSource;
                programGpuId = state.ProgramSource;
                return;
            }
        }
        previewGpuId = 0;
        programGpuId = 0;
    }

    public IReadOnlyList<PublishedVideoOutput> PublishedOutputs() =>
        Application.Current is App app ? HostPresentation.From(app.Session) : [];

    public void BindPreviewProgram(SwapchainHost preview, SwapchainHost program, ulong unitId)
    {
        preview.RetargetUnit(unitId, MixerNative.OutputPreview);
        program.RetargetUnit(unitId, MixerNative.OutputProgram);
    }

    public void BindMultiview(SwapchainHost host, MultiviewLayout layout) =>
        host.RetargetMonitor(layout.MonitorId, layout.GpuId);

    public void SyncPublishedVideo() { }
    public void Dispose() { }

    private static OverlayDesc OverlayDescFrom(OverlaySlot slot) => new()
    {
        SourceId = slot.SceneGpuId,
        Rect = new Interop.Rect { X = slot.X, Y = slot.Y, Width = slot.Width, Height = slot.Height },
        Crop = new Interop.Rect { X = slot.CropX, Y = slot.CropY, Width = slot.CropWidth, Height = slot.CropHeight },
        Opacity = slot.Opacity,
        Z = slot.Z,
        AudioFollow = slot.AudioFollow ? 1u : 0u,
        Hidden = slot.Hidden ? 1u : 0u
    };
}

internal sealed class RemoteEivizBackend : IEivizBackend
{
    private readonly int _handle;
    private readonly RemoteVideoPresenter _presenter = new();
    private string _status = "";
    private ulong _seenDocumentRevision;
    private ulong _seenSequence;
    private string _epoch = "";
    private bool _connected;
    private readonly Dictionary<ulong, (ulong Preview, ulong Program)> _buses = [];
    private readonly Dictionary<ulong, float> _mix = [];
    private byte[] _statusBuf = new byte[4096];
    private byte[] _liveBuf = new byte[1 << 16];
    private byte[] _snapBuf = new byte[1 << 20];
    private string _lastRemoteError = "";

    public RemoteEivizBackend(int handle) => _handle = handle;
    public bool IsRemote => true;
    public bool Connected => _connected;
    public bool CanPreviewInputs => false;
    public bool CanShowSceneThumbs => false;
    public ulong Revision { get; private set; }
    public string StatusText => _status;
    public bool RemotePreviewOk => _presenter.PreviewOk;
    public bool RemoteProgramOk => _presenter.ProgramOk;
    public event Action? Changed;

    public static RemoteEivizBackend Open(string url, string token)
    {
        var handle = MixerRemote.Open(url, token ?? "");
        if (handle <= 0)
            throw new InvalidOperationException(I18n.Loc.T("msg.remoteConnectFailed"));
        var backend = new RemoteEivizBackend(handle);
        backend.Pull(force: true);
        return backend;
    }

    public void Poll() => Pull(force: false);

    private void Pull(bool force)
    {
        var statusJson = MixerRemote.StatusText(_handle, ref _statusBuf);
        var connected = false;
        var lag = false;
        var error = "";
        var epoch = _epoch;
        var documentRevision = Revision;
        var sequence = _seenSequence;
        try
        {
            using var status = JsonDocument.Parse(string.IsNullOrEmpty(statusJson) ? "{}" : statusJson);
            connected = status.RootElement.TryGetProperty("connected", out var c) && c.GetBoolean();
            lag = status.RootElement.TryGetProperty("lag", out var l) && l.GetBoolean();
            error = status.RootElement.TryGetProperty("error", out var e) ? e.GetString() ?? "" : "";
            _lastRemoteError = error;
            if (status.RootElement.TryGetProperty("revision", out var r))
                Revision = r.GetUInt64();
            documentRevision = status.RootElement.TryGetProperty("documentRevision", out var dr)
                ? dr.GetUInt64()
                : Revision;
            if (documentRevision != 0)
                Revision = documentRevision;
            if (status.RootElement.TryGetProperty("sequence", out var seq))
                sequence = seq.GetUInt64();
            epoch = status.RootElement.TryGetProperty("epoch", out var ep) ? ep.GetString() ?? "" : epoch;
        }
        catch
        {
            connected = false;
        }
        _connected = connected;
        if (!connected)
            _status = I18n.Loc.T("msg.remoteDisconnected");
        else if (!string.IsNullOrEmpty(error))
            _status = string.IsNullOrEmpty(error) ? I18n.Loc.T("msg.remoteAuthFailed") : error;
        else if (lag)
            _status = I18n.Loc.T("msg.remoteResync");
        else
            _status = I18n.Loc.Format("msg.remoteConnected", Revision);

        if (force || sequence != _seenSequence)
        {
            PullLive();
            _seenSequence = sequence;
        }
        var docChanged = force || documentRevision != _seenDocumentRevision || epoch != _epoch;
        if (!docChanged || Application.Current is not App app)
            return;
        var json = MixerRemote.SnapshotText(_handle, ref _snapBuf);
        if (string.IsNullOrEmpty(json))
            return;
        var next = SessionStore.FromJson(json);
        next.SelectedUnitId = app.Session.SelectedUnitId;
        _seenDocumentRevision = documentRevision;
        _epoch = epoch;
        app.ReplaceDocument(next);
        SyncPublishedVideo();
        Changed?.Invoke();
    }

    public bool Cut(ulong unitId, bool swap) =>
        MixerRemote.Cut(_handle, unitId, swap ? 1u : 0u) == 0;

    public bool Preview(ulong unitId, ulong sceneGpuId) =>
        MixerRemote.Preview(_handle, unitId, sceneGpuId) == 0;

    public bool Auto(ulong unitId, MixingUnitEntry unit, TransitionPreset preset)
    {
        var code = MixerRemote.Auto(
            _handle, unitId, preset.Kind, preset.DurationMsFor(unit),
            preset.Swap ? 1u : 0u, preset.KeepPreview ? 1u : 0u,
            preset.Easing, preset.Direction, preset.DipR, preset.DipG, preset.DipB,
            preset.DipA <= 0 ? 1 : preset.DipA, preset.Softness, preset.Param);
        return code == 0;
    }

    public bool SetMix(ulong unitId, float mix, TransitionPreset? preset)
    {
        _ = preset;
        _mix[unitId] = mix;
        return MixerRemote.SetMix(_handle, unitId, mix) == 0;
    }

    public bool OverlayAuto(ulong unitId, uint index, uint durationMs, bool toOn) =>
        MixerRemote.OverlayAuto(_handle, unitId, index, durationMs, toOn ? 1u : 0u) == 0;

    public bool VideoPlay(ulong inputId, bool playing) =>
        MixerRemote.VideoPlay(_handle, inputId, playing ? 1u : 0u) == 0;

    public bool VideoLoop(ulong inputId, bool looping) =>
        MixerRemote.VideoLoop(_handle, inputId, looping ? 1u : 0u) == 0;

    public bool VideoSeek(ulong inputId, long positionHns) =>
        MixerRemote.VideoSeek(_handle, inputId, positionHns) == 0;

    public bool Mutate(string json, ulong expectedRevision, out string error)
    {
        var code = MixerRemote.MutateCode(_handle, json, expectedRevision);
        Pull(force: true);
        if (code == 0)
        {
            error = "";
            return true;
        }
        error = MutateError(code);
        return false;
    }

    private string MutateError(int code)
    {
        if (!string.IsNullOrEmpty(_lastRemoteError))
            return _lastRemoteError;
        return code == -3
            ? I18n.Loc.T("msg.revisionConflict")
            : I18n.Loc.Error("Mutate session", code < 0 ? -code : code);
    }

    public bool UploadMedia(string path, string kind, string name, bool videoLoop, ulong expectedRevision, out string error)
    {
        var code = MixerRemote.Upload(_handle, path, kind, name, videoLoop ? 1u : 0u, expectedRevision);
        if (code == 0)
        {
            error = "";
            Pull(force: true);
            return true;
        }
        error = I18n.Loc.T("msg.uploadFailed");
        return false;
    }

    public bool TryGetMix(ulong unitId, out float mix) =>
        _mix.TryGetValue(unitId, out mix);

    public void BusSources(ulong unitId, out ulong previewGpuId, out ulong programGpuId)
    {
        if (_buses.TryGetValue(unitId, out var pair))
        {
            previewGpuId = pair.Preview;
            programGpuId = pair.Program;
            return;
        }
        previewGpuId = 0;
        programGpuId = 0;
    }

    private void PullLive()
    {
        try
        {
            var json = MixerRemote.LiveText(_handle, ref _liveBuf);
            if (string.IsNullOrEmpty(json))
                return;
            using var doc = JsonDocument.Parse(json);
            if (!doc.RootElement.TryGetProperty("units", out var units))
                return;
            foreach (var unit in units.EnumerateObject())
            {
                if (!ulong.TryParse(unit.Name, out var id))
                    continue;
                var preview = unit.Value.TryGetProperty("previewSource", out var previewEl)
                    ? previewEl.GetUInt64()
                    : 0;
                var program = unit.Value.TryGetProperty("programSource", out var programEl)
                    ? programEl.GetUInt64()
                    : 0;
                _buses[id] = (preview, program);
                if (unit.Value.TryGetProperty("mix", out var mixEl))
                    _mix[id] = mixEl.GetSingle();
            }
        }
        catch
        {
        }
    }

    public IReadOnlyList<PublishedVideoOutput> PublishedOutputs() =>
        Application.Current is App app ? HostPresentation.From(app.Session) : [];

    public void BindPreviewProgram(SwapchainHost preview, SwapchainHost program, ulong unitId) =>
        _presenter.Bind(preview, program, RemoteVideoCatalog.FromPrefs(true), RemoteVideoCatalog.FromPrefs(false));

    public void BindMultiview(SwapchainHost host, MultiviewLayout layout) =>
        _presenter.BindMultiview(host, layout, PublishedOutputs());

    public void SyncPublishedVideo() => _presenter.Sync(PublishedOutputs());

    public void Dispose()
    {
        _presenter.Release();
        MixerRemote.Close(_handle);
    }
}

internal static class HostPresentation
{
    public static List<PublishedVideoOutput> From(Session session) =>
        session.Outputs
            .Where(output =>
                output.Enabled
                && output.Transport is OutputTransport.Omt or OutputTransport.Ndi
                && output.SourceKind is OutputSourceKind.MuPreview or OutputSourceKind.MuProgram or OutputSourceKind.Multiview)
            .Select(output => new PublishedVideoOutput(
                output.Id, output.Name, output.Transport, output.SourceKind, output.UnitId, output.SourceId))
            .ToList();
}

internal sealed class RemoteVideoPresenter
{
    private const ulong PreviewSourceId = 0x0005_0001;
    private const ulong ProgramSourceId = 0x0005_0002;
    private const ulong PreviewMonitor = 0x0006_0001;
    private const ulong ProgramMonitor = 0x0006_0002;
    private const ulong MultiviewMonitor = 0x0006_0003;
    private const ulong MultiviewSourceBase = 0x0005_0100;
    private readonly HashSet<ulong> _connected = [];
    public bool PreviewOk { get; private set; }
    public bool ProgramOk { get; private set; }

    public void Bind(SwapchainHost preview, SwapchainHost program, RemoteVideoChoice previewChoice, RemoteVideoChoice programChoice)
    {
        PreviewOk = Connect(PreviewSourceId, previewChoice);
        ProgramOk = Connect(ProgramSourceId, programChoice);
        BindOne(preview, PreviewMonitor, PreviewOk ? PreviewSourceId : null);
        BindOne(program, ProgramMonitor, ProgramOk ? ProgramSourceId : null);
    }

    public void BindMultiview(SwapchainHost host, MultiviewLayout layout, IReadOnlyList<PublishedVideoOutput> outputs)
    {
        SyncMultiview(outputs);
        BindOne(host, MultiviewMonitor | (layout.Id << 8), SourceForMultiview(outputs, layout.GpuId));
    }

    public void Sync(IReadOnlyList<PublishedVideoOutput> outputs) => SyncMultiview(outputs);

    private void SyncMultiview(IReadOnlyList<PublishedVideoOutput> outputs)
    {
        var keep = new HashSet<ulong> { PreviewSourceId, ProgramSourceId };
        foreach (var output in outputs.Where(item => item.SourceKind == OutputSourceKind.Multiview))
        {
            var id = MultiviewSourceBase | output.Id;
            keep.Add(id);
            if (_connected.Contains(id))
                continue;
            if (Connect(id, new RemoteVideoChoice(output.Transport, output.Name)))
                _connected.Add(id);
        }
        foreach (var id in _connected.ToArray())
        {
            if (keep.Contains(id))
                continue;
            MixerNative.DestroySource(id);
            _connected.Remove(id);
        }
    }

    private static ulong? SourceForMultiview(IReadOnlyList<PublishedVideoOutput> outputs, ulong layoutGpuId)
    {
        var matches = outputs.Where(item => item.SourceKind == OutputSourceKind.Multiview && item.SourceId == layoutGpuId).ToList();
        if (matches.Count != 1)
            return null;
        return MultiviewSourceBase | matches[0].Id;
    }

    private bool Connect(ulong id, RemoteVideoChoice choice)
    {
        MixerNative.DestroySource(id);
        _connected.Remove(id);
        if (choice.IsEmpty)
            return false;
        var code = choice.Transport == OutputTransport.Ndi
            ? MixerNative.ConnectNdi(id, choice.Address, 3, 0)
            : MixerNative.ConnectOmt(id, choice.Address, 1, 3, 0);
        if (code != 0)
            return false;
        _connected.Add(id);
        return true;
    }

    private void BindOne(SwapchainHost host, ulong monitorId, ulong? sourceId)
    {
        if (sourceId is ulong id)
            host.RetargetMonitor(monitorId, id);
        else
        {
            host.ReleaseNative();
            host.IsMonitor = true;
            host.MonitorId = 0;
            host.SourceId = 0;
        }
    }

    public void Release()
    {
        foreach (var id in _connected.ToArray())
            MixerNative.DestroySource(id);
        _connected.Clear();
        MixerNative.DestroySource(PreviewSourceId);
        MixerNative.DestroySource(ProgramSourceId);
        PreviewOk = false;
        ProgramOk = false;
    }
}

internal readonly record struct RemoteVideoChoice(OutputTransport Transport, string Address)
{
    public bool IsEmpty => string.IsNullOrWhiteSpace(Address);
}

internal sealed class RemoteVideoItem
{
    public required RemoteVideoChoice Choice { get; init; }
    public required string Label { get; init; }
    public override string ToString() => Label;
}

internal static class RemoteVideoCatalog
{
    public static List<RemoteVideoItem> List(Session session)
    {
        var items = new List<RemoteVideoItem>
        {
            new() { Choice = new RemoteVideoChoice(OutputTransport.Omt, ""), Label = I18n.Loc.T("chrome.videoNone") }
        };
        var seen = new HashSet<string>(StringComparer.Ordinal);
        void Add(OutputTransport transport, string address)
        {
            if (string.IsNullOrWhiteSpace(address))
                return;
            var key = $"{transport}:{address}";
            if (!seen.Add(key))
                return;
            var prefix = transport == OutputTransport.Ndi ? "NDI" : "OMT";
            items.Add(new RemoteVideoItem
            {
                Choice = new RemoteVideoChoice(transport, address),
                Label = $"{prefix}  {address}"
            });
        }
        foreach (var line in Split(MixerNative.DiscoverText()))
            Add(OutputTransport.Omt, line);
        foreach (var line in Split(MixerNative.DiscoverNdiText()))
            Add(OutputTransport.Ndi, line);
        foreach (var output in session.Outputs.Where(item =>
                     item.Enabled && item.Transport is OutputTransport.Omt or OutputTransport.Ndi))
            Add(output.Transport, output.Name);
        return items;
    }

    public static RemoteVideoChoice FromPrefs(bool preview)
    {
        var prefs = AppPrefs.Current;
        var address = preview ? prefs.PreviewVideoAddress : prefs.ProgramVideoAddress;
        var transport = ParseTransport(preview ? prefs.PreviewVideoTransport : prefs.ProgramVideoTransport);
        if (!string.IsNullOrWhiteSpace(address))
            return new RemoteVideoChoice(transport, address);
        return UniqueOutput(preview ? OutputSourceKind.MuPreview : OutputSourceKind.MuProgram);
    }

    public static void Save(bool preview, RemoteVideoChoice choice)
    {
        var name = choice.Transport == OutputTransport.Ndi ? "NDI" : "OMT";
        if (preview)
        {
            AppPrefs.Current.PreviewVideoAddress = choice.Address;
            AppPrefs.Current.PreviewVideoTransport = name;
        }
        else
        {
            AppPrefs.Current.ProgramVideoAddress = choice.Address;
            AppPrefs.Current.ProgramVideoTransport = name;
        }
        AppPrefs.Current.Save();
    }

    public static OutputTransport ParseTransport(string? value) =>
        string.Equals(value, "NDI", StringComparison.OrdinalIgnoreCase)
            ? OutputTransport.Ndi
            : OutputTransport.Omt;

    private static RemoteVideoChoice UniqueOutput(OutputSourceKind kind)
    {
        if (Application.Current is not App app)
            return new RemoteVideoChoice(OutputTransport.Omt, "");
        var matches = HostPresentation.From(app.Session)
            .Where(item => item.SourceKind == kind && item.UnitId == app.Session.SelectedUnitId)
            .ToList();
        if (matches.Count != 1)
            return new RemoteVideoChoice(OutputTransport.Omt, "");
        return new RemoteVideoChoice(matches[0].Transport, matches[0].Name);
    }

    private static IEnumerable<string> Split(string text) =>
        string.IsNullOrWhiteSpace(text)
            ? []
            : text.Split('\n', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries);
}

internal sealed class DisconnectedRemoteBackend : IEivizBackend
{
    public bool IsRemote => true;
    public bool Connected => false;
    public bool CanPreviewInputs => false;
    public bool CanShowSceneThumbs => false;
    public ulong Revision => 0;
    public string StatusText => I18n.Loc.T("msg.remoteIdle");
    public event Action? Changed { add { } remove { } }
    public void Poll() { }
    public bool Cut(ulong unitId, bool swap) { _ = (unitId, swap); return false; }
    public bool Preview(ulong unitId, ulong sceneGpuId) { _ = (unitId, sceneGpuId); return false; }
    public bool Auto(ulong unitId, MixingUnitEntry unit, TransitionPreset preset) { _ = (unitId, unit, preset); return false; }
    public bool SetMix(ulong unitId, float mix, TransitionPreset? preset) { _ = (unitId, mix, preset); return false; }
    public bool OverlayAuto(ulong unitId, uint index, uint durationMs, bool toOn) { _ = (unitId, index, durationMs, toOn); return false; }
    public bool VideoPlay(ulong inputId, bool playing) { _ = (inputId, playing); return false; }
    public bool VideoLoop(ulong inputId, bool looping) { _ = (inputId, looping); return false; }
    public bool VideoSeek(ulong inputId, long positionHns) { _ = (inputId, positionHns); return false; }
    public bool Mutate(string json, ulong expectedRevision, out string error)
    {
        error = StatusText;
        _ = (json, expectedRevision);
        return false;
    }
    public bool UploadMedia(string path, string kind, string name, bool videoLoop, ulong expectedRevision, out string error)
    {
        error = StatusText;
        _ = (path, kind, name, videoLoop, expectedRevision);
        return false;
    }
    public bool TryGetMix(ulong unitId, out float mix) { mix = 0; _ = unitId; return false; }
    public void BusSources(ulong unitId, out ulong previewGpuId, out ulong programGpuId)
    {
        previewGpuId = 0;
        programGpuId = 0;
        _ = unitId;
    }
    public IReadOnlyList<PublishedVideoOutput> PublishedOutputs() => [];
    public void BindPreviewProgram(SwapchainHost preview, SwapchainHost program, ulong unitId)
    {
        preview.ReleaseNative();
        program.ReleaseNative();
        preview.IsMonitor = true;
        program.IsMonitor = true;
        preview.MonitorId = 0;
        program.MonitorId = 0;
        preview.SourceId = 0;
        program.SourceId = 0;
        _ = unitId;
    }
    public void BindMultiview(SwapchainHost host, MultiviewLayout layout)
    {
        host.ReleaseNative();
        _ = layout;
    }
    public void SyncPublishedVideo() { }
    public void Dispose() { }
}
