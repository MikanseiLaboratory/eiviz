using Eiviz.Host.Interop;

namespace Eiviz.Host;

internal readonly record struct VideoRoles(bool OnProgram, bool OnPreview);

internal sealed class VideoTransport
{
    private readonly Dictionary<ulong, VideoRoles> _previous = [];

    public void Tick(Session session, IEnumerable<ulong> previewWindows)
    {
        var roles = Collect(session, previewWindows);
        foreach (var input in session.Inputs)
        {
            if (input.Kind != InputKind.Video)
                continue;
            roles.TryGetValue(input.Id, out var now);
            _previous.TryGetValue(input.Id, out var prev);
            var roseProgram = now.OnProgram && !prev.OnProgram;
            var fellProgram = !now.OnProgram && prev.OnProgram;
            var rosePreview = now.OnPreview && !prev.OnPreview;
            var paused = Matches(input.VideoPauseWhen, roseProgram, fellProgram, rosePreview);
            var restarted = Matches(input.VideoRestartWhen, roseProgram, fellProgram, rosePreview);
            if (restarted)
                MixerNative.VideoSeek(input.Id, 0);
            if (paused)
                MixerNative.VideoSetPlaying(input.Id, 0);
            else if (restarted || ShouldPlay(input.VideoPlayWhen, roseProgram, rosePreview, now))
                MixerNative.VideoSetPlaying(input.Id, 1);
            _previous[input.Id] = now;
        }
    }

    public void Forget(ulong id) => _previous.Remove(id);

    private static bool ShouldPlay(VideoPlayWhen when, bool roseProgram, bool rosePreview, VideoRoles now) =>
        when switch
        {
            VideoPlayWhen.OnActive => roseProgram,
            VideoPlayWhen.OnPreview => rosePreview,
            VideoPlayWhen.Always => now.OnProgram || now.OnPreview,
            _ => false
        };

    private static bool Matches(VideoTriggerWhen when, bool roseProgram, bool fellProgram, bool rosePreview) =>
        when switch
        {
            VideoTriggerWhen.OnActive => roseProgram,
            VideoTriggerWhen.OnDeactivated => fellProgram,
            VideoTriggerWhen.OnPreview => rosePreview,
            _ => false
        };

    internal static Dictionary<ulong, VideoRoles> Collect(Session session, IEnumerable<ulong> previewWindows)
    {
        var roles = new Dictionary<ulong, VideoRoles>();
        foreach (var unit in session.Units)
        {
            UnitState state = default;
            unsafe
            {
                if (MixerNative.GetUnitState(unit.Id, &state) != 0)
                    continue;
            }
            Mark(session, roles, state.ProgramSource, program: true, preview: false);
            Mark(session, roles, state.PreviewSource, program: false, preview: true);
            if (state.Mix > 0.001f)
            {
                var incoming = state.IncomingSource != 0 ? state.IncomingSource : state.PreviewSource;
                Mark(session, roles, incoming, program: true, preview: false);
            }
            foreach (var source in OverlaySources(state))
                Mark(session, roles, source, program: true, preview: false);
        }
        foreach (var id in previewWindows)
            Mark(session, roles, id, program: false, preview: true);
        return roles;
    }

    private static void Mark(Session session, Dictionary<ulong, VideoRoles> roles, ulong id, bool program, bool preview)
    {
        if (id == 0 || id >= MixerNative.MultiviewBase)
            return;
        if (id >= MixerNative.SceneBase)
        {
            var scene = session.Scenes.FirstOrDefault(item => item.GpuId == id);
            if (scene is null)
                return;
            foreach (var layer in scene.Layers)
                Mark(session, roles, layer.InputId, program, preview);
            return;
        }
        roles.TryGetValue(id, out var current);
        roles[id] = new VideoRoles(current.OnProgram || program, current.OnPreview || preview);
    }

    private static IEnumerable<ulong> OverlaySources(UnitState state)
    {
        var count = Math.Min(8, (int)state.OverlayCount);
        if (count > 0) yield return state.Overlay0.SourceId;
        if (count > 1) yield return state.Overlay1.SourceId;
        if (count > 2) yield return state.Overlay2.SourceId;
        if (count > 3) yield return state.Overlay3.SourceId;
        if (count > 4) yield return state.Overlay4.SourceId;
        if (count > 5) yield return state.Overlay5.SourceId;
        if (count > 6) yield return state.Overlay6.SourceId;
        if (count > 7) yield return state.Overlay7.SourceId;
    }
}
