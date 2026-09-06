using System.Runtime.InteropServices;

namespace Eiviz.Host;

internal static class GpuUtilization
{
    private static readonly object Gate = new();
    private static readonly Sampler Engine = new();
    private static float _last;

    public static float Percent()
    {
        lock (Gate)
        {
            try
            {
                _last = Engine.Sample();
            }
            catch
            {
                _last = 0;
            }
            return _last;
        }
    }

    public static float Last()
    {
        lock (Gate)
            return _last;
    }

    private sealed class Sampler : IDisposable
    {
        private const uint PdhFmtDouble = 0x00000200;
        private const int PdhMoreData = unchecked((int)0x800007D2);
        private const string Wild = @"\GPU Engine(*)\Utilization Percentage";

        private nint _query;
        private readonly List<nint> _counters = [];
        private readonly int _pid = Environment.ProcessId;
        private bool _primed;
        private int _emptyStreak;

        public Sampler()
        {
            if (PdhOpenQueryW(null, nint.Zero, out _query) != 0)
                _query = 0;
        }

        public float Sample()
        {
            if (_query == nint.Zero)
                return 0;
            if (_counters.Count == 0 || _emptyStreak >= 8)
            {
                Rebuild();
                _emptyStreak = 0;
            }
            if (_counters.Count == 0)
                return 0;
            if (PdhCollectQueryData(_query) != 0)
                return 0;
            if (!_primed)
            {
                _primed = true;
                return 0;
            }
            double sum = 0;
            var any = false;
            foreach (var counter in _counters)
            {
                if (PdhGetFormattedCounterValue(counter, PdhFmtDouble, out _, out var value) != 0)
                    continue;
                if (double.IsNaN(value.Value) || value.Value < 0)
                    continue;
                sum += value.Value;
                any = true;
            }
            if (!any)
                _emptyStreak++;
            return (float)Math.Clamp(sum, 0, 100);
        }

        private void Rebuild()
        {
            ClearCounters();
            var chars = 0;
            var status = PdhExpandWildCardPathW(null, Wild, nint.Zero, ref chars, 0);
            if (status != PdhMoreData && status != 0 || chars <= 1)
                return;
            var buffer = new char[chars];
            if (PdhExpandWildCardPathW(null, Wild, buffer, ref chars, 0) != 0)
                return;
            var needle = $"pid_{_pid}_";
            var start = 0;
            for (var i = 0; i < buffer.Length; i++)
            {
                if (buffer[i] != '\0')
                    continue;
                var len = i - start;
                if (len <= 0)
                    break;
                var path = new string(buffer, start, len);
                start = i + 1;
                if (!path.Contains(needle, StringComparison.OrdinalIgnoreCase))
                    continue;
                if (path.Contains("engtype_Copy", StringComparison.OrdinalIgnoreCase)
                    || path.Contains("engtype_copy", StringComparison.OrdinalIgnoreCase))
                    continue;
                if (PdhAddEnglishCounterW(_query, path, nint.Zero, out var counter) == 0)
                    _counters.Add(counter);
            }
            _primed = false;
        }

        private void ClearCounters()
        {
            foreach (var counter in _counters)
                PdhRemoveCounter(counter);
            _counters.Clear();
        }

        public void Dispose()
        {
            ClearCounters();
            if (_query != nint.Zero)
            {
                PdhCloseQuery(_query);
                _query = 0;
            }
        }

        [DllImport("pdh.dll", CharSet = CharSet.Unicode)]
        private static extern int PdhOpenQueryW(string? dataSource, nint userData, out nint query);

        [DllImport("pdh.dll", CharSet = CharSet.Unicode)]
        private static extern int PdhExpandWildCardPathW(
            string? dataSource,
            string wildCardPath,
            [Out] char[]? expanded,
            ref int pathListLength,
            uint flags);

        [DllImport("pdh.dll", CharSet = CharSet.Unicode)]
        private static extern int PdhExpandWildCardPathW(
            string? dataSource,
            string wildCardPath,
            nint expanded,
            ref int pathListLength,
            uint flags);

        [DllImport("pdh.dll", CharSet = CharSet.Unicode)]
        private static extern int PdhAddEnglishCounterW(nint query, string path, nint userData, out nint counter);

        [DllImport("pdh.dll")]
        private static extern int PdhCollectQueryData(nint query);

        [DllImport("pdh.dll")]
        private static extern int PdhGetFormattedCounterValue(
            nint counter,
            uint format,
            out uint type,
            out PdhFmtCounterValue value);

        [DllImport("pdh.dll")]
        private static extern int PdhRemoveCounter(nint counter);

        [DllImport("pdh.dll")]
        private static extern int PdhCloseQuery(nint query);

        [StructLayout(LayoutKind.Explicit)]
        private struct PdhFmtCounterValue
        {
            [FieldOffset(0)] public uint CStatus;
            [FieldOffset(8)] public double Value;
        }
    }
}
