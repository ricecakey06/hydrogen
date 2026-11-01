using Launcher.Contracts;
using Launcher.Plugins.Authentication;
using Launcher.Plugins.Infrastructure;
using Launcher.Plugins.Java;
using Launcher.Plugins.Minecraft;
using Launcher.Plugins.Storage;
using Launcher.Runtime;

namespace Launcher.Plugins;

public sealed record FunctionalPluginOptions(string? MicrosoftClientId = null, string? DataDirectory = null);

/// <summary>The production service graph. Activation only reads local state; network work is lazy or context-owned.</summary>
public static class FunctionalPlugins
{
    public static PluginDefinition Platform(IPlatformEnvironment environment) => new("platform.environment", (context, _) =>
    { context.Provide(environment); return ValueTask.CompletedTask; }, provides: [typeof(IPlatformEnvironment)]);

    public static PluginDefinition SessionCredentials { get; } = new("credentials", (context, _) =>
    { context.Provide<ICredentialStore>(new SessionCredentialStore(context.GetRequiredService<ILauncherStorage>())); return ValueTask.CompletedTask; },
        requires: [typeof(ILauncherStorage)], provides: [typeof(ICredentialStore)]);

    public static PluginDefinition OfflineAccount { get; } = new("account.offline", async (context, token) =>
    {
        var service = new OfflineAccountService(context, context.GetRequiredService<ILauncherStorage>());
        context.Subscribe<DataDirectoryChanged>((_, stopping) => new ValueTask(service.LoadAsync(stopping)));
        await service.LoadAsync(token).ConfigureAwait(false);
        context.Own(await context.GetRequiredService<IAccountProviderRegistry>().RegisterOfflineAsync(service, token).ConfigureAwait(false));
        context.Provide<IOfflineAccountService>(service);
    }, requires: [typeof(ILauncherStorage), typeof(IAccountProviderRegistry)], provides: [typeof(IOfflineAccountService)]);

    public static IReadOnlyList<PluginDefinition> Create(FunctionalPluginOptions options) =>
    [
        new("storage", async (context, token) =>
        {
            var platform = context.GetRequiredService<IPlatformEnvironment>();
            var service = new LauncherStorage(context, options.DataDirectory ?? platform.DefaultDataDirectory,
                options.DataDirectory is null ? platform.DataLocationFile : null, options.DataDirectory is null ? platform.DefaultGameDirectory : null);
            await service.InitializeAsync(token).ConfigureAwait(false);
            context.Provide<ILauncherStorage>(service);
            context.Provide<ILauncherSettingsService>(service);
        }, requires: [typeof(IPlatformEnvironment)], provides: [typeof(ILauncherStorage), typeof(ILauncherSettingsService)]),
        new("network", (context, _) => { context.Provide<INetworkService>(new NetworkService()); return ValueTask.CompletedTask; }, provides: [typeof(INetworkService)]),
        new("process", (context, _) => { context.Provide<IProcessService>(new ProcessService()); return ValueTask.CompletedTask; }, provides: [typeof(IProcessService)]),
        new("diagnostics", (context, _) =>
        { context.Provide<IDiagnosticsService>(new DiagnosticsService(context.GetRequiredService<ILauncherStorage>())); return ValueTask.CompletedTask; },
            requires: [typeof(ILauncherStorage)], provides: [typeof(IDiagnosticsService)]),
        new("minecraft.content", (context, _) =>
        {
            var service = new ContentService(context.GetRequiredService<IGameFileService>(), context.GetRequiredService<ILauncherStorage>());
            context.Provide<IVersionCatalog>(service);
            context.Provide<IGameContentService>(service);
            return ValueTask.CompletedTask;
        }, requires: [typeof(IGameFileService), typeof(ILauncherStorage)], provides: [typeof(IVersionCatalog), typeof(IGameContentService)]),
        new("minecraft.files", (context, _) =>
        {
            var service = new GameFileService(context.GetRequiredService<ILauncherStorage>());
            context.Provide<IGameFileService>(service);
            context.Provide<IGameDownloadRegistry>(service);
            return ValueTask.CompletedTask;
        }, requires: [typeof(ILauncherStorage)], provides: [typeof(IGameFileService), typeof(IGameDownloadRegistry)]),
        new("minecraft.instances", async (context, token) =>
        {
            var service = new InstanceService(context, context.GetRequiredService<ILauncherStorage>(), context.GetRequiredService<IVersionCatalog>(),
                context.GetRequiredService<IPlatformEnvironment>());
            context.Subscribe<DataDirectoryChanged>((_, stopping) => new ValueTask(service.LoadAsync(stopping)));
            await service.LoadAsync(token).ConfigureAwait(false);
            context.Provide<IInstanceService>(service);
            context.Provide<IInstanceRepository>(service);
        }, requires: [typeof(ILauncherStorage), typeof(IVersionCatalog), typeof(IPlatformEnvironment)], provides: [typeof(IInstanceService), typeof(IInstanceRepository)]),
        new("java.runtime", (context, _) =>
        {
            context.Provide<IJavaRuntimeService>(new JavaRuntimeService(context.GetRequiredService<IPlatformEnvironment>(), context.GetRequiredService<IProcessService>(),
                context.GetRequiredService<ILauncherSettingsService>(), context.GetRequiredService<IInstanceService>()));
            return ValueTask.CompletedTask;
        }, requires: [typeof(IPlatformEnvironment), typeof(IProcessService), typeof(ILauncherSettingsService), typeof(IInstanceService)], provides: [typeof(IJavaRuntimeService)]),
        new("minecraft.install", (context, _) =>
        {
            var service = new GameInstallService(context, context.GetRequiredService<IGameContentService>(),
                context.GetRequiredService<IInstanceRepository>(), context.GetRequiredService<IPlatformEnvironment>());
            context.Subscribe<DataDirectoryChanged>((_, _) => { service.Clear(); return ValueTask.CompletedTask; });
            context.Subscribe<InstanceDeleted>((deleted, _) => { service.Forget(deleted.InstanceId); return ValueTask.CompletedTask; });
            context.Provide<IGameInstallService>(service);
            return ValueTask.CompletedTask;
        }, requires: [typeof(IGameContentService), typeof(IInstanceRepository), typeof(IPlatformEnvironment)], provides: [typeof(IGameInstallService)]),
        new("account.microsoft", (context, activationToken) =>
        {
            var network = context.GetRequiredService<INetworkService>();
            var service = new MicrosoftAccountService(context, network, context.GetRequiredService<ICredentialStore>(), context.GetRequiredService<ILauncherStorage>(),
                new MicrosoftOAuthClient(network, context.GetRequiredService<IPlatformEnvironment>(), options.MicrosoftClientId), options.MicrosoftClientId);
            context.Subscribe<DataDirectoryChanged>((changed, stopping) => new ValueTask(service.ResetForDataDirectoryAsync(stopping)));
            context.Provide<IMicrosoftAccountService>(service);
            _ = context.Run(service.RestoreAsync);
            return ValueTask.CompletedTask;
        }, requires: [typeof(INetworkService), typeof(ICredentialStore), typeof(ILauncherStorage), typeof(IPlatformEnvironment)], provides: [typeof(IMicrosoftAccountService)]),
        new("account.manager", async (context, token) =>
        {
            var service = new AccountRouter(context, context.GetRequiredService<IMicrosoftAccountService>(), context.GetRequiredService<ILauncherStorage>());
            context.Subscribe<AccountProviderStateChanged>((change, stopping) => { service.OnProviderStateChanged(change); return ValueTask.CompletedTask; });
            context.Subscribe<DataDirectoryChanged>((_, stopping) => new ValueTask(service.LoadPreferencesAsync(stopping)));
            await service.LoadPreferencesAsync(token).ConfigureAwait(false);
            context.Provide<IAccountService>(service);
            context.Provide<IGameAccountService>(service);
            context.Provide<IAccountModeService>(service);
            context.Provide<IAccountProviderRegistry>(service);
        }, requires: [typeof(IMicrosoftAccountService), typeof(ILauncherStorage)],
            provides: [typeof(IAccountService), typeof(IGameAccountService), typeof(IAccountModeService), typeof(IAccountProviderRegistry)]),
        new("minecraft.launch", (context, _) =>
        {
            context.Provide<IGameLaunchService>(new GameLaunchService(context, context.GetRequiredService<IGameAccountService>(), context.GetRequiredService<IGameContentService>(),
                context.GetRequiredService<IJavaRuntimeService>(), context.GetRequiredService<ILauncherSettingsService>(), context.GetRequiredService<IInstanceRepository>(),
                context.GetRequiredService<IPlatformEnvironment>(), context.GetRequiredService<IProcessService>(), context.GetRequiredService<IDiagnosticsService>()));
            return ValueTask.CompletedTask;
        }, requires: [typeof(IGameAccountService), typeof(IGameContentService), typeof(IJavaRuntimeService), typeof(ILauncherSettingsService), typeof(IInstanceRepository),
            typeof(IPlatformEnvironment), typeof(IProcessService), typeof(IDiagnosticsService)], provides: [typeof(IGameLaunchService)]),
    ];
}
