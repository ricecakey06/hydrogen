using System.Runtime.Versioning;
using Launcher.Contracts;
using Launcher.Runtime;

namespace Launcher.Platform.Windows;

/// <summary>Windows adapters. Only the Windows release composition includes them.</summary>
[SupportedOSPlatform("windows")]
public static class WindowsPlugins
{
    public static PluginDefinition Environment(IPlatformEnvironment common) => new("platform.windows.environment", (context, _) =>
    {
        context.Provide<IPlatformEnvironment>(new WindowsEnvironment(common));
        return ValueTask.CompletedTask;
    }, provides: [typeof(IPlatformEnvironment)]);

    public static PluginDefinition Credentials { get; } = new("credentials", (context, _) =>
    {
        context.Provide<ICredentialStore>(new WindowsCredentialStore(context.GetRequiredService<ILauncherStorage>()));
        return ValueTask.CompletedTask;
    }, requires: [typeof(ILauncherStorage)], provides: [typeof(ICredentialStore)]);

    /// <summary>Clips the main window to its rounded outline and enables the backdrop blur behind it (DESIGN.md 9.4).</summary>
    public static PluginDefinition MainWindowBackdrop { get; } = new("platform.windows.window", async (context, cancellationToken) =>
    {
        var window = context.GetRequiredService<IMainWindow>();
        context.Own(await window.EnableBackdropBlurAsync(new WindowRegionShaper(), cancellationToken));
    }, requires: [typeof(IMainWindow)]);
}
