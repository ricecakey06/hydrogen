namespace Launcher.Contracts;

/// <summary>Local plugin inventory and restart-scoped package changes, provided by <c>plugins.manager</c>.</summary>
public interface IPluginManager
{
    string PluginsDirectory { get; }
    Task<PluginInventory> GetInventoryAsync(CancellationToken cancellationToken);
    Task SetEnabledAsync(string id, bool enabled, CancellationToken cancellationToken);
    Task InstallAsync(string packagePath, CancellationToken cancellationToken);
    Task UninstallAsync(string id, CancellationToken cancellationToken);
    Task UndoUninstallAsync(string id, CancellationToken cancellationToken);
}

public enum PluginOrigin { BuiltIn, Installed }
public enum PluginRunState { Active, Disabled, Waiting, Failed, NotLoaded }

public sealed record ManagedPlugin(
    string Id, string Name, string Version, string Description, string Author,
    PluginOrigin Origin, PluginRunState State, bool IsEnabled, bool IsRequired,
    bool IsPendingRemoval, bool RequiresRestart, string? Problem, string? RequiredReason,
    IReadOnlyList<string> Dependencies, IReadOnlyList<string> Provides, string? Directory);

public sealed record PluginInventory(IReadOnlyList<ManagedPlugin> Plugins, string? Notice, bool IsReadOnly);

/// <summary>A deliberately user-facing failure; arbitrary plugin exception payloads are never shown in its place.</summary>
public sealed class PluginManagementException(string message) : InvalidOperationException(message);
