namespace Launcher.Contracts;

/// <summary>Named, isolated game instances. Several instances may use the same game version.</summary>
public interface IInstanceService
{
    IReadOnlyList<InstanceInfo> Instances { get; }
    string? SelectedInstanceId { get; }
    string? Notice => null;

    Task SelectAsync(string instanceId, CancellationToken cancellationToken);

    /// <summary>
    /// Creates a pending instance. The caller starts installation through IGameInstallService;
    /// only installed instances may be selected. A blank name is generated automatically.
    /// </summary>
    Task<InstanceInfo> CreateAsync(string gameVersion, CancellationToken cancellationToken);
    Task<InstanceInfo> CreateAsync(string gameVersion, string? name, CancellationToken cancellationToken);
    Task RenameAsync(string instanceId, string name, CancellationToken cancellationToken);

    /// <summary>
    /// Call only after user confirmation. Removes this instance and its private data, never shared game files.
    /// Refuses while the instance is being installed, launched or used by a running game.
    /// </summary>
    Task DeleteAsync(string instanceId, CancellationToken cancellationToken);

    InstanceOverrides GetOverrides(string instanceId);

    /// <summary>Saves nothing and returns the problems when any value is invalid.</summary>
    Task<IReadOnlyList<SettingIssue>> SaveOverridesAsync(string instanceId, InstanceOverrides overrides, CancellationToken cancellationToken);
}

/// <param name="Id">A stable, file-system-safe identifier. It is not the game version.</param>
public sealed record InstanceInfo(string Id, string GameVersion)
{
    public string Name { get; init; } = GameVersion;
    public bool IsInstalled { get; init; }
}

/// <summary>Per-instance overrides. A null member inherits the launcher-wide default.</summary>
public sealed record InstanceOverrides(JavaChoice? Java, int? MaxMemoryMiB, string? JvmArguments)
{
    public static InstanceOverrides Inherit { get; } = new(null, null, null);
}

/// <summary>Published when instances, installation readiness, names or selection change.</summary>
public sealed record InstancesChanged(string? SelectedInstanceId);

/// <summary>The instance was removed; consumers should discard its cached jobs and other transient state.</summary>
public sealed record InstanceDeleted(string InstanceId);
