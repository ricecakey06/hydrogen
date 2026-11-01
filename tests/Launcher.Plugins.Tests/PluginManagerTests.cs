using System.IO.Compression;
using System.Text.Json;
using Launcher.Contracts;
using Launcher.PluginFixtures;
using Launcher.Plugins;
using Launcher.Runtime;

namespace Launcher.Plugins.Tests;

public sealed class PluginManagerTests : IDisposable
{
    private readonly string _root = Path.Combine(Path.GetTempPath(), "hydrogen-plugin-tests", Guid.NewGuid().ToString("N"));
    private string DirectoryPath => Path.Combine(_root, "Plugins");

    private PluginManager Create(string version = "1.0.0") => new(DirectoryPath,
    [
        Official("core.app", typeof(CoreAppModule), version),
        Official("core.data", typeof(CoreDataModule), version),
        Official("optional.effect", typeof(OptionalEffectModule), version),
    ], ["core.app"]);

    private static BundledPlugin Official(string id, Type entry, string version = "1.0.0", bool enabled = true)
        => new(id, id, "Official fixture", "Fixture.dll", entry.FullName!, File.ReadAllBytes(entry.Assembly.Location),
            version, EnabledByDefault: enabled);

    [Fact]
    public async Task Inventory_uses_host_state_and_protects_transitive_core_dependencies()
    {
        using var manager = Create();
        await using var host = new PluginHost(await manager.PrepareStartupAsync());
        await host.StartAsync();
        manager.RecordRuntime(await host.GetSnapshotAsync());
        var inventory = await manager.GetInventoryAsync(default);
        Assert.Equal(4, inventory.Plugins.Count);
        Assert.All(inventory.Plugins, plugin => Assert.Equal(PluginRunState.Active, plugin.State));
        Assert.True(inventory.Plugins.Single(plugin => plugin.Id == "core.data").IsRequired);
        Assert.Contains("core.data", inventory.Plugins.Single(plugin => plugin.Id == "core.app").Dependencies);
        await Assert.ThrowsAsync<PluginManagementException>(() => manager.SetEnabledAsync("core.data", false, default));
        await Assert.ThrowsAsync<PluginManagementException>(() => manager.UninstallAsync("optional.effect", default));
    }

    [Fact]
    public async Task Optional_toggle_is_saved_but_does_not_modify_the_running_composition()
    {
        using (var manager = Create())
        {
            await using var host = new PluginHost(await manager.PrepareStartupAsync());
            await host.StartAsync();
            manager.RecordRuntime(await host.GetSnapshotAsync());
            await manager.SetEnabledAsync("optional.effect", false, default);
            var plugin = (await manager.GetInventoryAsync(default)).Plugins.Single(plugin => plugin.Id == "optional.effect");
            Assert.Equal(PluginRunState.Active, plugin.State);
            Assert.False(plugin.IsEnabled);
            Assert.True(plugin.RequiresRestart);
            Assert.Equal(4, (await host.GetSnapshotAsync()).Count);
        }
        using var restarted = Create();
        Assert.DoesNotContain(await restarted.PrepareStartupAsync(), plugin => plugin.Id == "optional.effect");
        var after = (await restarted.GetInventoryAsync(default)).Plugins.Single(plugin => plugin.Id == "optional.effect");
        Assert.False(after.IsEnabled);
        Assert.False(after.RequiresRestart);
    }

    [Fact]
    public async Task Installing_does_not_execute_code_and_a_restart_loads_the_real_module()
    {
        var package = CreatePackage();
        var marker = Path.Combine(_root, "module-executed.txt");
        AppContext.SetData("Hydrogen.PluginFixture.Marker", marker);
        try
        {
            using (var manager = Create())
            {
                await manager.PrepareStartupAsync();
                await manager.InstallAsync(package, default);
                Assert.False(File.Exists(marker));
                var info = (await manager.GetInventoryAsync(default)).Plugins.Single(plugin => plugin.Id == "example.echo");
                Assert.True(info.RequiresRestart);
                Assert.Equal(PluginOrigin.Installed, info.Origin);
                await Assert.ThrowsAsync<PluginManagementException>(() => manager.InstallAsync(package, default));
            }
            using var restarted = Create();
            await using var host = new PluginHost(await restarted.PrepareStartupAsync());
            await host.StartAsync();
            restarted.RecordRuntime(await host.GetSnapshotAsync());
            Assert.True(File.Exists(marker));
            var loaded = (await restarted.GetInventoryAsync(default)).Plugins.Single(plugin => plugin.Id == "example.echo");
            Assert.Equal(PluginRunState.Active, loaded.State);
            Assert.False(loaded.RequiresRestart);
            Assert.Empty(host.GetErrors());
        }
        finally { AppContext.SetData("Hydrogen.PluginFixture.Marker", null); }
    }

    [Fact]
    public async Task Uninstall_is_deferred_can_be_undone_and_only_removes_its_own_package()
    {
        var package = CreatePackage();
        Directory.CreateDirectory(Path.Combine(_root, "Game", "saves"));
        var save = Path.Combine(_root, "Game", "saves", "level.dat");
        File.WriteAllText(save, "keep");
        using (var manager = Create())
        {
            await manager.PrepareStartupAsync();
            await manager.InstallAsync(package, default);
            await manager.UninstallAsync("example.echo", default);
            Assert.True(Directory.Exists(Path.Combine(DirectoryPath, "example.echo")));
            Assert.True((await manager.GetInventoryAsync(default)).Plugins.Single(plugin => plugin.Id == "example.echo").IsPendingRemoval);
            await manager.UndoUninstallAsync("example.echo", default);
            Assert.False((await manager.GetInventoryAsync(default)).Plugins.Single(plugin => plugin.Id == "example.echo").IsPendingRemoval);
            await manager.UninstallAsync("example.echo", default);
        }
        using var restarted = Create();
        Assert.DoesNotContain(await restarted.PrepareStartupAsync(), plugin => plugin.Id == "example.echo");
        Assert.False(Directory.Exists(Path.Combine(DirectoryPath, "example.echo")));
        Assert.Equal("keep", File.ReadAllText(save));
    }

    [Fact]
    public async Task Enabled_dependents_prevent_disabling_or_removing_their_dependency()
    {
        using var manager = Create();
        await manager.PrepareStartupAsync();
        await manager.InstallAsync(CreatePackage(), default);
        await manager.InstallAsync(CreatePackage("example.dependent", typeof(DependentModule).FullName!, ["example.echo"]), default);
        await Assert.ThrowsAsync<PluginManagementException>(() => manager.SetEnabledAsync("example.echo", false, default));
        await Assert.ThrowsAsync<PluginManagementException>(() => manager.UninstallAsync("example.echo", default));
        await manager.SetEnabledAsync("example.dependent", false, default);
        await manager.SetEnabledAsync("example.echo", false, default);
        await Assert.ThrowsAsync<PluginManagementException>(() => manager.SetEnabledAsync("example.dependent", true, default));
    }

    [Fact]
    public async Task Bad_assemblies_and_conflicting_plugins_do_not_stop_the_core()
    {
        using (var manager = Create())
        {
            await manager.PrepareStartupAsync();
            await manager.InstallAsync(CreatePackage("example.bad", bytes: [1, 2, 3]), default);
            await manager.InstallAsync(CreatePackage("example.conflict", typeof(ConflictingModule).FullName!), default);
        }
        using var restarted = Create();
        await using var host = new PluginHost(await restarted.PrepareStartupAsync());
        await host.StartAsync();
        restarted.RecordRuntime(await host.GetSnapshotAsync());
        Assert.All(await host.GetSnapshotAsync(), plugin => Assert.Equal(PluginState.Active, plugin.State));
        var inventory = await restarted.GetInventoryAsync(default);
        Assert.Equal(PluginRunState.Failed, inventory.Plugins.Single(plugin => plugin.Id == "example.bad").State);
        Assert.Contains("冲突", inventory.Plugins.Single(plugin => plugin.Id == "example.conflict").Problem);
    }

    [Fact]
    public async Task Activation_failure_is_visible_without_exposing_plugin_exception_payloads()
    {
        using (var manager = Create())
        {
            await manager.PrepareStartupAsync();
            await manager.InstallAsync(CreatePackage("example.failure", typeof(FailingModule).FullName!), default);
        }
        using var restarted = Create();
        await using var host = new PluginHost(await restarted.PrepareStartupAsync());
        await host.StartAsync();
        restarted.RecordRuntime(await host.GetSnapshotAsync());
        var info = (await restarted.GetInventoryAsync(default)).Plugins.Single(plugin => plugin.Id == "example.failure");
        Assert.Equal(PluginRunState.Failed, info.State);
        Assert.DoesNotContain("Sensitive", info.Problem);
        Assert.Equal(PluginState.Active, (await host.GetSnapshotAsync()).Single(plugin => plugin.Id == "core.app").State);
    }

    [Theory]
    [InlineData("../outside.txt")]
    [InlineData("/absolute.txt")]
    [InlineData("C:/outside.txt")]
    [InlineData("folder/../../outside.txt")]
    [InlineData("folder\\..\\outside.txt")]
    [InlineData("CON.txt")]
    [InlineData("folder./outside.txt")]
    public async Task Unsafe_archive_paths_are_rejected_and_staging_files_are_cleaned(string entry)
    {
        using var manager = Create();
        await manager.PrepareStartupAsync();
        var package = CreatePackage(extraEntry: entry);
        await Assert.ThrowsAsync<PluginManagementException>(() => manager.InstallAsync(package, default));
        Assert.False(File.Exists(Path.Combine(_root, "outside.txt")));
        Assert.Equal(new[] { "core.app", "core.data", "optional.effect" }, Directory.EnumerateDirectories(DirectoryPath).Select(Path.GetFileName).Order(StringComparer.Ordinal));
    }

    [Fact]
    public async Task Symbolic_links_inside_archives_are_rejected()
    {
        var package = CreatePackage();
        using (var archive = ZipFile.Open(package, ZipArchiveMode.Update))
        {
            var link = archive.CreateEntry("link");
            link.ExternalAttributes = unchecked((int)0xA1FF0000);
            using var writer = new StreamWriter(link.Open());
            writer.Write("../outside");
        }
        using var manager = Create();
        await manager.PrepareStartupAsync();
        await Assert.ThrowsAsync<PluginManagementException>(() => manager.InstallAsync(package, default));
        Assert.Equal(new[] { "core.app", "core.data", "optional.effect" }, Directory.EnumerateDirectories(DirectoryPath).Select(Path.GetFileName).Order(StringComparer.Ordinal));
    }

    [Fact]
    public async Task Corrupt_saved_settings_preserve_the_core_and_disable_external_code()
    {
        using (var manager = Create())
        {
            await manager.PrepareStartupAsync();
            await manager.InstallAsync(CreatePackage(), default);
        }
        File.WriteAllText(Path.Combine(DirectoryPath, "plugins.json"), "{ broken");
        using var restarted = Create();
        Assert.DoesNotContain(await restarted.PrepareStartupAsync(), plugin => plugin.Id == "example.echo");
        var inventory = await restarted.GetInventoryAsync(default);
        Assert.True(inventory.IsReadOnly);
        Assert.NotNull(inventory.Notice);
        await Assert.ThrowsAsync<PluginManagementException>(() => restarted.SetEnabledAsync("optional.effect", false, default));
        Assert.Equal("{ broken", File.ReadAllText(Path.Combine(DirectoryPath, "plugins.json")));
    }

    [Fact]
    public async Task Unsupported_manifests_and_reserved_builtin_ids_are_not_installed()
    {
        using var manager = Create();
        await manager.PrepareStartupAsync();
        await Assert.ThrowsAsync<PluginManagementException>(() => manager.InstallAsync(CreatePackage(apiVersion: 2), default));
        await Assert.ThrowsAsync<PluginManagementException>(() => manager.InstallAsync(CreatePackage("core.app"), default));
        Assert.Equal(new[] { "core.app", "core.data", "optional.effect" }, Directory.EnumerateDirectories(DirectoryPath).Select(Path.GetFileName).Order(StringComparer.Ordinal));
    }

    private string CreatePackage(string id = "example.echo", string? entryType = null, string[]? dependencies = null,
        byte[]? bytes = null, string? extraEntry = null, int apiVersion = 1)
    {
        Directory.CreateDirectory(_root);
        var path = Path.Combine(_root, Guid.NewGuid().ToString("N") + ".hplugin");
        using var archive = ZipFile.Open(path, ZipArchiveMode.Create);
        using (var writer = new StreamWriter(archive.CreateEntry("plugin.json").Open()))
            writer.Write(JsonSerializer.Serialize(new { schemaVersion = 1, apiVersion, id, name = "Test plugin", version = "1.0.0",
                description = "Local fixture", author = "Tests", entryAssembly = "Fixture.dll", entryType = entryType ?? typeof(EchoModule).FullName,
                dependencies = dependencies ?? [] }));
        using (var output = archive.CreateEntry("Fixture.dll").Open())
            output.Write(bytes ?? File.ReadAllBytes(typeof(EchoModule).Assembly.Location));
        if (extraEntry is not null)
        {
            using var writer = new StreamWriter(archive.CreateEntry(extraEntry).Open());
            writer.Write("invalid");
        }
        return path;
    }

    [Fact]
    public async Task A_locked_removal_does_not_prevent_the_core_from_starting()
    {
        using (var manager = Create())
        {
            await manager.PrepareStartupAsync();
            await manager.InstallAsync(CreatePackage(), default);
            await manager.UninstallAsync("example.echo", default);
        }
        using var fileLock = new FileStream(Path.Combine(DirectoryPath, ".lock"), FileMode.Open, FileAccess.ReadWrite, FileShare.None);
        using var restarted = Create();
        await using var host = new PluginHost(await restarted.PrepareStartupAsync());
        await host.StartAsync();
        Assert.Equal(PluginState.Active, (await host.GetSnapshotAsync()).Single(plugin => plugin.Id == "core.app").State);
        Assert.NotNull((await restarted.GetInventoryAsync(default)).Notice);
    }

    [Fact]
    public async Task Missing_package_dependencies_are_reported_as_waiting_without_executing_the_module()
    {
        using (var manager = Create())
        {
            await manager.PrepareStartupAsync();
            await manager.InstallAsync(CreatePackage("example.dependent", typeof(DependentModule).FullName!, ["example.missing"]), default);
        }
        using var restarted = Create();
        await restarted.PrepareStartupAsync();
        var info = (await restarted.GetInventoryAsync(default)).Plugins.Single(plugin => plugin.Id == "example.dependent");
        Assert.Equal(PluginRunState.Waiting, info.State);
        Assert.Contains("example.missing", info.Problem);
    }

    public void Dispose()
    {
        if (Directory.Exists(_root))
        {
            var expectedRoot = Path.GetFullPath(Path.Combine(Path.GetTempPath(), "hydrogen-plugin-tests")) + Path.DirectorySeparatorChar;
            if (!Path.GetFullPath(_root).StartsWith(expectedRoot, StringComparison.OrdinalIgnoreCase)) throw new InvalidOperationException();
            for (var attempt = 0; ; attempt++)
            {
                // Collectible load contexts release mapped DLLs asynchronously after the host and its delegates die.
                GC.Collect();
                GC.WaitForPendingFinalizers();
                GC.Collect();
                try { Directory.Delete(_root, recursive: true); break; }
                catch (Exception error) when (attempt < 3 && error is IOException or UnauthorizedAccessException) { }
            }
        }
    }

    [Fact]
    public async Task First_start_releases_official_packages_and_reuses_unchanged_files()
    {
        using (var manager = Create())
        {
            await manager.PrepareStartupAsync();
            var inventory = await manager.GetInventoryAsync(default);
            foreach (var item in inventory.Plugins.Where(item => item.Id != PluginManager.PluginId))
            {
                Assert.Equal(PluginOrigin.BuiltIn, item.Origin);
                Assert.Equal(Path.Combine(DirectoryPath, item.Id), item.Directory);
                Assert.True(File.Exists(Path.Combine(item.Directory!, "plugin.json")));
                Assert.True(File.Exists(Path.Combine(item.Directory!, "Fixture.dll")));
            }
        }
        var assembly = Path.Combine(DirectoryPath, "core.app", "Fixture.dll");
        var timestamp = DateTime.UtcNow.AddDays(-1);
        File.SetLastWriteTimeUtc(assembly, timestamp);
        using var restarted = Create();
        await restarted.PrepareStartupAsync();
        Assert.Equal(timestamp, File.GetLastWriteTimeUtc(assembly));
    }

    [Fact]
    public async Task Upgrade_preserves_official_and_external_preferences_and_external_files()
    {
        using (var manager = Create())
        {
            await manager.PrepareStartupAsync();
            await manager.SetEnabledAsync("optional.effect", false, default);
            await manager.InstallAsync(CreatePackage(), default);
            await manager.SetEnabledAsync("example.echo", false, default);
        }
        var state = File.ReadAllBytes(Path.Combine(DirectoryPath, "plugins.json"));
        var external = File.ReadAllBytes(Path.Combine(DirectoryPath, "example.echo", "Fixture.dll"));
        using var upgraded = Create("2.0.0");
        var definitions = await upgraded.PrepareStartupAsync();
        Assert.DoesNotContain(definitions, definition => definition.Id is "optional.effect" or "example.echo");
        var inventory = await upgraded.GetInventoryAsync(default);
        Assert.Equal("2.0.0", inventory.Plugins.Single(item => item.Id == "optional.effect").Version);
        Assert.Equal(state, File.ReadAllBytes(Path.Combine(DirectoryPath, "plugins.json")));
        Assert.Equal(external, File.ReadAllBytes(Path.Combine(DirectoryPath, "example.echo", "Fixture.dll")));
    }

    [Fact]
    public async Task Missing_or_tampered_official_packages_are_repaired_without_enabling_disabled_plugins()
    {
        using (var manager = Create())
        {
            await manager.PrepareStartupAsync();
            await manager.SetEnabledAsync("optional.effect", false, default);
        }
        File.Delete(Path.Combine(DirectoryPath, "core.data", "plugin.json"));
        File.WriteAllBytes(Path.Combine(DirectoryPath, "optional.effect", "Fixture.dll"), [1, 2, 3]);
        File.WriteAllText(Path.Combine(DirectoryPath, "optional.effect", "injected.dll"), "unowned dependency");
        using var repaired = Create();
        var definitions = await repaired.PrepareStartupAsync();
        Assert.Contains(definitions, definition => definition.Id == "core.data");
        Assert.DoesNotContain(definitions, definition => definition.Id == "optional.effect");
        Assert.Equal(File.ReadAllBytes(typeof(CoreAppModule).Assembly.Location),
            File.ReadAllBytes(Path.Combine(DirectoryPath, "optional.effect", "Fixture.dll")));
        Assert.False(File.Exists(Path.Combine(DirectoryPath, "optional.effect", "injected.dll")));
    }

    [Fact]
    public async Task Interrupted_directory_swap_is_recovered_on_startup()
    {
        using (var manager = Create()) await manager.PrepareStartupAsync();
        Directory.Move(Path.Combine(DirectoryPath, "core.data"), Path.Combine(DirectoryPath, ".previous-core.data"));
        using var restarted = Create();
        Assert.Contains(await restarted.PrepareStartupAsync(), definition => definition.Id == "core.data");
        Assert.False(Directory.Exists(Path.Combine(DirectoryPath, ".previous-core.data")));
    }

    [Fact]
    public async Task Official_defaults_apply_only_when_no_saved_preference_exists()
    {
        using (var manager = new PluginManager(DirectoryPath,
            [Official("optional.effect", typeof(OptionalEffectModule), enabled: false)], []))
        {
            Assert.DoesNotContain(await manager.PrepareStartupAsync(), definition => definition.Id == "optional.effect");
            await manager.SetEnabledAsync("optional.effect", true, default);
        }
        using var upgraded = new PluginManager(DirectoryPath,
            [Official("optional.effect", typeof(OptionalEffectModule), "2.0.0", enabled: false)], []);
        Assert.Contains(await upgraded.PrepareStartupAsync(), definition => definition.Id == "optional.effect");
    }

    [Fact]
    public async Task A_removed_official_module_is_not_loaded_as_an_external_plugin()
    {
        using (var manager = Create()) await manager.PrepareStartupAsync();
        using var reduced = new PluginManager(DirectoryPath,
            [Official("core.app", typeof(CoreAppModule)), Official("core.data", typeof(CoreDataModule))],
            ["core.app"], ["optional.effect"]);
        Assert.DoesNotContain(await reduced.PrepareStartupAsync(), definition => definition.Id == "optional.effect");
        await Assert.ThrowsAsync<PluginManagementException>(() => reduced.InstallAsync(
            CreatePackage("optional.effect", typeof(OptionalEffectModule).FullName!), default));
    }

    [Fact]
    public async Task Required_plugins_stay_enabled_even_if_saved_preferences_disable_them()
    {
        Directory.CreateDirectory(DirectoryPath);
        File.WriteAllText(Path.Combine(DirectoryPath, "plugins.json"),
            """{"schemaVersion":1,"plugins":{"core.app":{"enabled":false},"core.data":{"enabled":false}}}""");
        using var manager = Create();
        var definitions = await manager.PrepareStartupAsync();
        Assert.Contains(definitions, definition => definition.Id == "core.app");
        Assert.Contains(definitions, definition => definition.Id == "core.data");
        await Assert.ThrowsAsync<PluginManagementException>(() => manager.UninstallAsync("core.data", default));
    }
}
