using System.Text.Json;
using System.Reflection;
using System.Runtime.Loader;
using Launcher.Contracts;
using Launcher.Runtime;

namespace Launcher.Plugins;

/// <summary>Owns the local catalog. Saved changes never mutate a running game or unload their own UI.</summary>
public sealed class PluginManager : IPluginManager, IDisposable
{
    public const string PluginId = "plugins.manager";
    private readonly List<BundledPlugin> _builtIns;
    private readonly HashSet<string> _officialIds;
    private readonly Dictionary<string, PluginDefinition> _definitions;
    private readonly string[] _requiredRoots;
    private readonly Dictionary<string, string> _officialAssemblies = new(StringComparer.OrdinalIgnoreCase);
    private readonly HashSet<string> _required = new(StringComparer.Ordinal);
    private readonly HashSet<string> _startupEnabled = new(StringComparer.Ordinal);
    private readonly Dictionary<string, string> _loadProblems = new(StringComparer.Ordinal);
    private readonly HashSet<string> _waiting = new(StringComparer.Ordinal);
    private readonly List<PluginLoadContext> _loadContexts = [];
    private readonly SemaphoreSlim _gate = new(1, 1);
    private IReadOnlyDictionary<string, PluginSnapshot> _runtime = new Dictionary<string, PluginSnapshot>();
    private string StatePath => Path.Combine(PluginsDirectory, "plugins.json");
    private string? _stateProblem;
    private string? _startupNotice;
    private string? _catalogProblem;
    private bool _prepared;

    public PluginManager(string directory, IEnumerable<BundledPlugin> builtIns, IEnumerable<string> requiredRoots,
        IEnumerable<string>? reservedOfficialIds = null)
    {
        PluginsDirectory = Path.GetFullPath(directory);
        _builtIns = builtIns.ToList();
        _officialIds = new HashSet<string>(_builtIns.Select(plugin => plugin.Id)
            .Concat(reservedOfficialIds ?? []).Append(PluginId), StringComparer.Ordinal);
        _definitions = new(StringComparer.Ordinal)
        {
            [PluginId] = new PluginDefinition(PluginId,
            (context, _) => { context.Provide<IPluginManager>(this, ownsService: false); return ValueTask.CompletedTask; },
            provides: [typeof(IPluginManager)]),
        };
        if (_builtIns.Select(plugin => plugin.Id).Append(PluginId).Distinct(StringComparer.Ordinal).Count() != _builtIns.Count + 1)
            throw new ArgumentException("Duplicate official plugin ID.", nameof(builtIns));
        _requiredRoots = requiredRoots.Append(PluginId).ToArray();
        _required.Add(PluginId);
        AssemblyLoadContext.Default.Resolving += ResolveOfficialAssembly;
    }

    private bool IsOfficial(string id) => _officialIds.Contains(id);

    private Assembly? ResolveOfficialAssembly(AssemblyLoadContext context, AssemblyName name)
        => name.Name is not null && _officialAssemblies.TryGetValue(name.Name, out var path)
            ? context.LoadFromAssemblyPath(path) : null;

    private void RequireCore()
    {
        var providers = _definitions.Values.SelectMany(definition => definition.Provides.Select(type => (type, definition.Id)))
            .ToDictionary(pair => pair.type, pair => pair.Id);
        void Require(string id)
        {
            if (!_required.Add(id)) return;
            if (!_definitions.TryGetValue(id, out var definition)) throw new PluginManagementException("缺少必需的官方插件：" + id);
            foreach (var contract in definition.Requires)
                if (providers.TryGetValue(contract, out var provider)) Require(provider);
        }
        foreach (var id in _requiredRoots) Require(id);
    }

    public string PluginsDirectory { get; }

    /// <summary>Must run before host activation, and at most once. Installed code runs only here.</summary>
    public async Task<IReadOnlyList<PluginDefinition>> PrepareStartupAsync(CancellationToken token = default)
    {
        await _gate.WaitAsync(token).ConfigureAwait(false);
        try
        {
            if (_prepared) throw new InvalidOperationException("The plugin startup composition was already prepared.");
            _prepared = true;
            if (_builtIns.Count > 0)
            {
                Directory.CreateDirectory(PluginsDirectory);
                if ((File.GetAttributes(PluginsDirectory) & FileAttributes.ReparsePoint) != 0)
                    throw new PluginManagementException("插件根目录不能使用符号链接或目录联接。");
                if (!_builtIns.All(plugin => BundledPluginFiles.IsCurrent(PluginsDirectory, plugin)))
                {
                    await using var fileLock = AcquireFileLock();
                    foreach (var plugin in _builtIns)
                        await BundledPluginFiles.SynchronizeAsync(PluginsDirectory, plugin, token).ConfigureAwait(false);
                }
                foreach (var plugin in _builtIns)
                {
                    var path = PackageFiles.Within(PackageFiles.Within(PluginsDirectory, plugin.Id), plugin.EntryAssembly);
                    _officialAssemblies.TryAdd(AssemblyName.GetAssemblyName(path).Name!, path);
                }
            }
            // Read every official definition before applying preferences, so a disabled dependency
            // cannot escape the transitive required-plugin protection.
            var catalog = ReadInstalled();
            foreach (var item in catalog.Where(item => IsOfficial(item.Id)))
            {
                if (item.Problem is not null) throw new PluginManagementException(item.Problem);
                _definitions.Add(item.Id, LoadDefinition(item));
            }
            RequireCore();
            var settings = ReadSettings();
            if (_stateProblem is null && Directory.Exists(PluginsDirectory))
            {
                var removals = settings.Plugins.Where(pair => pair.Value.PendingRemoval).ToArray();
                if (removals.Length > 0)
                {
                    try
                    {
                        await using var fileLock = AcquireFileLock();
                        settings = ReadSettings();
                        foreach (var (id, preference) in settings.Plugins.Where(pair => pair.Value.PendingRemoval).ToArray())
                        {
                            try
                            {
                                if (!PluginPackage.IsId(id) || _definitions.ContainsKey(id)) continue;
                                PackageFiles.DeleteOwnedDirectory(PluginsDirectory, PackageFiles.Within(PluginsDirectory, id));
                                settings.Plugins.Remove(id);
                            }
                            catch (Exception error) when (error is IOException or UnauthorizedAccessException or PluginManagementException)
                            { _loadProblems[id] = "插件文件正在使用或无法访问，卸载将在下次启动时重试。"; }
                        }
                        if (_stateProblem is null) await SaveSettingsAsync(settings, token).ConfigureAwait(false);
                    }
                    catch (Exception error) when (error is IOException or UnauthorizedAccessException or PluginManagementException)
                    { _startupNotice = "待卸载插件暂时无法移除，请关闭其他启动器后重试。其余插件仍可正常使用。"; }
                }
            }
            catalog = ReadInstalled();
            var selected = _definitions.Values.Where(plugin => Enabled(plugin.Id, settings, builtIn: true)).ToList();
            _startupEnabled.UnionWith(selected.Select(plugin => plugin.Id));
            var remaining = catalog.Where(item => item.Package is not null && item.Problem is null
                    && !IsOfficial(item.Id) && Enabled(item.Id, settings, builtIn: false) && !Preference(item.Id, settings).PendingRemoval)
                .ToDictionary(item => item.Id, StringComparer.Ordinal);
            while (remaining.Count > 0)
            {
                var progressed = false;
                foreach (var item in remaining.Values.ToArray())
                {
                    var package = item.Package!;
                    if (package.Dependencies.Any(remaining.ContainsKey)) continue;
                    remaining.Remove(item.Id);
                    progressed = true;
                    _startupEnabled.Add(item.Id);
                    var missing = package.Dependencies.Where(id => selected.All(definition => definition.Id != id)).ToArray();
                    if (missing.Length > 0)
                    {
                        _loadProblems[item.Id] = "缺少已启用的依赖插件：" + string.Join("、", missing);
                        _waiting.Add(item.Id);
                        continue;
                    }
                    try
                    {
                        var definition = LoadDefinition(item);
                        // Validate duplicate IDs, service conflicts and cycles before touching the live host.
                        await using (var validation = new PluginHost(selected.Append(definition))) { }
                        _definitions.Add(item.Id, definition);
                        selected.Add(definition);
                    }
                    catch (Exception error)
                    {
                        _loadProblems[item.Id] = error is PluginManagementException managed ? managed.Message.Trim()
                            : error is PluginCompositionException ? "插件 ID、服务或依赖关系与现有插件冲突。"
                            : $"插件入口加载失败（{error.GetType().Name}），请检查插件兼容性。";
                    }
                }
                if (progressed) continue;
                foreach (var id in remaining.Keys)
                {
                    _startupEnabled.Add(id);
                    _loadProblems[id] = "插件依赖形成循环，无法加载。";
                }
                break;
            }
            return selected.AsReadOnly();
        }
        finally { _gate.Release(); }
    }

    private PluginDefinition LoadDefinition(InstalledPackage item)
    {
        PackageFiles.CheckNoLinks(item.Directory);
        var load = new PluginLoadContext(PackageFiles.Within(item.Directory, item.Package!.EntryAssembly));
        try
        {
            var definition = load.Create(item.Package, IsOfficial(item.Id));
            _loadContexts.Add(load);
            return definition;
        }
        catch { load.Unload(); throw; }
    }

    /// <summary>Captured by the composition owner after activation, outside managed plugin callbacks.</summary>
    public void RecordRuntime(IReadOnlyList<PluginSnapshot> snapshots)
        => _runtime = snapshots.Where(snapshot => snapshot.ScopePath == "application").ToDictionary(snapshot => snapshot.Id, StringComparer.Ordinal);

    public async Task<PluginInventory> GetInventoryAsync(CancellationToken cancellationToken)
    {
        await _gate.WaitAsync(cancellationToken).ConfigureAwait(false);
        try
        {
            var settings = ReadSettings();
            var installed = ReadInstalled();
            var plugins = new List<ManagedPlugin>
            {
                Describe(PluginId, "插件管理", "0.1.0", "查看插件状态，安装和管理本地扩展。", "Hydrogen",
                    PluginOrigin.BuiltIn, null, [], null, settings),
            };
            plugins.AddRange(installed.Select(item => Describe(item.Id, item.Package?.Name ?? item.Id,
                item.Package?.Version ?? "未知", item.Package?.Description ?? "无法读取此插件的清单。", item.Package?.Author ?? "未知作者",
                IsOfficial(item.Id) ? PluginOrigin.BuiltIn : PluginOrigin.Installed, item.Directory,
                item.Package?.Dependencies ?? [], item.Problem, settings)));
            var notices = new[] { _stateProblem, _catalogProblem, _startupNotice }.Where(message => message is not null).ToArray();
            return new PluginInventory(plugins.AsReadOnly(), notices.Length == 0 ? null : string.Join("\n", notices),
                _stateProblem is not null || _catalogProblem is not null);
        }
        finally { _gate.Release(); }
    }

    private ManagedPlugin Describe(string id, string name, string version, string description, string author,
        PluginOrigin origin, string? directory, IReadOnlyList<string> declaredDependencies, string? problem, SavedSettings settings)
    {
        var preference = Preference(id, settings);
        var enabled = Enabled(id, settings, origin == PluginOrigin.BuiltIn);
        _runtime.TryGetValue(id, out var runtime);
        if (_loadProblems.TryGetValue(id, out var loadProblem)) problem ??= loadProblem;
        var state = runtime?.State switch
        {
            PluginState.Active => PluginRunState.Active,
            PluginState.Waiting => PluginRunState.Waiting,
            PluginState.Failed => PluginRunState.Failed,
            _ => _waiting.Contains(id) ? PluginRunState.Waiting : problem is not null ? PluginRunState.Failed
                : _startupEnabled.Contains(id) ? PluginRunState.NotLoaded : PluginRunState.Disabled,
        };
        if (runtime?.State == PluginState.Waiting) problem = "等待服务：" + string.Join("、", runtime.MissingServices.Select(type => type.Name));
        if (runtime?.State == PluginState.Failed) problem = "插件激活失败，请检查插件版本及其依赖。";
        var dependencies = Dependencies(id).Concat(declaredDependencies).Distinct(StringComparer.Ordinal).ToArray();
        var provides = _definitions.TryGetValue(id, out var definition) ? definition.Provides.Select(type => type.Name).ToArray() : [];
        return new ManagedPlugin(id, name, version, description, author, origin, state, enabled, _required.Contains(id),
            preference.PendingRemoval, preference.PendingRemoval || enabled != _startupEnabled.Contains(id), problem,
            _required.Contains(id) ? "主窗口、主界面和插件管理所依赖的必需插件，不能停用或卸载。" : null,
            dependencies, provides, directory);
    }

    private IEnumerable<string> Dependencies(string id)
    {
        if (!_definitions.TryGetValue(id, out var definition)) return [];
        return definition.Requires.SelectMany(contract => _definitions.Values.Where(provider => provider.Provides.Contains(contract)).Select(provider => provider.Id));
    }

    public Task SetEnabledAsync(string id, bool enabled, CancellationToken cancellationToken)
        => ChangeAsync(async (settings, installed, token) =>
        {
            EnsureKnown(id, installed);
            if (_required.Contains(id) && !enabled) throw new PluginManagementException("此插件是启动器的必需依赖，不能停用。 ");
            if (Preference(id, settings).PendingRemoval) throw new PluginManagementException("请先撤销此插件的待卸载操作。 ");
            var item = installed.FirstOrDefault(item => item.Id == id);
            if (enabled && item?.Problem is not null) throw new PluginManagementException(item.Problem);
            var dependencies = Dependencies(id).Concat(item?.Package?.Dependencies ?? []).Distinct(StringComparer.Ordinal);
            if (enabled)
            {
                var missing = dependencies.Where(dependency => !Enabled(dependency, settings, _definitions.ContainsKey(dependency)
                        && IsOfficial(dependency)) || Preference(dependency, settings).PendingRemoval
                        || (!_definitions.ContainsKey(dependency) && !installed.Any(plugin => plugin.Id == dependency && plugin.Problem is null))).ToArray();
                if (missing.Length > 0) throw new PluginManagementException("请先安装并启用依赖插件：" + string.Join("、", missing));
            }
            else EnsureNoDependents(id, settings, installed);
            settings.Plugins[id] = new PluginPreference(enabled, false);
            await SaveSettingsAsync(settings, token).ConfigureAwait(false);
        }, cancellationToken);

    public Task UninstallAsync(string id, CancellationToken cancellationToken)
        => ChangeAsync(async (settings, installed, token) =>
        {
            if (IsOfficial(id)) throw new PluginManagementException("官方插件随启动器发布；可选插件可以停用，文件由启动器维护。 ");
            EnsureKnown(id, installed);
            EnsureNoDependents(id, settings, installed);
            settings.Plugins[id] = Preference(id, settings) with { PendingRemoval = true };
            await SaveSettingsAsync(settings, token).ConfigureAwait(false);
        }, cancellationToken);

    public Task UndoUninstallAsync(string id, CancellationToken cancellationToken)
        => ChangeAsync(async (settings, installed, token) =>
        {
            EnsureKnown(id, installed);
            settings.Plugins[id] = Preference(id, settings) with { PendingRemoval = false };
            await SaveSettingsAsync(settings, token).ConfigureAwait(false);
        }, cancellationToken);

    public Task InstallAsync(string packagePath, CancellationToken cancellationToken)
        => ChangeAsync(async (settings, installed, token) =>
        {
            var stage = PackageFiles.Within(PluginsDirectory, ".install-" + Guid.NewGuid().ToString("N"));
            Directory.CreateDirectory(stage);
            string? committed = null;
            try
            {
                await PackageFiles.ExtractAsync(packagePath, stage, token).ConfigureAwait(false);
                var package = PluginPackage.Read(Path.Combine(stage, "plugin.json"));
                if (IsOfficial(package.Id) || installed.Any(plugin => plugin.Id == package.Id))
                    throw new PluginManagementException("此插件 ID 已经存在，请先卸载原有外置插件并重启。 ");
                if (!File.Exists(PackageFiles.Within(stage, package.EntryAssembly)))
                    throw new PluginManagementException("插件包缺少 entryAssembly 指定的入口 DLL。 ");
                var destination = PackageFiles.Within(PluginsDirectory, package.Id);
                token.ThrowIfCancellationRequested();
                Directory.Move(stage, destination);
                committed = destination;
                settings.Plugins[package.Id] = new PluginPreference(true, false);
                await SaveSettingsAsync(settings, token).ConfigureAwait(false);
                committed = null;
            }
            finally
            {
                if (committed is not null) PackageFiles.DeleteOwnedDirectory(PluginsDirectory, committed);
                PackageFiles.DeleteOwnedDirectory(PluginsDirectory, stage);
            }
        }, cancellationToken);

    private void EnsureNoDependents(string id, SavedSettings settings, List<InstalledPackage> installed)
    {
        var dependents = _definitions.Keys.Concat(installed.Select(item => item.Id)).Distinct(StringComparer.Ordinal)
            .Where(candidate => candidate != id && Enabled(candidate, settings, IsOfficial(candidate))
                && !Preference(candidate, settings).PendingRemoval
                && Dependencies(candidate).Concat(installed.FirstOrDefault(item => item.Id == candidate)?.Package?.Dependencies ?? []).Contains(id)).ToArray();
        if (dependents.Length > 0) throw new PluginManagementException("请先停用依赖此插件的扩展：" + string.Join("、", dependents));
    }

    private void EnsureKnown(string id, List<InstalledPackage> installed)
    {
        if (!PluginPackage.IsId(id) || (!IsOfficial(id) && !installed.Any(plugin => plugin.Id == id)))
            throw new PluginManagementException("未找到此插件，请刷新列表。 ");
    }

    private async Task ChangeAsync(Func<SavedSettings, List<InstalledPackage>, CancellationToken, Task> change, CancellationToken token)
    {
        await _gate.WaitAsync(token).ConfigureAwait(false);
        try
        {
            Directory.CreateDirectory(PluginsDirectory);
            if ((File.GetAttributes(PluginsDirectory) & FileAttributes.ReparsePoint) != 0)
                throw new PluginManagementException("插件根目录不能使用符号链接或目录联接。 ");
            await using var fileLock = AcquireFileLock();
            var settings = ReadSettings();
            if (_stateProblem is not null) throw new PluginManagementException(_stateProblem);
            var installed = ReadInstalled();
            if (_catalogProblem is not null) throw new PluginManagementException(_catalogProblem);
            await change(settings, installed, token).ConfigureAwait(false);
        }
        catch (OperationCanceledException) when (token.IsCancellationRequested) { throw; }
        catch (Exception error) when (error is IOException or UnauthorizedAccessException or System.IO.InvalidDataException)
        { throw new PluginManagementException("无法读取或保存插件文件。请检查插件包、目录权限及是否有其他启动器正在修改插件。 "); }
        finally { _gate.Release(); }
    }

    private FileStream AcquireFileLock()
    {
        if ((File.GetAttributes(PluginsDirectory) & FileAttributes.ReparsePoint) != 0)
            throw new PluginManagementException("插件根目录不能使用符号链接或目录联接。");
        return new FileStream(Path.Combine(PluginsDirectory, ".lock"), FileMode.OpenOrCreate, FileAccess.ReadWrite, FileShare.None);
    }

    private SavedSettings ReadSettings()
    {
        _stateProblem = null;
        if (!File.Exists(StatePath)) return new SavedSettings();
        try
        {
            if (new FileInfo(StatePath).Length > 1024 * 1024) throw new JsonException();
            var settings = JsonSerializer.Deserialize<SavedSettings>(File.ReadAllText(StatePath), PluginPackage.Json);
            if (settings is null || settings.SchemaVersion != 1 || settings.Plugins is null
                || settings.Plugins.Any(pair => !PluginPackage.IsId(pair.Key) || pair.Value is null)) throw new JsonException();
            return settings;
        }
        catch (Exception error) when (error is IOException or UnauthorizedAccessException or JsonException)
        {
            _stateProblem = "插件配置无法读取，已暂停外置插件加载和配置修改。请检查插件目录中的 plugins.json 后重启。";
            return new SavedSettings();
        }
    }

    private async Task SaveSettingsAsync(SavedSettings settings, CancellationToken token)
    {
        Directory.CreateDirectory(PluginsDirectory);
        var temporary = Path.Combine(PluginsDirectory, ".settings-" + Guid.NewGuid().ToString("N"));
        try
        {
            await File.WriteAllTextAsync(temporary, JsonSerializer.Serialize(settings, PluginPackage.Json), token).ConfigureAwait(false);
            token.ThrowIfCancellationRequested();
            File.Move(temporary, StatePath, overwrite: true);
        }
        finally { if (File.Exists(temporary)) File.Delete(temporary); }
    }

    private bool Enabled(string id, SavedSettings settings, bool builtIn)
        => _required.Contains(id) || ((_stateProblem is null || builtIn) && Preference(id, settings).Enabled);

    private PluginPreference Preference(string id, SavedSettings settings)
        => settings.Plugins.GetValueOrDefault(id) ?? new PluginPreference(_builtIns.FirstOrDefault(plugin => plugin.Id == id)?.EnabledByDefault ?? true, false);

    private List<InstalledPackage> ReadInstalled()
    {
        var packages = new List<InstalledPackage>();
        _catalogProblem = null;
        if (!Directory.Exists(PluginsDirectory)) return packages;
        string[] directories;
        try
        {
            if ((File.GetAttributes(PluginsDirectory) & FileAttributes.ReparsePoint) != 0)
                throw new PluginManagementException("插件根目录不能使用符号链接或目录联接。");
            directories = Directory.GetDirectories(PluginsDirectory);
        }
        catch (Exception error) when (error is IOException or UnauthorizedAccessException or PluginManagementException)
        {
            _catalogProblem = "无法读取插件目录，已暂停外置插件加载和配置修改。请检查目录权限及是否使用了目录联接。";
            return packages;
        }
        foreach (var directory in directories.Order(StringComparer.Ordinal))
        {
            var id = Path.GetFileName(directory);
            if (!PluginPackage.IsId(id)) continue;
            try
            {
                if (IsOfficial(id) && !_builtIns.Any(plugin => plugin.Id == id))
                {
                    _startupNotice = "已跳过当前发行未提供或由宿主直接管理的官方插件目录；原有启停设置已保留。";
                    continue;
                }
                PackageFiles.CheckNoLinks(directory);
                var package = PluginPackage.Read(Path.Combine(directory, "plugin.json"));
                if (package.Id != id) throw new PluginManagementException("插件目录名与清单 ID 不一致。 ");
                packages.Add(new InstalledPackage(id, directory, package, null));
            }
            catch (Exception error) when (error is IOException or UnauthorizedAccessException or PluginManagementException)
            {
                packages.Add(new InstalledPackage(id, directory, null,
                    error is PluginManagementException managed ? managed.Message.Trim() : "无法读取插件清单，请检查插件文件。"));
            }
        }
        return packages;
    }

    public void Dispose()
    {
        AssemblyLoadContext.Default.Resolving -= ResolveOfficialAssembly;
        foreach (var load in _loadContexts) load.Unload();
        _loadContexts.Clear();
        _definitions.Clear();
        _runtime = new Dictionary<string, PluginSnapshot>();
        _gate.Dispose();
    }

    private sealed record PluginPreference(bool Enabled, bool PendingRemoval);
    private sealed record SavedSettings
    {
        public int SchemaVersion { get; init; } = 1;
        public Dictionary<string, PluginPreference> Plugins { get; init; } = new(StringComparer.Ordinal);
    }
    private sealed record InstalledPackage(string Id, string Directory, PluginPackage? Package, string? Problem);
}
