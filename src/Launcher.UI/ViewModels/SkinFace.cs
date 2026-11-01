using Avalonia;
using Avalonia.Media;
using Avalonia.Media.Imaging;

namespace Launcher.UI.ViewModels;

/// <summary>The 8 × 8 face and hat layers of a Minecraft skin texture, drawn without smoothing by the view.</summary>
internal sealed class SkinFace : IDisposable
{
    private readonly Bitmap _skin;

    private SkinFace(Bitmap skin)
    {
        _skin = skin;
        Face = new CroppedBitmap { Source = skin, SourceRect = new PixelRect(8, 8, 8, 8) };
        if (skin.PixelSize.Width >= 48) Hat = new CroppedBitmap { Source = skin, SourceRect = new PixelRect(40, 8, 8, 8) };
    }

    public IImage Face { get; }
    public IImage? Hat { get; }

    public static SkinFace? TryCreate(byte[] png)
    {
        Bitmap skin;
        try { skin = new Bitmap(new MemoryStream(png, writable: false)); }
        catch (Exception) { return null; } // Decoder failures are platform specific; an unreadable skin shows the placeholder.
        if (skin.PixelSize.Width >= 16 && skin.PixelSize.Height >= 16) return new SkinFace(skin);
        skin.Dispose();
        return null;
    }

    public void Dispose()
    {
        (Face as IDisposable)?.Dispose();
        (Hat as IDisposable)?.Dispose();
        _skin.Dispose();
    }
}
