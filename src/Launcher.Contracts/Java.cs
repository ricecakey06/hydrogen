namespace Launcher.Contracts;

/// <summary>Discovery and inspection of existing Java runtimes. Provided by <c>java.runtime</c>.</summary>
public interface IJavaRuntimeService
{
    /// <summary>The result of the latest scan.</summary>
    IReadOnlyList<JavaRuntimeInfo> KnownRuntimes { get; }

    Task<IReadOnlyList<JavaRuntimeInfo>> ScanAsync(CancellationToken cancellationToken);

    /// <summary>Returns null when the file is not a usable Java executable.</summary>
    Task<JavaRuntimeInfo?> InspectAsync(string executablePath, CancellationToken cancellationToken);
}

public sealed record JavaRuntimeInfo(string ExecutablePath, string Version, string Architecture);

/// <summary>A null <see cref="ExecutablePath"/> selects a runtime that matches the game version automatically.</summary>
public sealed record JavaChoice(string? ExecutablePath)
{
    public static JavaChoice Automatic { get; } = new((string?)null);
}
