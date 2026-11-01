using Avalonia.Threading;
using Launcher.Contracts;
using Launcher.Runtime;
using Launcher.UI.Shell;
using Launcher.UI.ViewModels;
using Launcher.UI.Views;

namespace Launcher.UI;

/// <summary>The built-in interface plugins. They expect <see cref="DesktopUi"/> to be running its loop on the main thread.</summary>
public static class UiPlugins
{
    public const string ShellId = "ui.shell";
    public const string LauncherUiId = "ui.launcher";

    /// <summary>
    /// The main window, its content slot and the shared visual style. <see cref="IMainWindow"/> lets a platform adapter
    /// enable the backdrop blur; without one the window stays plainly transparent around its rounded outline.
    /// </summary>
    public static PluginDefinition Shell
    {
        get => new PluginDefinition(ShellId, async (context, cancellationToken) =>
        {
            var shell = await Dispatcher.UIThread.InvokeAsync(() => new ShellHost(DesktopUi.Current), DispatcherPriority.Normal, cancellationToken);
            try
            {
                context.Provide<IShell>(shell);
                context.Provide<IMainWindow>(shell);
            }
            catch
            {
                await shell.DisposeAsync();
                throw;
            }
        }, provides: [typeof(IShell), typeof(IMainWindow)]);
    }

    /// <summary>The home page, account interaction, version selection and settings.</summary>
    public static PluginDefinition LauncherUi { get; } = new(LauncherUiId, ActivateLauncherUiAsync, requires:
    [
        typeof(IShell), typeof(IAccountService), typeof(IAccountModeService), typeof(IVersionCatalog), typeof(IInstanceService),
        typeof(IJavaRuntimeService), typeof(ILauncherSettingsService), typeof(IGameFileService), typeof(IGameInstallService), typeof(IGameLaunchService), typeof(IPluginManager),
    ]);

    private static async ValueTask ActivateLauncherUiAsync(Context context, CancellationToken cancellationToken)
    {
        var shell = context.GetRequiredService<IShell>();
        var services = new LauncherServices(
            context.GetRequiredService<IAccountService>(),
            context.GetRequiredService<IAccountModeService>(),
            context.GetRequiredService<IVersionCatalog>(),
            context.GetRequiredService<IInstanceService>(),
            context.GetRequiredService<IJavaRuntimeService>(),
            context.GetRequiredService<ILauncherSettingsService>(),
            context.GetRequiredService<IGameFileService>(),
            context.GetRequiredService<IGameInstallService>(),
            context.GetRequiredService<IGameLaunchService>(),
            context.GetRequiredService<IPluginManager>());
        var operations = new UiOperations(context);

        // These are only touched on the UI thread.
        LauncherViewModel? root = null;
        IDisposable? content = null;
        IDisposable? closing = null;
        context.OnDisposeAsync(async () => await Dispatcher.UIThread.InvokeAsync(() =>
        {
            closing?.Dispose();
            content?.Dispose();
            root?.Dispose();
        }));

        // Subscribe before the first read. Notifications only trigger a re-read, so their delivery order does not matter.
        Refresh<AccountStateChanged>(context, () => root?.RefreshAccount());
        Refresh<AccountModesChanged>(context, () => root?.RefreshAccount());
        Refresh<InstancesChanged>(context, () => root?.RefreshInstances());
        Refresh<LaunchStateChanged>(context, () => root?.RefreshLaunch());
        Refresh<InstallStateChanged>(context, () => root?.RefreshDownloads());

        await Dispatcher.UIThread.InvokeAsync(() =>
        {
            root = new LauncherViewModel(services, shell, operations);
            content = shell.SetContent(new LauncherView { DataContext = root });
            closing = shell.InterceptClose(root.HandleCloseRequest);
        }, DispatcherPriority.Normal, cancellationToken);
    }

    private static void Refresh<T>(Context context, Action refresh)
        => context.Subscribe<T>(async (_, token) => await Dispatcher.UIThread.InvokeAsync(refresh, DispatcherPriority.Normal, token));
}
