using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using Launcher.Contracts;
using Launcher.Plugins.Minecraft;

namespace Launcher.Launch.Tests;

public sealed class CacheOnlyTests
{
    [Fact]
    public async Task Without_a_download_plugin_valid_files_are_reused_and_missing_or_corrupt_files_fail_explicitly()
    {
        await using var test = await TestScope.CreateAsync();
        Directory.CreateDirectory(test.Root);
        var bytes = Encoding.UTF8.GetBytes("verified local game file");
        var path = Path.Combine(test.Root, "client.jar");
        var file = new DownloadFile(new Uri("https://unused.invalid/client.jar"), path, Convert.ToHexString(SHA1.HashData(bytes)), bytes.Length);
        var files = new GameFileService(test.Storage);
        Assert.False(files.DownloadsEnabled);
        var missing = await Assert.ThrowsAsync<GameLaunchException>(() => files.EnsureFileAsync(file, default));
        Assert.Equal(LaunchFailureKind.DownloadsDisabled, missing.Kind);
        await File.WriteAllBytesAsync(path, bytes);
        await files.EnsureFileAsync(file, default);
        await File.WriteAllBytesAsync(path, new byte[bytes.Length]);
        var corrupt = await Assert.ThrowsAsync<GameLaunchException>(() => files.EnsureFileAsync(file, default));
        Assert.Equal(LaunchFailureKind.DownloadsDisabled, corrupt.Kind);
    }

    [Fact]
    public async Task Cached_catalog_is_available_without_a_provider_and_an_empty_cache_does_not_trigger_network_access()
    {
        await using var test = await TestScope.CreateAsync();
        var files = new GameFileService(test.Storage);
        var catalog = new ContentService(files, test.Storage);
        var empty = await Assert.ThrowsAsync<GameLaunchException>(() => catalog.GetVersionsAsync(default));
        Assert.Equal(LaunchFailureKind.DownloadsDisabled, empty.Kind);
        using var manifest = JsonDocument.Parse("""{"versions":[{"id":"26.1.2","type":"release"}]}""");
        await test.Storage.WriteAsync(Path.Combine("cache", "manifests", "catalog.json"), new { schemaVersion = 1, manifest = manifest.RootElement }, default);
        Assert.Equal("26.1.2", Assert.Single(await catalog.GetVersionsAsync(default)).Id);
        await File.WriteAllTextAsync(test.Storage.Resolve(Path.Combine("cache", "manifests", "catalog.json")), "corrupt catalog");
        var corrupt = await Assert.ThrowsAsync<GameLaunchException>(() => new ContentService(files, test.Storage).GetVersionsAsync(default));
        Assert.Equal(LaunchFailureKind.DownloadsDisabled, corrupt.Kind);
    }

    [Fact]
    public async Task Removing_a_provider_cancels_and_awaits_downloads_before_releasing_its_registration()
    {
        await using var test = await TestScope.CreateAsync();
        var files = new GameFileService(test.Storage);
        var provider = new BlockingProvider();
        var registration = await files.RegisterAsync(provider, default);
        var work = files.EnsureFileAsync(new(new Uri("https://unused.invalid/file"), Path.Combine(test.Root, "file"), new string('a', 40), 10), default);
        await provider.Entered.Task.WaitAsync(TimeSpan.FromSeconds(5));
        Assert.NotEmpty(await test.Storage.ChangeDataDirectoryAsync(Path.Combine(Path.GetDirectoryName(test.Root)!, "next data"), default));
        await registration.DisposeAsync();
        Assert.True(provider.Stopped);
        Assert.False(files.DownloadsEnabled);
        var failure = await Assert.ThrowsAsync<GameLaunchException>(() => work);
        Assert.Equal(LaunchFailureKind.DownloadsDisabled, failure.Kind);
    }

    [Fact]
    public async Task An_invalid_download_provider_cannot_mark_unverified_files_as_ready()
    {
        await using var test = await TestScope.CreateAsync();
        var files = new GameFileService(test.Storage);
        await using var registration = await files.RegisterAsync(new EmptyProvider(), default);
        await Assert.ThrowsAsync<InvalidDataException>(() => files.EnsureFileAsync(new(new Uri("https://unused.invalid/file"), Path.Combine(test.Root, "file"), new string('a', 40), 10), default));
    }

    private sealed class BlockingProvider : IGameDownloadProvider
    {
        public TaskCompletionSource Entered { get; } = new(TaskCreationOptions.RunContinuationsAsynchronously);
        public bool Stopped { get; private set; }
        public Task<JsonElement> GetCatalogAsync(CancellationToken cancellationToken) => throw new NotSupportedException();
        public async Task DownloadAsync(DownloadFile file, CancellationToken cancellationToken)
        {
            Entered.SetResult();
            try { await Task.Delay(Timeout.Infinite, cancellationToken); }
            finally { Stopped = true; }
        }
    }
    private sealed class EmptyProvider : IGameDownloadProvider
    {
        public Task<JsonElement> GetCatalogAsync(CancellationToken cancellationToken) => throw new NotSupportedException();
        public Task DownloadAsync(DownloadFile file, CancellationToken cancellationToken) => Task.CompletedTask;
    }
}
