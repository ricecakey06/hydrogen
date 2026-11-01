using System.Runtime.InteropServices;
using System.Runtime.Versioning;
using Launcher.Contracts;

namespace Launcher.Platform.Windows;

/// <summary>
/// Gives the window a rounded region. DWM clips both the drawn content and the backdrop blur to it, so the blur no
/// longer fills the square corners. Region edges are not antialiased; the window's own outline is drawn inside them.
/// </summary>
[SupportedOSPlatform("windows")]
internal sealed class WindowRegionShaper : IWindowShaper
{
    public void Apply(nint handle, string handleKind, WindowOutline? outline)
    {
        if (handle == 0 || handleKind != "HWND") return;
        // The region's right and bottom edges are exclusive.
        var region = outline is null
            ? 0
            : CreateRoundRectRgn(0, 0, outline.Width + 1, outline.Height + 1, outline.CornerRadius * 2, outline.CornerRadius * 2);
        // On success the system owns the region; otherwise it is still ours to delete.
        if (SetWindowRgn(handle, region, 1) == 0 && region != 0) DeleteObject(region);
    }

    [DllImport("gdi32.dll")]
    [DefaultDllImportSearchPaths(DllImportSearchPath.System32)]
    private static extern nint CreateRoundRectRgn(int left, int top, int right, int bottom, int widthEllipse, int heightEllipse);

    [DllImport("gdi32.dll")]
    [DefaultDllImportSearchPaths(DllImportSearchPath.System32)]
    private static extern int DeleteObject(nint handle);

    [DllImport("user32.dll")]
    [DefaultDllImportSearchPaths(DllImportSearchPath.System32)]
    private static extern int SetWindowRgn(nint window, nint region, int redraw);
}
