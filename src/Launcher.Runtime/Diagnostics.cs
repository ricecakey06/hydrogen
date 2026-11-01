namespace Launcher.Runtime;

public enum PluginState { Waiting, Active, Failed }

public enum RuntimeErrorKind { Activation, Cleanup, BackgroundTask, EventHandler }

/// <summary>Structured host diagnostics; a diagnostics plugin may choose how to sanitize and persist them.</summary>
public sealed record RuntimeError(RuntimeErrorKind Kind, string ContextPath, string PluginId, Exception Exception);

public sealed record PluginSnapshot(
    string ScopePath,
    string Id,
    PluginState State,
    IReadOnlyList<Type> MissingServices,
    Exception? ActivationError);

public sealed class PluginCompositionException(string message) : InvalidOperationException(message);
