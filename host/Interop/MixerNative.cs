using System.Runtime.InteropServices;
using System.Text;
using Eiviz.Host.I18n;

namespace Eiviz.Host.Interop;

internal static partial class MixerNative
{
    private const string LibraryName = "eiviz_mixer";

    internal const ulong Color = 1;
    internal const ulong Bars = 2;
    internal const ulong Black = 3;
    internal const ulong Blue = 4;
    internal const ulong IncomingPreview = 0;
    internal const ulong IncomingProgram = unchecked((ulong)-1);
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
    internal const uint TransitionWipe = 3;
    internal const uint TransitionSlide = 4;
    internal const uint TransitionPush = 5;
    internal const uint TransitionIris = 6;
    internal const uint TransitionBlinds = 7;
    internal const uint TransitionZoom = 8;
    internal const uint TransitionAdditive = 9;
    internal const uint TransitionCube = 10;
    internal const uint TransitionCrossZoom = 11;
    internal const uint TransitionFlyRotate = 12;
    internal const uint TransitionBarnDoor = 13;
    internal const uint TransitionClock = 14;
    internal const uint TransitionLorez = 15;
    internal const uint TransitionMetamix = 16;
    internal const uint TransitionTile = 17;
    internal const uint TransitionFlip = 18;
    internal const uint TransitionGlitch = 19;
    internal const uint TransitionSwirl = 20;
    internal const uint TransitionLumaMorph = 21;
    internal const uint TransitionParts = 22;
    internal const uint TransitionStatic = 23;
    internal const uint TransitionShiftRgb = 24;
    internal const uint TransitionDisplace = 25;
    internal const uint TransitionRipple = 26;
    internal const uint TransitionGridDissolve = 27;
    internal const uint TransitionCubeZoom = 28;
    internal const uint TransitionPageCurl = 29;
    internal const uint TransitionKaleidoscope = 30;
    internal const uint TransitionPolar = 31;
    internal const uint TransitionFilmBurn = 32;
    internal const uint TransitionZoomBlur = 33;
    internal const uint TransitionMultitask = 34;
    internal const uint TransitionHeart = 35;
    internal const uint TransitionDiamond = 36;
    internal const uint TransitionStar = 37;
    internal const uint TransitionRollerDoor = 38;
    internal const uint TransitionPixelSort = 39;
    internal const uint TransitionDatamosh = 40;
    internal const uint TransitionVisualDissolve = 41;
    internal const uint TransitionOpticalFlow = 42;
    internal const uint TransitionBloom = 43;
    internal const uint TransitionCustom = 50;
    internal const uint TransitionStinger = 100;
    internal const uint DurationFrames = 0;
    internal const uint DurationMs = 1;
    internal const uint EasingLinear = 0;
    internal const uint EasingIn = 1;
    internal const uint EasingOut = 2;
    internal const uint EasingInOut = 3;
    internal const uint EasingSmoothstep = 4;

    internal static string TransitionLabel(uint kind) => TransitionCatalog.Label(kind);
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
    internal static partial int Cut(ulong unitId, uint swap, ulong incomingSource);

    [LibraryImport(LibraryName, EntryPoint = "mixer_unit_auto")]
    internal static partial int Auto(
        ulong unitId,
        uint kind,
        uint durationMs,
        uint swap,
        uint keepPreview,
        uint easing,
        uint direction,
        float dipR,
        float dipG,
        float dipB,
        float dipA,
        ulong incomingSource,
        float softness,
        float param);

    [LibraryImport(LibraryName, EntryPoint = "mixer_unit_overlay_auto")]
    internal static unsafe partial int OverlayAuto(ulong unitId, uint targetEnabled, uint durationMs, OverlayDesc* desc);

    [LibraryImport(LibraryName, EntryPoint = "mixer_unit_set_custom_wgsl", StringMarshalling = StringMarshalling.Utf8)]
    internal static partial int SetCustomWgsl(ulong unitId, string? wgsl);

    [LibraryImport(LibraryName, EntryPoint = "mixer_validate_custom_wgsl", StringMarshalling = StringMarshalling.Utf8)]
    internal static partial int ValidateCustomWgsl(string? wgsl);

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
    internal static partial int VideoStart(ulong id, string path, uint capture, uint format, uint width, uint height, uint fpsNum, uint fpsDen, uint frameBufferFrames, uint preloadRam);

    [LibraryImport(LibraryName, EntryPoint = "mixer_video_enum_captures")]
    internal static unsafe partial int VideoEnumCaptures(MixerVideoCaptureInfo* devices, uint capacity);

    [LibraryImport(LibraryName, EntryPoint = "mixer_video_enum_capture_modes", StringMarshalling = StringMarshalling.Utf8)]
    internal static unsafe partial int VideoEnumCaptureModes(string deviceId, MixerVideoCaptureMode* modes, uint capacity);

    [LibraryImport(LibraryName, EntryPoint = "mixer_video_set_playing")]
    internal static partial int VideoSetPlaying(ulong id, uint playing);

    [LibraryImport(LibraryName, EntryPoint = "mixer_video_set_loop")]
    internal static partial int VideoSetLoop(ulong id, uint looping);

    [LibraryImport(LibraryName, EntryPoint = "mixer_video_seek")]
    internal static partial int VideoSeek(ulong id, long hns);

    [LibraryImport(LibraryName, EntryPoint = "mixer_video_copy_info")]
    internal static unsafe partial int CopyVideoInfo(ulong id, MixerVideoInfo* info);

    [LibraryImport(LibraryName, EntryPoint = "mixer_omt_connect", StringMarshalling = StringMarshalling.Utf8)]
    internal static partial int ConnectOmt(ulong id, string address, uint useGpu, uint frameBufferFrames, uint quality);

    [LibraryImport(LibraryName, EntryPoint = "mixer_omt_set_quality")]
    internal static partial int SetOmtQuality(ulong id, uint quality);

    [LibraryImport(LibraryName, EntryPoint = "mixer_ndi_connect", StringMarshalling = StringMarshalling.Utf8)]
    internal static partial int ConnectNdi(ulong id, string address, uint frameBufferFrames, uint lowBandwidth);

    [LibraryImport(LibraryName, EntryPoint = "mixer_set_live_save")]
    internal static partial int SetLiveSave(ulong id, uint mode, uint flags);

    [LibraryImport(LibraryName, EntryPoint = "mixer_define_scene")]
    internal static unsafe partial int DefineScene(ulong sceneId, uint width, uint height, uint count, OverlayDesc* layers);

    [LibraryImport(LibraryName, EntryPoint = "mixer_destroy_scene")]
    internal static partial int DestroyScene(ulong sceneId);

    [LibraryImport(LibraryName, EntryPoint = "mixer_define_generator")]
    internal static partial int DefineGenerator(ulong id, uint kind, float r, float g, float b, float a, uint scroll);

    [LibraryImport(LibraryName, EntryPoint = "mixer_generator_set_tone")]
    internal static partial int GeneratorSetTone(ulong id, float hz, float levelDbfs);

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

    [LibraryImport(LibraryName, EntryPoint = "mixer_copy_rebar_info")]
    internal static unsafe partial int CopyRebarInfo(MixerRebarInfo* info);

    [LibraryImport(LibraryName, EntryPoint = "mixer_set_rebar_optimization")]
    internal static partial int SetRebarOptimization(uint enabled);

    [LibraryImport(LibraryName, EntryPoint = "mixer_set_ndi_gpu_upload")]
    internal static partial int SetNdiGpuUpload(uint enabled);

    [LibraryImport(LibraryName, EntryPoint = "mixer_set_bus_colors")]
    internal static partial int SetBusColors(
        byte prvR, byte prvG, byte prvB,
        byte pgmR, byte pgmG, byte pgmB,
        byte inR, byte inG, byte inB);

    [LibraryImport(LibraryName, EntryPoint = "mixer_set_mv_label")]
    internal static partial int SetMvLabel(ulong sceneId, float size, uint percent, uint top);

    [LibraryImport(LibraryName, EntryPoint = "mixer_set_frame_buffer")]
    internal static partial int SetFrameBuffer(uint frames);

    [LibraryImport(LibraryName, EntryPoint = "mixer_set_monitor_present_interval")]
    internal static partial int SetMonitorPresentInterval(ulong monitorId, uint frames);

    [LibraryImport(LibraryName, EntryPoint = "mixer_last_error")]
    internal static unsafe partial int LastError(byte* buffer, nuint capacity);

    [LibraryImport(LibraryName, EntryPoint = "mixer_take_fatal")]
    internal static unsafe partial int TakeFatal(byte* buffer, nuint capacity);

    [LibraryImport(LibraryName, EntryPoint = "mixer_session_load", StringMarshalling = StringMarshalling.Utf8)]
    internal static unsafe partial int SessionLoad(string path, byte* buffer, nuint capacity);

    [LibraryImport(LibraryName, EntryPoint = "mixer_session_save", StringMarshalling = StringMarshalling.Utf8)]
    internal static unsafe partial int SessionSave(string path, byte* json, nuint length);

    [LibraryImport(LibraryName, EntryPoint = "mixer_session_canonicalize")]
    internal static unsafe partial int SessionCanonicalize(byte* json, nuint length, byte* buffer, nuint capacity);

    internal static string? PreloadRamWarning() => LastErrorText() switch
    {
        "preload-ram-overflow" => Loc.T("error.videoPreloadRam"),
        "preload-ram-failed" => Loc.T("error.videoPreloadFailed"),
        _ => null
    };

    internal static string LastErrorText()
    {
        var buffer = new byte[1024];
        unsafe
        {
            fixed (byte* ptr = buffer)
            {
                var n = LastError(ptr, (nuint)buffer.Length);
                return n > 0 ? Encoding.UTF8.GetString(buffer, 0, n) : string.Empty;
            }
        }
    }

    internal static string TakeFatalText()
    {
        var buffer = new byte[1024];
        unsafe
        {
            fixed (byte* ptr = buffer)
            {
                var n = TakeFatal(ptr, (nuint)buffer.Length);
                return n > 0 ? Encoding.UTF8.GetString(buffer, 0, n) : string.Empty;
            }
        }
    }

    internal static string SessionLoadText(string path)
    {
        var buffer = new byte[1 << 20];
        unsafe
        {
            fixed (byte* ptr = buffer)
            {
                var n = SessionLoad(path, ptr, (nuint)buffer.Length);
                if (n <= 0)
                    ThrowIfFailed(n == 0 ? 5 : n, "Load session");
                return Encoding.UTF8.GetString(buffer, 0, n);
            }
        }
    }

    internal static void SessionSaveText(string path, string json)
    {
        var bytes = Encoding.UTF8.GetBytes(json);
        unsafe
        {
            fixed (byte* ptr = bytes)
            {
                ThrowIfFailed(SessionSave(path, ptr, (nuint)bytes.Length), "Save session");
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

    internal static List<(string Name, string Id)> EnumVideoCaptures()
    {
        var list = new List<(string, string)>();
        var buffer = new MixerVideoCaptureInfo[64];
        unsafe
        {
            fixed (MixerVideoCaptureInfo* ptr = buffer)
            {
                var n = VideoEnumCaptures(ptr, (uint)buffer.Length);
                for (var i = 0; i < n && i < buffer.Length; i++)
                {
                    var current = ptr + i;
                    var id = ReadFixedUtf8(current->Id, 512);
                    var name = ReadFixedUtf8(current->Name, 256);
                    if (!string.IsNullOrEmpty(id) && !string.IsNullOrEmpty(name))
                        list.Add((name, id));
                }
            }
        }
        return list;
    }

    internal static List<MixerVideoCaptureMode> EnumVideoCaptureModes(string deviceId)
    {
        var list = new List<MixerVideoCaptureMode>();
        var buffer = new MixerVideoCaptureMode[64];
        unsafe
        {
            fixed (MixerVideoCaptureMode* ptr = buffer)
            {
                var n = VideoEnumCaptureModes(deviceId, ptr, (uint)buffer.Length);
                for (var i = 0; i < n && i < buffer.Length; i++)
                    list.Add(buffer[i]);
            }
        }
        return list;
    }

    private static unsafe string ReadFixedUtf8(byte* ptr, int cap)
    {
        var n = 0;
        while (n < cap && ptr[n] != 0)
            n++;
        return n == 0 ? "" : Encoding.UTF8.GetString(new ReadOnlySpan<byte>(ptr, n));
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
        throw new InvalidOperationException(Loc.Error(action, code));
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
internal unsafe struct MixerVideoCaptureInfo
{
    public fixed byte Id[512];
    public fixed byte Name[256];
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
    public Rect Crop;
    public float Opacity;
    public int Z;
    public uint AudioFollow;
    public uint Hidden;
    public nint Label;
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
    public ulong RamBytes;
    public ulong VramBytes;
    public ulong ComposeVramBytes;
    public ulong DelayVramBytes;
}

[StructLayout(LayoutKind.Sequential)]
internal unsafe struct MixerRebarInfo
{
    public uint Available;
    public uint Active;
    public uint Uma;
    public uint GpuUploadHeaps;
    public ulong BarBytes;
    public ulong VramBytes;
    public fixed byte Adapter[128];
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
    public uint TransitionEasing;
    public uint TransitionDirection;
    public uint KeepPreview;
    public uint Pad;
    public float DipR;
    public float DipG;
    public float DipB;
    public float DipA;
    public ulong IncomingSource;
    public float Softness;
    public float Param;
}

[StructLayout(LayoutKind.Sequential)]
internal struct MixerVideoCaptureMode
{
    public uint Width;
    public uint Height;
    public uint FpsNum;
    public uint FpsDen;
    public uint Format;
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
