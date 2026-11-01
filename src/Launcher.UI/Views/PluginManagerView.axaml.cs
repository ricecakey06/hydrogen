using System.ComponentModel;
using Avalonia;
using Avalonia.Controls;
using Launcher.UI.Shell;
using Launcher.UI.ViewModels;

namespace Launcher.UI.Views;

internal sealed partial class PluginManagerView : UserControl
{
    private PluginManagerViewModel? _model;
    private IDisposable? _modal;
    private bool _attached;

    public PluginManagerView()
    {
        InitializeComponent();
        DataContextChanged += (_, _) => BindModel();
    }

    protected override void OnAttachedToVisualTree(VisualTreeAttachmentEventArgs e)
    {
        base.OnAttachedToVisualTree(e);
        _attached = true;
        BindModel();
    }

    protected override void OnDetachedFromVisualTree(VisualTreeAttachmentEventArgs e)
    {
        _attached = false;
        ReleaseModel();
        base.OnDetachedFromVisualTree(e);
    }

    private void BindModel()
    {
        var next = _attached ? DataContext as PluginManagerViewModel : null;
        if (ReferenceEquals(next, _model)) return;
        ReleaseModel();
        _model = next;
        if (_model is null) return;
        _model.PropertyChanged += OnModelChanged;
        UpdateModal();
    }

    private void OnModelChanged(object? sender, PropertyChangedEventArgs e)
    {
        if (e.PropertyName == nameof(PluginManagerViewModel.IsInstallOpen)) UpdateModal();
    }

    private void UpdateModal()
    {
        if (_model?.IsInstallOpen == true)
        {
            if (_modal is not null) return;
            if (TopLevel.GetTopLevel(this) is ShellWindow window)
                _modal = window.ShowModal(new PluginInstallView { DataContext = _model }, _model.CloseInstall);
        }
        else
        {
            _modal?.Dispose();
            _modal = null;
        }
    }

    private void ReleaseModel()
    {
        if (_model is not null)
        {
            _model.PropertyChanged -= OnModelChanged;
            _model.CloseInstall();
            _model = null;
        }
        _modal?.Dispose();
        _modal = null;
    }
}
