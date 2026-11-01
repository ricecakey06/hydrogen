using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;
using Launcher.Contracts;
using Launcher.Plugins.Minecraft;
using Launcher.Plugins.Storage;

namespace Launcher.Launch.Tests;

public sealed class GameLayoutTests
{
    [Fact]
    public async Task Portable_games_are_beside_the_executable_and_custom_data_roots_keep_their_own_minecraft_directory()
    {
        await using var test = await TestScope.CreateAsync();
        var install = Path.Combine(test.Root, "portable");
        var data = Path.Combine(install, "LauncherData");
        var games = Path.Combine(install, ".minecraft");
        var storage = new LauncherStorage(test.Context, data, gameRoot: games);
        await storage.InitializeAsync(default);
        Assert.Equal(games, storage.GameRoot);
        var instances = new InstanceService(test.Context, storage, new FixedCatalog(), new StubPlatform());
        await instances.LoadAsync(default);
        var instance = await instances.CreateAsync("26.1.2", default);
        Assert.Equal(Path.Combine(games, "versions", instance.Id), instances.GetGameDirectory(instance.Id));
        Assert.Empty(await storage.ChangeDataDirectoryAsync(Path.Combine(test.Root, "custom"), default));
        Assert.Equal(Path.Combine(test.Root, "custom", ".minecraft"), storage.GameRoot);
        Assert.True(Directory.Exists(Path.Combine(games, "versions", instance.Id)));
    }

    [Fact]
    public async Task Legacy_saves_and_options_are_copied_once_without_removing_originals_or_overwriting_new_files()
    {
        await using var test = await TestScope.CreateAsync();
        var instance = await test.Instances.CreateAsync("26.1.2", default);
        var legacy = Path.Combine(test.Root, "instances", instance.Id, ".minecraft");
        Directory.CreateDirectory(Path.Combine(legacy, "saves", "My World"));
        await File.WriteAllTextAsync(Path.Combine(legacy, "saves", "My World", "level.dat"), "world data");
        await File.WriteAllTextAsync(Path.Combine(legacy, "options.txt"), "old options");
        var next = test.Instances.GetGameDirectory(instance.Id);
        await File.WriteAllTextAsync(Path.Combine(next, "options.txt"), "new options");
        var record = Path.Combine(test.Root, "instances", instance.Id, "instance.json");
        var document = JsonNode.Parse(await File.ReadAllTextAsync(record))!;
        Assert.True(document.AsObject().Remove("layoutVersion"));
        await File.WriteAllTextAsync(record, document.ToJsonString());
        await using var reopened = await TestScope.CreateAsync(test.Root);
        Assert.Null(reopened.Instances.Notice);
        Assert.Equal("world data", await File.ReadAllTextAsync(Path.Combine(next, "saves", "My World", "level.dat")));
        Assert.Equal("new options", await File.ReadAllTextAsync(Path.Combine(next, "options.txt")));
        Assert.Equal("world data", await File.ReadAllTextAsync(Path.Combine(legacy, "saves", "My World", "level.dat")));
        await File.WriteAllTextAsync(Path.Combine(legacy, "new-old-file.txt"), "do not reimport");
        await reopened.Instances.LoadAsync(default);
        Assert.False(File.Exists(Path.Combine(next, "new-old-file.txt")));
    }

    [Theory]
    [InlineData(true)]
    [InlineData(false)]
    public async Task Legacy_instances_keep_their_game_directory_and_only_complete_local_content_is_installed(bool complete)
    {
        await using var test = await TestScope.CreateAsync();
        var instance = await test.Instances.CreateAsync("26.1.2", default);
        var record = Path.Combine(test.Root, "instances", instance.Id, "instance.json");
        var document = JsonNode.Parse(await File.ReadAllTextAsync(record))!;
        document.AsObject().Remove("gameDirectoryName");
        document["instance"]!.AsObject().Remove("name");
        document["instance"]!.AsObject().Remove("isInstalled");
        await File.WriteAllTextAsync(record, document.ToJsonString());
        await test.Storage.WriteAsync(Path.Combine("instances", "index.json"), new { schemaVersion = 1, selectedInstanceId = instance.Id }, default);

        string Hash(byte[] bytes) => Convert.ToHexString(SHA1.HashData(bytes)).ToLowerInvariant();
        var client = Encoding.UTF8.GetBytes("client");
        var index = Encoding.UTF8.GetBytes("{\"objects\":{}}");
        var metadata = JsonSerializer.SerializeToUtf8Bytes(new
        {
            id = instance.GameVersion, type = "release", javaVersion = new { majorVersion = 25 },
            arguments = StubContent.Metadata().Document.GetProperty("arguments"),
            downloads = new { client = new { url = "https://unused.invalid/client.jar", sha1 = Hash(client), size = client.Length } },
            assetIndex = new { id = "legacy", url = "https://unused.invalid/index.json", sha1 = Hash(index), size = index.Length },
            libraries = Array.Empty<object>(),
        });
        var manifest = JsonSerializer.SerializeToElement(new { versions = new[]
        {
            new { id = instance.GameVersion, type = "release", url = "https://unused.invalid/version.json", sha1 = Hash(metadata) },
        } });
        await test.Storage.WriteAsync(Path.Combine("cache", "manifests", "catalog.json"), new { schemaVersion = 1, manifest }, default);
        var legacy = Path.Combine(test.Storage.GameRoot, "versions", instance.GameVersion);
        Directory.CreateDirectory(legacy);
        await File.WriteAllBytesAsync(Path.Combine(legacy, instance.GameVersion + ".json"), metadata);
        await File.WriteAllBytesAsync(Path.Combine(legacy, instance.GameVersion + ".jar"), complete ? client : Encoding.UTF8.GetBytes("broken"));
        await File.WriteAllTextAsync(Path.Combine(legacy, "options.txt"), "existing settings");
        var indexes = Path.Combine(test.Storage.GameRoot, "assets", "indexes");
        Directory.CreateDirectory(indexes);
        await File.WriteAllBytesAsync(Path.Combine(indexes, "legacy.json"), index);

        await using var reopened = await TestScope.CreateAsync(test.Root);
        Assert.Null(reopened.Instances.Notice);
        var restored = Assert.Single(reopened.Instances.Instances);
        Assert.Equal(instance.GameVersion, restored.Name);
        Assert.Equal(complete, restored.IsInstalled);
        Assert.Equal(complete ? instance.Id : null, reopened.Instances.SelectedInstanceId);
        Assert.Equal(legacy, reopened.Instances.GetGameDirectory(instance.Id));
        Assert.Equal("existing settings", await File.ReadAllTextAsync(Path.Combine(legacy, "options.txt")));
    }

    [Theory]
    [InlineData("versions/26.1.2/26.1.2.jar", "cache/clients/26.1.2.jar")]
    [InlineData("versions/26.1.2/26.1.2.json", "cache/manifests/{hash}.json")]
    [InlineData("libraries/org/example/library.jar", "cache/libraries/org/example/library.jar")]
    [InlineData("assets/objects/ab/abcdef", "cache/assets/objects/ab/abcdef")]
    public async Task Verified_legacy_payloads_are_imported_without_a_download_provider(string next, string previous)
    {
        await using var test = await TestScope.CreateAsync();
        var payload = Encoding.UTF8.GetBytes("verified legacy cache");
        var hash = Convert.ToHexString(SHA1.HashData(payload)).ToLowerInvariant();
        var oldPath = test.Storage.Resolve(previous.Replace("{hash}", hash, StringComparison.Ordinal));
        Directory.CreateDirectory(Path.GetDirectoryName(oldPath)!);
        await File.WriteAllBytesAsync(oldPath, payload);
        var destination = Path.Combine(test.Storage.GameRoot, next);
        var files = new GameFileService(test.Storage);
        await files.EnsureFileAsync(new(new Uri("https://unused.invalid/file"), destination,
            hash, payload.Length), default);
        Assert.Equal(payload, await File.ReadAllBytesAsync(destination));
        Assert.Equal(payload, await File.ReadAllBytesAsync(oldPath));
        Assert.False(files.DownloadsEnabled);
    }

    [Fact]
    public async Task Corrupt_legacy_content_is_preserved_but_never_imported_as_a_valid_game_file()
    {
        await using var test = await TestScope.CreateAsync();
        var oldPath = test.Storage.Resolve("cache/clients/26.1.2.jar");
        Directory.CreateDirectory(Path.GetDirectoryName(oldPath)!);
        await File.WriteAllTextAsync(oldPath, "corrupt cache");
        var next = Path.Combine(test.Storage.GameRoot, "versions", "26.1.2", "26.1.2.jar");
        var error = await Assert.ThrowsAsync<GameLaunchException>(() => new GameFileService(test.Storage).EnsureFileAsync(
            new(new Uri("https://unused.invalid/file"), next, new string('a', 40), 13), default));
        Assert.Equal(LaunchFailureKind.DownloadsDisabled, error.Kind);
        Assert.False(File.Exists(next));
        Assert.Equal("corrupt cache", await File.ReadAllTextAsync(oldPath));
    }
}
