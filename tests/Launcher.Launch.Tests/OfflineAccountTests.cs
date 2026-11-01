using Launcher.Contracts;
using Launcher.Plugins;
using Launcher.Plugins.Authentication;
using Launcher.Plugins.Infrastructure;
using Launcher.Plugins.Minecraft;
using Launcher.Runtime;

namespace Launcher.Launch.Tests;

public sealed class OfflineAccountTests
{
    [Fact]
    public void Offline_uuid_matches_the_java_name_uuid_format_and_preserves_name_case()
    {
        Assert.Equal("b50ad385829d3141a2167e7d7539ba7f", OfflineAccountService.CreateUuid("Notch"));
        Assert.Equal(OfflineAccountService.CreateUuid("Player"), OfflineAccountService.CreateUuid("Player"));
        Assert.NotEqual(OfflineAccountService.CreateUuid("Player"), OfflineAccountService.CreateUuid("player"));
    }

    [Fact]
    public async Task Offline_identity_needs_no_microsoft_sign_in_and_keeps_online_credentials_when_switching_back()
    {
        await using var test = await TestScope.CreateAsync();
        var microsoft = new MicrosoftStub(signedIn: true);
        var router = await CreateRouterAsync(test, microsoft);
        var offline = new OfflineAccountService(test.Context, test.Storage);
        await offline.LoadAsync(default);
        await using var registration = await router.RegisterOfflineAsync(offline, default);
        Assert.Equal(AccountLoginMode.Microsoft, router.Snapshot.Mode);
        await router.SelectAsync(AccountLoginMode.Offline, default);
        var session = await router.GetSessionAsync(default);
        Assert.Equal(AccountLoginMode.Offline, session.Mode);
        Assert.Equal("0", session.AccessToken);
        Assert.Equal("legacy", session.UserType);
        Assert.Equal("", session.Xuid);
        Assert.Equal("", session.ClientId);
        Assert.Equal(0, microsoft.GameCalls);
        Assert.Equal(0, microsoft.SignInCalls);
        await router.SignOutAsync(default);
        Assert.Equal(AccountStatus.SignedOut, router.State.Status);
        Assert.Equal(0, microsoft.SignOutCalls);
        await router.SelectAsync(AccountLoginMode.Microsoft, default);
        Assert.Equal("OnlinePlayer", router.State.Profile!.PlayerName);
        Assert.Equal("existing-online-token", (await router.GetSessionAsync(default)).AccessToken);
        Assert.Equal(0, microsoft.SignInCalls);
    }

    [Fact]
    public async Task Offline_name_and_selected_mode_survive_restart_but_disabled_plugins_fall_back_to_microsoft()
    {
        await using var test = await TestScope.CreateAsync();
        var router = await CreateRouterAsync(test, new MicrosoftStub());
        var offline = new OfflineAccountService(test.Context, test.Storage);
        await offline.LoadAsync(default);
        await using var registration = await router.RegisterOfflineAsync(offline, default);
        await router.SetOfflinePlayerNameAsync("OfflineTester", default);
        await router.SelectAsync(AccountLoginMode.Offline, default);
        var identity = await router.GetSessionAsync(default);
        await using var reopened = await TestScope.CreateAsync(test.Root);
        var nextRouter = await CreateRouterAsync(reopened, new MicrosoftStub());
        Assert.False(nextRouter.Snapshot.OfflineAvailable);
        Assert.Equal(AccountLoginMode.Microsoft, nextRouter.Snapshot.Mode);
        var nextOffline = new OfflineAccountService(reopened.Context, reopened.Storage);
        await nextOffline.LoadAsync(default);
        await using var nextRegistration = await nextRouter.RegisterOfflineAsync(nextOffline, default);
        Assert.Equal(AccountLoginMode.Offline, nextRouter.Snapshot.Mode);
        var restored = await nextRouter.GetSessionAsync(default);
        Assert.Equal("OfflineTester", restored.PlayerName);
        Assert.Equal(identity.Uuid, restored.Uuid);
        await nextRegistration.DisposeAsync();
        Assert.False(nextRouter.Snapshot.OfflineAvailable);
        Assert.Equal(AccountLoginMode.Microsoft, nextRouter.Snapshot.Mode);
        await Assert.ThrowsAsync<LauncherOperationException>(() => nextRouter.SelectAsync(AccountLoginMode.Offline, default));
    }

    [Theory]
    [InlineData("ab")]
    [InlineData("two words")]
    [InlineData("中文名称")]
    [InlineData("../Player")]
    [InlineData("abcdefghijklmnopq")]
    public async Task Invalid_offline_names_do_not_replace_saved_identity(string invalid)
    {
        await using var test = await TestScope.CreateAsync();
        var offline = new OfflineAccountService(test.Context, test.Storage);
        await offline.LoadAsync(default);
        await offline.SetPlayerNameAsync("Valid_Name", default);
        var path = Path.Combine(test.Root, "accounts", "offline.json");
        var before = await File.ReadAllTextAsync(path);
        await Assert.ThrowsAsync<LauncherOperationException>(() => offline.SetPlayerNameAsync(invalid, default));
        Assert.Equal(before, await File.ReadAllTextAsync(path));
        Assert.Equal("Valid_Name", (await offline.GetSessionAsync(default)).PlayerName);
    }

    [Fact]
    public async Task Inactive_microsoft_notifications_do_not_replace_the_selected_offline_account()
    {
        await using var test = await TestScope.CreateAsync();
        var router = await CreateRouterAsync(test, new MicrosoftStub());
        var offline = new OfflineAccountService(test.Context, test.Storage);
        await offline.LoadAsync(default);
        await using var registration = await router.RegisterOfflineAsync(offline, default);
        await router.SelectAsync(AccountLoginMode.Offline, default);
        var changes = 0;
        test.Context.Subscribe<AccountStateChanged>((_, _) => { Interlocked.Increment(ref changes); return ValueTask.CompletedTask; });
        await test.Context.PublishAsync(new AccountProviderStateChanged(AccountLoginMode.Microsoft, new(AccountStatus.Expired, null)));
        Assert.Equal(0, changes);
        Assert.Equal("Player", router.State.Profile!.PlayerName);
    }

    [Fact]
    public async Task Future_offline_configuration_is_preserved_and_reported()
    {
        await using var test = await TestScope.CreateAsync();
        var path = test.Storage.Resolve(Path.Combine("accounts", "offline.json"));
        Directory.CreateDirectory(Path.GetDirectoryName(path)!);
        const string future = "{\"schemaVersion\":99,\"playerName\":\"DoNotOverwrite\"}";
        await File.WriteAllTextAsync(path, future);
        var offline = new OfflineAccountService(test.Context, test.Storage);
        await offline.LoadAsync(default);
        Assert.Equal(AccountStatus.SignedOut, offline.State.Status);
        Assert.NotNull(offline.State.Detail);
        await Assert.ThrowsAsync<LauncherOperationException>(() => offline.SetPlayerNameAsync("AnotherName", default));
        Assert.Equal(future, await File.ReadAllTextAsync(path));
    }

    [Fact]
    public async Task Offline_launch_uses_local_identity_and_does_not_redact_the_zero_token_from_ordinary_log_numbers()
    {
        await using var test = await TestScope.CreateAsync();
        var microsoft = new MicrosoftStub();
        var router = await CreateRouterAsync(test, microsoft);
        var offline = new OfflineAccountService(test.Context, test.Storage);
        await offline.LoadAsync(default);
        await using var registration = await router.RegisterOfflineAsync(offline, default);
        await router.SelectAsync(AccountLoginMode.Offline, default);
        await router.SetOfflinePlayerNameAsync("Custom_Name", default);
        var instance = await test.Instances.CreateAsync("26.1.2", default);
        var processes = new StubProcesses();
        var launch = new GameLaunchService(test.Context, router, new StubContent(), new StubJava(), test.Storage, test.Instances,
            new StubPlatform(), processes, new DiagnosticsService(test.Storage));
        var exited = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        test.Context.Subscribe<LaunchStateChanged>((change, _) =>
        { if (change.Snapshot.Phase == LaunchPhase.Exited) exited.TrySetResult(); return ValueTask.CompletedTask; });
        await launch.LaunchAsync(instance.Id, default);
        Assert.Equal(LaunchPhase.Running, launch.GetSnapshot(instance.Id).Phase);
        var arguments = processes.LastRequest!.Arguments.ToList();
        Assert.Equal("Custom_Name", arguments[arguments.IndexOf("--username") + 1]);
        Assert.Equal("0", arguments[arguments.IndexOf("--accessToken") + 1]);
        Assert.Equal("legacy", arguments[arguments.IndexOf("--userType") + 1]);
        Assert.Equal(OfflineAccountService.CreateUuid("Custom_Name"), arguments[arguments.IndexOf("--uuid") + 1]);
        Assert.Equal(0, microsoft.GameCalls);
        processes.Process!.Exit.SetResult(0);
        await exited.Task.WaitAsync(TimeSpan.FromSeconds(5));
        var log = await File.ReadAllTextAsync(Assert.Single(Directory.GetFiles(Path.Combine(test.Root, "logs"))));
        Assert.Contains("Using offline mode.", log);
        Assert.Contains("25.0.1", log);
        Assert.Contains("exited with code 0", log);
        Assert.Empty(test.Host.GetErrors());
    }

    [Fact]
    public async Task Offline_plugin_is_optional_disabled_by_default_and_enabled_on_restart()
    {
        await using var test = await TestScope.CreateAsync();
        IAccountModeService? modes = null;
        var consumer = new PluginDefinition("test.consumer", (context, _) =>
        { modes = context.GetRequiredService<IAccountModeService>(); return ValueTask.CompletedTask; }, requires: [typeof(IAccountModeService), typeof(IGameLaunchService)]);
        var definitions = new[] { FunctionalPlugins.Platform(new StubPlatform()), FunctionalPlugins.SessionCredentials, consumer }
            .Concat(FunctionalPlugins.Create(new(DataDirectory: test.Root)));
        BundledPlugin[] bundled = [new("account.offline", "Offline", "Test plugin", "Launcher.Features.dll",
            typeof(OfflineModule).FullName!, File.ReadAllBytes(typeof(OfflineModule).Assembly.Location), EnabledByDefault: false)];
        var directory = Path.Combine(test.Root, "Plugins");
        using (var manager = new PluginManager(directory, bundled, []))
        {
            var startup = await manager.PrepareStartupAsync();
            Assert.DoesNotContain(startup, definition => definition.Id == "account.offline");
            var info = (await manager.GetInventoryAsync(default)).Plugins.Single(plugin => plugin.Id == "account.offline");
            Assert.False(info.IsRequired);
            Assert.False(info.IsEnabled);
            await manager.SetEnabledAsync("account.offline", true, default);
        }
        using var restarted = new PluginManager(directory, bundled, []);
        await using var host = new PluginHost(definitions.Concat(await restarted.PrepareStartupAsync()));
        await host.StartAsync();
        Assert.All(await host.GetSnapshotAsync(), plugin => Assert.Equal(PluginState.Active, plugin.State));
        Assert.NotNull(modes);
        Assert.True(modes.Snapshot.OfflineAvailable);
        await modes.SelectAsync(AccountLoginMode.Offline, default);
        Assert.Equal(AccountLoginMode.Offline, modes.Snapshot.Mode);
        Assert.Empty(host.GetErrors());
    }

    private static async Task<AccountRouter> CreateRouterAsync(TestScope test, IMicrosoftAccountService microsoft)
    {
        var router = new AccountRouter(test.Context, microsoft, test.Storage);
        await router.LoadPreferencesAsync(default);
        test.Context.Subscribe<AccountProviderStateChanged>((change, _) => { router.OnProviderStateChanged(change); return ValueTask.CompletedTask; });
        return router;
    }
    private sealed class MicrosoftStub(bool signedIn = false) : IMicrosoftAccountService
    {
        public AccountState State { get; private set; } = signedIn ? new(AccountStatus.SignedIn, new("OnlinePlayer", null)) : new(AccountStatus.SignedOut, null, AccountFailureKind.Configuration);
        public int GameCalls { get; private set; }
        public int SignInCalls { get; private set; }
        public int SignOutCalls { get; private set; }
        public Task SignInAsync(CancellationToken cancellationToken) { SignInCalls++; throw new InvalidOperationException("No real login expected."); }
        public Task SignOutAsync(CancellationToken cancellationToken) { SignOutCalls++; State = new(AccountStatus.SignedOut, null); return Task.CompletedTask; }
        public Task<GameAccountSession> GetSessionAsync(CancellationToken cancellationToken)
        {
            GameCalls++;
            return State.Status == AccountStatus.SignedIn ? Task.FromResult(new GameAccountSession("OnlinePlayer", "123456781234123412341234567890ab",
                "existing-online-token", "xuid", "client", DateTimeOffset.UtcNow.AddHours(1))) : throw new AccountAuthenticationException(AccountFailureKind.Configuration);
        }
    }
}
