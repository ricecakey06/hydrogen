using Avalonia;
using Avalonia.Threading;

namespace Launcher.UI;

/// <summary>
/// The Avalonia platform on the process main thread. The entry initializes it before the plugin host starts and keeps
/// running it until the host has shut down, so plugin cleanup can still close windows.
/// </summary>
public sealed class DesktopUi
{
    private readonly CancellationTokenSource _shutdown = new();

    private DesktopUi() { }

    internal static DesktopUi Current { get; private set; } = null!;

    /// <summary>Cancelled when the user quits the launcher. Callbacks run on the thread pool, never inside UI events.</summary>
    public CancellationToken ShutdownRequested => _shutdown.Token;

    /// <summary>Must be called on the thread that later calls <see cref="Run"/>.</summary>
    public static DesktopUi Initialize(Func<AppBuilder, AppBuilder> configurePlatform)
    {
        ArgumentNullException.ThrowIfNull(configurePlatform);
        configurePlatform(AppBuilder.Configure<App>()).SetupWithoutStarting();
        return Current = new DesktopUi();
    }

    public void Run(CancellationToken stop) => Dispatcher.UIThread.MainLoop(stop);

    internal void RequestShutdown() => _ = _shutdown.CancelAsync();
}
