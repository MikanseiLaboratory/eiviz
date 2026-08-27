using System.Text;
using Eiviz.Host.Interop;

namespace Eiviz.Host.Media;

internal static class AudioGraphSync
{
    public static void Push(Session session)
    {
        session.EnsureDefaultBuses();
        var keep = session.Buses.Select(item => item.Id).ToHashSet();
        unsafe
        {
            var n = MixerNative.AudioBusCount();
            var live = new List<ulong>();
            for (var i = 0; i < n; i++)
            {
                MixerAudioBusInfo info = default;
                if (MixerNative.AudioBusGet((uint)i, &info) != 0)
                    continue;
                live.Add(info.Id);
            }
            foreach (var id in live)
            {
                if (!keep.Contains(id))
                    MixerNative.AudioBusRemove(id);
            }
        }
        foreach (var bus in session.Buses)
        {
            MixerNative.AudioBusUpsert(
                bus.Id,
                bus.Name,
                (uint)bus.Role,
                (uint)bus.DeviceKind,
                bus.DeviceId ?? "",
                bus.MapLeft,
                bus.MapRight,
                bus.Exclusive ? 1u : 0u);
            MixerNative.AudioSetBusGain(bus.Id, MixerNative.MixerGain(bus.Gain), bus.Mute ? 1u : 0u);
        }
        unsafe
        {
            var n = MixerNative.AudioBusCount();
            for (var i = 0; i < n; i++)
            {
                MixerAudioBusInfo info = default;
                if (MixerNative.AudioBusGet((uint)i, &info) != 0)
                    continue;
                var busId = info.Id;
                var bit = info.Bit;
                var match = session.Buses.FirstOrDefault(item => item.Id == busId);
                if (match is not null)
                    match.Bit = bit;
            }
        }
        foreach (var input in session.Inputs)
        {
            MixerNative.AudioSetInput(
                input.Id,
                input.BusMask == 0 ? 1u : input.BusMask,
                MixerNative.MixerGain(input.Gain),
                input.Mute ? 1u : 0u);
        }
        foreach (var unit in session.Units)
        {
            MixerNative.AudioSetUnitLink(unit.Id, unit.AudioBusId == 0 ? 1 : unit.AudioBusId, (uint)unit.AudioLink);
        }
        MixerNative.AudioSetHeadphoneCue(session.SelectedUnitId);
        MixerNative.AudioSetHeadphoneCopyMaster(session.HeadphoneCopyMaster ? 1u : 0u);
    }

    public static List<(uint Kind, uint Channels, string Id, string Name)> EnumerateDevices(uint kind)
    {
        var list = new List<(uint, uint, string, string)>();
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
                        ReadUtf8(current->Name, 256)));
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
