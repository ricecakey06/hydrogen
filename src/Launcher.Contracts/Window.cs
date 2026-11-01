namespace Launcher.Contracts;

/// <summary>The main window as offered to platform adapters. Provided by <c>ui.shell</c>.</summary>
public interface IMainWindow
{
    /// <summary>
    /// Turns on the system backdrop blur behind the window body while the registration lives. Without it the window
    /// stays transparent outside its rounded outline and the body is solid, because an unclipped blur would fill the
    /// whole rectangle and show as square corners (DESIGN.md 9.4). The shaper is called on the UI thread before the blur
    /// is requested, whenever the outline changes, and with a null outline when the registration is disposed.
    /// </summary>
    Task<IAsyncDisposable> EnableBackdropBlurAsync(IWindowShaper shaper, CancellationToken cancellationToken);
}

/// <summary>Clips a native window to its rounded outline. Implemented by platform adapters.</summary>
public interface IWindowShaper
{
    /// <param name="handle">The native handle reported by the UI toolkit.</param>
    /// <param name="handleKind">Identifies the handle type, for example <c>HWND</c>.</param>
    /// <param name="outline">The outline to clip to, or null to remove clipping.</param>
    void Apply(nint handle, string handleKind, WindowOutline? outline);
}

/// <summary>A rounded rectangle in physical pixels, relative to the top-left corner of the window.</summary>
public sealed record WindowOutline(int Width, int Height, int CornerRadius);
