using Avalonia;
using Avalonia.Markup.Xaml;

namespace Launcher.UI;

public sealed partial class App : Application
{
    public override void Initialize() => AvaloniaXamlLoader.Load(this);
}
