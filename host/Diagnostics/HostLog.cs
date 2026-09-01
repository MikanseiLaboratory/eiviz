using System.IO;
using System.Runtime.InteropServices;

namespace Eiviz.Host;

internal static class HostLog
{
    internal static event Action<string>? LineWritten;

    internal static string DirectoryPath =>
        Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "eiviz");

    internal static string HostFilePath => Path.Combine(DirectoryPath, "eiviz-host.log");

    internal static string MixerFilePath => Path.Combine(DirectoryPath, "eiviz-mixer.log");

    internal static void Install()
    {
        Directory.CreateDirectory(DirectoryPath);
        AppDomain.CurrentDomain.UnhandledException += (_, args) =>
        {
            if (args.ExceptionObject is Exception ex)
                WriteCrash(ex);
            else
                Write("ERROR", args.ExceptionObject?.ToString() ?? "unhandled");
            TryWriteMinidump();
        };
        TaskScheduler.UnobservedTaskException += (_, args) =>
        {
            WriteException(args.Exception);
            args.SetObserved();
        };
        Write("INFO", "host log init");
    }

    internal static void Write(string level, string message)
    {
        var line = $"{DateTime.UtcNow:O} {level} {message}{Environment.NewLine}";
        try
        {
            Directory.CreateDirectory(DirectoryPath);
            File.AppendAllText(HostFilePath, line);
            if (level is "ERROR")
                File.AppendAllText(Path.Combine(AppContext.BaseDirectory, "host-error.txt"), line);
            LineWritten?.Invoke(line.TrimEnd('\r', '\n'));
        }
        catch
        {
            // Logging must not throw back into the mixer/UI path.
        }
    }

    internal static void WriteException(Exception ex) => Write("ERROR", ex.ToString());

    internal static void WriteCrash(Exception ex)
    {
        Write("ERROR", ex.ToString());
        try
        {
            File.AppendAllText(Path.Combine(DirectoryPath, "eiviz-crash.log"), ex + Environment.NewLine);
        }
        catch
        {
        }
    }

    private static void TryWriteMinidump()
    {
        try
        {
            var path = Path.Combine(DirectoryPath, "eiviz-crash.dmp");
            using var file = new FileStream(path, FileMode.Create, FileAccess.ReadWrite);
            MiniDumpWriteDump(
                NativeMethods.GetCurrentProcess(),
                NativeMethods.GetCurrentProcessId(),
                file.SafeFileHandle,
                0,
                nint.Zero,
                nint.Zero,
                nint.Zero);
        }
        catch
        {
        }
    }

    [DllImport("dbghelp.dll", SetLastError = true)]
    private static extern bool MiniDumpWriteDump(
        nint process,
        uint processId,
        Microsoft.Win32.SafeHandles.SafeFileHandle file,
        uint dumpType,
        nint exceptionParam,
        nint userStreamParam,
        nint callbackParam);

    private static class NativeMethods
    {
        [DllImport("kernel32.dll")]
        internal static extern nint GetCurrentProcess();

        [DllImport("kernel32.dll")]
        internal static extern uint GetCurrentProcessId();
    }
}
