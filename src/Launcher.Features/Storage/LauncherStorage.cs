using System.Text.Json;
using Launcher.Contracts;
using Launcher.Runtime;

namespace Launcher.Plugins.Storage;

public sealed class LauncherStorage : ILauncherStorage, ILauncherSettingsService
{
    private readonly Context _context;
    private readonly string _defaultRoot;
    private readonly string _locationFile;
    private readonly string? _defaultGameRoot;
    private readonly SemaphoreSlim _gate = new(1, 1);
    private readonly object _sync = new();
    private int _users;
    private string _root;
    private string? _problem;
    private LauncherSettings _settings = Defaults;
    public static LauncherSettings Defaults => new(JavaChoice.Automatic, SettingsValidation.Memory.RecommendedMiB,
        new(GameWindowMode.GameDefault, 854, 480), AfterLaunchBehavior.MinimizeAndRestore, "");

    public LauncherStorage(Context context, string defaultRoot, string? locationFile = null, string? gameRoot = null)
    {
        _context = context;
        _defaultRoot = Path.GetFullPath(defaultRoot);
        _root = _defaultRoot;
        _locationFile = locationFile ?? JsonFiles.Within(_defaultRoot, "data-location.json");
        _defaultGameRoot = gameRoot is null ? null : Path.GetFullPath(gameRoot);
    }

    public string Root { get { lock (_sync) return _root; } }
    public string? Problem { get { lock (_sync) return _problem; } }
    public string? Notice => Problem;
    public string DataDirectory => Root;
    public string GameRoot { get { lock (_sync) return _defaultGameRoot is not null
        && string.Equals(_root, _defaultRoot, OperatingSystem.IsWindows() ? StringComparison.OrdinalIgnoreCase : StringComparison.Ordinal)
        ? _defaultGameRoot : Path.Combine(_root, ".minecraft"); } }
    public string GameDirectory => GameRoot;
    public LauncherSettings Settings { get { lock (_sync) return _settings; } }
    public MemoryAdvice MemoryAdvice => SettingsValidation.Memory;

    public async Task InitializeAsync(CancellationToken token)
    {
        try
        {
            var location = await JsonFiles.ReadAsync<Location>(_locationFile, token).ConfigureAwait(false);
            if (location is not null)
            {
                if (!Path.IsPathFullyQualified(location.Directory)) throw new InvalidDataException();
                _root = Path.GetFullPath(location.Directory);
            }
            _settings = await ReadSettingsAsync(Root, token).ConfigureAwait(false);
        }
        catch (Exception error) when (IsStorageError(error))
        { _problem = "配置无法读取或格式不兼容。原文件已保留，请检查配置或选择其他数据目录。"; }
    }

    public string Resolve(string relativePath) => JsonFiles.Within(Root, relativePath);

    public IDisposable AcquireUsage()
    {
        lock (_sync)
        {
            if (_users < 0) throw new LauncherOperationException("数据目录正在切换，请稍后重试。");
            _users++;
            return new Usage(this);
        }
    }

    public async Task<T?> ReadAsync<T>(string relativePath, CancellationToken cancellationToken)
    {
        using var usage = AcquireUsage();
        return await JsonFiles.ReadAsync<T>(Resolve(relativePath), cancellationToken).ConfigureAwait(false);
    }

    public async Task WriteAsync<T>(string relativePath, T value, CancellationToken cancellationToken)
    {
        using var usage = AcquireUsage();
        if (Problem is not null) throw new LauncherOperationException(Problem);
        var destination = Resolve(relativePath);
        var isCache = Path.GetRelativePath(Root, destination).StartsWith("cache" + Path.DirectorySeparatorChar, StringComparison.Ordinal);
        await JsonFiles.WriteAsync(destination, value, cancellationToken, protectExisting: !isCache).ConfigureAwait(false);
    }

    public async Task<IReadOnlyList<SettingIssue>> SaveAsync(LauncherSettings settings, CancellationToken cancellationToken)
    {
        var issues = ValidateSettings(settings);
        if (issues.Count > 0) return issues;
        await _gate.WaitAsync(cancellationToken).ConfigureAwait(false);
        try
        {
            if (Problem is { } problem) return [new(SettingField.DataDirectory, problem)];
            await WriteAsync("settings.json", new SettingsFile(1, settings, SettingsValidation.MachineId), cancellationToken).ConfigureAwait(false);
            lock (_sync) _settings = settings;
            return [];
        }
        catch (Exception error) when (IsStorageError(error))
        { return [new(SettingField.DataDirectory, "设置未保存，请检查数据目录的权限和剩余空间。")]; }
        finally { _gate.Release(); }
    }

    public async Task<IReadOnlyList<SettingIssue>> ChangeDataDirectoryAsync(string path, CancellationToken cancellationToken)
    {
        if (string.IsNullOrWhiteSpace(path) || !Path.IsPathFullyQualified(path))
            return [new(SettingField.DataDirectory, "请选择一个完整的本地目录。")];
        var changed = false;
        await _gate.WaitAsync(cancellationToken).ConfigureAwait(false);
        try
        {
            var nextRoot = Path.GetFullPath(path);
            if (string.Equals(nextRoot, Root, OperatingSystem.IsWindows() ? StringComparison.OrdinalIgnoreCase : StringComparison.Ordinal)) return [];
            lock (_sync)
            {
                if (_users != 0) return [new(SettingField.DataDirectory, "请等待当前登录、文件操作或游戏结束后再更换数据目录。")];
                // All usage acquisition is synchronized with this root switch through the sentinel below.
                _users = -1;
            }
            try
            {
                Directory.CreateDirectory(nextRoot);
                var probe = JsonFiles.Within(nextRoot, ".write-check-" + Guid.NewGuid().ToString("N"));
                await using (var stream = new FileStream(probe, FileMode.CreateNew, FileAccess.Write, FileShare.None, 1, FileOptions.DeleteOnClose))
                    await stream.FlushAsync(cancellationToken).ConfigureAwait(false);
                var nextSettings = await ReadSettingsAsync(nextRoot, cancellationToken).ConfigureAwait(false);
                await JsonFiles.WriteAsync(_locationFile, new Location(1, nextRoot), cancellationToken).ConfigureAwait(false);
                lock (_sync) { _root = nextRoot; _settings = nextSettings; _problem = null; }
                changed = true;
            }
            finally { lock (_sync) _users = 0; }
        }
        catch (Exception error) when (IsStorageError(error) || error is ArgumentException or NotSupportedException)
        { return [new(SettingField.DataDirectory, "该目录不可写或配置不兼容，数据目录没有更改。")]; }
        finally { _gate.Release(); }
        if (changed) await _context.NotifyAsync(new DataDirectoryChanged(Root)).ConfigureAwait(false);
        return [];
    }

    private static async Task<LauncherSettings> ReadSettingsAsync(string root, CancellationToken token)
    {
        var file = await JsonFiles.ReadAsync<SettingsFile>(JsonFiles.Within(root, "settings.json"), token).ConfigureAwait(false);
        if (file is null) return Defaults;
        if (file.Settings is null) throw new InvalidDataException();
        var settings = file.Settings;
        if (file.MachineId != SettingsValidation.MachineId)
            settings = settings with { Java = JavaChoice.Automatic, MaxMemoryMiB = Math.Clamp(settings.MaxMemoryMiB, 512, SettingsValidation.Memory.TotalMiB) };
        if (ValidateSettings(settings).Count != 0) throw new InvalidDataException();
        return settings;
    }

    private static List<SettingIssue> ValidateSettings(LauncherSettings settings)
    {
        if (settings.Java is null || settings.Window is null || settings.JvmArguments is null)
            return [new(SettingField.Window, "设置内容不完整。")];
        var issues = SettingsValidation.Check(settings.MaxMemoryMiB, settings.JvmArguments, settings.Java).ToList();
        if (!Enum.IsDefined(settings.DownloadSource)) issues.Add(new(SettingField.DownloadSource, "请选择有效的游戏下载源。"));
        if (!Enum.IsDefined(settings.Window.Mode) || !Enum.IsDefined(settings.AfterLaunch)
            || settings.Window.Mode == GameWindowMode.Custom && (settings.Window.Width is < 320 or > 16384 || settings.Window.Height is < 240 or > 16384))
            issues.Add(new(SettingField.Window, "自定义窗口宽度应为 320–16384，高度应为 240–16384。"));
        return issues;
    }

    internal static bool IsStorageError(Exception error) => error is IOException or InvalidDataException or UnauthorizedAccessException or JsonException or ArgumentException or NotSupportedException;
    private sealed record SettingsFile(int SchemaVersion, LauncherSettings Settings, string? MachineId = null);
    private sealed record Location(int SchemaVersion, string Directory);
    private sealed class Usage(LauncherStorage owner) : IDisposable
    {
        private LauncherStorage? _owner = owner;
        public void Dispose() { if (Interlocked.Exchange(ref _owner, null) is { } storage) lock (storage._sync) storage._users--; }
    }
}
