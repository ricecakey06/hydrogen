using System.Collections.ObjectModel;
using Launcher.Contracts;

namespace Launcher.UI.ViewModels;

/// <summary>Per-instance overrides for Java, memory and JVM arguments (DESIGN.md 5). Anything else is launcher-wide.</summary>
internal sealed class InstanceSettingsViewModel : PageViewModel
{
    private readonly LauncherViewModel _root;
    private readonly InstanceInfo _instance;

    public InstanceSettingsViewModel(LauncherViewModel root, InstanceInfo instance)
    {
        _root = root;
        _instance = instance;
        Title = $"版本设置 · {instance.Name}";
        InstanceName = instance.Name;
        var overrides = root.Services.Instances.GetOverrides(instance.Id);
        var defaults = root.Services.Settings.Settings;
        Java = new JavaSelector(root, inheritable: true, overrides.Java);
        InheritMemory = overrides.MaxMemoryMiB is null;
        InheritMemoryLabel = $"跟随启动器设置（{defaults.MaxMemoryMiB} MB）";
        Memory = new MemoryEditor(root.Services.Settings.MemoryAdvice, overrides.MaxMemoryMiB ?? defaults.MaxMemoryMiB);
        InheritJvmArguments = overrides.JvmArguments is null;
        JvmArguments = overrides.JvmArguments ?? defaults.JvmArguments;
    }

    public JavaSelector Java { get; }
    public string InstanceName { get; set => SetProperty(ref field, value); }
    public string GameVersion => _instance.GameVersion;
    public MemoryEditor Memory { get; }
    public string InheritMemoryLabel { get; }
    public bool InheritMemory { get; set => SetProperty(ref field, value); }
    public bool InheritJvmArguments { get; set => SetProperty(ref field, value); }
    public string JvmArguments { get; set => SetProperty(ref field, value); } = "";
    public ObservableCollection<string> Issues { get; } = [];
    public string? SavedMessage { get; private set => SetProperty(ref field, value); }
    public bool IsSaving { get; private set => SetProperty(ref field, value); }
    public bool IsConfirmingDelete { get; private set => SetProperty(ref field, value); }
    public string DeleteName { get; private set => SetProperty(ref field, value); } = "";
    public void PrepareFiles() => _root.StartInstall(_instance);

    public void RequestDelete()
    {
        if (IsSaving) return;
        DeleteName = _root.Services.Instances.Instances.FirstOrDefault(instance => instance.Id == _instance.Id)?.Name ?? _instance.Name;
        Issues.Clear();
        SavedMessage = null;
        IsConfirmingDelete = true;
    }

    public void CancelDelete() { if (!IsSaving) IsConfirmingDelete = false; }

    public void ConfirmDelete()
    {
        if (!IsConfirmingDelete || IsSaving) return;
        IsSaving = true;
        var accepted = _root.Operations.Start(async token =>
        {
            try
            {
                await _root.Services.Instances.DeleteAsync(_instance.Id, token);
                _root.ShowVersions();
            }
            catch (LauncherOperationException error)
            {
                IsConfirmingDelete = false;
                Issues.Add(error.Message);
            }
            finally { IsSaving = false; }
        });
        if (!accepted) IsSaving = false;
    }

    public override bool TryGoBack()
    {
        if (IsSaving) return true;
        if (!IsConfirmingDelete) return false;
        CancelDelete();
        return true;
    }

    public void Rename()
    {
        if (IsSaving || IsConfirmingDelete) return;
        var name = InstanceName;
        IsSaving = true;
        Issues.Clear();
        SavedMessage = null;
        var accepted = _root.Operations.Start(async token =>
        {
            try
            {
                await _root.Services.Instances.RenameAsync(_instance.Id, name, token);
                InstanceName = name.Trim();
                Title = $"版本设置 · {InstanceName}";
                SavedMessage = "实例名称已保存。";
            }
            catch (LauncherOperationException error) { Issues.Add(error.Message); }
            finally { IsSaving = false; }
        });
        if (!accepted) IsSaving = false;
    }

    public void Save()
    {
        if (IsSaving || IsConfirmingDelete) return;
        var overrides = new InstanceOverrides(
            Java.Choice,
            InheritMemory ? null : Memory.Value,
            InheritJvmArguments ? null : JvmArguments.Trim());
        IsSaving = true;
        Issues.Clear();
        SavedMessage = null;
        var accepted = _root.Operations.Start(async token =>
        {
            try
            {
                var issues = await _root.Services.Instances.SaveOverridesAsync(_instance.Id, overrides, token);
                foreach (var issue in issues) Issues.Add(issue.Message);
                if (issues.Count == 0) SavedMessage = "已保存，下次启动时生效。";
            }
            finally { IsSaving = false; }
        });
        if (!accepted) IsSaving = false;
    }
}
