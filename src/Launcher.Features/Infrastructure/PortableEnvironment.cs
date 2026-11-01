using System.Diagnostics;
using System.Runtime.InteropServices;
using Launcher.Contracts;

namespace Launcher.Plugins.Infrastructure;

/// <summary>Standard-library desktop integration; Windows adds registry and vendor discovery in its adapter.</summary>
public class PortableEnvironment : IPlatformEnvironment
{
    public virtual PlatformTarget Target { get; } = new(OperatingSystem.IsWindows() ? "windows" : OperatingSystem.IsMacOS() ? "osx" : "linux",
        RuntimeInformation.OSArchitecture.ToString().ToLowerInvariant(), Environment.OSVersion.Version.ToString());
    public virtual string DefaultDataDirectory => Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "Hydrogen");

    public virtual IEnumerable<string> FindJavaCandidates()
    {
        var executable = Target.OperatingSystem == "windows" ? "java.exe" : "java";
        foreach (var name in new[] { "JAVA_HOME", "JDK_HOME" })
            if (Environment.GetEnvironmentVariable(name) is { Length: > 0 } home) yield return Path.Combine(home, "bin", executable);
        foreach (var directory in (Environment.GetEnvironmentVariable("PATH") ?? "").Split(Path.PathSeparator, StringSplitOptions.RemoveEmptyEntries))
            yield return Path.Combine(directory.Trim('"'), executable);
        var user = Environment.GetFolderPath(Environment.SpecialFolder.UserProfile);
        foreach (var root in new[] { Path.Combine(user, ".jdks"), Path.Combine(user, ".sdkman", "candidates", "java"),
            "/usr/lib/jvm", "/usr/java", "/opt/java", "/Library/Java/JavaVirtualMachines", Path.Combine(user, "Library", "Java", "JavaVirtualMachines") })
            foreach (var candidate in JavaUnder(root, executable, 3)) yield return candidate;
        var minecraft = Target.OperatingSystem == "osx" ? Path.Combine(user, "Library", "Application Support", "minecraft") : Path.Combine(user, ".minecraft");
        foreach (var candidate in JavaUnder(Path.Combine(minecraft, "runtime"), executable, 5)) yield return candidate;
    }

    public static IEnumerable<string> JavaUnder(string root, string executable, int depth)
    {
        if (depth < 0 || !Directory.Exists(root)) return [];
        try
        {
            var own = Path.Combine(root, "bin", executable);
            var result = new List<string>();
            if (File.Exists(own)) result.Add(own);
            foreach (var directory in Directory.EnumerateDirectories(root).Take(100))
            {
                if ((File.GetAttributes(directory) & FileAttributes.ReparsePoint) != 0) continue;
                result.AddRange(JavaUnder(directory, executable, depth - 1));
            }
            return result;
        }
        catch (Exception error) when (error is IOException or UnauthorizedAccessException) { return []; }
    }

    public Task OpenBrowserAsync(Uri uri, CancellationToken cancellationToken)
    {
        cancellationToken.ThrowIfCancellationRequested();
        if (uri.Scheme != "https") throw new InvalidOperationException("An HTTPS browser URL is required.");
        using var process = Process.Start(new ProcessStartInfo(uri.AbsoluteUri) { UseShellExecute = true });
        return Task.CompletedTask;
    }
}
