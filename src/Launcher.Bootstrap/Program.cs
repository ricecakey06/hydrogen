using Avalonia;
using Launcher.Runtime;
using Launcher.UI;
using Launcher.Plugins;
using System.Runtime.CompilerServices;
using System.Runtime.Loader;

namespace Launcher.Bootstrap;

internal static class Program
{
    // Avalonia needs the process main thread (required on macOS); Windows shell dialogs need it to be STA.
    [STAThread]
    [System.Diagnostics.CodeAnalysis.UnconditionalSuppressMessage("SingleFile", "IL3000",
        Justification = "The check requires official assemblies to be loaded from released plugin files; bundled copies are a packaging error.")]
    private static int Main(string[] args)
    {
        if (args.Any(argument => argument != "--check") || args.Length > 1)
        {
            Console.Error.WriteLine("Usage: Launcher [--check]");
            return 2;
        }

        try
        {
            var bundled = BuiltInPlugins.Create();
            using var manager = new PluginManager(Path.Combine(AppContext.BaseDirectory, "LauncherData", "Plugins"),
                bundled, ["ui.shell", "ui.launcher"], BuiltInPlugins.ReservedIds);
            var definitions = manager.PrepareStartupAsync().GetAwaiter().GetResult();
            if (args.Length == 1)
            {
                foreach (var name in bundled.Select(plugin => Path.GetFileNameWithoutExtension(plugin.EntryAssembly)).Distinct())
                {
                    var assembly = AssemblyLoadContext.Default.Assemblies.Single(assembly => assembly.GetName().Name == name);
                    if (!assembly.Location.StartsWith(manager.PluginsDirectory + Path.DirectorySeparatorChar,
                        OperatingSystem.IsWindows() ? StringComparison.OrdinalIgnoreCase : StringComparison.Ordinal))
                        throw new InvalidOperationException("Official assembly was not loaded from the plugin directory: " + name);
                }
            }
            return RunDesktop(args, manager, definitions);
        }
        catch (Exception error)
        {
            Console.Error.WriteLine(error is Launcher.Contracts.PluginManagementException ? error.Message
                : $"无法准备启动器插件，请检查插件目录权限并关闭其他启动器后重试。（{error.GetType().Name}）");
            return 1;
        }
    }

    // Do not JIT any UI references until its assembly has been released and its resolver registered.
    [MethodImpl(MethodImplOptions.NoInlining)]
    private static int RunDesktop(string[] args, PluginManager manager, IReadOnlyList<PluginDefinition> definitions)
    {
        var desktop = DesktopUi.Initialize(builder => builder.UsePlatformDetect());
        using var shutdown = CancellationTokenSource.CreateLinkedTokenSource(desktop.ShutdownRequested);
        using var uiStop = new CancellationTokenSource();
        ConsoleCancelEventHandler cancel = (_, signal) => { signal.Cancel = true; shutdown.Cancel(); };
        Console.CancelKeyPress += cancel;
        try
        {
            // The host runs on the thread pool. The UI loop keeps running until the host has been disposed,
            // because plugin cleanup closes windows on the UI thread.
            var host = Task.Run(async () =>
            {
                try { return await RunHostAsync(manager, definitions, args.Length == 1, shutdown.Token); }
                finally { await uiStop.CancelAsync(); }
            });
            desktop.Run(uiStop.Token);
            return host.GetAwaiter().GetResult();
        }
        finally
        {
            Console.CancelKeyPress -= cancel;
        }
    }

    private static async Task<int> RunHostAsync(PluginManager manager, IReadOnlyList<PluginDefinition> definitions, bool check, CancellationToken shutdown)
    {
        var host = new PluginHost(definitions);
        try
        {
            await host.StartAsync(shutdown);
            var plugins = await host.GetSnapshotAsync(shutdown);
            manager.RecordRuntime(plugins);
            Console.WriteLine($"Hydrogen host: {plugins.Count} plugin(s), {plugins.Count(plugin => plugin.State == PluginState.Active)} active.");
            foreach (var plugin in plugins.Where(plugin => plugin.State != PluginState.Active))
                Console.WriteLine($"{plugin.ScopePath}/{plugin.Id}: {plugin.State}; missing: {string.Join(", ", plugin.MissingServices.Select(type => type.Name))}");
            if (check)
            {
                var inventory = await manager.GetInventoryAsync(shutdown);
                if (inventory.IsReadOnly || inventory.Plugins.Any(plugin => plugin.IsEnabled && !plugin.IsPendingRemoval
                    && plugin.State is Launcher.Contracts.PluginRunState.Failed or Launcher.Contracts.PluginRunState.Waiting or Launcher.Contracts.PluginRunState.NotLoaded))
                    return 1;
            }
            // Without an active shell there is no window to close, so do not keep an invisible process alive.
            if (!check && plugins.Any(plugin => plugin.Id == UiPlugins.ShellId && plugin.State == PluginState.Active))
            {
                Console.WriteLine("Host running. Close the window or press Ctrl+C to stop.");
                await Task.Delay(Timeout.InfiniteTimeSpan, shutdown);
            }
            if (plugins.Any(plugin => plugin.State != PluginState.Active)) return 1;
        }
        catch (OperationCanceledException) when (shutdown.IsCancellationRequested) { }
        finally
        {
            await host.DisposeAsync();
        }

        // The entry point prints host metadata only; the diagnostics plugin owns redacted per-attempt game logs.
        foreach (var error in host.GetErrors())
            Console.Error.WriteLine($"{error.ContextPath}: {error.Kind} ({error.Exception.GetType().Name})");
        return host.GetErrors().Count == 0 ? 0 : 1;
    }
}
