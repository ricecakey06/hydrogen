namespace Launcher.Contracts;

/// <summary>Launch orchestration and game process state. Provided by <c>minecraft.launch</c>.</summary>
public interface IGameLaunchService
{
    LaunchSnapshot GetSnapshot(string instanceId);

    /// <summary>Instances that are being prepared or whose game process is still running.</summary>
    IReadOnlyList<LaunchSnapshot> ActiveSessions { get; }

    /// <summary>
    /// Completes once the game process has started or the attempt ended. Expected failures are reported
    /// through the snapshot. Cancellation only stops preparation and never terminates a running game.
    /// Throws <see cref="InvalidOperationException"/> when the instance is already launching or running.
    /// </summary>
    Task LaunchAsync(string instanceId, CancellationToken cancellationToken);
}

public enum LaunchPhase { Idle, CheckingEnvironment, PreparingFiles, Starting, Running, Exited, Failed, Cancelled }

/// <param name="Total">Null when the amount of work is not known yet.</param>
public sealed record LaunchProgress(long Completed, long? Total, string? CurrentFile = null,
    int CompletedFiles = 0, int TotalFiles = 0, IReadOnlyList<string>? ActiveFiles = null)
{
    public LaunchProgress(long completed, long? total) : this(completed, total, null) { }
    public void Deconstruct(out long completed, out long? total) { completed = Completed; total = Total; }
}

public enum LaunchFailureKind
{
    SignInRequired,
    JavaMissing,
    JavaIncompatible,
    Network,
    FileVerification,
    DirectoryNotWritable,
    PlatformUnsupported,
    ProcessExitedEarly,
    Unexpected,
    InvalidSettings,
    InstanceInUse,
    InsufficientSpace,
    DownloadsDisabled,
}

/// <param name="Detail">Optional redacted context such as the required Java version.</param>
/// <param name="LogPath">A redacted local log for this attempt, if one exists.</param>
public sealed record LaunchFailure(LaunchFailureKind Kind, string? Detail = null, string? LogPath = null);

public sealed record LaunchSnapshot(
    string InstanceId,
    LaunchPhase Phase,
    LaunchProgress? Progress = null,
    LaunchFailure? Failure = null,
    int? ExitCode = null,
    AfterLaunchBehavior? AfterLaunch = null)
{
    public bool IsActive => Phase is LaunchPhase.CheckingEnvironment or LaunchPhase.PreparingFiles
        or LaunchPhase.Starting or LaunchPhase.Running;
}

/// <summary>Published on every phase or progress change of any instance.</summary>
public sealed record LaunchStateChanged(LaunchSnapshot Snapshot);
