using System.Runtime.InteropServices;
using System.Text;

namespace Eiviz.Host.Interop;

internal static partial class MixerNative
{
    private const string LibraryName = "eiviz_mixer";

    internal const ulong Color = 1;
    internal const ulong Bars = 2;
    internal const ulong Black = 3;
    internal const ulong Blue = 4;
    internal const ulong SceneBase = 0x0001_0000;
    internal const ulong MultiviewBase = 0x0002_0000;
    internal const ulong LabelBase = 0x0003_0000;
    internal const ulong AudioBusPeakBase = 0x0004_0000UL;
    internal const uint OutputProgram = 0;
    internal const uint OutputPreview = 1;
    internal const uint OutputMultiview = 2;
    internal const uint TransitionCut = 0;
    internal const uint TransitionFade = 1;
    internal const uint TransitionDip = 2;
    internal const uint FormatUyvy = 0;
    internal const uint FormatBgra = 1;
    internal const uint FormatRgba = 3;
    internal const uint OutOmt = 0;
    internal const uint OutNdi = 1;
    internal const uint OutDeckLink = 2;
    internal const uint SrcKindScene = 0;
    internal const uint SrcKindMuPreview = 1;
    internal const uint SrcKindMuProgram = 2;
    internal const uint SrcKindMuMultiview = 3;
    internal const uint SrcKindInput = 4;
    internal const uint GenSolid = 0;
    internal const uint GenBars = 1;
    internal const uint SaveAlwaysLow = 0;
    internal const uint SaveNotOnProgram = 1;
    internal const uint SaveNotOnPreviewOrProgram = 2;
    internal const uint SaveAlwaysFull = 3;
    internal const uint SaveFlagMultiview = 1;
    internal const ulong MuSourceFlag = 0x8000_0000_0000_0000UL;
    internal const ulong MuBusPreview = 0x1000_0000_0000_0000UL;
    internal const ulong MuIdMask = 0x0FFF_FFFF_FFFF_FFFFUL;
    internal static uint VideoFormat = FormatUyvy;

    internal static float MixerGain(float gain) => gain < 0f ? 1f : gain;

    internal static ulong SceneGpuId(ulong sceneId) => SceneBase | sceneId;
    internal static ulong MuProgram(ulong unitId) => MuSourceFlag | (unitId & MuIdMask);
    internal static ulong MuPreview(ulong unitId) => MuSourceFlag | MuBusPreview | (unitId & MuIdMask);

    [LibraryImport(LibraryName, EntryPoint = "mixer_ping")]
    internal static partial uint Ping();

    [LibraryImport(LibraryName, EntryPoint = "mixer_create")]
    internal static partial int Create(ulong adapterLuid, uint fpsNumerator, uint fpsDenominator);

    [LibraryImport(LibraryName, EntryPoint = "mixer_destroy")]
    internal static partial void Destroy();

    [LibraryImport(LibraryName, EntryPoint = "mixer_create_unit")]
    internal static partial int CreateUnit(ulong unitId, uint width, uint height);

    [LibraryImport(LibraryName, EntryPoint = "mixer_destroy_unit")]
    internal static partial int DestroyUnit(ulong unitId);

    [LibraryImport(LibraryName, EntryPoint = "mixer_unit_configure")]
    internal static partial int ConfigureUnit(ulong unitId, uint width, uint height, uint fpsNum, uint fpsDen);

    [LibraryImport(LibraryName, EntryPoint = "mixer_unit_attach_output")]
    internal static partial int AttachOutput(ulong unitId, nint hwnd, uint width, uint height, uint kind);

    [LibraryImport(LibraryName, EntryPoint = "mixer_unit_resize_output")]
    internal static partial int ResizeOutput(ulong unitId, uint kind, nint hwnd, uint width, uint height);

    [LibraryImport(LibraryName, EntryPoint = "mixer_unit_detach_output")]
    internal static partial int DetachOutput(ulong unitId, uint kind, nint hwnd);

    [LibraryImport(LibraryName, EntryPoint = "mixer_attach_monitor")]
    internal static partial int AttachMonitor(ulong monitorId, ulong sourceId, nint hwnd, uint width, uint height);

    [LibraryImport(LibraryName, EntryPoint = "mixer_resize_monitor")]
    internal static partial int ResizeMonitor(ulong monitorId, uint width, uint height);

    [LibraryImport(LibraryName, EntryPoint = "mixer_detach_monitor")]
    internal static partial int DetachMonitor(ulong monitorId);

    [LibraryImport(LibraryName, EntryPoint = "mixer_monitor_set_source")]
    internal static partial int SetMonitorSource(ulong monitorId, ulong sourceId);

    [LibraryImport(LibraryName, EntryPoint = "mixer_unit_set_state")]
    internal static unsafe partial int SetUnitState(ulong unitId, UnitState* state);

    [LibraryImport(LibraryName, EntryPoint = "mixer_unit_get_state")]
    internal static unsafe partial int GetUnitState(ulong unitId, UnitState* state);

    [LibraryImport(LibraryName, EntryPoint = "mixer_unit_cut")]
    internal static partial int Cut(ulong unitId, uint swap);

    [LibraryImport(LibraryName, EntryPoint = "mixer_unit_auto")]
    internal static partial int Auto(ulong unitId, uint durationMs, uint swap);

    [LibraryImport(LibraryName, EntryPoint = "mixer_register_source")]
    internal static partial int RegisterSource(ulong id, uint width, uint height, uint format);

    [LibraryImport(LibraryName, EntryPoint = "mixer_destroy_source")]
    internal static partial int DestroySource(ulong id);

    [LibraryImport(LibraryName, EntryPoint = "mixer_flush_audio")]
    internal static partial int FlushAudio(ulong id);

    [LibraryImport(LibraryName, EntryPoint = "mixer_audio_bus_upsert", StringMarshalling = StringMarshalling.Utf8)]
    internal static partial int AudioBusUpsert(ulong id, string name, uint role, uint deviceKind, string deviceId, int mapLeft, int mapRight, uint exclusive);

    [LibraryImport(LibraryName, EntryPoint = "mixer_audio_bus_remove")]
    internal static partial int AudioBusRemove(ulong id);

    [LibraryImport(LibraryName, EntryPoint = "mixer_audio_bus_count")]
    internal static partial int AudioBusCount();

    [LibraryImport(LibraryName, EntryPoint = "mixer_audio_bus_get")]
    internal static unsafe partial int AudioBusGet(uint index, MixerAudioBusInfo* info);

    [LibraryImport(LibraryName, EntryPoint = "mixer_audio_set_input")]
    internal static partial int AudioSetInput(ulong id, uint busMask, float gain, uint mute);

    [LibraryImport(LibraryName, EntryPoint = "mixer_audio_set_bus_gain")]
    internal static partial int AudioSetBusGain(ulong id, float gain, uint mute);

    [LibraryImport(LibraryName, EntryPoint = "mixer_audio_set_unit_link")]
    internal static partial int AudioSetUnitLink(ulong unitId, ulong busId, uint mode);

    [LibraryImport(LibraryName, EntryPoint = "mixer_audio_set_headphone_cue")]
    internal static partial int AudioSetHeadphoneCue(ulong unitId);

    [LibraryImport(LibraryName, EntryPoint = "mixer_audio_set_headphone_copy_master")]
    internal static partial int AudioSetHeadphoneCopyMaster(uint enabled);

    [LibraryImport(LibraryName, EntryPoint = "mixer_audio_enum_devices")]
    internal static unsafe partial int AudioEnumDevices(uint kind, MixerAudioDeviceInfo* devices, uint capacity);

    [LibraryImport(LibraryName, EntryPoint = "mixer_audio_device_channels", StringMarshalling = StringMarshalling.Utf8)]
    internal static partial int AudioDeviceChannels(uint kind, string deviceId);

    [LibraryImport(LibraryName, EntryPoint = "mixer_bind_multiview")]
    internal static partial int BindMultiview(ulong sceneId, ulong previewUnit, ulong programUnit);

    [LibraryImport(LibraryName, EntryPoint = "mixer_copy_follow_audio")]
    internal static unsafe partial int CopyFollowAudio(float* samples, uint capacity);

    [LibraryImport(LibraryName, EntryPoint = "mixer_copy_monitor_audio")]
    internal static unsafe partial int CopyMonitorAudio(ulong id, float* samples, uint capacity, int* sampleRate, int* channels);

    [LibraryImport(LibraryName, EntryPoint = "mixer_push_frame")]
    internal static unsafe partial int PushFrame(ulong id, byte* ptr, uint stride, uint height, long pts);

    [LibraryImport(LibraryName, EntryPoint = "mixer_push_audio")]
    internal static unsafe partial int PushAudio(ulong id, int sampleRate, int channels, uint frames, long pts, float* planar);

    [LibraryImport(LibraryName, EntryPoint = "mixer_load_still", StringMarshalling = StringMarshalling.Utf8)]
    internal static partial int LoadStill(ulong id, string path);

    [LibraryImport(LibraryName, EntryPoint = "mixer_video_start", StringMarshalling = StringMarshalling.Utf8)]
    internal static partial int VideoStart(ulong id, string path, uint capture, uint format);

    [LibraryImport(LibraryName, EntryPoint = "mixer_video_set_playing")]
    internal static partial int VideoSetPlaying(ulong id, uint playing);

    [LibraryImport(LibraryName, EntryPoint = "mixer_video_seek")]
    internal static partial int VideoSeek(ulong id, long hns);

    [LibraryImport(LibraryName, EntryPoint = "mixer_video_copy_info")]
    internal static unsafe partial int CopyVideoInfo(ulong id, MixerVideoInfo* info);

    [LibraryImport(LibraryName, EntryPoint = "mixer_omt_connect", StringMarshalling = StringMarshalling.Utf8)]
    internal static partial int ConnectOmt(ulong id, string address, uint useGpu, uint frameBufferFrames);

    [LibraryImport(LibraryName, EntryPoint = "mixer_ndi_connect", StringMarshalling = StringMarshalling.Utf8)]
    internal static partial int ConnectNdi(ulong id, string address, uint frameBufferFrames);

    [LibraryImport(LibraryName, EntryPoint = "mixer_set_live_save")]
    internal static partial int SetLiveSave(ulong id, uint mode, uint flags);

    [LibraryImport(LibraryName, EntryPoint = "mixer_define_scene")]
    internal static unsafe partial int DefineScene(ulong sceneId, uint width, uint height, uint count, OverlayDesc* layers);

    [LibraryImport(LibraryName, EntryPoint = "mixer_destroy_scene")]
    internal static partial int DestroyScene(ulong sceneId);

    [LibraryImport(LibraryName, EntryPoint = "mixer_define_generator")]
    internal static partial int DefineGenerator(ulong id, uint kind, float r, float g, float b, float a, uint scroll);

    [LibraryImport(LibraryName, EntryPoint = "mixer_output_add", StringMarshalling = StringMarshalling.Utf8)]
    internal static partial int OutputAdd(ulong outputId, uint transport, string name, uint sourceKind, ulong sourceId, ulong unitId, uint useGpu);

    [LibraryImport(LibraryName, EntryPoint = "mixer_output_remove")]
    internal static partial int OutputRemove(ulong outputId);

    [LibraryImport(LibraryName, EntryPoint = "mixer_omt_start_send", StringMarshalling = StringMarshalling.Utf8)]
    internal static partial int StartSend(ulong unitId, string name);

    [LibraryImport(LibraryName, EntryPoint = "mixer_omt_discover")]
    internal static unsafe partial int Discover(byte* buffer, nuint capacity);

    [LibraryImport(LibraryName, EntryPoint = "mixer_ndi_discover")]
    internal static unsafe partial int DiscoverNdi(byte* buffer, nuint capacity);

    [LibraryImport(LibraryName, EntryPoint = "mixer_copy_audio_peaks")]
    internal static unsafe partial int CopyAudioPeaks(AudioPeak* peaks, uint capacity);

    [LibraryImport(LibraryName, EntryPoint = "mixer_copy_stats")]
    internal static unsafe partial int CopyStats(MixerStats* stats);

    [LibraryImport(LibraryName, EntryPoint = "mixer_copy_source_usage")]
    internal static unsafe partial int CopySourceUsage(SourceUsage* usage, uint capacity);

    [LibraryImport(LibraryName, EntryPoint = "mixer_set_frame_buffer")]
    internal static partial int SetFrameBuffer(uint frames);

    [LibraryImport(LibraryName, EntryPoint = "mixer_set_monitor_present_interval")]
    internal static partial int SetMonitorPresentInterval(ulong monitorId, uint frames);

    [LibraryImport(LibraryName, EntryPoint = "mixer_last_error")]
    internal static unsafe partial int LastError(byte* buffer, nuint capacity);

    internal static string LastErrorText()
    {
        var buffer = new byte[512];
        unsafe
        {
            fixed (byte* ptr = buffer)
            {
                var n = LastError(ptr, (nuint)buffer.Length);
                return n > 0 ? Encoding.UTF8.GetString(buffer, 0, n) : string.Empty;
            }
        }
    }

    internal static string DiscoverText()
    {
        var buffer = new byte[8192];
        unsafe
        {
            fixed (byte* ptr = buffer)
            {
                var n = Discover(ptr, (nuint)buffer.Length);
                if (n <= 0)
                    return string.Empty;
                return Encoding.UTF8.GetString(buffer, 0, n);
            }
        }
    }

    internal static string DiscoverNdiText()
    {
        var buffer = new byte[8192];
        unsafe
        {
            fixed (byte* ptr = buffer)
            {
                var n = DiscoverNdi(ptr, (nuint)buffer.Length);
                if (n <= 0)
                    return string.Empty;
                return Encoding.UTF8.GetString(buffer, 0, n);
            }
        }
    }

    internal static bool TryCopyVideoInfo(ulong id, out MixerVideoInfo info)
    {
        unsafe
        {
            var local = stackalloc MixerVideoInfo[1];
            var code = CopyVideoInfo(id, local);
            info = local[0];
            return code == 0;
        }
    }

    internal static void ThrowIfFailed(int code, string action)
    {
        if (code == 0)
            return;
        var detail = LastErrorText();
        throw new InvalidOperationException(string.IsNullOrEmpty(detail) ? $"{action} failed ({code})." : $"{action}: {detail}");
    }
}

[StructLayout(LayoutKind.Sequential)]
internal struct MixerVideoInfo
{
    public uint Playing;
    public uint IsFile;
    public long PositionHns;
    public long DurationHns;
}

[StructLayout(LayoutKind.Sequential)]
internal struct Rect
{
    public float X;
    public float Y;
    public float Width;
    public float Height;
}

[StructLayout(LayoutKind.Sequential)]
internal struct OverlayDesc
{
    public ulong SourceId;
    public Rect Rect;
    public float Opacity;
    public int Z;
    public uint AudioFollow;
    public uint Pad;
}

[StructLayout(LayoutKind.Sequential)]
internal struct AudioPeak
{
    public ulong SourceId;
    public float Left;
    public float Right;
}

[StructLayout(LayoutKind.Sequential)]
internal struct MixerStats
{
    public float RenderMs;
    public float FrameBudgetMs;
}

[StructLayout(LayoutKind.Sequential)]
internal struct SourceUsage
{
    public ulong SourceId;
    public uint Width;
    public uint Height;
    public ulong RamBytes;
    public ulong VramBytes;
}

[StructLayout(LayoutKind.Sequential)]
internal struct UnitState
{
    public ulong ProgramSource;
    public ulong PreviewSource;
    public float Mix;
    public uint TransitionKind;
    public uint OverlayCount;
    public uint MvSlotCount;
    public OverlayDesc Overlay0;
    public OverlayDesc Overlay1;
    public OverlayDesc Overlay2;
    public OverlayDesc Overlay3;
    public OverlayDesc Overlay4;
    public OverlayDesc Overlay5;
    public OverlayDesc Overlay6;
    public OverlayDesc Overlay7;
    public ulong Mv0;
    public ulong Mv1;
    public ulong Mv2;
    public ulong Mv3;
    public ulong Mv4;
    public ulong Mv5;
    public ulong Mv6;
    public ulong Mv7;
    public ulong Mv8;
    public ulong Mv9;
    public ulong Mv10;
    public ulong Mv11;
    public ulong Mv12;
    public ulong Mv13;
    public ulong Mv14;
    public ulong Mv15;
}

[StructLayout(LayoutKind.Sequential)]
internal unsafe struct MixerAudioBusInfo
{
    public ulong Id;
    public uint Role;
    public uint DeviceKind;
    public int MapLeft;
    public int MapRight;
    public uint Exclusive;
    public uint Bit;
    public fixed byte Name[64];
    public fixed byte DeviceId[256];
}

[StructLayout(LayoutKind.Sequential)]
internal unsafe struct MixerAudioDeviceInfo
{
    public uint Kind;
    public uint Channels;
    public fixed byte Id[256];
    public fixed byte Name[256];
}
