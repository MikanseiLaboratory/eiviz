using System.Text;
using Eiviz.Host.Interop;

namespace Eiviz.Host.Media;

internal static class AudioGraphSync
{
    public static void Push(Session session)
    {
        foreach (var unit in session.Units)
        {
            MixerNative.AudioUnitBusSet(
                unit.Id,
                (uint)unit.Audio.DeviceKind,
                unit.Audio.DeviceId ?? "",
                unit.Audio.MapLeft,
                unit.Audio.MapRight);
            MixerNative.AudioSetBusGain(unit.Id, MixerNative.MixerGain(unit.Audio.Gain), unit.Audio.Mute ? 1u : 0u);
            MixerNative.AudioSetUnitLink(unit.Id, (uint)unit.AudioLink);
        }
        MixerNative.AudioHeadphoneSet(
            (uint)session.Headphone.DeviceKind,
            session.Headphone.DeviceId ?? "",
            session.Headphone.MapLeft,
            session.Headphone.MapRight);
        foreach (var input in session.Inputs)
        {
            var units = input.Kind == InputKind.Mix ? [] : input.AudioUnits.ToArray();
            unsafe
            {
                fixed (ulong* ptr = units)
                {
                    MixerNative.AudioSetInput(
                        input.Id,
                        units.Length == 0 ? null : ptr,
                        (uint)units.Length,
                        MixerNative.MixerGain(input.Gain),
                        input.Mute ? 1u : 0u);
                }
            }
        }
        MixerNative.AudioSetHeadphoneCue(session.SelectedUnitId);
        MixerNative.AudioSetHeadphoneCopyMonitor(session.HeadphoneCopyMonitor ? 1u : 0u);
    }

    public static void SetInput(ulong id, IReadOnlyList<ulong> units, float gain, bool mute)
    {
        var copy = units as ulong[] ?? units.ToArray();
        unsafe
        {
            fixed (ulong* ptr = copy)
            {
                MixerNative.AudioSetInput(
                    id,
                    copy.Length == 0 ? null : ptr,
                    (uint)copy.Length,
                    gain,
                    mute ? 1u : 0u);
            }
        }
    }

    public static List<(uint Kind, uint Channels, string Id, string Name, uint Direction, uint Caps)> EnumerateDevices(uint kind)
    {
        var list = new List<(uint, uint, string, string, uint, uint)>();
        var buffer = new MixerAudioDeviceInfo[64];
        unsafe
        {
            fixed (MixerAudioDeviceInfo* ptr = buffer)
            {
                var n = MixerNative.AudioEnumDevices(kind, ptr, (uint)buffer.Length);
                for (var i = 0; i < n && i < buffer.Length; i++)
                {
                    var current = ptr + i;
                    list.Add((
                        current->Kind,
                        current->Channels,
                        ReadUtf8(current->Id, 256),
                        ReadUtf8(current->Name, 256),
                        current->Direction,
                        current->Caps));
                }
            }
        }
        return list;
    }

    private static unsafe string ReadUtf8(byte* ptr, int cap)
    {
        var n = 0;
        while (n < cap && ptr[n] != 0)
            n++;
        return n == 0 ? "" : Encoding.UTF8.GetString(new ReadOnlySpan<byte>(ptr, n));
    }
}
