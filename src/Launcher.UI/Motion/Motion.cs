using System.Diagnostics;
using Avalonia;
using Avalonia.Animation;
using Avalonia.Animation.Easings;
using Avalonia.Controls;
using Avalonia.Media;
using Avalonia.Styling;
using Avalonia.VisualTree;

namespace Launcher.UI.Motion;

/// <summary>Small, cancellable entrance and text-change animations owned by the visual tree.</summary>
public sealed class Motion : AvaloniaObject
{
    public static readonly AttachedProperty<int> EntranceDelayProperty =
        AvaloniaProperty.RegisterAttached<Motion, Control, int>("EntranceDelay", -1);

    public static readonly AttachedProperty<bool> AnimateTextProperty =
        AvaloniaProperty.RegisterAttached<Motion, TextBlock, bool>("AnimateText");

    private static readonly AttachedProperty<AnimationState?> StateProperty =
        AvaloniaProperty.RegisterAttached<Motion, Control, AnimationState?>("State");

    static Motion()
    {
        EntranceDelayProperty.Changed.AddClassHandler<Control>((control, _) => Configure(control));
        AnimateTextProperty.Changed.AddClassHandler<TextBlock>((control, _) => Configure(control));
    }

    private Motion() { }

    public static int GetEntranceDelay(Control control) => control.GetValue(EntranceDelayProperty);
    public static void SetEntranceDelay(Control control, int value) => control.SetValue(EntranceDelayProperty, value);
    public static bool GetAnimateText(TextBlock control) => control.GetValue(AnimateTextProperty);
    public static void SetAnimateText(TextBlock control, bool value) => control.SetValue(AnimateTextProperty, value);

    private static void Configure(Control control)
    {
        if (control.GetValue(StateProperty) is null)
            control.SetValue(StateProperty, new AnimationState(control));
    }

    internal static Animation FadeMove(TimeSpan duration, double fromOpacity, double toOpacity,
        double fromOffset, double toOffset, bool horizontal = false)
    {
        var offsetProperty = horizontal ? TranslateTransform.XProperty : TranslateTransform.YProperty;
        return new Animation
        {
            Duration = duration,
            Easing = new CubicEaseOut(),
            FillMode = FillMode.Backward,
            Children =
            {
                new KeyFrame
                {
                    Cue = new Cue(0),
                    Setters = { new Setter(Visual.OpacityProperty, fromOpacity), new Setter(offsetProperty, fromOffset) },
                },
                new KeyFrame
                {
                    Cue = new Cue(1),
                    Setters = { new Setter(Visual.OpacityProperty, toOpacity), new Setter(offsetProperty, toOffset) },
                },
            },
        };
    }

    private sealed class AnimationState
    {
        private readonly Control _control;
        private CancellationTokenSource? _current;
        private long _lastTextChange;

        public AnimationState(Control control)
        {
            _control = control;
            control.AttachedToVisualTree += OnAttached;
            control.DetachedFromVisualTree += OnDetached;
            control.PropertyChanged += OnPropertyChanged;
        }

        private void OnAttached(object? sender, VisualTreeAttachmentEventArgs e) => Enter();
        private void OnDetached(object? sender, VisualTreeAttachmentEventArgs e) => _current?.Cancel();

        private void OnPropertyChanged(object? sender, AvaloniaPropertyChangedEventArgs e)
        {
            if (e.Property == Visual.IsVisibleProperty)
            {
                if (_control.IsVisible) Enter();
                else _current?.Cancel();
            }
            else if (e.Property == TextBlock.TextProperty && _control.GetValue(AnimateTextProperty))
            {
                // Fast percentage updates stay legible instead of restarting a fade on every tick.
                var now = Stopwatch.GetTimestamp();
                var quiet = _lastTextChange == 0 || Stopwatch.GetElapsedTime(_lastTextChange, now).TotalMilliseconds >= 250;
                _lastTextChange = now;
                if (quiet) Play(0, 140, 3, 0.45);
            }
        }

        private void Enter()
        {
            if (GetEntranceDelay(_control) is var delay and >= 0) Play(delay, 300, 10, 0);
        }

        // Every replacement cancels its predecessor; detaching never leaves a timer or animation running.
        private async void Play(int delay, int duration, double offset, double opacity)
        {
            if (!_control.IsAttachedToVisualTree() || !_control.IsEffectivelyVisible) return;
            _current?.Cancel();
            var cancellation = new CancellationTokenSource();
            _current = cancellation;
            try
            {
                var animation = FadeMove(TimeSpan.FromMilliseconds(duration), opacity, 1, offset, 0);
                animation.Delay = TimeSpan.FromMilliseconds(delay);
                await animation.RunAsync(_control, cancellation.Token);
            }
            catch (OperationCanceledException) when (cancellation.IsCancellationRequested) { }
            finally
            {
                if (ReferenceEquals(_current, cancellation)) _current = null;
                cancellation.Dispose();
            }
        }
    }
}
