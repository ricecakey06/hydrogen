using System.ComponentModel;
using System.Runtime.CompilerServices;

namespace Launcher.UI.ViewModels;

/// <summary>View models are created, changed and disposed on the UI thread only.</summary>
internal abstract class ViewModelBase : INotifyPropertyChanged
{
    public event PropertyChangedEventHandler? PropertyChanged;

    protected bool SetProperty<T>(ref T field, T value, [CallerMemberName] string? propertyName = null)
    {
        if (EqualityComparer<T>.Default.Equals(field, value)) return false;
        field = value;
        OnPropertyChanged(propertyName);
        return true;
    }

    protected void OnPropertyChanged([CallerMemberName] string? propertyName = null)
        => PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(propertyName));
}

/// <summary>A secondary page shown below the shared page header.</summary>
internal abstract class PageViewModel : ViewModelBase
{
    public string Title { get; protected set => SetProperty(ref field, value); } = "";

    /// <summary>Returns true when the page handled the back navigation itself.</summary>
    public virtual bool TryGoBack() => false;
}
