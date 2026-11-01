using Launcher.Contracts;
using Launcher.Plugins.Storage;
using Launcher.Runtime;
using System.Diagnostics;
using System.ComponentModel;
using System.Text.Json;

namespace Launcher.Plugins.Minecraft;

public sealed class InstanceService(Context context, ILauncherStorage storage, IVersionCatalog catalog,
    IPlatformEnvironment platform) : IInstanceService, IInstanceRepository
{
    private readonly SemaphoreSlim _gate = new(1, 1);
    private readonly object _sync = new();
    private readonly Dictionary<string, InstanceFile> _instances = new(StringComparer.Ordinal);
    private readonly HashSet<string> _launches = new(StringComparer.Ordinal);
    private string? _selected;
    private string? _problem;
    public IReadOnlyList<InstanceInfo> Instances { get { lock (_sync) return _instances.Values.Select(f => f.Instance).ToArray(); } }
    public string? SelectedInstanceId { get { lock (_sync) return _selected; } }
    public string? Notice { get { lock (_sync) return _problem; } }

    public async Task LoadAsync(CancellationToken token)
    {
        await _gate.WaitAsync(token).ConfigureAwait(false);
        try
        {
            using var usage = storage.AcquireUsage();
            var loaded = new Dictionary<string, InstanceFile>(StringComparer.Ordinal);
            string? problem = null;
            var root = storage.Resolve("instances");
            if (Directory.Exists(root))
                foreach (var directory in Directory.EnumerateDirectories(root))
                {
                    var id = Path.GetFileName(directory);
                    if (!IsId(id)) continue;
                    try
                    {
                        var file = await storage.ReadAsync<InstanceFile>(Relative(id), token).ConfigureAwait(false);
                        if (file is null) continue;
                        if (!IsValidFile(file, id)) throw new InvalidDataException();
                        if (file.LayoutVersion < 2 || file.GameDirectoryName is null)
                        {
                            using var migrationLock = OpenLock(id);
                            if (file.LayoutVersion < 2) await ImportLegacyGameAsync(file, token).ConfigureAwait(false);
                            if (file.GameDirectoryName is null)
                                file = file with { GameDirectoryName = file.Instance.GameVersion,
                                    Instance = file.Instance with { IsInstalled = await HasCompleteLocalFilesAsync(file.Instance.GameVersion, token).ConfigureAwait(false) } };
                            file = file with { LayoutVersion = 2 };
                            await storage.WriteAsync(Relative(id), file, token).ConfigureAwait(false);
                        }
                        loaded.Add(id, ForCurrentMachine(file));
                    }
                    catch (Exception error) when (LauncherStorage.IsStorageError(error) || error is GameLaunchException)
                    { problem = "部分实例配置无法读取或版本不兼容。原文件已保留，请检查实例目录。"; }
                }
            string? selected = null;
            try { selected = (await storage.ReadAsync<InstanceIndex>(Path.Combine("instances", "index.json"), token).ConfigureAwait(false))?.SelectedInstanceId; }
            catch (Exception error) when (LauncherStorage.IsStorageError(error))
            { problem = "实例选择配置无法读取。原文件已保留，请检查 instances/index.json。"; }
            lock (_sync)
            {
                _instances.Clear();
                foreach (var (id, file) in loaded) _instances.Add(id, file);
                _selected = selected is not null && loaded.TryGetValue(selected, out var selectedFile) && selectedFile.Instance.IsInstalled
                    ? selected : loaded.Values.FirstOrDefault(file => file.Instance.IsInstalled)?.Instance.Id;
                _problem = problem;
            }
        }
        catch (Exception error) when (LauncherStorage.IsStorageError(error))
        { lock (_sync) { _instances.Clear(); _selected = null; _problem = "实例目录无法读取，请检查权限或更换数据目录。"; } }
        finally { _gate.Release(); }
        await context.NotifyAsync(new InstancesChanged(SelectedInstanceId)).ConfigureAwait(false);
    }

    public async Task SelectAsync(string instanceId, CancellationToken cancellationToken)
    {
        await _gate.WaitAsync(cancellationToken).ConfigureAwait(false);
        try
        {
            using var usage = storage.AcquireUsage();
            using var catalogLock = OpenCatalogLock();
            lock (_sync) { EnsureWritable(); _ = Find(instanceId); }
            var latest = await ReadInstanceAsync(instanceId, cancellationToken).ConfigureAwait(false);
            if (!latest.Instance.IsInstalled) throw new LauncherOperationException("此实例尚未下载完成。");
            await storage.WriteAsync(Path.Combine("instances", "index.json"), new InstanceIndex(1, instanceId), cancellationToken).ConfigureAwait(false);
            lock (_sync) { _instances[instanceId] = latest; _selected = instanceId; }
        }
        catch (Exception error) when (LauncherStorage.IsStorageError(error))
        { throw new LauncherOperationException("版本选择未保存，请检查数据目录的权限和剩余空间。"); }
        finally { _gate.Release(); }
        await context.NotifyAsync(new InstancesChanged(instanceId)).ConfigureAwait(false);
    }

    public Task<InstanceInfo> CreateAsync(string gameVersion, CancellationToken cancellationToken)
        => CreateAsync(gameVersion, null, cancellationToken);

    public async Task<InstanceInfo> CreateAsync(string gameVersion, string? name, CancellationToken cancellationToken)
    {
        if (!ContentService.IsSupportedVersion(gameVersion)) throw new LauncherOperationException("暂不支持此游戏版本。");
        var versions = await catalog.GetVersionsAsync(cancellationToken).ConfigureAwait(false);
        if (!versions.Any(v => v.Id == gameVersion)) throw new LauncherOperationException("官方目录中没有此正式版，请刷新版本列表。");
        var automaticName = string.IsNullOrWhiteSpace(name);
        name = automaticName ? gameVersion : NormalizeName(name!);
        InstanceInfo instance;
        await _gate.WaitAsync(cancellationToken).ConfigureAwait(false);
        try
        {
            using var usage = storage.AcquireUsage();
            using var catalogLock = OpenCatalogLock();
            lock (_sync) EnsureWritable();
            var existingFiles = new List<InstanceFile>();
            var instancesRoot = storage.Resolve("instances");
            foreach (var directory in Directory.EnumerateDirectories(instancesRoot))
            {
                var id = Path.GetFileName(directory);
                if (!IsId(id)) continue;
                var existing = await storage.ReadAsync<InstanceFile>(Relative(id), cancellationToken).ConfigureAwait(false);
                if (existing is null) continue;
                if (!IsValidFile(existing, id)) throw new InvalidDataException();
                existingFiles.Add(existing);
            }
            var sameName = existingFiles.FirstOrDefault(file => string.Equals(file.Instance.Name, name, StringComparison.OrdinalIgnoreCase));
            if (sameName is not null && !sameName.Instance.IsInstalled && sameName.Instance.GameVersion == gameVersion)
            {
                // Resume an unfinished named instance without leaving duplicate invisible records after cancellation.
                try
                {
                    using var unused = OpenLock(sameName.Instance.Id);
                    lock (_sync) _instances[sameName.Instance.Id] = ForCurrentMachine(sameName);
                    return sameName.Instance;
                }
                catch (Exception error) when (error is IOException or GameLaunchException)
                { if (!automaticName) throw new LauncherOperationException("同名实例正在使用，请为新实例填写不同的名称。"); }
            }
            if (automaticName)
            {
                var suffix = 2;
                while (existingFiles.Any(file => string.Equals(file.Instance.Name, name, StringComparison.OrdinalIgnoreCase)))
                    name = $"{gameVersion} ({suffix++})";
            }
            else if (sameName is not null) throw new LauncherOperationException("该实例名称已存在，请填写不同的名称。");
            instance = new InstanceInfo(Guid.NewGuid().ToString("N"), gameVersion) { Name = name };
            var file = new InstanceFile(1, instance, InstanceOverrides.Inherit, SettingsValidation.MachineId, 2, instance.Id);
            await storage.WriteAsync(Relative(instance.Id), file, cancellationToken).ConfigureAwait(false);
            Directory.CreateDirectory(GameDirectory(file));
            lock (_sync) _instances.Add(instance.Id, file);
        }
        catch (Exception error) when (LauncherStorage.IsStorageError(error))
        { throw new LauncherOperationException("无法创建实例，请检查数据目录的权限和剩余空间。"); }
        finally { _gate.Release(); }
        await context.NotifyAsync(new InstancesChanged(SelectedInstanceId)).ConfigureAwait(false);
        return instance;
    }

    public async Task RenameAsync(string instanceId, string name, CancellationToken cancellationToken)
    {
        name = NormalizeName(name);
        await _gate.WaitAsync(cancellationToken).ConfigureAwait(false);
        try
        {
            using var usage = storage.AcquireUsage();
            lock (_sync) { EnsureWritable(); _ = Find(instanceId); }
            using var catalogLock = OpenCatalogLock();
            using var instanceLock = OpenLock(instanceId);
            foreach (var directory in Directory.EnumerateDirectories(storage.Resolve("instances")))
            {
                var id = Path.GetFileName(directory);
                if (!IsId(id) || id == instanceId) continue;
                var other = await storage.ReadAsync<InstanceFile>(Relative(id), cancellationToken).ConfigureAwait(false);
                if (other is not null && !IsValidFile(other, id)) throw new InvalidDataException();
                if (other is not null && string.Equals(other.Instance.Name, name, StringComparison.OrdinalIgnoreCase))
                    throw new LauncherOperationException("该实例名称已存在，请填写不同的名称。");
            }
            var latest = await ReadInstanceAsync(instanceId, cancellationToken).ConfigureAwait(false);
            var next = latest with { Instance = latest.Instance with { Name = name } };
            await storage.WriteAsync(Relative(instanceId), next, cancellationToken).ConfigureAwait(false);
            lock (_sync) _instances[instanceId] = next;
        }
        catch (GameLaunchException) { throw new LauncherOperationException("实例正在使用，请稍后重命名。"); }
        catch (Exception error) when (LauncherStorage.IsStorageError(error))
        { throw new LauncherOperationException("实例正在使用或名称无法保存，请检查数据目录。"); }
        finally { _gate.Release(); }
        await context.NotifyAsync(new InstancesChanged(SelectedInstanceId)).ConfigureAwait(false);
    }

    public InstanceOverrides GetOverrides(string instanceId) { lock (_sync) return Find(instanceId).Overrides; }

    public async Task DeleteAsync(string instanceId, CancellationToken cancellationToken)
    {
        await _gate.WaitAsync(cancellationToken).ConfigureAwait(false);
        try
        {
            using var usage = storage.AcquireUsage();
            lock (_sync) { EnsureWritable(); _ = Find(instanceId); }
            using var catalogLock = OpenCatalogLock();
            var directory = GetDirectory(instanceId, ".");
            using (var instanceLock = OpenLock(instanceId))
            {
                var latest = await ReadInstanceAsync(instanceId, cancellationToken).ConfigureAwait(false);
                var remaining = new Dictionary<string, InstanceFile>(StringComparer.Ordinal);
                foreach (var candidate in Directory.EnumerateDirectories(storage.Resolve("instances")))
                {
                    var id = Path.GetFileName(candidate);
                    if (!IsId(id) || id == instanceId) continue;
                    var other = await storage.ReadAsync<InstanceFile>(Relative(id), cancellationToken).ConfigureAwait(false);
                    if (other is null) continue;
                    if (!IsValidFile(other, id)) throw new InvalidDataException();
                    remaining.Add(id, ForCurrentMachine(other));
                }
                var selected = (await storage.ReadAsync<InstanceIndex>(Path.Combine("instances", "index.json"), cancellationToken).ConfigureAwait(false))?.SelectedInstanceId;
                if (selected is null || !remaining.TryGetValue(selected, out var selectedFile) || !selectedFile.Instance.IsInstalled)
                    selected = remaining.Values.FirstOrDefault(file => file.Instance.IsInstalled)?.Instance.Id;

                // Legacy per-version directories contain shared JSON/JAR files and are always retained.
                var privateGame = latest.GameDirectoryName == instanceId ? GameDirectory(latest) : null;
                await Task.Run(() =>
                {
                    // Resolve and inspect both absolute targets before deleting anything. Never follow links.
                    CheckNoLinks(directory);
                    if (privateGame is not null && Directory.Exists(privateGame)) CheckNoLinks(privateGame);
                    cancellationToken.ThrowIfCancellationRequested();
                    // Once destructive work begins, finish committing instead of reporting cancellation halfway through.
                    if (privateGame is not null && Directory.Exists(privateGame)) Directory.Delete(privateGame, recursive: true);
                    foreach (var child in Directory.EnumerateFileSystemEntries(directory))
                    {
                        if (Path.GetFileName(child) is "launch.lock" or "instance.json") continue;
                        if (Directory.Exists(child)) Directory.Delete(child, recursive: true);
                        else File.Delete(child);
                    }
                }, cancellationToken).ConfigureAwait(false);
                await storage.WriteAsync(Path.Combine("instances", "index.json"), new InstanceIndex(1, selected), CancellationToken.None).ConfigureAwait(false);
                File.Delete(storage.Resolve(Relative(instanceId)));
                lock (_sync)
                {
                    _instances.Clear();
                    foreach (var (id, file) in remaining) _instances.Add(id, file);
                    _selected = selected;
                }
            }
            // A stale host may briefly hold the now-orphaned lock. The missing instance.json already prevents reuse.
            try { File.Delete(GetDirectory(instanceId, "launch.lock")); Directory.Delete(directory, recursive: false); }
            catch (Exception error) when (error is IOException or UnauthorizedAccessException) { }
        }
        catch (GameLaunchException) { throw new LauncherOperationException("实例正在安装或运行，请等待结束后再删除。"); }
        catch (Exception error) when (LauncherStorage.IsStorageError(error))
        { throw new LauncherOperationException("实例正在使用或文件无法删除，请检查目录权限及文件占用；含目录链接的实例不能自动删除。"); }
        finally { _gate.Release(); }
        await context.NotifyAsync(new InstanceDeleted(instanceId)).ConfigureAwait(false);
        await NotifyInstancesChangedAsync().ConfigureAwait(false);
    }

    public async Task<IReadOnlyList<SettingIssue>> SaveOverridesAsync(string instanceId, InstanceOverrides overrides, CancellationToken cancellationToken)
    {
        var issues = SettingsValidation.Check(overrides.MaxMemoryMiB, overrides.JvmArguments, overrides.Java);
        if (issues.Count > 0) return issues;
        await _gate.WaitAsync(cancellationToken).ConfigureAwait(false);
        try
        {
            using var usage = storage.AcquireUsage();
            lock (_sync)
            {
                EnsureWritable();
                if (_launches.Contains(instanceId)) return [new(SettingField.Java, "请在该版本的游戏退出后再修改版本设置。")];
                _ = Find(instanceId);
            }
            // The same cross-process lock used by launch also protects settings writes.
            using var fileLock = OpenLock(instanceId);
            var latest = await ReadInstanceAsync(instanceId, cancellationToken).ConfigureAwait(false);
            var next = latest with { Overrides = overrides, MachineId = SettingsValidation.MachineId };
            await storage.WriteAsync(Relative(instanceId), next, cancellationToken).ConfigureAwait(false);
            lock (_sync) _instances[instanceId] = next;
            return [];
        }
        catch (LauncherOperationException error) { return [new(SettingField.DataDirectory, error.Message)]; }
        catch (GameLaunchException error) { return [new(SettingField.DataDirectory, error.Message)]; }
        catch (Exception error) when (LauncherStorage.IsStorageError(error))
        { return [new(SettingField.DataDirectory, "实例正在使用或设置无法保存，请检查目录权限。")]; }
        finally { _gate.Release(); }
    }

    public string GetGameDirectory(string instanceId) { lock (_sync) return GameDirectory(Find(instanceId)); }

    public IInstanceLease AcquireLaunch(string instanceId)
    {
        var usage = storage.AcquireUsage();
        try
        {
            lock (_sync)
            {
                EnsureWritable();
                _ = Find(instanceId);
                if (_launches.Contains(instanceId)) throw new LauncherOperationException("此版本已经在启动或运行。");
                var fileLock = OpenLock(instanceId);
                try
                {
                    var latest = JsonFiles.ReadAsync<InstanceFile>(storage.Resolve(Relative(instanceId)), default).GetAwaiter().GetResult();
                    if (latest is null || !IsValidFile(latest, instanceId) || latest.LayoutVersion != 2)
                        throw new LauncherOperationException("实例配置已更改，请重新选择版本。");
                    latest = ForCurrentMachine(latest);
                    _instances[instanceId] = latest;
                    _launches.Add(instanceId);
                    return new Lease(this, usage, fileLock, storage.GameRoot, latest.Instance, latest.Overrides);
                }
                catch { fileLock.Dispose(); throw; }
            }
        }
        catch { usage.Dispose(); throw; }
    }

    private FileStream OpenLock(string instanceId)
    {
        var path = GetDirectory(instanceId, "launch.lock");
        Directory.CreateDirectory(Path.GetDirectoryName(path)!);
        var stream = new FileStream(path, FileMode.OpenOrCreate, FileAccess.ReadWrite, FileShare.None);
        try
        {
            if (stream.Length is > 0 and <= 4096)
            {
                var marker = JsonSerializer.Deserialize<ProcessMarker>(stream);
                if (marker is not null && IsRunning(marker)) throw new GameLaunchException(LaunchFailureKind.InstanceInUse, "该版本的游戏仍在运行，请等待退出。");
            }
            stream.SetLength(0);
            return stream;
        }
        catch { stream.Dispose(); throw; }
    }
    private FileStream OpenCatalogLock()
    {
        var path = storage.Resolve(Path.Combine("instances", "catalog.lock"));
        Directory.CreateDirectory(Path.GetDirectoryName(path)!);
        return new FileStream(path, FileMode.OpenOrCreate, FileAccess.ReadWrite, FileShare.None);
    }
    private static InstanceFile ForCurrentMachine(InstanceFile file) => file.MachineId == SettingsValidation.MachineId ? file
        : file with { Overrides = file.Overrides with { Java = file.Overrides.Java?.ExecutablePath is null ? file.Overrides.Java : JavaChoice.Automatic } };
    private string GetDirectory(string id, string child)
    { if (!IsId(id)) throw new LauncherOperationException("实例标识无效。"); return storage.Resolve(Path.Combine("instances", id, child)); }
    private string GameDirectory(InstanceFile file) => JsonFiles.Within(storage.GameRoot,
        Path.Combine("versions", file.GameDirectoryName ?? file.Instance.GameVersion));
    private async Task ImportLegacyGameAsync(InstanceFile record, CancellationToken token)
    {
        var legacy = GetDirectory(record.Instance.Id, ".minecraft");
        if (!Directory.Exists(legacy)) return;
        var destination = GameDirectory(record);
        async Task CopyDirectoryAsync(string source, string target)
        {
            token.ThrowIfCancellationRequested();
            if ((File.GetAttributes(source) & FileAttributes.ReparsePoint) != 0) throw new IOException("Linked legacy directories cannot be imported.");
            Directory.CreateDirectory(target);
            foreach (var file in Directory.EnumerateFiles(source))
            {
                token.ThrowIfCancellationRequested();
                var next = JsonFiles.Within(target, Path.GetFileName(file));
                if (File.Exists(next)) continue;
                if ((File.GetAttributes(file) & FileAttributes.ReparsePoint) != 0) throw new IOException("Linked legacy files cannot be imported.");
                var temporary = next + "." + Guid.NewGuid().ToString("N") + ".import";
                try
                {
                    await using (var input = new FileStream(file, FileMode.Open, FileAccess.Read, FileShare.Read, 81920, true))
                    await using (var output = new FileStream(temporary, FileMode.CreateNew, FileAccess.Write, FileShare.None, 81920, true))
                        await input.CopyToAsync(output, token).ConfigureAwait(false);
                    File.Move(temporary, next, overwrite: false);
                }
                finally { if (File.Exists(temporary)) File.Delete(temporary); }
            }
            foreach (var directory in Directory.EnumerateDirectories(source))
                await CopyDirectoryAsync(directory, JsonFiles.Within(target, Path.GetFileName(directory))).ConfigureAwait(false);
        }
        await CopyDirectoryAsync(legacy, destination).ConfigureAwait(false);
    }
    private async Task<bool> HasCompleteLocalFilesAsync(string gameVersion, CancellationToken token)
    {
        // No download provider is registered: legacy readiness checks only verify/reuse local files.
        var local = new ContentService(new GameFileService(storage), storage);
        try
        {
            var metadata = await local.ResolveAsync(gameVersion, token).ConfigureAwait(false);
            await local.PrepareAsync(metadata, platform.Target, storage.GameRoot, null, token).ConfigureAwait(false);
            return true;
        }
        catch (Exception error) when (LauncherStorage.IsStorageError(error)
            || error is GameLaunchException or KeyNotFoundException or InvalidOperationException or HttpRequestException)
        { return false; }
    }
    private async Task<InstanceFile> ReadInstanceAsync(string id, CancellationToken token)
    {
        var file = await storage.ReadAsync<InstanceFile>(Relative(id), token).ConfigureAwait(false);
        if (file is null || !IsValidFile(file, id)) throw new InvalidDataException("Invalid instance configuration.");
        return ForCurrentMachine(file);
    }
    private async Task<bool> MarkInstalledAsync(string instanceId, CancellationToken token)
    {
        await _gate.WaitAsync(token).ConfigureAwait(false);
        try
        {
            var latest = await ReadInstanceAsync(instanceId, token).ConfigureAwait(false);
            if (latest.Instance.IsInstalled) return false;
            var next = latest with { Instance = latest.Instance with { IsInstalled = true } };
            bool select;
            lock (_sync) { EnsureWritable(); select = _selected is null; }
            if (select) await storage.WriteAsync(Path.Combine("instances", "index.json"), new InstanceIndex(1, instanceId), token).ConfigureAwait(false);
            await storage.WriteAsync(Relative(instanceId), next, token).ConfigureAwait(false);
            lock (_sync) { _instances[instanceId] = next; if (select) _selected = instanceId; }
            return true;
        }
        finally { _gate.Release(); }
    }
    private Task NotifyInstancesChangedAsync() => context.NotifyAsync(new InstancesChanged(SelectedInstanceId));
    private static void CheckNoLinks(string path)
    {
        if ((File.GetAttributes(path) & FileAttributes.ReparsePoint) != 0) throw new IOException("Linked instance paths cannot be deleted.");
        if (Directory.Exists(path))
            foreach (var child in Directory.EnumerateFileSystemEntries(path)) CheckNoLinks(child);
    }
    private static string NormalizeName(string? name)
    {
        name = name?.Trim() ?? "";
        if (!IsValidName(name)) throw new LauncherOperationException("实例名称应为 1–64 个字符，不能包含控制字符。");
        return name;
    }
    private static bool IsValidName(string? name) => !string.IsNullOrWhiteSpace(name) && name.Length <= 64 && !name.Any(char.IsControl);
    private static bool IsValidFile(InstanceFile file, string id) => file.Instance is { } instance && instance.Id == id
        && ContentService.IsSupportedVersion(instance.GameVersion) && IsValidName(instance.Name) && file.Overrides is not null
        && file.LayoutVersion is >= 1 and <= 2
        && (file.GameDirectoryName is null || file.GameDirectoryName == instance.Id || file.GameDirectoryName == instance.GameVersion);
    private InstanceFile Find(string id) => _instances.GetValueOrDefault(id) ?? throw new LauncherOperationException("所选实例不存在，请重新选择版本。");
    private void EnsureWritable() { if ((_problem ?? storage.Problem) is { } problem) throw new LauncherOperationException(problem); }
    private static bool IsId(string id) => id.Length == 32 && id.All(c => c is >= '0' and <= '9' or >= 'a' and <= 'f');
    private static string Relative(string id) => Path.Combine("instances", id, "instance.json");
    private static bool IsRunning(ProcessMarker marker)
    {
        try
        {
            using var process = Process.GetProcessById(marker.ProcessId);
            return !process.HasExited && process.StartTime.ToUniversalTime().Ticks == marker.StartTimeUtcTicks;
        }
        catch (Exception error) when (error is ArgumentException or InvalidOperationException) { return false; }
        catch (Win32Exception) { return true; } // Unknown ownership is safer than a second writer to a live game directory.
    }
    private sealed record InstanceFile(int SchemaVersion, InstanceInfo Instance, InstanceOverrides Overrides, string? MachineId = null,
        int LayoutVersion = 1, string? GameDirectoryName = null);
    private sealed record InstanceIndex(int SchemaVersion, string? SelectedInstanceId);
    private sealed record ProcessMarker(int ProcessId, long StartTimeUtcTicks);
    private sealed class Lease(InstanceService owner, IDisposable usage, FileStream fileLock, string root, InstanceInfo instance, InstanceOverrides overrides) : IInstanceLease
    {
        private int _disposed;
        private bool _installedChanged;
        public string DataRoot => root;
        public InstanceInfo Instance => instance;
        public InstanceOverrides Overrides => overrides;
        public async Task MarkInstalledAsync(CancellationToken cancellationToken)
        {
            ObjectDisposedException.ThrowIf(Volatile.Read(ref _disposed) != 0, this);
            _installedChanged |= await owner.MarkInstalledAsync(instance.Id, cancellationToken).ConfigureAwait(false);
        }
        public void MarkRunning(int processId)
        {
            try
            {
                using var process = Process.GetProcessById(processId);
                var marker = new ProcessMarker(processId, process.StartTime.ToUniversalTime().Ticks);
                fileLock.Position = 0;
                fileLock.SetLength(0);
                JsonSerializer.Serialize(fileLock, marker);
                fileLock.Flush(flushToDisk: true);
            }
            catch (Exception error) when (error is ArgumentException or InvalidOperationException or Win32Exception) { }
        }
        public void Dispose()
        {
            if (Interlocked.Exchange(ref _disposed, 1) != 0) return;
            lock (owner._sync) { owner._launches.Remove(instance.Id); fileLock.Dispose(); usage.Dispose(); }
            if (_installedChanged) _ = owner.NotifyInstancesChangedAsync();
        }
    }
}
