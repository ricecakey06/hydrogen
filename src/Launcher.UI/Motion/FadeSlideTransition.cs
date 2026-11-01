using Avalonia;
using Avalonia.Animation;
using Avalonia.Controls;
using Avalonia.Input;
using Avalonia.Media;
using Avalonia.Threading;

namespace Launcher.UI.Motion;

/// <summary>A short directional transition, independent of window size and safe to interrupt.</summary>
public sealed class FadeSlideTransition : IPageTransition
{
    public TimeSpan Duration { get; set; } = TimeSpan.FromMilliseconds(240);
    public TimeSpan ExitDuration { get; set; } = TimeSpan.FromMilliseconds(160);
    public double Distance { get; set; } = 24;
    public double ExitDistance { get; set; } = 12;

    public async Task Start(Visual? from, Visual? to, bool forward, CancellationToken cancellationToken)
    {
        if (cancellationToken.IsCancellationRequested) return;
        var direction = forward ? 1 : -1;
        if (from is InputElement outgoing) outgoing.IsEnabled = false;
        if (to is not null)
        {
            // Hide the incoming presenter while its first templates, bindings and layout settle.
            to.IsVisible = true;
            to.Opacity = 0;
            to.RenderTransform = null;
        }
        try
        {
            // Starting during Arrange consumes the first animation while cold controls are still being built.
            await Dispatcher.UIThread.InvokeAsync(() => { }, DispatcherPriority.Loaded, cancellationToken);
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested) { return; }
        if (cancellationToken.IsCancellationRequested) return;
        var owner = TopLevel.GetTopLevel(to ?? from);
        if (owner is not null)
        {
            var completion = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
            TimeSpan? firstFrame = null;
            var fromOpacity = from?.Opacity ?? 1;
            var fromTransform = new TranslateTransform();
            var toTransform = new TranslateTransform(Distance * direction, 0);
            if (from is not null) from.RenderTransform = fromTransform;
            if (to is not null)
            {
                to.RenderTransform = toTransform;
                if (to is InputElement input) input.IsEnabled = true;
            }
            void Detached(object? sender, VisualTreeAttachmentEventArgs e) => completion.TrySetCanceled();
            if (from is not null) from.DetachedFromVisualTree += Detached;
            if (to is not null) to.DetachedFromVisualTree += Detached;
            using var cancellation = cancellationToken.Register(() => completion.TrySetCanceled(cancellationToken));
            void Frame(TimeSpan timestamp)
            {
                if (completion.Task.IsCompleted) return;
                // The clock starts at the first rendered frame, not the last global animation tick.
                // Cold font/template work therefore cannot consume the transition before it is visible.
                firstFrame ??= timestamp;
                var elapsed = timestamp - firstFrame.Value;
                var enter = Progress(elapsed, Duration);
                var exit = Progress(elapsed, ExitDuration);
                if (from is not null)
                {
                    from.Opacity = fromOpacity * (1 - exit);
                    fromTransform.X = -ExitDistance * direction * exit;
                }
                if (to is not null)
                {
                    to.Opacity = enter;
                    toTransform.X = Distance * direction * (1 - enter);
                }
                if (elapsed >= Duration && elapsed >= ExitDuration) completion.TrySetResult();
                else owner.RequestAnimationFrame(Frame);
            }
            owner.RequestAnimationFrame(Frame);
            try { await completion.Task; }
            catch (OperationCanceledException) { return; }
            finally
            {
                if (from is not null) from.DetachedFromVisualTree -= Detached;
                if (to is not null) to.DetachedFromVisualTree -= Detached;
            }
        }
        // A cancelled transition may already have its presenters reused by the next navigation.
        if (cancellationToken.IsCancellationRequested) return;
        if (from is not null)
        {
            from.IsVisible = false;
            from.Opacity = 1;
            from.RenderTransform = null;
            if (from is InputElement input) input.IsEnabled = true;
        }
        if (to is not null)
        {
            to.Opacity = 1;
            to.RenderTransform = null;
            if (to is InputElement input) input.IsEnabled = true;
        }
    }

    private static double Progress(TimeSpan elapsed, TimeSpan duration)
    {
        var fraction = duration <= TimeSpan.Zero ? 1 : Math.Clamp(elapsed.TotalMilliseconds / duration.TotalMilliseconds, 0, 1);
        return 1 - Math.Pow(1 - fraction, 3);
    }
}
