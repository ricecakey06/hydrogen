using System.Text.Json;

namespace Launcher.Contracts;

/// <summary>Official metadata and verified content, provided alongside the version catalog.</summary>
public interface IGameContentService
{
    Task<GameMetadata> ResolveAsync(string version, CancellationToken cancellationToken);
    Task<PreparedGame> PrepareAsync(GameMetadata metadata, PlatformTarget target, string dataRoot,
        IProgress<LaunchProgress>? progress, CancellationToken cancellationToken);
}

public sealed record GameMetadata(string Id, int JavaMajorVersion, JsonElement Document);
public sealed record NativeArchive(string Path, IReadOnlyList<string> Excludes);
public sealed record PreparedGame(GameMetadata Metadata, string ClientPath, IReadOnlyList<string> Classpath,
    IReadOnlyList<NativeArchive> Natives, string AssetsDirectory, string AssetIndex, string? LoggingArgument);

/// <summary>Required cache access; it remains available when the optional download module is absent.</summary>
public interface IGameFileService
{
    bool DownloadsEnabled { get; }
    Task<JsonElement> GetCatalogAsync(CancellationToken cancellationToken);
    Task EnsureFileAsync(DownloadFile file, CancellationToken cancellationToken);
}

/// <summary>Implemented exclusively by the removable game-download plugin.</summary>
public interface IGameDownloadProvider
{
    Task<JsonElement> GetCatalogAsync(CancellationToken cancellationToken);
    Task DownloadAsync(DownloadFile file, CancellationToken cancellationToken);
}

public interface IGameDownloadRegistry
{
    ValueTask<IAsyncDisposable> RegisterAsync(IGameDownloadProvider provider, CancellationToken cancellationToken);
}

/// <summary>Private paths and an exclusive per-instance lease. UI only needs IInstanceService.</summary>
public interface IInstanceRepository
{
    string GetGameDirectory(string instanceId);
    IInstanceLease AcquireLaunch(string instanceId);
}

public interface IInstanceLease : IDisposable
{
    string DataRoot { get; }
    InstanceInfo Instance { get; }
    InstanceOverrides Overrides { get; }
    Task MarkInstalledAsync(CancellationToken cancellationToken);
    void MarkRunning(int processId);
}

public sealed class LauncherOperationException(string message) : InvalidOperationException(message);

public sealed class GameLaunchException(LaunchFailureKind kind, string detail) : Exception(detail)
{
    public LaunchFailureKind Kind { get; } = kind;
}

public sealed class AccountAuthenticationException(AccountFailureKind kind) : Exception("Account authentication did not complete.")
{
    public AccountFailureKind Kind { get; } = kind;
}
