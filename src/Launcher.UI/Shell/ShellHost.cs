using Avalonia.Controls;
using Avalonia.Platform.Storage;
using Avalonia.Threading;
using Launcher.Contracts;

namespace Launcher.UI.Shell;

/// <summary>Owns the main window for one activation of <c>ui.shell</c>. Created and disposed through the UI dispatcher.</summary>
internal sealed class ShellHost : IShell, IMainWindow, IAsyncDisposable
{
    private readonly DesktopUi _desktop;
    private readonly ShellWindow _window = new();
    private readonly List<Func<bool>> _closeHandlers = [];
    private bool _disposed;

    public ShellHost(DesktopUi desktop)
    {
        Dispatcher.UIThread.VerifyAccess();
        _desktop = desktop;
        _window.CloseRequested += OnCloseRequested;
        _window.Show();
    }

    public IStorageProvider StorageProvider => _window.StorageProvider;
    public ILauncher SystemLauncher => _window.Launcher;

    public IDisposable SetContent(Control content)
    {
        Dispatcher.UIThread.VerifyAccess();
        _window.ContentSlot.Content = content;
        return new Registration(() =>
        {
            if (ReferenceEquals(_window.ContentSlot.Content, content)) _window.ContentSlot.Content = null;
        });
    }

    public IDisposable InterceptClose(Func<bool> handler)
    {
        Dispatcher.UIThread.VerifyAccess();
        _closeHandlers.Add(handler);
        return new Registration(() => _closeHandlers.Remove(handler));
    }

    public void Minimize()
    {
        Dispatcher.UIThread.VerifyAccess();
        _window.WindowState = WindowState.Minimized;
    }

    public void Restore()
    {
        Dispatcher.UIThread.VerifyAccess();
        _window.Show();
        _window.WindowState = WindowState.Normal;
        _window.Activate();
    }

    public void Hide()
    {
        Dispatcher.UIThread.VerifyAccess();
        _window.Hide();
    }

    public void Quit()
    {
        Dispatcher.UIThread.VerifyAccess();
        if (!_disposed) _desktop.RequestShutdown();
    }

    public async Task<IAsyncDisposable> EnableBackdropBlurAsync(IWindowShaper shaper, CancellationToken cancellationToken)
    {
        ArgumentNullException.ThrowIfNull(shaper);
        await Dispatcher.UIThread.InvokeAsync(() =>
        {
            ObjectDisposedException.ThrowIf(_disposed, this);
            _window.UseBackdropBlur(shaper);
        }, DispatcherPriority.Normal, cancellationToken);
        return new AsyncRegistration(async () => await Dispatcher.UIThread.InvokeAsync(() =>
        {
            if (!_disposed && ReferenceEquals(_window.BackdropShaper, shaper)) _window.UseBackdropBlur(null);
        }));
    }

    public async ValueTask DisposeAsync()
    {
        await Dispatcher.UIThread.InvokeAsync(() =>
        {
            if (_disposed) return;
            _disposed = true;
            _closeHandlers.Clear();
            _window.CloseRequested -= OnCloseRequested;
            _window.CloseForShutdown();
        });
    }

    private void OnCloseRequested(object? sender, EventArgs e)
    {
        for (var index = _closeHandlers.Count - 1; index >= 0; index--)
            if (_closeHandlers[index]()) return;
        Quit();
    }

    private sealed class Registration(Action release) : IDisposable
    {
        private Action? _release = release;

        public void Dispose()
        {
            Dispatcher.UIThread.VerifyAccess();
            Interlocked.Exchange(ref _release, null)?.Invoke();
        }
    }

    private sealed class AsyncRegistration(Func<Task> release) : IAsyncDisposable
    {
        private Func<Task>? _release = release;

        public async ValueTask DisposeAsync()
        {
            if (Interlocked.Exchange(ref _release, null) is { } release) await release();
        }
    }
}
