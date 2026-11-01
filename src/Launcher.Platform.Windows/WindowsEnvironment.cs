using System.Runtime.Versioning;
using System.Security.Cryptography;
using System.Text;
using Launcher.Contracts;
using Microsoft.Win32;

namespace Launcher.Platform.Windows;

[SupportedOSPlatform("windows")]
public sealed class WindowsEnvironment(IPlatformEnvironment common) : IPlatformEnvironment
{
    public PlatformTarget Target => common.Target;
    public string DefaultDataDirectory => Path.Combine(AppContext.BaseDirectory, "LauncherData");
    public string DataLocationFile => Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "Hydrogen", "Locations",
        Convert.ToHexString(SHA256.HashData(Encoding.UTF8.GetBytes(Path.GetFullPath(AppContext.BaseDirectory).ToUpperInvariant())))[..24] + ".json");
    public Task OpenBrowserAsync(Uri uri, CancellationToken cancellationToken) => common.OpenBrowserAsync(uri, cancellationToken);
    public string GetJavaProbeExecutable(string path) => Path.GetFileName(path).Equals("javaw.exe", StringComparison.OrdinalIgnoreCase)
        ? Path.Combine(Path.GetDirectoryName(path)!, "java.exe") : path;

    public IEnumerable<string> FindJavaCandidates()
    {
        var candidates = new List<string>(common.FindJavaCandidates());
        foreach (var view in new[] { RegistryView.Registry64, RegistryView.Registry32 })
            foreach (var keyPath in new[] { @"SOFTWARE\JavaSoft\Java Runtime Environment", @"SOFTWARE\JavaSoft\Java Development Kit", @"SOFTWARE\JavaSoft\JRE", @"SOFTWARE\JavaSoft\JDK" })
            {
                try
                {
                    using var machine = RegistryKey.OpenBaseKey(RegistryHive.LocalMachine, view);
                    using var key = machine.OpenSubKey(keyPath);
                    if (key is null) continue;
                    foreach (var version in key.GetSubKeyNames())
                    {
                        using var runtime = key.OpenSubKey(version);
                        if (runtime?.GetValue("JavaHome") is string home) candidates.Add(Path.Combine(home, "bin", "java.exe"));
                    }
                }
                catch (Exception error) when (error is System.Security.SecurityException or UnauthorizedAccessException or IOException) { }
            }
        foreach (var programFiles in new[] { Environment.GetFolderPath(Environment.SpecialFolder.ProgramFiles), Environment.GetFolderPath(Environment.SpecialFolder.ProgramFilesX86) })
            foreach (var vendor in new[] { "Java", "Eclipse Adoptium", "Eclipse Foundation", "Microsoft", "Zulu", "BellSoft", "Amazon Corretto", "Semeru" })
                candidates.AddRange(Under(Path.Combine(programFiles, vendor), 3));
        candidates.AddRange(Under(Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData), ".minecraft", "runtime"), 5));
        candidates.AddRange(Under(Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "Programs", "Eclipse Adoptium"), 3));
        return candidates.Distinct(StringComparer.OrdinalIgnoreCase);
    }

    private static IEnumerable<string> Under(string root, int depth)
    {
        var result = new List<string>();
        if (depth < 0 || !Directory.Exists(root)) return result;
        try
        {
            var executable = Path.Combine(root, "bin", "java.exe");
            if (File.Exists(executable)) result.Add(executable);
            foreach (var directory in Directory.EnumerateDirectories(root).Take(100))
                if ((File.GetAttributes(directory) & FileAttributes.ReparsePoint) == 0) result.AddRange(Under(directory, depth - 1));
        }
        catch (Exception error) when (error is IOException or UnauthorizedAccessException) { }
        return result;
    }
}
