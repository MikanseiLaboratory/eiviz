using System.Runtime.InteropServices;
using Eiviz.Host.Interop;

namespace Eiviz.Host.Media;

internal static class WaveOutMonitor
{
    private const int Rate = 48000;
    private const int Channels = 2;
    private const int BlockMs = 20;
    private const int BlockFrames = Rate * BlockMs / 1000;

    private static readonly object Gate = new();
    private static bool _started;

    public static void EnsureStarted()
    {
        lock (Gate)
        {
            if (_started)
                return;
            _started = true;
            var thread = new Thread(Run) { IsBackground = true, Name = "eiviz-waveout" };
            thread.Start();
        }
    }

    public static void Remove(ulong sourceId) => MixerNative.FlushAudio(sourceId);

    private static void Run()
    {
        var format = new WaveFormatEx
        {
            FormatTag = 1,
            Channels = (ushort)Channels,
            SamplesPerSec = Rate,
            BitsPerSample = 16,
            BlockAlign = (ushort)(Channels * 2),
            AvgBytesPerSec = Rate * Channels * 2
        };
        if (waveOutOpen(out var handle, 0xFFFFFFFFu, ref format, nint.Zero, nint.Zero, 0) != 0)
            return;
        var blockBytes = BlockFrames * Channels * 2;
        var size = Marshal.SizeOf<WaveHdr>();
        var buffers = new short[2][];
        var pins = new GCHandle[2];
        var headers = new nint[2];
        var mixed = new float[BlockFrames * Channels];
        try
        {
            for (var i = 0; i < 2; i++)
            {
                buffers[i] = new short[BlockFrames * Channels];
                pins[i] = GCHandle.Alloc(buffers[i], GCHandleType.Pinned);
                headers[i] = Marshal.AllocHGlobal(size);
                var hdr = new WaveHdr
                {
                    Data = pins[i].AddrOfPinnedObject(),
                    BufferLength = blockBytes
                };
                Marshal.StructureToPtr(hdr, headers[i], false);
                waveOutPrepareHeader(handle, headers[i], size);
                Mix(buffers[i], mixed);
                waveOutWrite(handle, headers[i], size);
            }
            while (true)
            {
                for (var i = 0; i < 2; i++)
                {
                    var hdr = Marshal.PtrToStructure<WaveHdr>(headers[i]);
                    if ((hdr.Flags & 1) == 0)
                        continue;
                    Mix(buffers[i], mixed);
                    hdr.BufferLength = blockBytes;
                    hdr.Flags &= ~1;
                    Marshal.StructureToPtr(hdr, headers[i], false);
                    waveOutWrite(handle, headers[i], size);
                }
                Thread.Sleep(2);
            }
        }
        finally
        {
            waveOutReset(handle);
            for (var i = 0; i < 2; i++)
            {
                if (headers[i] != nint.Zero)
                {
                    waveOutUnprepareHeader(handle, headers[i], size);
                    Marshal.FreeHGlobal(headers[i]);
                }
                if (pins[i].IsAllocated)
                    pins[i].Free();
            }
            waveOutClose(handle);
        }
    }

    private static void Mix(short[] dest, float[] mixed)
    {
        Array.Clear(mixed);
        unsafe
        {
            fixed (float* ptr = mixed)
                MixerNative.CopyFollowAudio(ptr, (uint)mixed.Length);
        }
        for (var i = 0; i < dest.Length; i++)
            dest[i] = (short)Math.Clamp((int)(mixed[i] * 32767f), short.MinValue, short.MaxValue);
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct WaveFormatEx
    {
        public ushort FormatTag;
        public ushort Channels;
        public int SamplesPerSec;
        public int AvgBytesPerSec;
        public ushort BlockAlign;
        public ushort BitsPerSample;
        public ushort Size;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct WaveHdr
    {
        public nint Data;
        public int BufferLength;
        public int BytesRecorded;
        public nint User;
        public int Flags;
        public int Loops;
        public nint Next;
        public nint Reserved;
    }

    [DllImport("winmm.dll")]
    private static extern int waveOutOpen(out nint handle, uint device, ref WaveFormatEx format, nint callback, nint instance, uint flags);

    [DllImport("winmm.dll")]
    private static extern int waveOutPrepareHeader(nint handle, nint header, int size);

    [DllImport("winmm.dll")]
    private static extern int waveOutUnprepareHeader(nint handle, nint header, int size);

    [DllImport("winmm.dll")]
    private static extern int waveOutWrite(nint handle, nint header, int size);

    [DllImport("winmm.dll")]
    private static extern int waveOutReset(nint handle);

    [DllImport("winmm.dll")]
    private static extern int waveOutClose(nint handle);
}
