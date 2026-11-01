namespace Launcher.Contracts;

/// <summary>Launcher-wide defaults and the data directory. Provided by <c>storage</c>.</summary>
public interface ILauncherSettingsService
{
    LauncherSettings Settings { get; }
    string DataDirectory { get; }
    string GameDirectory => Path.Combine(DataDirectory, ".minecraft");
    MemoryAdvice MemoryAdvice { get; }
    string? Notice => null;

    /// <summary>Saves nothing and returns the problems when any value is invalid.</summary>
    Task<IReadOnlyList<SettingIssue>> SaveAsync(LauncherSettings settings, CancellationToken cancellationToken);

    /// <summary>Uses a new data root. Existing data is never moved or deleted implicitly.</summary>
    Task<IReadOnlyList<SettingIssue>> ChangeDataDirectoryAsync(string path, CancellationToken cancellationToken);
}

/// <param name="MaxMemoryMiB">The Java heap limit, not the total memory of the game process.</param>
public sealed record LauncherSettings(
    JavaChoice Java,
    int MaxMemoryMiB,
    GameWindowSettings Window,
    AfterLaunchBehavior AfterLaunch,
    string JvmArguments,
    GameDownloadSource DownloadSource = GameDownloadSource.Official);

public enum GameDownloadSource { Official, Bmclapi }

public enum GameWindowMode { GameDefault, Custom, Fullscreen }

/// <param name="Width">Used only with <see cref="GameWindowMode.Custom"/>.</param>
public sealed record GameWindowSettings(GameWindowMode Mode, int Width, int Height);

public enum AfterLaunchBehavior { MinimizeAndRestore, KeepOpen }

public sealed record MemoryAdvice(int TotalMiB, int RecommendedMiB);

public enum SettingField { Java, MaxMemory, Window, JvmArguments, DataDirectory, DownloadSource }

/// <param name="Message">User-facing text. It must not contain credentials.</param>
public sealed record SettingIssue(SettingField Field, string Message);
