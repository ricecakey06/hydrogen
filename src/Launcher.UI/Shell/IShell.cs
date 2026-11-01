using Avalonia.Controls;
using Avalonia.Platform.Storage;

namespace Launcher.UI.Shell;

/// <summary>The main window slot and window behaviours offered by <c>ui.shell</c>. Members must be used on the UI thread.</summary>
internal interface IShell
{
    IStorageProvider StorageProvider { get; }
    ILauncher SystemLauncher { get; }

    /// <summary>Shows <paramref name="content"/> in the window body. Disposing the handle clears it if it is still shown.</summary>
    IDisposable SetContent(Control content);

    /// <summary>The newest handler that returns true takes over the user's close request; otherwise the launcher quits.</summary>
    IDisposable InterceptClose(Func<bool> handler);

    void Minimize();
    void Restore();
    void Hide();
    void Quit();
}
