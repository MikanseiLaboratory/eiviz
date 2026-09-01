using System.IO;
using System.Text;

namespace Eiviz.Host;

internal sealed class LogTail
{
    private const int BackfillBytes = 64 * 1024;
    private static readonly Encoding Utf8 = new UTF8Encoding(encoderShouldEmitUTF8Identifier: false);

    private readonly string _path;
    private long _offset = -1;
    private string _carry = "";
    private bool _dropPartial;

    internal LogTail(string path) => _path = path;

    internal IReadOnlyList<string> ReadNew()
    {
        var lines = new List<string>();
        try
        {
            if (!File.Exists(_path))
                return lines;
            using var stream = new FileStream(
                _path,
                FileMode.Open,
                FileAccess.Read,
                FileShare.ReadWrite | FileShare.Delete);
            if (_offset < 0)
            {
                _offset = Math.Max(0, stream.Length - BackfillBytes);
                _dropPartial = _offset > 0;
                _carry = "";
            }
            else if (stream.Length < _offset)
            {
                _offset = 0;
                _dropPartial = false;
                _carry = "";
            }
            if (stream.Length <= _offset)
                return lines;
            stream.Position = _offset;
            var buffer = new byte[stream.Length - _offset];
            var n = stream.Read(buffer, 0, buffer.Length);
            _offset += n;
            var text = _carry + Utf8.GetString(buffer, 0, n);
            var start = 0;
            if (_dropPartial)
            {
                var cut = text.IndexOf('\n');
                if (cut < 0)
                {
                    _carry = "";
                    return lines;
                }
                start = cut + 1;
                _dropPartial = false;
            }
            for (var i = start; i < text.Length; i++)
            {
                if (text[i] != '\n')
                    continue;
                var end = i > start && text[i - 1] == '\r' ? i - 1 : i;
                lines.Add(text[start..end]);
                start = i + 1;
            }
            _carry = start < text.Length ? text[start..] : "";
        }
        catch
        {
            // A log viewer must not disturb the mixer.
        }
        return lines;
    }
}
