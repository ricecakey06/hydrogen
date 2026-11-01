using System.Collections.ObjectModel;
using Avalonia.Platform.Storage;
using Launcher.Contracts;

namespace Launcher.UI.ViewModels;

/// <param name="Choice">Null only for the option that inherits the launcher-wide default.</param>
internal sealed record JavaOption(string Title, string? Detail, JavaChoice? Choice)
{
    public bool HasDetail => Detail is not null;
}

/// <summary>Java selection shared by both settings pages: detected runtimes, rescan and manual selection.</summary>
internal sealed class JavaSelector : ViewModelBase
{
    private readonly LauncherViewModel _root;
    private readonly bool _inheritable;

    public JavaSelector(LauncherViewModel root, bool inheritable, JavaChoice? current)
    {
        _root = root;
        _inheritable = inheritable;
        Rebuild(current);
        if (root.Services.Java.KnownRuntimes.Count == 0) Scan();
    }

    public ObservableCollection<JavaOption> Options { get; } = [];
    public JavaOption? Selected { get; set => SetProperty(ref field, value); }
    public bool IsScanning { get; private set => SetProperty(ref field, value); }
    public string? Message { get; private set { if (SetProperty(ref field, value)) OnPropertyChanged(nameof(HasMessage)); } }
    public bool HasMessage => Message is not null;

    /// <summary>Null means "inherit the launcher-wide default".</summary>
    public JavaChoice? Choice => Selected?.Choice ?? (_inheritable ? null : JavaChoice.Automatic);

    public void Scan()
    {
        if (IsScanning) return;
        IsScanning = true;
        Message = null;
        var accepted = _root.Operations.Start(async token =>
        {
            try
            {
                var found = await _root.Services.Java.ScanAsync(token);
                Rebuild(Choice);
                if (found.Count == 0) Message = "没有找到 Java。可以手动选择 Java 可执行文件。";
            }
            finally { IsScanning = false; }
        });
        if (!accepted) IsScanning = false;
    }

    public void Choose() => _root.Operations.Start(async token =>
    {
        var files = await _root.Shell.StorageProvider.OpenFilePickerAsync(new FilePickerOpenOptions
        {
            Title = "选择 Java 可执行文件",
            AllowMultiple = false,
        });
        if (files.Count == 0 || files[0].TryGetLocalPath() is not { } path) return;
        var runtime = await _root.Services.Java.InspectAsync(path, token);
        if (runtime is null)
        {
            Message = "所选文件不是可用的 Java 可执行文件。";
            return;
        }
        Message = null;
        var option = Describe(runtime);
        if (Options.FirstOrDefault(existing => existing.Choice == option.Choice) is { } duplicate) Options.Remove(duplicate);
        Options.Add(option);
        Selected = option;
    });

    private void Rebuild(JavaChoice? current)
    {
        Options.Clear();
        if (_inheritable) Options.Add(new JavaOption("跟随启动器设置", null, null));
        Options.Add(new JavaOption("自动选择", "按游戏版本要求匹配已安装的 Java", JavaChoice.Automatic));
        foreach (var runtime in _root.Services.Java.KnownRuntimes) Options.Add(Describe(runtime));
        if (current?.ExecutablePath is { } path && Options.All(option => option.Choice != current))
            Options.Add(new JavaOption("手动指定", path, current));
        Selected = Options.FirstOrDefault(option => option.Choice == current) ?? Options[0];
    }

    private static JavaOption Describe(JavaRuntimeInfo runtime)
        => new($"Java {runtime.Version} · {runtime.Architecture}", runtime.ExecutablePath, new JavaChoice(runtime.ExecutablePath));
}

/// <summary>The Java heap limit as a slider plus a number box. It is not the total memory of the game process.</summary>
internal sealed class MemoryEditor : ViewModelBase
{
    private const int Step = 256;
    private int _value;

    public MemoryEditor(MemoryAdvice advice, int value)
    {
        Advice = advice;
        Maximum = Math.Max(advice.TotalMiB, value);
        _value = value;
    }

    public MemoryAdvice Advice { get; }
    public double SliderMinimum => 512;
    public double SliderMaximum => Maximum;
    public decimal InputMinimum => 512;
    public decimal InputMaximum => Maximum;
    public int Maximum { get; }

    public int Value
    {
        get => _value;
        set
        {
            if (!SetProperty(ref _value, value)) return;
            OnPropertyChanged(nameof(SliderValue));
            OnPropertyChanged(nameof(InputValue));
        }
    }

    public double SliderValue
    {
        get => _value;
        set => Value = (int)Math.Round(value / Step) * Step;
    }

    public decimal? InputValue
    {
        get => _value;
        set { if (value is { } input) Value = (int)input; }
    }

    public string AdviceText => $"建议 {Advice.RecommendedMiB} MB，本机内存 {Advice.TotalMiB} MB。这是 Java 堆内存上限，游戏进程实际占用会更高。";
}
