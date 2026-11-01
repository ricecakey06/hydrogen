namespace Launcher.Contracts;

/// <summary>Platform-specific discovery and system integration, selected by the release composition.</summary>
public interface IPlatformEnvironment
{
    PlatformTarget Target { get; }
    string DefaultDataDirectory { get; }
    string DefaultGameDirectory => Path.Combine(AppContext.BaseDirectory, ".minecraft");
    string DataLocationFile => Path.Combine(DefaultDataDirectory, "data-location.json");
    IEnumerable<string> FindJavaCandidates();
    string GetJavaProbeExecutable(string path) => path;
    Task OpenBrowserAsync(Uri uri, CancellationToken cancellationToken);
}

public sealed record PlatformTarget(string OperatingSystem, string Architecture, string Version)
{
    public char ClasspathSeparator => OperatingSystem == "windows" ? ';' : ':';
}

/// <summary>Schema-versioned, atomic JSON storage. Usage leases prevent changing roots during an operation or game.</summary>
public interface ILauncherStorage
{
    string Root { get; }
    string GameRoot => Path.Combine(Root, ".minecraft");
    string? Problem { get; }
    string Resolve(string relativePath);
    IDisposable AcquireUsage();
    Task<T?> ReadAsync<T>(string relativePath, CancellationToken cancellationToken);
    Task WriteAsync<T>(string relativePath, T value, CancellationToken cancellationToken);
}

public sealed record DataDirectoryChanged(string Directory);

/// <summary>Only encrypted or session-only storage is permitted. Implemented in the platform adapter.</summary>
public interface ICredentialStore
{
    bool IsPersistent { get; }
    Task<byte[]?> ReadAsync(CancellationToken cancellationToken);
    Task SaveAsync(byte[] credentials, CancellationToken cancellationToken);
    Task DeleteAsync(CancellationToken cancellationToken);
}

public interface INetworkService
{
    Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken cancellationToken);
}

/// <summary>Game GETs use transport settings separate from authentication requests.</summary>
public static class NetworkRequestOptions
{
    public static HttpRequestOptionsKey<bool> GameDownload { get; } = new("Hydrogen.GameDownload");
}

public sealed record DownloadFile(Uri Url, string Destination, string Sha1, long? Size, IProgress<long>? Progress = null)
{
    // Keep the original constructor and deconstruction shape for existing compiled plugins.
    public DownloadFile(Uri url, string destination, string sha1, long? size) : this(url, destination, sha1, size, null) { }
    public void Deconstruct(out Uri url, out string destination, out string sha1, out long? size)
    { url = Url; destination = Destination; sha1 = Sha1; size = Size; }
}

public interface IProcessService
{
    /// <summary>Uses an argument array, never a shell. Disposing a handle does not kill the process.</summary>
    IRunningProcess Start(ProcessRequest request, Func<ProcessOutput, ValueTask>? output = null);
}

public sealed record ProcessRequest(string Executable, IReadOnlyList<string> Arguments, string WorkingDirectory,
    IReadOnlyDictionary<string, string?>? Environment = null);
public sealed record ProcessOutput(string Text, bool IsError);

public interface IRunningProcess : IAsyncDisposable
{
    int Id { get; }
    Task<int> WaitForExitAsync(CancellationToken cancellationToken);
    void Terminate();
}

public interface IDiagnosticsService
{
    Task<ILauncherLog> CreateLogAsync(string category, CancellationToken cancellationToken);
}

public interface ILauncherLog : IAsyncDisposable
{
    string Path { get; }
    void AddSecrets(params string[] secrets);
    ValueTask WriteAsync(string message, CancellationToken cancellationToken = default);
}
