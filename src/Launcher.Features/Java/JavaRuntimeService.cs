using System.Collections.Concurrent;
using System.ComponentModel;
using System.Text.RegularExpressions;
using Launcher.Contracts;

namespace Launcher.Plugins.Java;

public sealed class JavaRuntimeService(IPlatformEnvironment platform, IProcessService processes, ILauncherSettingsService settings, IInstanceService instances) : IJavaRuntimeService
{
    private readonly object _sync = new();
    private IReadOnlyList<JavaRuntimeInfo> _known = [];
    public IReadOnlyList<JavaRuntimeInfo> KnownRuntimes { get { lock (_sync) return _known; } }

    public async Task<IReadOnlyList<JavaRuntimeInfo>> ScanAsync(CancellationToken cancellationToken)
    {
        var paths = new List<string>();
        if (settings.Settings.Java.ExecutablePath is { } saved) paths.Add(saved);
        foreach (var instance in instances.Instances)
            if (instances.GetOverrides(instance.Id).Java?.ExecutablePath is { } path) paths.Add(path);
        paths.AddRange(await Task.Run(() => platform.FindJavaCandidates().Take(512).ToArray(), cancellationToken).ConfigureAwait(false));
        var found = new ConcurrentBag<JavaRuntimeInfo>();
        await Parallel.ForEachAsync(paths.Distinct(platform.Target.OperatingSystem == "windows" ? StringComparer.OrdinalIgnoreCase : StringComparer.Ordinal),
            new ParallelOptions { MaxDegreeOfParallelism = 4, CancellationToken = cancellationToken }, async (path, token) =>
            { if (await InspectAsync(path, token).ConfigureAwait(false) is { } runtime) found.Add(runtime); }).ConfigureAwait(false);
        var result = found.OrderByDescending(runtime => MajorVersion(runtime.Version)).ThenBy(runtime => runtime.ExecutablePath, StringComparer.Ordinal).ToArray();
        lock (_sync) _known = result;
        return result;
    }

    public async Task<JavaRuntimeInfo?> InspectAsync(string executablePath, CancellationToken cancellationToken)
    {
        if (!Path.IsPathFullyQualified(executablePath) || !File.Exists(executablePath)) return null;
        var path = platform.GetJavaProbeExecutable(executablePath);
        if (!File.Exists(path)) return null;
        using var timeout = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        timeout.CancelAfter(TimeSpan.FromSeconds(5));
        var lines = new ConcurrentQueue<string>();
        try
        {
            await using var process = processes.Start(new ProcessRequest(path, ["-XshowSettings:properties", "-version"], Path.GetDirectoryName(path)!), output =>
            { if (lines.Count < 256) lines.Enqueue(output.Text); return ValueTask.CompletedTask; });
            try
            {
                if (await process.WaitForExitAsync(timeout.Token).ConfigureAwait(false) != 0) return null;
            }
            catch (OperationCanceledException) { process.Terminate(); if (cancellationToken.IsCancellationRequested) throw; return null; }
            return ParseSettings(Path.GetFullPath(path), string.Join('\n', lines));
        }
        catch (Exception error) when (error is IOException or Win32Exception or UnauthorizedAccessException) { return null; }
    }

    public static JavaRuntimeInfo? ParseSettings(string path, string output)
    {
        var version = Regex.Match(output, @"(?m)^\s*java\.version\s*=\s*(\S+)", RegexOptions.CultureInvariant).Groups[1].Value;
        if (version.Length == 0) version = Regex.Match(output, "(?:openjdk|java) version \"([^\"]+)\"", RegexOptions.CultureInvariant).Groups[1].Value;
        var arch = Regex.Match(output, @"(?m)^\s*os\.arch\s*=\s*(\S+)", RegexOptions.CultureInvariant).Groups[1].Value;
        arch = arch.ToLowerInvariant() switch { "amd64" or "x86_64" or "x64" => "x64", "aarch64" or "arm64" => "arm64", "x86" or "i386" or "i486" or "i586" or "i686" => "x86", _ => "" };
        return MajorVersion(version) > 0 && arch.Length > 0 ? new(path, version, arch) : null;
    }

    public static int MajorVersion(string version)
    {
        var parts = version.Split('.', '-', '+', '_');
        if (parts.Length == 0 || !int.TryParse(parts[0], out var major)) return 0;
        return major == 1 && parts.Length > 1 && int.TryParse(parts[1], out var legacy) ? legacy : major;
    }
}
