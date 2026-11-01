using Avalonia;
using Avalonia.Controls;
using Avalonia.Input;
using Avalonia.Interactivity;
using Avalonia.Platform;
using Avalonia.Threading;
using Avalonia.VisualTree;
using Launcher.Contracts;

namespace Launcher.UI.Shell;

internal sealed partial class ShellWindow : Window
{
    private const double DesignWidth = 633;
    private const double DesignHeight = 733;
    private const double DesignCornerRadius = 26;
    private const double MinimumScale = 0.6;

    // Plain transparency keeps the rounded corners clean everywhere. Blur is requested only while a platform adapter
    // clips the window to its outline, because the system blur fills the whole window rectangle.
    private static readonly WindowTransparencyLevel[] PlainLevels = [WindowTransparencyLevel.Transparent];
    private static readonly WindowTransparencyLevel[] BlurLevels =
        [WindowTransparencyLevel.Blur, WindowTransparencyLevel.AcrylicBlur, WindowTransparencyLevel.Transparent];

    private readonly Dictionary<Control, ResizeEdges> _grips;
    private readonly List<Cursor> _cursors = [];
    private ResizeDrag? _resize;
    private IPlatformSettings? _platformSettings;
    private bool _closeAllowed;
    private Action? _dismissModal;

    public ShellWindow()
    {
        InitializeComponent();
        TransparencyLevelHint = PlainLevels;
        _grips = new Dictionary<Control, ResizeEdges>
        {
            [GripLeft] = ResizeEdges.Left,
            [GripRight] = ResizeEdges.Right,
            [GripTop] = ResizeEdges.Top,
            [GripBottom] = ResizeEdges.Bottom,
            [GripTopLeft] = ResizeEdges.Top | ResizeEdges.Left,
            [GripTopRight] = ResizeEdges.Top | ResizeEdges.Right,
            [GripBottomLeft] = ResizeEdges.Bottom | ResizeEdges.Left,
            [GripBottomRight] = ResizeEdges.Bottom | ResizeEdges.Right,
        };
        foreach (var (grip, edges) in _grips)
        {
            var cursor = new Cursor(CursorFor(edges));
            _cursors.Add(cursor);
            grip.Cursor = cursor;
            grip.PointerPressed += OnGripPressed;
            grip.PointerMoved += OnGripMoved;
            grip.PointerReleased += OnGripReleased;
            grip.PointerCaptureLost += (_, _) => _resize = null;
        }
        TitleBar.PointerPressed += OnTitleBarPointerPressed;
        MinimizeButton.Click += (_, _) => WindowState = WindowState.Minimized;
        CloseButton.Click += (_, _) => Close();
        ScalingChanged += (_, _) => ApplyShape();
        AddHandler(KeyDownEvent, OnModalKeyDown, RoutingStrategies.Tunnel);
        SetContentScale(MinimumScale);
    }

    /// <summary>Raised instead of closing when the user asks to close the window.</summary>
    public event EventHandler? CloseRequested;

    public ContentControl ContentSlot => Slot;

    /// <summary>Owns one window-wide modal. The registration restores the previous focus on dismissal.</summary>
    public IDisposable ShowModal(Control content, Action dismiss)
    {
        Dispatcher.UIThread.VerifyAccess();
        if (ModalSlot.Content is not null) throw new InvalidOperationException("A window modal is already open.");
        var previousFocus = FocusManager?.GetFocusedElement() as Control;
        _dismissModal = dismiss;
        ModalSlot.Content = content;
        ModalLayer.IsVisible = true;
        ChromeAndPages.IsEnabled = false;
        ResizeGrips.IsHitTestVisible = false;
        Dispatcher.UIThread.Post(() =>
        {
            if (!ReferenceEquals(ModalSlot.Content, content)) return;
            var target = content.GetVisualDescendants().OfType<Control>()
                .FirstOrDefault(control => control.Focusable && control.IsEffectivelyEnabled && control.IsEffectivelyVisible);
            if (target is not null) target.Focus();
            else ModalSlot.Focus();
        }, DispatcherPriority.Loaded);
        return new ModalRegistration(() =>
        {
            Dispatcher.UIThread.VerifyAccess();
            if (!ReferenceEquals(ModalSlot.Content, content)) return;
            ModalSlot.Content = null;
            ModalLayer.IsVisible = false;
            _dismissModal = null;
            ChromeAndPages.IsEnabled = true;
            ResizeGrips.IsHitTestVisible = true;
            if (!_closeAllowed && previousFocus is { IsEffectivelyVisible: true, IsEffectivelyEnabled: true }
                && previousFocus.IsAttachedToVisualTree()) previousFocus.Focus();
        });
    }

    private void OnModalKeyDown(object? sender, KeyEventArgs e)
    {
        if (e.Key != Key.Escape || _dismissModal is null) return;
        e.Handled = true;
        _dismissModal();
    }

    public IWindowShaper? BackdropShaper { get; private set; }

    /// <summary>Closes the window for good. Used when <c>ui.shell</c> is deactivated.</summary>
    public void CloseForShutdown()
    {
        _closeAllowed = true;
        Close();
    }

    /// <summary>Requests the backdrop blur while <paramref name="shaper"/> clips the window; null returns to plain transparency.</summary>
    public void UseBackdropBlur(IWindowShaper? shaper)
    {
        var previous = BackdropShaper;
        BackdropShaper = shaper;
        if (shaper is not null)
        {
            // Clip first so the blur never appears in the square corners.
            ApplyShape();
            TransparencyLevelHint = BlurLevels;
            return;
        }
        TransparencyLevelHint = PlainLevels;
        if (previous is not null && TryGetPlatformHandle() is { } handle)
            previous.Apply(handle.Handle, handle.HandleDescriptor ?? "", null);
    }

    protected override void OnOpened(EventArgs e)
    {
        base.OnOpened(e);
        _platformSettings = this.GetPlatformSettings();
        if (_platformSettings is not null) _platformSettings.ColorValuesChanged += OnColorValuesChanged;
        UpdateMaterial();
        ApplyShape();
    }

    protected override void OnClosed(EventArgs e)
    {
        if (_platformSettings is not null) _platformSettings.ColorValuesChanged -= OnColorValuesChanged;
        _platformSettings = null;
        foreach (var cursor in _cursors) cursor.Dispose();
        _cursors.Clear();
        base.OnClosed(e);
    }

    protected override void OnClosing(WindowClosingEventArgs e)
    {
        base.OnClosing(e);
        if (_closeAllowed) return;
        if (_dismissModal is not null && e.CloseReason != WindowCloseReason.OSShutdown)
        {
            e.Cancel = true;
            _dismissModal();
            return;
        }
        // The plugin decides what closing means; the OS is never blocked from ending the session.
        if (e.CloseReason != WindowCloseReason.OSShutdown) e.Cancel = true;
        CloseRequested?.Invoke(this, EventArgs.Empty);
    }

    protected override void OnPropertyChanged(AvaloniaPropertyChangedEventArgs change)
    {
        base.OnPropertyChanged(change);
        if (change.Property == ActualTransparencyLevelProperty) UpdateMaterial();
        else if (change.Property == ClientSizeProperty) ApplyShape();
    }

    private void OnColorValuesChanged(object? sender, PlatformColorValues e) => UpdateMaterial();

    // Glass only where the platform really blurs the backdrop and high contrast is off; otherwise a readable solid body.
    private void UpdateMaterial()
    {
        var level = ActualTransparencyLevel;
        var blurred = level == WindowTransparencyLevel.Blur || level == WindowTransparencyLevel.AcrylicBlur;
        var highContrast = _platformSettings?.GetColorValues().ContrastPreference == ColorContrastPreference.High;
        Body.Classes.Set("glass", blurred && !highContrast);
    }

    private void ApplyShape()
    {
        if (BackdropShaper is not { } shaper || TryGetPlatformHandle() is not { } handle) return;
        var size = ClientSize;
        var scaling = RenderScaling;
        var contentScale = Math.Min(size.Width / DesignWidth, size.Height / DesignHeight);
        shaper.Apply(handle.Handle, handle.HandleDescriptor ?? "", new WindowOutline(
            (int)Math.Ceiling(size.Width * scaling),
            (int)Math.Ceiling(size.Height * scaling),
            (int)Math.Round(DesignCornerRadius * contentScale * scaling)));
    }

    private void OnTitleBarPointerPressed(object? sender, PointerPressedEventArgs e)
    {
        if (e.GetCurrentPoint(this).Properties.IsLeftButtonPressed) BeginMoveDrag(e);
    }

    // The design surface scales uniformly, so the window keeps its aspect ratio. Resizing is done here rather than by the
    // system because borderless windows have no system resize frame on every platform, and none of them keeps a ratio.
    private void SetContentScale(double scale)
    {
        Width = DesignWidth * scale;
        Height = DesignHeight * scale;
    }

    private double MaximumScale(Screen? screen)
    {
        if (screen is null) return double.PositiveInfinity;
        var work = screen.WorkingArea.Size.ToSize(screen.Scaling);
        return Math.Max(MinimumScale, Math.Min(work.Width / DesignWidth, work.Height / DesignHeight));
    }

    private void OnGripPressed(object? sender, PointerPressedEventArgs e)
    {
        if (sender is not Control grip || !e.GetCurrentPoint(this).Properties.IsLeftButtonPressed) return;
        var size = ClientSize;
        e.Pointer.Capture(grip);
        _resize = new ResizeDrag(
            _grips[grip], this.PointToScreen(e.GetPosition(this)), Position,
            Math.Min(size.Width / DesignWidth, size.Height / DesignHeight), RenderScaling,
            MaximumScale(Screens.ScreenFromWindow(this) ?? Screens.Primary));
        e.Handled = true;
    }

    private void OnGripMoved(object? sender, PointerEventArgs e)
    {
        if (_resize is not { } drag) return;
        var pointer = this.PointToScreen(e.GetPosition(this));
        var dx = (pointer.X - drag.Pointer.X) / drag.Scaling;
        var dy = (pointer.Y - drag.Pointer.Y) / drag.Scaling;
        var width = DesignWidth * drag.Scale;
        var height = DesignHeight * drag.Scale;
        double? byWidth = drag.Edges.HasFlag(ResizeEdges.Left) ? (width - dx) / DesignWidth
            : drag.Edges.HasFlag(ResizeEdges.Right) ? (width + dx) / DesignWidth : null;
        double? byHeight = drag.Edges.HasFlag(ResizeEdges.Top) ? (height - dy) / DesignHeight
            : drag.Edges.HasFlag(ResizeEdges.Bottom) ? (height + dy) / DesignHeight : null;
        // A corner follows whichever direction the pointer moved further.
        var scale = (byWidth, byHeight) switch
        {
            ({ } w, { } h) => Math.Abs(w - drag.Scale) >= Math.Abs(h - drag.Scale) ? w : h,
            ({ } w, null) => w,
            (null, { } h) => h,
            _ => drag.Scale,
        };
        scale = Math.Clamp(scale, MinimumScale, drag.MaximumScale);

        // Dragging the left or top side keeps the opposite side in place.
        var position = new PixelPoint(
            drag.Edges.HasFlag(ResizeEdges.Left) ? drag.Position.X + (int)Math.Round((width - DesignWidth * scale) * drag.Scaling) : drag.Position.X,
            drag.Edges.HasFlag(ResizeEdges.Top) ? drag.Position.Y + (int)Math.Round((height - DesignHeight * scale) * drag.Scaling) : drag.Position.Y);
        SetContentScale(scale);
        if (position != Position) Position = position;
    }

    private void OnGripReleased(object? sender, PointerReleasedEventArgs e)
    {
        if (_resize is null) return;
        _resize = null;
        e.Pointer.Capture(null);
    }

    private static StandardCursorType CursorFor(ResizeEdges edges) => edges switch
    {
        ResizeEdges.Top | ResizeEdges.Left => StandardCursorType.TopLeftCorner,
        ResizeEdges.Top | ResizeEdges.Right => StandardCursorType.TopRightCorner,
        ResizeEdges.Bottom | ResizeEdges.Left => StandardCursorType.BottomLeftCorner,
        ResizeEdges.Bottom | ResizeEdges.Right => StandardCursorType.BottomRightCorner,
        ResizeEdges.Left or ResizeEdges.Right => StandardCursorType.SizeWestEast,
        _ => StandardCursorType.SizeNorthSouth,
    };

    private sealed class ModalRegistration(Action release) : IDisposable
    {
        private Action? _release = release;
        public void Dispose() => Interlocked.Exchange(ref _release, null)?.Invoke();
    }

    [Flags]
    private enum ResizeEdges { Left = 1, Top = 2, Right = 4, Bottom = 8 }

    private sealed record ResizeDrag(ResizeEdges Edges, PixelPoint Pointer, PixelPoint Position, double Scale, double Scaling, double MaximumScale);
}
