using Avalonia.Threading;
using Launcher.Contracts;
using Launcher.Runtime;

namespace Launcher.UI;

internal sealed record LauncherServices(
    IAccountService Account,
    IAccountModeService AccountModes,
    IVersionCatalog Versions,
    IInstanceService Instances,
    IJavaRuntimeService Java,
    ILauncherSettingsService Settings,
    IGameFileService GameFiles,
    IGameInstallService Install,
    IGameLaunchService Launch,
    IPluginManager Plugins);

/// <summary>
/// Runs user-initiated work on the UI thread as a task owned by the <c>ui.launcher</c> context, so deactivation cancels
/// and awaits it. Unexpected faults are recorded in host diagnostics; view models handle the expected ones.
/// </summary>
internal sealed class UiOperations(Context context)
{
    /// <summary>Returns false when the plugin is stopping and accepts no new work.</summary>
    public bool Start(Func<CancellationToken, Task> work)
    {
        try
        {
            _ = context.Run(token => Dispatcher.UIThread.InvokeAsync(() => work(token)));
            return true;
        }
        catch (ObjectDisposedException) { return false; }
    }
}
