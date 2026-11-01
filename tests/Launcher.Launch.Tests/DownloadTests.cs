using System.Net;
using System.Net.Http.Headers;
using System.Security.Cryptography;
using System.Text;
using Launcher.Contracts;
using Launcher.Downloads;
using Launcher.Plugins;
using Launcher.Plugins.Infrastructure;
using Launcher.Plugins.Minecraft;
using Launcher.Runtime;

namespace Launcher.Launch.Tests;

public sealed class DownloadTests
{
    [Fact]
    public async Task Packaged_plugin_is_enabled_by_default_and_can_be_disabled_without_losing_local_cache_services()
    {
        await using var test = await TestScope.CreateAsync();
        IGameFileService? files = null;
        var definitions = FunctionalPlugins.Create(new(DataDirectory: test.Root)).Concat([
            FunctionalPlugins.Platform(new StubPlatform()), FunctionalPlugins.SessionCredentials,
            new PluginDefinition("test.consumer", (context, _) =>
            { files = context.GetRequiredService<IGameFileService>(); return ValueTask.CompletedTask; }, requires: [typeof(IGameFileService)])]);
        BundledPlugin[] bundled = [new("minecraft.download", "游戏下载", "Downloads", "Launcher.Downloads.dll",
            typeof(DownloadModule).FullName!, File.ReadAllBytes(typeof(DownloadModule).Assembly.Location))];
        var directory = Path.Combine(test.Root, "Plugins");
        using (var manager = new PluginManager(directory, bundled, []))
        {
            await using var host = new PluginHost(definitions.Concat(await manager.PrepareStartupAsync()));
            await host.StartAsync();
            manager.RecordRuntime(await host.GetSnapshotAsync());
            var item = (await manager.GetInventoryAsync(default)).Plugins.Single(item => item.Id == "minecraft.download");
            Assert.Equal(PluginOrigin.BuiltIn, item.Origin);
            Assert.True(item.IsEnabled);
            Assert.False(item.IsRequired);
            Assert.True(files!.DownloadsEnabled);
            await manager.SetEnabledAsync(item.Id, false, default);
            Assert.True(files.DownloadsEnabled);
            Assert.Empty(host.GetErrors());
        }
        using var restarted = new PluginManager(directory, bundled, []);
        var startup = await restarted.PrepareStartupAsync();
        Assert.DoesNotContain(startup, definition => definition.Id == "minecraft.download");
        await using var next = new PluginHost(definitions.Concat(startup));
        await next.StartAsync();
        Assert.False(files!.DownloadsEnabled);
        Assert.All(await next.GetSnapshotAsync(), item => Assert.Equal(PluginState.Active, item.State));
    }

    [Theory]
    [InlineData(GameDownloadSource.Official)]
    [InlineData(GameDownloadSource.Bmclapi)]
    public async Task Downloads_retry_bad_hashes_reuse_cache_and_repair_corrupt_files(GameDownloadSource source)
    {
        await using var test = await TestScope.CreateAsync();
        Assert.Empty(await test.Storage.SaveAsync(test.Storage.Settings with { DownloadSource = source }, default));
        var bytes = Encoding.UTF8.GetBytes("verified game content 中文");
        var calls = 0;
        using var network = new NetworkService(new ScriptedHandler((request, _) =>
        {
            Assert.True(request.Options.TryGetValue(NetworkRequestOptions.GameDownload, out var game) && game);
            Assert.Equal(source == GameDownloadSource.Bmclapi ? "bmclapi2.bangbang93.com" : "libraries.minecraft.net", request.RequestUri!.Host);
            return Task.FromResult(ScriptedHandler.Bytes(Interlocked.Increment(ref calls) == 1 ? new byte[bytes.Length] : bytes));
        }));
        var files = new GameFileService(test.Storage);
        await using var registration = await files.RegisterAsync(new GameDownloadProvider(network, test.Storage), default);
        var file = Spec(test.Root, bytes);
        await files.EnsureFileAsync(file, default);
        Assert.Equal(2, calls);
        await files.EnsureFileAsync(file, default);
        Assert.Equal(2, calls);
        await File.WriteAllBytesAsync(file.Destination, new byte[bytes.Length]);
        await files.EnsureFileAsync(file, default);
        Assert.Equal(3, calls);
        Assert.Equal(bytes, await File.ReadAllBytesAsync(file.Destination));
        Assert.Empty(Directory.EnumerateFiles(test.Root, "*.part", SearchOption.AllDirectories));
    }

    [Theory]
    [InlineData(true)]
    [InlineData(false)]
    public async Task Large_files_use_ranges_or_fall_back_when_the_server_ignores_ranges(bool supportsRanges)
    {
        await using var test = await TestScope.CreateAsync();
        var bytes = new byte[GameDownloadProvider.ParallelDownloadThreshold];
        new Random(42).NextBytes(bytes);
        var calls = 0;
        using var network = new NetworkService(new ScriptedHandler((request, _) =>
        {
            Interlocked.Increment(ref calls);
            var range = request.Headers.Range?.Ranges.Single();
            if (!supportsRanges || range is null) return Task.FromResult(ScriptedHandler.Bytes(bytes));
            var response = ScriptedHandler.Bytes(bytes.AsSpan((int)range.From!.Value, (int)(range.To!.Value - range.From.Value + 1)).ToArray());
            response.StatusCode = HttpStatusCode.PartialContent;
            response.Content.Headers.ContentRange = new ContentRangeHeaderValue(range.From.Value, range.To.Value, bytes.Length);
            return Task.FromResult(response);
        }));
        var file = Spec(test.Root, bytes);
        await new GameDownloadProvider(network).DownloadAsync(file, default);
        Assert.Equal(supportsRanges ? 17 : 2, calls);
        Assert.Equal(bytes, await File.ReadAllBytesAsync(file.Destination));
    }

    [Fact]
    public async Task Invalid_range_responses_never_replace_existing_files()
    {
        await using var test = await TestScope.CreateAsync();
        var bytes = new byte[GameDownloadProvider.ParallelDownloadThreshold];
        using var network = new NetworkService(new ScriptedHandler((_, _) =>
        {
            var response = ScriptedHandler.Bytes([0]);
            response.StatusCode = HttpStatusCode.PartialContent;
            response.Content.Headers.ContentRange = new ContentRangeHeaderValue(1, 1, bytes.Length);
            return Task.FromResult(response);
        }));
        var file = Spec(test.Root, bytes);
        Directory.CreateDirectory(test.Root);
        await File.WriteAllTextAsync(file.Destination, "keep");
        await Assert.ThrowsAsync<InvalidDataException>(() => new GameDownloadProvider(network).DownloadAsync(file, default));
        Assert.Equal("keep", await File.ReadAllTextAsync(file.Destination));
        Assert.Empty(Directory.EnumerateFiles(test.Root, "*.part", SearchOption.AllDirectories));
    }

    [Fact]
    public async Task Cancellation_preserves_existing_files_and_cleans_temporary_files()
    {
        await using var test = await TestScope.CreateAsync();
        Directory.CreateDirectory(test.Root);
        var entered = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        using var network = new NetworkService(new ScriptedHandler(async (_, token) =>
        { entered.SetResult(); await Task.Delay(Timeout.Infinite, token); return ScriptedHandler.Bytes([]); }));
        var file = Spec(test.Root, [1, 2, 3, 4]);
        await File.WriteAllTextAsync(file.Destination, "keep");
        using var stopping = new CancellationTokenSource();
        var download = new GameDownloadProvider(network).DownloadAsync(file, stopping.Token);
        await entered.Task.WaitAsync(TimeSpan.FromSeconds(5));
        await stopping.CancelAsync();
        await Assert.ThrowsAnyAsync<OperationCanceledException>(() => download);
        Assert.Equal("keep", await File.ReadAllTextAsync(file.Destination));
        Assert.Empty(Directory.EnumerateFiles(test.Root, "*.part", SearchOption.AllDirectories));
    }

    [Fact]
    public async Task Mirror_rate_limits_fall_back_to_official_and_respect_retry_after()
    {
        await using var test = await TestScope.CreateAsync();
        Assert.Empty(await test.Storage.SaveAsync(test.Storage.Settings with { DownloadSource = GameDownloadSource.Bmclapi }, default));
        var hosts = new List<string>();
        byte[] bytes = [1, 2, 3, 4];
        using var network = new NetworkService(new ScriptedHandler((request, _) =>
        {
            hosts.Add(request.RequestUri!.Host);
            if (request.RequestUri.Host != "bmclapi2.bangbang93.com") return Task.FromResult(ScriptedHandler.Bytes(bytes));
            var response = new HttpResponseMessage(HttpStatusCode.TooManyRequests);
            response.Headers.RetryAfter = new RetryConditionHeaderValue(TimeSpan.FromMinutes(1));
            return Task.FromResult(response);
        }));
        var provider = new GameDownloadProvider(network, test.Storage);
        var file = Spec(test.Root, bytes);
        await provider.DownloadAsync(file, default);
        await provider.DownloadAsync(file with { Destination = Path.Combine(test.Root, "second.jar") }, default);
        Assert.Equal(new[] { "bmclapi2.bangbang93.com", "libraries.minecraft.net", "libraries.minecraft.net" }, hosts);
    }

    [Fact]
    public async Task Redirects_cannot_downgrade_game_requests_to_http()
    {
        await using var test = await TestScope.CreateAsync();
        using var network = new NetworkService(new ScriptedHandler((request, _) =>
        {
            Assert.Equal("https", request.RequestUri!.Scheme);
            var response = new HttpResponseMessage(HttpStatusCode.Redirect);
            response.Headers.Location = new Uri("http://unsafe.invalid/game.jar");
            return Task.FromResult(response);
        }));
        var file = Spec(test.Root, [1]);
        await Assert.ThrowsAsync<InvalidDataException>(() => new GameDownloadProvider(network).DownloadAsync(file, default));
        Assert.False(File.Exists(file.Destination));
    }

    [Fact]
    public async Task Catalog_is_downloaded_and_cached_for_later_offline_use()
    {
        await using var test = await TestScope.CreateAsync();
        using var network = new NetworkService(new ScriptedHandler((request, _) =>
        {
            Assert.Equal(GameDownloadProvider.ManifestUri, request.RequestUri);
            return Task.FromResult(ScriptedHandler.Json("""{"versions":[{"id":"1.21.1","type":"release"}]}"""));
        }));
        var files = new GameFileService(test.Storage);
        var content = new ContentService(files, test.Storage);
        await using (var registration = await files.RegisterAsync(new GameDownloadProvider(network), default))
            Assert.Equal("1.21.1", Assert.Single(await content.GetVersionsAsync(default)).Id);
        Assert.Equal("1.21.1", Assert.Single(await new ContentService(files, test.Storage).GetVersionsAsync(default)).Id);
    }

    [Fact]
    public async Task Source_preferences_survive_restart_and_invalid_values_do_not_overwrite_settings()
    {
        await using var test = await TestScope.CreateAsync();
        Assert.Equal(GameDownloadSource.Official, test.Storage.Settings.DownloadSource);
        Assert.Empty(await test.Storage.SaveAsync(test.Storage.Settings with { DownloadSource = GameDownloadSource.Bmclapi }, default));
        await using var reopened = await TestScope.CreateAsync(test.Root);
        Assert.Equal(GameDownloadSource.Bmclapi, reopened.Storage.Settings.DownloadSource);
        var path = Path.Combine(test.Root, "settings.json");
        var before = await File.ReadAllTextAsync(path);
        Assert.Contains(await reopened.Storage.SaveAsync(reopened.Storage.Settings with { DownloadSource = (GameDownloadSource)99 }, default), issue => issue.Field == SettingField.DownloadSource);
        Assert.Equal(before, await File.ReadAllTextAsync(path));
    }

    private static DownloadFile Spec(string root, byte[] bytes) => new(new Uri("https://libraries.minecraft.net/test.jar"),
        Path.Combine(root, "game.jar"), Convert.ToHexString(SHA1.HashData(bytes)), bytes.Length);
}
