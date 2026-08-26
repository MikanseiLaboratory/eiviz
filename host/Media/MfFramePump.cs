using System.Diagnostics;
using System.IO;
using System.Runtime.InteropServices;
using Eiviz.Host.Interop;

namespace Eiviz.Host.Media;

internal sealed class MfFramePump : IDisposable
{
    private readonly CancellationTokenSource _stop = new();
    private readonly Thread _thread;

    private MfFramePump(ulong sourceId, string url, bool isCapture)
    {
        _thread = new Thread(() =>
        {
            try { Run(sourceId, url, isCapture); }
            catch (Exception ex)
            {
                File.WriteAllText(Path.Combine(AppContext.BaseDirectory, "host-error.txt"), ex.ToString());
            }
        })
        {
            IsBackground = true,
            Name = $"eiviz-mf-{sourceId}"
        };
        _thread.Start();
    }

    public static MfFramePump StartFile(ulong sourceId, string path)
    {
        if (!File.Exists(path))
            throw new FileNotFoundException("Video file not found.", path);
        return new MfFramePump(sourceId, path, false);
    }

    public static MfFramePump StartCapture(ulong sourceId, string symbolicLink) =>
        new(sourceId, symbolicLink, true);

    public static List<(string Name, string Link)> EnumerateCameras()
    {
        var list = new List<(string, string)>();
        Native.EnsureStarted();
        if (Native.MFCreateAttributes(out var attrs, 1) != 0 || attrs == nint.Zero)
            return list;
        try
        {
            var hr = Native.IMFAttributesSetGUID(
                attrs,
                Native.MfDevsourceAttributeSourceType,
                Native.MfDevsourceAttributeSourceTypeVidcap);
            if (hr != 0)
                return list;
            hr = Native.MFEnumDeviceSources(attrs, out var devices, out var count);
            if (hr != 0 || devices == nint.Zero)
                return list;
            try
            {
                for (var i = 0; i < count; i++)
                {
                    var activate = Marshal.ReadIntPtr(devices, i * nint.Size);
                    if (activate == nint.Zero)
                        continue;
                    var name = Native.GetString(activate, Native.MfDevsourceAttributeFriendlyName);
                    var link = Native.GetString(activate, Native.MfDevsourceAttributeSourceTypeVidcapSymbolic);
                    if (!string.IsNullOrEmpty(name) && !string.IsNullOrEmpty(link))
                        list.Add((name, link));
                    Marshal.Release(activate);
                }
            }
            finally
            {
                Marshal.FreeCoTaskMem(devices);
            }
        }
        finally
        {
            Marshal.Release(attrs);
        }
        return list;
    }

    private void Run(ulong sourceId, string url, bool isCapture)
    {
        Native.EnsureStarted();
        var reader = nint.Zero;
        var hr = isCapture
            ? Native.OpenCapture(url, out reader)
            : Native.OpenFile(url, out reader);
        if (hr != 0 || reader == nint.Zero)
            throw new InvalidOperationException($"Media Foundation could not open the source ({hr:X8}).");
        try
        {
            Native.SelectVideoStream(reader);
            var format = Native.ConfigureOutput(reader);
            var registered = false;
            uint registeredWidth = 0, registeredHeight = 0;
            long originPts = -1;
            var clock = Stopwatch.StartNew();
            while (!_stop.IsCancellationRequested)
            {
                hr = Native.ReadFrame(reader, format, out var width, out var height, out var stride, out var pixels, out var pts, out var endOfStream);
                if (hr != 0)
                {
                    File.WriteAllText(Path.Combine(AppContext.BaseDirectory, "host-error.txt"),
                        $"Video read failed ({hr:X8}) for {url}");
                    break;
                }
                if (endOfStream)
                {
                    Native.Restart(ref reader, url, isCapture, out format);
                    originPts = -1;
                    clock.Restart();
                    continue;
                }
                if (pixels is null)
                    continue;
                if (!registered || registeredWidth != width || registeredHeight != height)
                {
                    MixerNative.ThrowIfFailed(
                        MixerNative.RegisterSource(sourceId, width, height, MixerNative.FormatBgra),
                        "Register video source");
                    registered = true;
                    registeredWidth = width;
                    registeredHeight = height;
                }
                if (!isCapture)
                {
                    if (originPts < 0)
                        originPts = pts;
                    var due = TimeSpan.FromTicks(Math.Max(0, pts - originPts));
                    var wait = due - clock.Elapsed;
                    if (wait > TimeSpan.Zero && wait < TimeSpan.FromSeconds(2))
                        _stop.Token.WaitHandle.WaitOne(wait);
                }
                unsafe
                {
                    fixed (byte* ptr = pixels)
                    {
                        MixerNative.PushFrame(sourceId, ptr, (uint)Math.Abs(stride), height, pts);
                    }
                }
            }
        }
        finally
        {
            Marshal.Release(reader);
        }
    }

    public void Dispose()
    {
        _stop.Cancel();
        if (_thread.IsAlive)
            _thread.Join(500);
        _stop.Dispose();
    }

    private static class Native
    {
        private static int _startup;

        internal static readonly Guid MfDevsourceAttributeSourceType = new("c60ac5fe-252a-478f-a0ef-bc8fa5f67663");
        internal static readonly Guid MfDevsourceAttributeSourceTypeVidcap = new("8ac3587a-4ae7-42d8-99e0-0a6013eef90f");
        internal static readonly Guid MfDevsourceAttributeFriendlyName = new("60d0e559-52f8-4fa2-bbce-acdb34a8ec01");
        internal static readonly Guid MfDevsourceAttributeSourceTypeVidcapSymbolic = new("58f0aad8-22bf-4f8a-bb3d-d2c4978c6e2f");
        internal static readonly Guid MfMediaTypeVideo = new("73646976-0000-0010-8000-00aa00389b71");
        internal static readonly Guid MfVideoFormatRgb32 = new("00000016-0000-0010-8000-00aa00389b71");
        internal static readonly Guid MfMtMajorType = new("48eba18e-f8c9-4687-bf11-0a74c9f96a8f");
        internal static readonly Guid MfMtSubtype = new("f7e34c9a-42e8-4714-b74b-cb29d72c35e5");
        internal static readonly Guid MfMtFrameSize = new("1652c33d-d6b2-4012-b834-72030849a37d");
        internal static readonly Guid MfMtDefaultStride = new("644b4e48-1e02-4516-b0eb-c01ca9d49ac6");
        internal static readonly Guid MfSourceReaderEnableVideoProcessing = new("0fb3944b-ccb7-4e27-a219-596fbbca6958");
        internal static readonly Guid MfSourceReaderEnableAdvancedVideoProcessing = new("f81dda94-7ffb-453d-8133-614d645aeb2d");
        internal static readonly Guid MfReadwriteEnableHardwareTransforms = new("a634a91c-822b-41b9-a494-4de4643617cd");
        internal static readonly Guid MfVideoFormatNv12 = new("3231564e-0000-0010-8000-00aa00389b71");
        internal static readonly Guid MfVideoFormatYuy2 = new("32595559-0000-0010-8000-00aa00389b71");
        internal const int MfSourceReaderAllStreams = unchecked((int)0xFFFFFFFE);
        internal const int MfSourceReaderFirstVideo = unchecked((int)0xFFFFFFFC);

        internal enum PixelPack
        {
            Bgra,
            Nv12,
            Yuy2
        }

        internal static void EnsureStarted()
        {
            if (Interlocked.CompareExchange(ref _startup, 1, 0) == 0)
            {
                var hr = MFStartup(0x00020070, 1);
                if (hr != 0)
                    throw new InvalidOperationException($"MFStartup failed ({hr:X8}).");
            }
        }

        internal static int OpenCapture(string symbolicLink, out nint reader)
        {
            reader = nint.Zero;
            var hr = MFCreateAttributes(out var attrs, 2);
            if (hr != 0)
                return hr;
            try
            {
                hr = IMFAttributesSetGUID(attrs, MfDevsourceAttributeSourceType, MfDevsourceAttributeSourceTypeVidcap);
                if (hr != 0)
                    return hr;
                hr = IMFAttributesSetString(attrs, MfDevsourceAttributeSourceTypeVidcapSymbolic, symbolicLink);
                if (hr != 0)
                    return hr;
                hr = MFCreateDeviceSource(attrs, out var source);
                if (hr != 0)
                    return hr;
                try
                {
                    return MFCreateSourceReaderFromMediaSource(source, nint.Zero, out reader);
                }
                finally
                {
                    Marshal.Release(source);
                }
            }
            finally
            {
                Marshal.Release(attrs);
            }
        }

        internal static void SelectVideoStream(nint reader)
        {
            IMFSourceReaderSetStreamSelection(reader, MfSourceReaderAllStreams, 0);
            IMFSourceReaderSetStreamSelection(reader, MfSourceReaderFirstVideo, 1);
        }

        internal static PixelPack ConfigureOutput(nint reader)
        {
            SelectVideoStream(reader);
            if (TrySetSubtype(reader, MfVideoFormatRgb32))
                return PixelPack.Bgra;
            if (TrySetSubtype(reader, MfVideoFormatNv12))
                return PixelPack.Nv12;
            if (TrySetSubtype(reader, MfVideoFormatYuy2))
                return PixelPack.Yuy2;
            throw new InvalidOperationException("Media Foundation could not decode this file to RGB32, NV12, or YUY2.");
        }

        private static bool TrySetSubtype(nint reader, Guid subtype)
        {
            var hr = MFCreateMediaType(out var type);
            if (hr != 0)
                return false;
            try
            {
                if (IMFAttributesSetGUID(type, MfMtMajorType, MfMediaTypeVideo) != 0)
                    return false;
                if (IMFAttributesSetGUID(type, MfMtSubtype, subtype) != 0)
                    return false;
                return IMFSourceReaderSetCurrentMediaType(reader, MfSourceReaderFirstVideo, nint.Zero, type) == 0;
            }
            finally
            {
                Marshal.Release(type);
            }
        }

        internal static int OpenFile(string path, out nint reader)
        {
            reader = nint.Zero;
            var uri = path.StartsWith("file:", StringComparison.OrdinalIgnoreCase)
                ? path
                : new Uri(path).AbsoluteUri;
            var hr = MFCreateAttributes(out var attrs, 4);
            if (hr != 0)
                return hr;
            try
            {
                hr = IMFAttributesSetUINT32(attrs, MfReadwriteEnableHardwareTransforms, 1);
                if (hr != 0)
                    return hr;
                hr = IMFAttributesSetUINT32(attrs, MfSourceReaderEnableAdvancedVideoProcessing, 1);
                if (hr != 0)
                    return hr;
                hr = IMFAttributesSetUINT32(attrs, MfSourceReaderEnableVideoProcessing, 1);
                if (hr != 0)
                    return hr;
                hr = MFCreateSourceReaderFromURL(uri, attrs, out reader);
                if (hr == 0 && reader != nint.Zero)
                    return hr;
                return MFCreateSourceReaderFromURL(path, attrs, out reader);
            }
            finally
            {
                Marshal.Release(attrs);
            }
        }

        internal static void Restart(ref nint reader, string url, bool isCapture, out PixelPack format)
        {
            if (reader != nint.Zero)
            {
                Marshal.Release(reader);
                reader = nint.Zero;
            }
            var hr = isCapture ? OpenCapture(url, out reader) : OpenFile(url, out reader);
            if (hr != 0 || reader == nint.Zero)
                throw new InvalidOperationException($"Media Foundation restart failed ({hr:X8}).");
            format = ConfigureOutput(reader);
        }

        internal static int ReadFrame(nint reader, PixelPack pack, out uint width, out uint height, out int stride, out byte[]? pixels, out long pts, out bool endOfStream)
        {
            width = height = 0;
            stride = 0;
            pixels = null;
            pts = 0;
            endOfStream = false;
            var hr = IMFSourceReaderReadSample(
                reader,
                MfSourceReaderFirstVideo,
                0,
                out _,
                out var flags,
                out pts,
                out var sample);
            if (hr != 0)
                return hr;
            const int EndOfStream = 0x2;
            const int StreamTick = 0x100;
            if ((flags & EndOfStream) != 0)
            {
                if (sample != nint.Zero)
                    Marshal.Release(sample);
                endOfStream = true;
                return 0;
            }
            if ((flags & StreamTick) != 0 || sample == nint.Zero)
            {
                if (sample != nint.Zero)
                    Marshal.Release(sample);
                return 0;
            }
            try
            {
                hr = IMFSampleConvertToContiguousBuffer(sample, out var buffer);
                if (hr != 0)
                    return hr;
                try
                {
                    hr = IMFMediaBufferLock(buffer, out var data, out _, out var length);
                    if (hr != 0)
                        return hr;
                    try
                    {
                        ReadFrameSize(reader, out width, out height, out stride);
                        if (width == 0 || height == 0)
                            return 1;
                        pixels = pack switch
                        {
                            PixelPack.Nv12 => Nv12ToBgra(data, length, width, height, stride),
                            PixelPack.Yuy2 => Yuy2ToBgra(data, length, width, height, stride),
                            _ => CopyBgra(data, length, width, height, ref stride)
                        };
                        stride = (int)width * 4;
                        return pixels is null ? 1 : 0;
                    }
                    finally
                    {
                        IMFMediaBufferUnlock(buffer);
                    }
                }
                finally
                {
                    Marshal.Release(buffer);
                }
            }
            finally
            {
                Marshal.Release(sample);
            }
        }

        private static byte[] CopyBgra(nint data, int length, uint width, uint height, ref int stride)
        {
            if (stride == 0)
                stride = (int)width * 4;
            var row = Math.Abs(stride);
            var pixels = new byte[row * height];
            if (stride >= 0)
            {
                Marshal.Copy(data, pixels, 0, (int)Math.Min(length, pixels.Length));
            }
            else
            {
                for (var y = 0; y < height; y++)
                {
                    var src = nint.Add(data, ((int)height - 1 - y) * row);
                    Marshal.Copy(src, pixels, y * row, row);
                }
                stride = row;
            }
            return pixels;
        }

        private static byte[]? Nv12ToBgra(nint data, int length, uint width, uint height, int stride)
        {
            var yStride = stride == 0 ? (int)width : Math.Abs(stride);
            var ySize = yStride * (int)height;
            var needed = ySize + yStride * (int)height / 2;
            if (length < needed)
                return null;
            var src = new byte[needed];
            Marshal.Copy(data, src, 0, needed);
            var bgra = new byte[width * height * 4];
            var w = (int)width;
            var h = (int)height;
            for (var y = 0; y < h; y++)
            {
                var yOff = y * yStride;
                var uvOff = ySize + (y / 2) * yStride;
                var dstOff = y * w * 4;
                for (var x = 0; x < w; x++)
                {
                    var uvx = uvOff + (x & ~1);
                    YuvToBgra(src[yOff + x], src[uvx], src[uvx + 1], bgra, dstOff + x * 4);
                }
            }
            return bgra;
        }

        private static byte[]? Yuy2ToBgra(nint data, int length, uint width, uint height, int stride)
        {
            var row = stride == 0 ? (int)width * 2 : Math.Abs(stride);
            var needed = row * (int)height;
            if (length < needed)
                return null;
            var src = new byte[needed];
            Marshal.Copy(data, src, 0, needed);
            var bgra = new byte[width * height * 4];
            var w = (int)width;
            var h = (int)height;
            for (var y = 0; y < h; y++)
            {
                var srcOff = y * row;
                var dstOff = y * w * 4;
                for (var x = 0; x + 1 < w; x += 2)
                {
                    var off = srcOff + x * 2;
                    YuvToBgra(src[off], src[off + 1], src[off + 3], bgra, dstOff + x * 4);
                    YuvToBgra(src[off + 2], src[off + 1], src[off + 3], bgra, dstOff + (x + 1) * 4);
                }
            }
            return bgra;
        }

        private static void YuvToBgra(int y, int u, int v, byte[] dest, int offset)
        {
            var c = y - 16;
            var d = u - 128;
            var e = v - 128;
            dest[offset + 2] = ClampByte((298 * c + 409 * e + 128) >> 8);
            dest[offset + 1] = ClampByte((298 * c - 100 * d - 208 * e + 128) >> 8);
            dest[offset] = ClampByte((298 * c + 516 * d + 128) >> 8);
            dest[offset + 3] = 255;
        }

        private static byte ClampByte(int value) => (byte)Math.Clamp(value, 0, 255);

        internal static string GetString(nint attrs, Guid key)
        {
            var buffer = new char[512];
            var hr = IMFAttributesGetString(attrs, key, buffer, buffer.Length, out var length);
            if (hr != 0 || length <= 0)
                return string.Empty;
            return new string(buffer, 0, length);
        }

        private static void ReadFrameSize(nint reader, out uint width, out uint height, out int stride)
        {
            width = height = 0;
            stride = 0;
            var hr = IMFSourceReaderGetCurrentMediaType(reader, unchecked((int)0xFFFFFFFC), out var type);
            if (hr != 0)
                return;
            try
            {
                hr = IMFAttributesGetUINT64(type, MfMtFrameSize, out var packed);
                if (hr == 0)
                {
                    width = (uint)(packed >> 32);
                    height = (uint)(packed & 0xFFFFFFFF);
                }
                IMFAttributesGetUINT32(type, MfMtDefaultStride, out var s);
                stride = unchecked((int)s);
            }
            finally
            {
                Marshal.Release(type);
            }
        }

        [DllImport("mfplat.dll")] private static extern int MFStartup(int version, int flags);
        [DllImport("mfplat.dll")] internal static extern int MFCreateAttributes(out nint attrs, uint size);
        [DllImport("mfplat.dll")] internal static extern int MFCreateMediaType(out nint type);
        [DllImport("mf.dll")] internal static extern int MFEnumDeviceSources(nint attrs, out nint devices, out uint count);
        [DllImport("mf.dll")] internal static extern int MFCreateDeviceSource(nint attrs, out nint source);
        [DllImport("mfreadwrite.dll", CharSet = CharSet.Unicode)]
        internal static extern int MFCreateSourceReaderFromURL(string url, nint attrs, out nint reader);
        [DllImport("mfreadwrite.dll")]
        internal static extern int MFCreateSourceReaderFromMediaSource(nint source, nint attrs, out nint reader);

        internal static int IMFAttributesSetGUID(nint attrs, Guid key, Guid value) =>
            Invoke(attrs, 24, key, value);

        internal static int IMFAttributesSetUINT32(nint attrs, Guid key, uint value)
        {
            var vtable = Marshal.ReadIntPtr(attrs);
            var fn = Marshal.ReadIntPtr(vtable, 21 * nint.Size);
            var dlg = Marshal.GetDelegateForFunctionPointer<SetU32Dlg>(fn);
            return dlg(attrs, ref key, value);
        }

        internal static int IMFAttributesSetString(nint attrs, Guid key, string value) =>
            InvokeString(attrs, 25, key, value);

        internal static int IMFAttributesGetString(nint attrs, Guid key, char[] buffer, int cap, out int length)
        {
            length = 0;
            var vtable = Marshal.ReadIntPtr(attrs);
            var fn = Marshal.ReadIntPtr(vtable, 12 * nint.Size); // GetString is slot 9 + 3
            var dlg = Marshal.GetDelegateForFunctionPointer<GetStringDlg>(fn);
            return dlg(attrs, ref key, buffer, cap, out length);
        }

        internal static int IMFAttributesGetUINT64(nint attrs, Guid key, out ulong value)
        {
            var vtable = Marshal.ReadIntPtr(attrs);
            var fn = Marshal.ReadIntPtr(vtable, 8 * nint.Size); // GetUINT64 slot 5 + 3
            var dlg = Marshal.GetDelegateForFunctionPointer<GetU64Dlg>(fn);
            return dlg(attrs, ref key, out value);
        }

        internal static int IMFAttributesGetUINT32(nint attrs, Guid key, out uint value)
        {
            var vtable = Marshal.ReadIntPtr(attrs);
            var fn = Marshal.ReadIntPtr(vtable, 7 * nint.Size); // GetUINT32 slot 4 + 3
            var dlg = Marshal.GetDelegateForFunctionPointer<GetU32Dlg>(fn);
            return dlg(attrs, ref key, out value);
        }

        internal static int IMFSourceReaderSetStreamSelection(nint reader, int stream, int selected)
        {
            var vtable = Marshal.ReadIntPtr(reader);
            var fn = Marshal.ReadIntPtr(vtable, 4 * nint.Size);
            var dlg = Marshal.GetDelegateForFunctionPointer<SetStreamDlg>(fn);
            return dlg(reader, stream, selected);
        }

        internal static int IMFSourceReaderSetCurrentMediaType(nint reader, int stream, nint reserved, nint type)
        {
            var vtable = Marshal.ReadIntPtr(reader);
            var fn = Marshal.ReadIntPtr(vtable, 7 * nint.Size); // slot 4 + 3
            var dlg = Marshal.GetDelegateForFunctionPointer<SetTypeDlg>(fn);
            return dlg(reader, stream, reserved, type);
        }

        internal static int IMFSourceReaderGetCurrentMediaType(nint reader, int stream, out nint type)
        {
            var vtable = Marshal.ReadIntPtr(reader);
            var fn = Marshal.ReadIntPtr(vtable, 6 * nint.Size); // slot 3 + 3
            var dlg = Marshal.GetDelegateForFunctionPointer<GetTypeDlg>(fn);
            return dlg(reader, stream, out type);
        }

        internal static int IMFSourceReaderReadSample(nint reader, int stream, int control, out int actual, out int flags, out long timestamp, out nint sample)
        {
            var vtable = Marshal.ReadIntPtr(reader);
            var fn = Marshal.ReadIntPtr(vtable, 9 * nint.Size); // slot 6 + 3
            var dlg = Marshal.GetDelegateForFunctionPointer<ReadSampleDlg>(fn);
            return dlg(reader, stream, control, out actual, out flags, out timestamp, out sample);
        }

        internal static int IMFSampleConvertToContiguousBuffer(nint sample, out nint buffer)
        {
            var vtable = Marshal.ReadIntPtr(sample);
            var fn = Marshal.ReadIntPtr(vtable, 41 * nint.Size);
            var dlg = Marshal.GetDelegateForFunctionPointer<ConvertDlg>(fn);
            return dlg(sample, out buffer);
        }

        internal static int IMFMediaBufferLock(nint buffer, out nint data, out int max, out int current)
        {
            var vtable = Marshal.ReadIntPtr(buffer);
            var fn = Marshal.ReadIntPtr(vtable, 3 * nint.Size);
            var dlg = Marshal.GetDelegateForFunctionPointer<LockDlg>(fn);
            return dlg(buffer, out data, out max, out current);
        }

        internal static int IMFMediaBufferUnlock(nint buffer)
        {
            var vtable = Marshal.ReadIntPtr(buffer);
            var fn = Marshal.ReadIntPtr(vtable, 4 * nint.Size);
            var dlg = Marshal.GetDelegateForFunctionPointer<UnlockDlg>(fn);
            return dlg(buffer);
        }

        private static int Invoke(nint obj, int slot, Guid key, Guid value)
        {
            var vtable = Marshal.ReadIntPtr(obj);
            var fn = Marshal.ReadIntPtr(vtable, slot * nint.Size);
            var dlg = Marshal.GetDelegateForFunctionPointer<SetGuidDlg>(fn);
            return dlg(obj, ref key, ref value);
        }

        private static int InvokeString(nint obj, int slot, Guid key, string value)
        {
            var vtable = Marshal.ReadIntPtr(obj);
            var fn = Marshal.ReadIntPtr(vtable, slot * nint.Size);
            var dlg = Marshal.GetDelegateForFunctionPointer<SetStringDlg>(fn);
            return dlg(obj, ref key, value);
        }

        [UnmanagedFunctionPointer(CallingConvention.StdCall)]
        private delegate int SetU32Dlg(nint obj, ref Guid key, uint value);
        [UnmanagedFunctionPointer(CallingConvention.StdCall)]
        private delegate int SetGuidDlg(nint obj, ref Guid key, ref Guid value);
        [UnmanagedFunctionPointer(CallingConvention.StdCall)]
        private delegate int SetStringDlg(nint obj, ref Guid key, [MarshalAs(UnmanagedType.LPWStr)] string value);
        [UnmanagedFunctionPointer(CallingConvention.StdCall)]
        private delegate int GetStringDlg(nint obj, ref Guid key, [MarshalAs(UnmanagedType.LPArray)] char[] buffer, int cap, out int length);
        [UnmanagedFunctionPointer(CallingConvention.StdCall)]
        private delegate int GetU64Dlg(nint obj, ref Guid key, out ulong value);
        [UnmanagedFunctionPointer(CallingConvention.StdCall)]
        private delegate int GetU32Dlg(nint obj, ref Guid key, out uint value);
        [UnmanagedFunctionPointer(CallingConvention.StdCall)]
        private delegate int SetStreamDlg(nint obj, int stream, int selected);
        [UnmanagedFunctionPointer(CallingConvention.StdCall)]
        private delegate int SetTypeDlg(nint obj, int stream, nint reserved, nint type);
        [UnmanagedFunctionPointer(CallingConvention.StdCall)]
        private delegate int GetTypeDlg(nint obj, int stream, out nint type);
        [UnmanagedFunctionPointer(CallingConvention.StdCall)]
        private delegate int ReadSampleDlg(nint obj, int stream, int control, out int actual, out int flags, out long timestamp, out nint sample);
        [UnmanagedFunctionPointer(CallingConvention.StdCall)]
        private delegate int ConvertDlg(nint obj, out nint buffer);
        [UnmanagedFunctionPointer(CallingConvention.StdCall)]
        private delegate int LockDlg(nint obj, out nint data, out int max, out int current);
        [UnmanagedFunctionPointer(CallingConvention.StdCall)]
        private delegate int UnlockDlg(nint obj);
    }
}
