using System.Text.Json;
using System.Windows;
using Eiviz.Host.Interop;

namespace Eiviz.Host;

/// <summary>What a scene's animation is doing right now. Playback is live-only.</summary>
internal readonly record struct SceneAnimLive(
    ulong? ShownState,
    ulong? MovingState,
    ulong? SequenceId,
    int StepIndex,
    bool Holding)
{
    public static readonly SceneAnimLive Idle = new(null, null, null, -1, false);
}

/// <summary>Go To / Play commands and tally shared by the main window and the animation editor.</summary>
internal static class SceneAnimPlayback
{
    private static byte[] _liveBuf = new byte[1 << 16];

    public static bool GoTo(SceneEntry scene, ulong stateId) =>
        Application.Current is App app && app.Backend.SceneGoTo(scene.Id, stateId);

    public static bool Sequence(SceneEntry scene, ulong sequenceId, uint op) =>
        Application.Current is App app && app.Backend.SceneSequence(scene.Id, sequenceId, op);

    public static SceneAnimLive Read(SceneEntry scene)
    {
        if (Application.Current is not App app)
            return SceneAnimLive.Idle;
        if (app.Backend is RemoteEivizBackend remote)
            return ReadRemote(remote, scene);
        if (app.Backend.IsRemote)
            return SceneAnimLive.Idle;
        return ReadLocal(scene);
    }

    private static SceneAnimLive ReadLocal(SceneEntry scene)
    {
        var reached = new EivizReachedLayer[64];
        var moves = new EivizActiveMove[16];
        var sequences = new EivizActiveSequence[16];
        unsafe
        {
            fixed (EivizReachedLayer* reachedPtr = reached)
            fixed (EivizActiveMove* movePtr = moves)
            fixed (EivizActiveSequence* sequencePtr = sequences)
            {
                uint reachedCount, moveCount, sequenceCount;
                ulong cameraState;
                if (MixerNative.SceneAnimState(
                        scene.GpuId,
                        reachedPtr, 64, &reachedCount,
                        movePtr, 16, &moveCount,
                        sequencePtr, 16, &sequenceCount,
                        &cameraState) != 0)
                    return SceneAnimLive.Idle;
                var moving = moveCount > 0 ? moves[0].StateId : (ulong?)null;
                var span = reached.AsSpan(0, (int)Math.Min(reachedCount, 64u));
                var shown = WithCamera(scene, SameState(span), span.Length == 0, cameraState);
                if (sequenceCount == 0)
                    return new SceneAnimLive(shown, moving, null, -1, false);
                var active = sequences[0];
                return new SceneAnimLive(shown, moving, active.SequenceId, (int)active.StepIndex, active.Holding != 0);
            }
        }
    }

    private static ulong? SameState(ReadOnlySpan<EivizReachedLayer> reached)
    {
        if (reached.Length == 0)
            return null;
        var id = reached[0].StateId;
        foreach (var layer in reached)
        {
            if (layer.StateId != id)
                return null;
        }
        return id;
    }

    /// <summary>A state lights only once its camera has arrived too, when it carries one.</summary>
    private static ulong? WithCamera(SceneEntry scene, ulong? shown, bool noLayers, ulong cameraState)
    {
        if (shown is null && noLayers && cameraState != 0)
            shown = cameraState;
        if (shown is ulong id && StateHasCamera(scene, id) && cameraState != id)
            return null;
        return shown;
    }

    private static bool StateHasCamera(SceneEntry scene, ulong id)
    {
        foreach (var state in scene.States)
        {
            if (state.Id == id)
                return state.Camera is not null;
        }
        return false;
    }

    private static SceneAnimLive ReadRemote(RemoteEivizBackend remote, SceneEntry scene)
    {
        var json = MixerRemote.LiveText(remote.Handle, ref _liveBuf);
        if (string.IsNullOrEmpty(json))
            return SceneAnimLive.Idle;
        try
        {
            using var doc = JsonDocument.Parse(json);
            if (!doc.RootElement.TryGetProperty("scenes", out var scenes)
                || !scenes.TryGetProperty(scene.GpuId.ToString(), out var live))
                return SceneAnimLive.Idle;
            ulong? moving = null;
            if (live.TryGetProperty("moves", out var moves) && moves.GetArrayLength() > 0
                && moves[0].TryGetProperty("stateId", out var movingState))
                moving = movingState.GetUInt64();
            ulong? shown = null;
            var reachedCount = 0;
            if (live.TryGetProperty("reached", out var reached))
            {
                foreach (var layer in reached.EnumerateArray())
                {
                    reachedCount++;
                    var next = layer.GetProperty("stateId").GetUInt64();
                    if (shown is ulong current && current != next)
                    {
                        shown = null;
                        break;
                    }
                    shown = next;
                }
            }
            var cameraState = live.TryGetProperty("cameraState", out var cameraProp) ? cameraProp.GetUInt64() : 0UL;
            shown = WithCamera(scene, shown, reachedCount == 0, cameraState);
            if (!live.TryGetProperty("sequences", out var sequences) || sequences.GetArrayLength() == 0)
                return new SceneAnimLive(shown, moving, null, -1, false);
            var active = sequences[0];
            return new SceneAnimLive(
                shown,
                moving,
                active.GetProperty("sequenceId").GetUInt64(),
                active.GetProperty("stepIndex").GetInt32(),
                active.GetProperty("holding").GetBoolean());
        }
        catch (Exception ex) when (ex is JsonException or KeyNotFoundException or InvalidOperationException)
        {
            return SceneAnimLive.Idle;
        }
    }
}
