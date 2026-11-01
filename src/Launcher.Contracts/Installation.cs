namespace Launcher.Contracts;

/// <summary>Prepares game files independently of account login, Java discovery, and page navigation.</summary>
public interface IGameInstallService
{
    /// <summary>Active and failed downloads. Completed and cancelled tasks leave the queue.</summary>
    IReadOnlyList<InstallSnapshot> Jobs { get; }
    InstallSnapshot? GetSnapshot(string instanceId);
    Task InstallAsync(string instanceId, CancellationToken cancellationToken);
    void Cancel(string instanceId);
}

public enum InstallPhase { Resolving, Downloading, Completed, Failed, Cancelled }

public sealed record InstallSnapshot(string InstanceId, string GameVersion, InstallPhase Phase,
    LaunchProgress? Progress = null, string? Error = null)
{
    public string? InstanceName { get; init; }
    public bool IsActive => Phase is InstallPhase.Resolving or InstallPhase.Downloading;
}

public sealed record InstallStateChanged(string InstanceId);
