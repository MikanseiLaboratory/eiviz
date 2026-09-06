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
        Converters = { new JsonStringEnumConverter(JsonNamingPolicy.CamelCase) }
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
    private ulong _seenRevision;
    private string _epoch = "";

    public RemoteEivizBackend(int handle) => _handle = handle;
    public bool IsRemote => true;
    public bool CanPreviewInputs => false;
    public bool CanShowSceneThumbs => false;
    public ulong Revision { get; private set; }
    public string StatusText => _status;
    public event Action? Changed;

    public static RemoteEivizBackend Open(string url, string token)
    {
        var handle = MixerNative.RemoteOpen(url, token ?? "");
        if (handle <= 0)
            throw new InvalidOperationException(I18n.Loc.T("msg.remoteConnectFailed"));
        var backend = new RemoteEivizBackend(handle);
        backend.Pull(force: true);
        return backend;
    }

    public void Poll() => Pull(force: false);

    private void Pull(bool force)
    {
        var statusJson = MixerNative.RemoteStatusText(_handle);
        var connected = false;
        var lag = false;
        var error = "";
        var epoch = _epoch;
        try
        {
            using var status = JsonDocument.Parse(string.IsNullOrEmpty(statusJson) ? "{}" : statusJson);
            connected = status.RootElement.TryGetProperty("connected", out var c) && c.GetBoolean();
            lag = status.RootElement.TryGetProperty("lag", out var l) && l.GetBoolean();
            error = status.RootElement.TryGetProperty("error", out var e) ? e.GetString() ?? "" : "";
            if (status.RootElement.TryGetProperty("revision", out var r))
                Revision = r.GetUInt64();
            epoch = status.RootElement.TryGetProperty("epoch", out var ep) ? ep.GetString() ?? "" : epoch;
        }
        catch
        {
            connected = false;
        }
        if (!connected)
            _status = I18n.Loc.T("msg.remoteDisconnected");
        else if (!string.IsNullOrEmpty(error))
            _status = string.IsNullOrEmpty(error) ? I18n.Loc.T("msg.remoteAuthFailed") : error;
        else if (lag)
            _status = I18n.Loc.T("msg.remoteResync");
        else
            _status = I18n.Loc.Format("msg.remoteConnected", Revision);

        var docChanged = force || Revision != _seenRevision || epoch != _epoch;
        _epoch = epoch;
        if (!docChanged || Application.Current is not App app)
            return;
        var json = MixerNative.RemoteSnapshotText(_handle);
        if (string.IsNullOrEmpty(json))
            return;
        var next = SessionStore.FromJson(json);
        next.SelectedUnitId = app.Session.SelectedUnitId;
        _seenRevision = Revision;
        app.ReplaceDocument(next);
        SyncPublishedVideo();
        Changed?.Invoke();
    }

    public bool Cut(ulong unitId, bool swap) =>
        MixerNative.RemoteCut(_handle, unitId, swap ? 1u : 0u) == 0;

    public bool Preview(ulong unitId, ulong sceneGpuId) =>
        MixerNative.RemotePreview(_handle, unitId, sceneGpuId) == 0;

    public bool Auto(ulong unitId, MixingUnitEntry unit, TransitionPreset preset)
    {
        var code = MixerNative.RemoteAuto(
            _handle, unitId, preset.Kind, preset.DurationMsFor(unit),
            preset.Swap ? 1u : 0u, preset.KeepPreview ? 1u : 0u,
            preset.Easing, preset.Direction, preset.DipR, preset.DipG, preset.DipB,
            preset.DipA <= 0 ? 1 : preset.DipA, preset.Softness, preset.Param);
        return code == 0;
    }

    public bool SetMix(ulong unitId, float mix, TransitionPreset? preset)
    {
        _ = preset;
        return MixerNative.RemoteSetMix(_handle, unitId, mix) == 0;
    }

    public bool OverlayAuto(ulong unitId, uint index, uint durationMs, bool toOn) =>
        MixerNative.RemoteOverlayAuto(_handle, unitId, index, durationMs, toOn ? 1u : 0u) == 0;

    public bool VideoPlay(ulong inputId, bool playing) =>
        MixerNative.RemoteVideoPlay(_handle, inputId, playing ? 1u : 0u) == 0;

    public bool VideoLoop(ulong inputId, bool looping) =>
        MixerNative.RemoteVideoLoop(_handle, inputId, looping ? 1u : 0u) == 0;

    public bool VideoSeek(ulong inputId, long positionHns) =>
        MixerNative.RemoteVideoSeek(_handle, inputId, positionHns) == 0;

    public bool Mutate(string json, ulong expectedRevision, out string error)
    {
        var code = MixerNative.RemoteMutateCode(_handle, json, expectedRevision);
        if (code == 0)
        {
            error = "";
            Pull(force: true);
            return true;
        }
        error = code == -3 ? I18n.Loc.T("msg.revisionConflict") : I18n.Loc.Error("Mutate session", code < 0 ? -code : code);
        Pull(force: true);
        return false;
    }

    public bool UploadMedia(string path, string kind, string name, bool videoLoop, ulong expectedRevision, out string error)
    {
        var code = MixerNative.RemoteUpload(_handle, path, kind, name, videoLoop ? 1u : 0u, expectedRevision);
        if (code == 0)
        {
            error = "";
            Pull(force: true);
            return true;
        }
        error = I18n.Loc.T("msg.uploadFailed");
        return false;
    }

    public bool TryGetMix(ulong unitId, out float mix)
    {
        mix = 0;
        try
        {
            var json = MixerNative.RemoteLiveText(_handle);
            if (string.IsNullOrEmpty(json))
                return false;
            using var doc = JsonDocument.Parse(json);
            if (!doc.RootElement.TryGetProperty("units", out var units))
                return false;
            var key = unitId.ToString();
            if (!units.TryGetProperty(key, out var unit))
                return false;
            mix = unit.GetProperty("mix").GetSingle();
            return true;
        }
        catch
        {
            return false;
        }
    }

    public IReadOnlyList<PublishedVideoOutput> PublishedOutputs() =>
        Application.Current is App app ? HostPresentation.From(app.Session) : [];

    public void BindPreviewProgram(SwapchainHost preview, SwapchainHost program, ulong unitId) =>
        _presenter.Bind(preview, program, unitId, PublishedOutputs());

    public void BindMultiview(SwapchainHost host, MultiviewLayout layout) =>
        _presenter.BindMultiview(host, layout, PublishedOutputs());

    public void SyncPublishedVideo() => _presenter.Sync(PublishedOutputs());

    public void Dispose() => MixerNative.RemoteClose(_handle);
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
    private const ulong SourceBase = 0x0005_0000;
    private const ulong PreviewMonitor = 0x0006_0001;
    private const ulong ProgramMonitor = 0x0006_0002;
    private const ulong MultiviewMonitor = 0x0006_0003;
    private readonly HashSet<ulong> _connected = [];

    public void Sync(IReadOnlyList<PublishedVideoOutput> outputs)
    {
        var keep = new HashSet<ulong>();
        foreach (var output in outputs)
        {
            var id = SourceBase | output.Id;
            keep.Add(id);
            if (_connected.Contains(id))
                continue;
            var code = output.Transport == OutputTransport.Ndi
                ? MixerNative.ConnectNdi(id, output.Name, 3, 0)
                : MixerNative.ConnectOmt(id, output.Name, 1, 3, 0);
            if (code == 0)
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

    public void Bind(SwapchainHost preview, SwapchainHost program, ulong unitId, IReadOnlyList<PublishedVideoOutput> outputs)
    {
        Sync(outputs);
        BindOne(preview, PreviewMonitor, SourceFor(outputs, OutputSourceKind.MuPreview, unitId));
        BindOne(program, ProgramMonitor, SourceFor(outputs, OutputSourceKind.MuProgram, unitId));
    }

    public void BindMultiview(SwapchainHost host, MultiviewLayout layout, IReadOnlyList<PublishedVideoOutput> outputs)
    {
        Sync(outputs);
        BindOne(host, MultiviewMonitor | (layout.Id << 8), SourceForMultiview(outputs, layout.GpuId));
    }

    private static ulong? SourceFor(IReadOnlyList<PublishedVideoOutput> outputs, OutputSourceKind kind, ulong unitId)
    {
        var matches = outputs.Where(item => item.SourceKind == kind && item.UnitId == unitId).ToList();
        if (matches.Count != 1)
            return null;
        return SourceBase | matches[0].Id;
    }

    private static ulong? SourceForMultiview(IReadOnlyList<PublishedVideoOutput> outputs, ulong layoutGpuId)
    {
        var matches = outputs.Where(item => item.SourceKind == OutputSourceKind.Multiview && item.SourceId == layoutGpuId).ToList();
        if (matches.Count != 1)
            return null;
        return SourceBase | matches[0].Id;
    }

    private void BindOne(SwapchainHost host, ulong monitorId, ulong? sourceId)
    {
        if (sourceId is ulong id)
            host.RetargetMonitor(monitorId, id);
        else
            host.ReleaseNative();
    }
}

internal sealed class DisconnectedRemoteBackend : IEivizBackend
{
    public bool IsRemote => true;
    public bool CanPreviewInputs => false;
    public bool CanShowSceneThumbs => false;
    public ulong Revision => 0;
    public string StatusText => I18n.Loc.T("msg.remoteConnectFailed");
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
    public IReadOnlyList<PublishedVideoOutput> PublishedOutputs() => [];
    public void BindPreviewProgram(SwapchainHost preview, SwapchainHost program, ulong unitId)
    {
        preview.ReleaseNative();
        program.ReleaseNative();
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
