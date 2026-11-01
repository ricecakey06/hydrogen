using System.Text.Json;
using Launcher.Contracts;
using Launcher.Plugins.Storage;

namespace Launcher.Launch.Tests;

public sealed class StorageTests
{
    [Fact]
    public async Task A_user_writable_locator_can_select_data_when_the_executable_location_is_not_a_directory()
    {
        await using var test = await TestScope.CreateAsync();
        Directory.CreateDirectory(test.Root);
        var installPath = Path.Combine(test.Root, "blocked-install-location");
        await File.WriteAllTextAsync(installPath, "installation is not writable as a directory");
        var locator = Path.Combine(test.Root, "user configuration", "data-location.json");
        var storage = new LauncherStorage(test.Context, installPath, locator);
        await storage.InitializeAsync(default);
        var data = Path.Combine(test.Root, "chosen data");
        Assert.Empty(await storage.ChangeDataDirectoryAsync(data, default));
        Assert.True(File.Exists(locator));
        var reopened = new LauncherStorage(test.Context, installPath, locator);
        await reopened.InitializeAsync(default);
        Assert.Equal(data, reopened.Root);
        Assert.Null(reopened.Problem);
    }

    [Fact]
    public async Task A_migrated_settings_file_resets_machine_specific_java_without_losing_general_settings()
    {
        await using var test = await TestScope.CreateAsync();
        var settings = LauncherStorage.Defaults with { Java = new(Path.Combine(test.Root, "java", "bin", "java")), MaxMemoryMiB = 1024 };
        Assert.Empty(await test.Storage.SaveAsync(settings, default));
        var path = Path.Combine(test.Root, "settings.json");
        var saved = await File.ReadAllTextAsync(path);
        saved = saved.Replace(SettingsValidation.MachineId, "another-machine", StringComparison.Ordinal);
        await File.WriteAllTextAsync(path, saved);
        await using var reopened = await TestScope.CreateAsync(test.Root);
        Assert.Null(reopened.Storage.Settings.Java.ExecutablePath);
        Assert.Equal(1024, reopened.Storage.Settings.MaxMemoryMiB);
        Assert.Null(reopened.Storage.Problem);
    }

    [Fact]
    public async Task A_detached_live_process_marker_prevents_relaunch_after_the_host_releases_its_file_handle()
    {
        await using var test = await TestScope.CreateAsync();
        var instance = await test.Instances.CreateAsync("26.1.2", default);
        using (var lease = test.Instances.AcquireLaunch(instance.Id)) lease.MarkRunning(Environment.ProcessId);
        await using var other = await TestScope.CreateAsync(test.Root);
        var failure = Assert.Throws<GameLaunchException>(() => other.Instances.AcquireLaunch(instance.Id));
        Assert.Equal(LaunchFailureKind.InstanceInUse, failure.Kind);
        Assert.NotEmpty(await other.Instances.SaveOverridesAsync(instance.Id, new(null, 1024, null), default));
        await Assert.ThrowsAsync<LauncherOperationException>(() => other.Instances.DeleteAsync(instance.Id, default));
        Assert.Single(other.Instances.Instances);
    }

    [Fact]
    public async Task Two_hosts_can_create_same_version_instances_but_cannot_reuse_an_installed_name()
    {
        await using var first = await TestScope.CreateAsync();
        await using var second = await TestScope.CreateAsync(first.Root);
        var instance = await first.Instances.CreateAsync("26.1.2", "Survival", default);
        await first.CompleteInstanceAsync(instance.Id);
        var another = await second.Instances.CreateAsync("26.1.2", "红石", default);
        Assert.NotEqual(instance.Id, another.Id);
        await Assert.ThrowsAsync<LauncherOperationException>(() => second.Instances.CreateAsync("26.1.2", "SURVIVAL", default));
        await second.Instances.LoadAsync(default);
        Assert.Equal(2, second.Instances.Instances.Count);
        Assert.Equal(2, Directory.EnumerateDirectories(Path.Combine(first.Root, "instances")).Count());
    }

    [Fact]
    public async Task Same_version_instances_keep_separate_saves_and_preserve_names_across_renames_and_stale_hosts()
    {
        await using var test = await TestScope.CreateAsync();
        var first = await test.Instances.CreateAsync("26.1.2", "生存", default);
        var second = await test.Instances.CreateAsync("26.1.2", "Redstone", default);
        await test.CompleteInstanceAsync(first.Id);
        await test.CompleteInstanceAsync(second.Id);
        await test.Instances.SelectAsync(second.Id, default);
        var game = test.Instances.GetGameDirectory(first.Id);
        var otherGame = test.Instances.GetGameDirectory(second.Id);
        Assert.NotEqual(game, otherGame);
        await File.WriteAllTextAsync(Path.Combine(game, "options.txt"), "first instance options");
        Directory.CreateDirectory(Path.Combine(game, "saves"));
        await File.WriteAllTextAsync(Path.Combine(game, "saves", "level.dat"), "first instance world");
        Assert.False(File.Exists(Path.Combine(otherGame, "options.txt")));
        Assert.False(Directory.Exists(Path.Combine(otherGame, "saves")));

        var record = Path.Combine(test.Root, "instances", first.Id, "instance.json");
        var before = await File.ReadAllTextAsync(record);
        await Assert.ThrowsAsync<LauncherOperationException>(() => test.Instances.RenameAsync(first.Id, "REDSTONE", default));
        await Assert.ThrowsAsync<LauncherOperationException>(() => test.Instances.RenameAsync(first.Id, " ", default));
        await Assert.ThrowsAsync<LauncherOperationException>(() => test.Instances.RenameAsync(first.Id, "bad\nname", default));
        await Assert.ThrowsAsync<LauncherOperationException>(() => test.Instances.RenameAsync(first.Id, new string('a', 65), default));
        Assert.Equal(before, await File.ReadAllTextAsync(record));

        await using var stale = await TestScope.CreateAsync(test.Root);
        await test.Instances.RenameAsync(first.Id, "  生存 · 模组  ", default);
        Assert.Equal(game, test.Instances.GetGameDirectory(first.Id));
        var overrides = new InstanceOverrides(null, 1024, "-Dinstance=one");
        Assert.Empty(await stale.Instances.SaveOverridesAsync(first.Id, overrides, default));
        await using var reopened = await TestScope.CreateAsync(test.Root);
        Assert.Equal("生存 · 模组", reopened.Instances.Instances.Single(instance => instance.Id == first.Id).Name);
        Assert.Equal(overrides, reopened.Instances.GetOverrides(first.Id));
        Assert.Equal(second.Id, reopened.Instances.SelectedInstanceId);
        Assert.All(reopened.Instances.Instances, instance => Assert.True(instance.IsInstalled));
        Assert.Equal("first instance world", await File.ReadAllTextAsync(Path.Combine(game, "saves", "level.dat")));
        Assert.Equal("first instance options", await File.ReadAllTextAsync(Path.Combine(game, "options.txt")));
    }

    [Fact]
    public async Task Default_names_are_unique_and_cancelled_pending_instances_can_resume()
    {
        await using var test = await TestScope.CreateAsync();
        var first = await test.Instances.CreateAsync("26.1.2", default);
        Assert.Equal(first.Id, (await test.Instances.CreateAsync("26.1.2", default)).Id);
        await test.CompleteInstanceAsync(first.Id);
        var second = await test.Instances.CreateAsync("26.1.2", default);
        Assert.NotEqual(first.Id, second.Id);
        Assert.Equal("26.1.2 (2)", second.Name);
        Assert.NotEqual(test.Instances.GetGameDirectory(first.Id), test.Instances.GetGameDirectory(second.Id));
    }

    [Fact]
    public async Task Deletion_removes_only_the_target_instance_and_preserves_same_version_data_and_shared_files()
    {
        await using var test = await TestScope.CreateAsync();
        var first = await test.Instances.CreateAsync("26.1.2", "生存", default);
        var second = await test.Instances.CreateAsync("26.1.2", "红石", default);
        await test.CompleteInstanceAsync(first.Id);
        await test.CompleteInstanceAsync(second.Id);
        await test.Instances.SelectAsync(first.Id, default);
        Assert.Empty(await test.Instances.SaveOverridesAsync(second.Id, new(null, 1024, "-Dkeep=yes"), default));
        var firstGame = test.Instances.GetGameDirectory(first.Id);
        var secondGame = test.Instances.GetGameDirectory(second.Id);
        foreach (var game in new[] { firstGame, secondGame })
        {
            Directory.CreateDirectory(Path.Combine(game, "saves", "world"));
            await File.WriteAllTextAsync(Path.Combine(game, "saves", "world", "level.dat"), game);
        }
        var shared = new[] { "versions/26.1.2/26.1.2.jar", "versions/26.1.2/26.1.2.json", "libraries/test.jar", "assets/objects/ab/test" };
        foreach (var relative in shared)
        {
            var path = Path.Combine(test.Storage.GameRoot, relative);
            Directory.CreateDirectory(Path.GetDirectoryName(path)!);
            await File.WriteAllTextAsync(path, relative);
        }
        var otherRecord = Path.Combine(test.Root, "instances", second.Id, "instance.json");
        var otherBefore = await File.ReadAllTextAsync(otherRecord);
        await using var stale = await TestScope.CreateAsync(test.Root);
        var deleted = new List<string>();
        test.Context.Subscribe<InstanceDeleted>((change, _) => { deleted.Add(change.InstanceId); return ValueTask.CompletedTask; });

        await test.Instances.DeleteAsync(first.Id, default);
        Assert.Equal(new[] { first.Id }, deleted);
        Assert.False(Directory.Exists(firstGame));
        Assert.False(Directory.Exists(Path.Combine(test.Root, "instances", first.Id)));
        Assert.Equal(second.Id, Assert.Single(test.Instances.Instances).Id);
        Assert.Equal(second.Id, test.Instances.SelectedInstanceId);
        Assert.Equal(otherBefore, await File.ReadAllTextAsync(otherRecord));
        Assert.Equal(secondGame, await File.ReadAllTextAsync(Path.Combine(secondGame, "saves", "world", "level.dat")));
        foreach (var relative in shared) Assert.Equal(relative, await File.ReadAllTextAsync(Path.Combine(test.Storage.GameRoot, relative)));
        await Assert.ThrowsAsync<LauncherOperationException>(() => stale.Instances.SelectAsync(first.Id, default));
        Assert.NotEmpty(await stale.Instances.SaveOverridesAsync(first.Id, new(null, 2048, null), default));
        Assert.False(File.Exists(Path.Combine(test.Root, "instances", first.Id, "instance.json")));

        await using var reopened = await TestScope.CreateAsync(test.Root);
        Assert.Equal(second.Id, reopened.Instances.SelectedInstanceId);
        Assert.Equal(second.Id, Assert.Single(reopened.Instances.Instances).Id);
        var pending = await reopened.Instances.CreateAsync("26.1.2", "生存", default);
        Assert.NotEqual(first.Id, pending.Id);
        await reopened.Instances.DeleteAsync(pending.Id, default);
        Assert.Equal(second.Id, reopened.Instances.SelectedInstanceId);
        await reopened.Instances.DeleteAsync(second.Id, default);
        Assert.Null(reopened.Instances.SelectedInstanceId);
        Assert.Empty(reopened.Instances.Instances);
        foreach (var relative in shared) Assert.Equal(relative, await File.ReadAllTextAsync(Path.Combine(test.Storage.GameRoot, relative)));
        await reopened.Instances.LoadAsync(default);
        Assert.Null(reopened.Instances.SelectedInstanceId);
        Assert.Empty(reopened.Instances.Instances);
    }

    [Fact]
    public async Task Deletion_refuses_an_in_use_instance_or_cancelled_request_before_removing_data()
    {
        await using var test = await TestScope.CreateAsync();
        var instance = await test.Instances.CreateAsync("26.1.2", default);
        var game = test.Instances.GetGameDirectory(instance.Id);
        var options = Path.Combine(game, "options.txt");
        await File.WriteAllTextAsync(options, "keep");
        await using var other = await TestScope.CreateAsync(test.Root);
        using (test.Instances.AcquireLaunch(instance.Id))
        {
            await Assert.ThrowsAsync<LauncherOperationException>(() => test.Instances.DeleteAsync(instance.Id, default));
            await Assert.ThrowsAsync<LauncherOperationException>(() => other.Instances.DeleteAsync(instance.Id, default));
        }
        using var cancelled = new CancellationTokenSource();
        cancelled.Cancel();
        await Assert.ThrowsAnyAsync<OperationCanceledException>(() => test.Instances.DeleteAsync(instance.Id, cancelled.Token));
        Assert.Equal("keep", await File.ReadAllTextAsync(options));
        Assert.Single(test.Instances.Instances);
        await test.Instances.DeleteAsync(instance.Id, default);
        Assert.False(Directory.Exists(game));
    }

    [Fact]
    public async Task Deletion_preserves_legacy_shared_directories_and_rejects_tampered_directory_paths()
    {
        await using var test = await TestScope.CreateAsync();
        var instance = await test.Instances.CreateAsync("26.1.2", default);
        var record = Path.Combine(test.Root, "instances", instance.Id, "instance.json");
        var document = System.Text.Json.Nodes.JsonNode.Parse(await File.ReadAllTextAsync(record))!;
        var privateGame = test.Instances.GetGameDirectory(instance.Id);
        await File.WriteAllTextAsync(Path.Combine(privateGame, "options.txt"), "private data");
        document["gameDirectoryName"] = "../libraries";
        await File.WriteAllTextAsync(record, document.ToJsonString());
        await Assert.ThrowsAsync<LauncherOperationException>(() => test.Instances.DeleteAsync(instance.Id, default));
        Assert.Equal("private data", await File.ReadAllTextAsync(Path.Combine(privateGame, "options.txt")));
        document["gameDirectoryName"] = instance.GameVersion;
        await File.WriteAllTextAsync(record, document.ToJsonString());
        var legacy = Path.Combine(test.Storage.GameRoot, "versions", instance.GameVersion);
        Directory.CreateDirectory(legacy);
        await File.WriteAllTextAsync(Path.Combine(legacy, "options.txt"), "legacy settings");
        await File.WriteAllTextAsync(Path.Combine(legacy, "26.1.2.jar"), "shared client");
        var oldCopy = Path.Combine(test.Root, "instances", instance.Id, ".minecraft");
        Directory.CreateDirectory(oldCopy);
        await File.WriteAllTextAsync(Path.Combine(oldCopy, "options.txt"), "old private copy");
        await test.Instances.DeleteAsync(instance.Id, default);
        Assert.Equal("legacy settings", await File.ReadAllTextAsync(Path.Combine(legacy, "options.txt")));
        Assert.Equal("shared client", await File.ReadAllTextAsync(Path.Combine(legacy, "26.1.2.jar")));
        Assert.False(Directory.Exists(oldCopy));
        Assert.Empty(test.Instances.Instances);
    }

    [Fact]
    public async Task Settings_instances_selection_and_overrides_survive_restart_and_keep_game_data_isolated()
    {
        await using var test = await TestScope.CreateAsync();
        var saved = LauncherStorage.Defaults with { MaxMemoryMiB = 1024, JvmArguments = "-Dexample=\"a b\"" };
        Assert.Empty(await test.Storage.SaveAsync(saved, default));
        var first = await test.Instances.CreateAsync("26.1.2", default);
        var second = await test.Instances.CreateAsync("1.21.1", default);
        await test.CompleteInstanceAsync(first.Id);
        await test.CompleteInstanceAsync(second.Id);
        var overrides = new InstanceOverrides(new JavaChoice(Path.Combine(test.Root, "Java 25", "java.exe")), 1536, "-Dinstance=yes");
        Assert.Empty(await test.Instances.SaveOverridesAsync(first.Id, overrides, default));
        await test.Instances.SelectAsync(second.Id, default);
        var firstGame = test.Instances.GetGameDirectory(first.Id);
        var secondGame = test.Instances.GetGameDirectory(second.Id);
        Directory.CreateDirectory(Path.Combine(firstGame, "saves"));
        await File.WriteAllTextAsync(Path.Combine(firstGame, "options.txt"), "first settings");
        Assert.NotEqual(firstGame, secondGame);
        Assert.False(File.Exists(Path.Combine(secondGame, "options.txt")));
        await using var restarted = await TestScope.CreateAsync(test.Root);
        Assert.Equal(saved, restarted.Storage.Settings);
        Assert.Equal(2, restarted.Instances.Instances.Count);
        Assert.Equal(second.Id, restarted.Instances.SelectedInstanceId);
        Assert.Equal(overrides, restarted.Instances.GetOverrides(first.Id));
        Assert.Equal("first settings", await File.ReadAllTextAsync(Path.Combine(firstGame, "options.txt")));
        Assert.Empty(Directory.EnumerateFiles(test.Root, "*.tmp", SearchOption.AllDirectories));
    }

    [Fact]
    public async Task Future_or_corrupt_settings_are_preserved_and_writes_are_rejected()
    {
        await using var test = await TestScope.CreateAsync();
        Directory.CreateDirectory(test.Root);
        var path = Path.Combine(test.Root, "settings.json");
        const string future = "{\"schemaVersion\":99,\"customFutureData\":\"keep me\"}";
        await File.WriteAllTextAsync(path, future);
        await using var reopened = await TestScope.CreateAsync(test.Root);
        Assert.NotNull(reopened.Storage.Problem);
        Assert.NotEmpty(await reopened.Storage.SaveAsync(LauncherStorage.Defaults, default));
        Assert.Equal(future, await File.ReadAllTextAsync(path));
        Assert.NotEmpty(await test.Storage.SaveAsync(LauncherStorage.Defaults, default));
        Assert.Equal(future, await File.ReadAllTextAsync(path));
    }

    [Fact]
    public async Task Invalid_settings_do_not_replace_a_valid_file()
    {
        await using var test = await TestScope.CreateAsync();
        Assert.Empty(await test.Storage.SaveAsync(LauncherStorage.Defaults, default));
        var path = Path.Combine(test.Root, "settings.json");
        var before = await File.ReadAllTextAsync(path);
        Assert.NotEmpty(await test.Storage.SaveAsync(LauncherStorage.Defaults with { MaxMemoryMiB = 64, JvmArguments = "-XX:MaxHeapSize=64G" }, default));
        Assert.Equal(before, await File.ReadAllTextAsync(path));
    }

    [Fact]
    public async Task Switching_roots_discovers_existing_instances_and_never_moves_game_data()
    {
        await using var test = await TestScope.CreateAsync();
        var old = await test.Instances.CreateAsync("26.1.2", default);
        var oldGame = test.Instances.GetGameDirectory(old.Id);
        await File.WriteAllTextAsync(Path.Combine(oldGame, "options.txt"), "untouched");
        var next = Path.Combine(Path.GetDirectoryName(test.Root)!, "other data");
        Assert.Empty(await test.Storage.ChangeDataDirectoryAsync(next, default));
        Assert.Empty(test.Instances.Instances);
        var created = await test.Instances.CreateAsync("1.21.1", default);
        await test.CompleteInstanceAsync(created.Id);
        await test.Instances.SelectAsync(created.Id, default);
        await using var restarted = await TestScope.CreateAsync(test.Root);
        Assert.Equal(next, restarted.Storage.Root);
        Assert.Equal(created.Id, restarted.Instances.SelectedInstanceId);
        Assert.Equal("untouched", await File.ReadAllTextAsync(Path.Combine(oldGame, "options.txt")));
        Assert.Empty(await test.Storage.ChangeDataDirectoryAsync(test.Root, default));
        Assert.Equal(old.Id, Assert.Single(test.Instances.Instances).Id);
    }

    [Fact]
    public async Task Launch_leases_reject_root_changes_and_instance_edits_and_lock_across_hosts()
    {
        await using var test = await TestScope.CreateAsync();
        var instance = await test.Instances.CreateAsync("26.1.2", default);
        await using var other = await TestScope.CreateAsync(test.Root);
        using (var lease = test.Instances.AcquireLaunch(instance.Id))
        {
            Assert.NotEmpty(await test.Storage.ChangeDataDirectoryAsync(Path.Combine(Path.GetDirectoryName(test.Root)!, "next"), default));
            Assert.NotEmpty(await test.Instances.SaveOverridesAsync(instance.Id, new(null, 1024, null), default));
            Assert.Throws<IOException>(() => other.Instances.AcquireLaunch(instance.Id));
        }
        using var unlocked = other.Instances.AcquireLaunch(instance.Id);
        Assert.Equal(instance.Id, unlocked.Instance.Id);
    }

    [Fact]
    public async Task Unsupported_instance_schema_is_reported_and_not_overwritten()
    {
        await using var test = await TestScope.CreateAsync();
        var instance = await test.Instances.CreateAsync("26.1.2", default);
        var path = Path.Combine(test.Root, "instances", instance.Id, "instance.json");
        var json = await File.ReadAllTextAsync(path);
        using var document = JsonDocument.Parse(json);
        var future = json.Replace("\"schemaVersion\": 1", "\"schemaVersion\": 7", StringComparison.Ordinal);
        await File.WriteAllTextAsync(path, future);
        await test.Instances.LoadAsync(default);
        Assert.NotNull(test.Instances.Notice);
        await Assert.ThrowsAsync<LauncherOperationException>(() => test.Instances.CreateAsync("1.21.1", default));
        Assert.Equal(future, await File.ReadAllTextAsync(path));
    }
}
