using Eiviz.Host.Interop;

namespace Eiviz.Host;

internal static class SourceResolver
{
    public static bool TryResolveIncoming(Session session, string key, out ulong incoming)
    {
        incoming = MixerNative.IncomingPreview;
        if (string.IsNullOrWhiteSpace(key))
            return false;
        key = key.Trim();
        if (IsPreviewKey(key))
        {
            incoming = MixerNative.IncomingPreview;
            return true;
        }
        if (IsProgramKey(key))
        {
            incoming = MixerNative.IncomingProgram;
            return true;
        }
        if (SceneResolver.Resolve(session, key) is { } scene)
        {
            incoming = scene.GpuId;
            return true;
        }
        if (InputResolver.Resolve(session, key) is { } input)
        {
            incoming = input.Id;
            return true;
        }
        return false;
    }

    private static bool IsPreviewKey(string key) =>
        key is "0"
        || key.Equals("preview", StringComparison.OrdinalIgnoreCase)
        || key.Equals("prv", StringComparison.OrdinalIgnoreCase);

    private static bool IsProgramKey(string key) =>
        key is "-1"
        || key.Equals("program", StringComparison.OrdinalIgnoreCase)
        || key.Equals("pgm", StringComparison.OrdinalIgnoreCase);
}
