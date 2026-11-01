using Launcher.Runtime;

namespace Launcher.Plugins;

internal static class ContextNotifications
{
    internal static async Task NotifyAsync<T>(this Context context, T notification)
    {
        try { await context.PublishAsync(notification).ConfigureAwait(false); }
        catch (ObjectDisposedException) { }
        catch (OperationCanceledException) when (context.Stopping.IsCancellationRequested) { }
        catch (Exception) { /* The runtime records failed subscribers; state notifications must not undo a committed operation. */ }
    }
}
