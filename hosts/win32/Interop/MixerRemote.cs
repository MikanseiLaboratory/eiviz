using System.Runtime.InteropServices;
using System.Text;

namespace Eiviz.Host.Interop;

internal static partial class MixerRemote
{
    private const string LibraryName = "eiviz_remote";

    [LibraryImport(LibraryName, EntryPoint = "mixer_remote_open", StringMarshalling = StringMarshalling.Utf8)]
    internal static partial int Open(string url, string token);

    [LibraryImport(LibraryName, EntryPoint = "mixer_remote_close")]
    internal static partial int Close(int handle);

    [LibraryImport(LibraryName, EntryPoint = "mixer_remote_copy_snapshot")]
    internal static unsafe partial int CopySnapshot(int handle, byte* buffer, nuint capacity);

    [LibraryImport(LibraryName, EntryPoint = "mixer_remote_copy_live")]
    internal static unsafe partial int CopyLive(int handle, byte* buffer, nuint capacity);

    [LibraryImport(LibraryName, EntryPoint = "mixer_remote_copy_status")]
    internal static unsafe partial int CopyStatus(int handle, byte* buffer, nuint capacity);

    [LibraryImport(LibraryName, EntryPoint = "mixer_remote_cut")]
    internal static partial int Cut(int handle, ulong unitId, uint swap);

    [LibraryImport(LibraryName, EntryPoint = "mixer_remote_preview")]
    internal static partial int Preview(int handle, ulong unitId, ulong sceneId);

    [LibraryImport(LibraryName, EntryPoint = "mixer_remote_auto")]
    internal static partial int Auto(int handle, ulong unitId, uint kind, uint durationMs, uint swap, uint keepPreview, uint easing, uint direction, float dipR, float dipG, float dipB, float dipA, float softness, float param);

    [LibraryImport(LibraryName, EntryPoint = "mixer_remote_set_mix")]
    internal static partial int SetMix(int handle, ulong unitId, float value);

    [LibraryImport(LibraryName, EntryPoint = "mixer_remote_overlay_auto")]
    internal static partial int OverlayAuto(int handle, ulong unitId, uint index, uint durationMs, uint toOn);

    [LibraryImport(LibraryName, EntryPoint = "mixer_remote_mutate")]
    internal static unsafe partial int Mutate(int handle, byte* json, nuint length, ulong expectedRevision);

    [LibraryImport(LibraryName, EntryPoint = "mixer_remote_replace")]
    internal static unsafe partial int Replace(int handle, byte* json, nuint length, ulong expectedRevision);

    [LibraryImport(LibraryName, EntryPoint = "mixer_remote_video_play")]
    internal static partial int VideoPlay(int handle, ulong inputId, uint playing);

    [LibraryImport(LibraryName, EntryPoint = "mixer_remote_video_loop")]
    internal static partial int VideoLoop(int handle, ulong inputId, uint looping);

    [LibraryImport(LibraryName, EntryPoint = "mixer_remote_video_seek")]
    internal static partial int VideoSeek(int handle, ulong inputId, long positionHns);

    [LibraryImport(LibraryName, EntryPoint = "mixer_remote_audio_set_input")]
    internal static partial int AudioSetInput(int handle, ulong inputId, uint busMask, float gain, uint mute);

    [LibraryImport(LibraryName, EntryPoint = "mixer_remote_audio_set_bus")]
    internal static partial int AudioSetBus(int handle, ulong busId, float gain, uint mute);

    [LibraryImport(LibraryName, EntryPoint = "mixer_remote_upload", StringMarshalling = StringMarshalling.Utf8)]
    internal static partial int Upload(int handle, string path, string kind, string name, uint videoLoop, ulong expectedRevision);

    private unsafe delegate int Utf8Copy(byte* buffer, nuint cap);

    private static unsafe string CopyUtf8(Utf8Copy call, ref byte[] buffer)
    {
        for (;;)
        {
            unsafe
            {
                fixed (byte* ptr = buffer)
                {
                    var n = call(ptr, (nuint)buffer.Length);
                    if (n >= 0)
                        return n == 0 ? string.Empty : Encoding.UTF8.GetString(buffer, 0, n);
                    if (n == -1 && buffer.Length < 16 << 20)
                    {
                        buffer = new byte[buffer.Length * 2];
                        continue;
                    }
                    return string.Empty;
                }
            }
        }
    }

    internal static unsafe string SnapshotText(int handle, ref byte[] buffer) =>
        CopyUtf8((ptr, cap) => CopySnapshot(handle, ptr, cap), ref buffer);

    internal static unsafe string LiveText(int handle, ref byte[] buffer) =>
        CopyUtf8((ptr, cap) => CopyLive(handle, ptr, cap), ref buffer);

    internal static unsafe string StatusText(int handle, ref byte[] buffer) =>
        CopyUtf8((ptr, cap) => CopyStatus(handle, ptr, cap), ref buffer);

    internal static int MutateCode(int handle, string json, ulong expectedRevision)
    {
        var bytes = Encoding.UTF8.GetBytes(json);
        unsafe
        {
            fixed (byte* ptr = bytes)
                return Mutate(handle, ptr, (nuint)bytes.Length, expectedRevision);
        }
    }
}
