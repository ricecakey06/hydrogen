namespace Launcher.Contracts;

/// <summary>
/// Reserved contract for loader plugins on the instance setup page. No provider discovery, UI contributions,
/// or execution pipeline is enabled yet; the current installer prepares the official game only.
/// </summary>
public interface IInstancePreInstallProvider
{
    string Id { get; }
    string DisplayName { get; }

    /// <summary>Returns compatible choices for the selected game version; an empty list means unsupported.</summary>
    Task<IReadOnlyList<InstancePreInstallOption>> GetOptionsAsync(string gameVersion, CancellationToken cancellationToken);

    /// <summary>
    /// Prepares the selected loader after base game files are available and before the instance is marked installed.
    /// Writes must stay in the supplied private instance directory; shared game files must not be modified.
    /// A future host must persist the choice, own cancellation and hold the instance lease while calling this.
    /// </summary>
    Task PrepareAsync(InstancePreInstallContext context, string optionId,
        IProgress<LaunchProgress>? progress, CancellationToken cancellationToken);
}

public sealed record InstancePreInstallOption(string Id, string DisplayName);

public sealed record InstancePreInstallContext(InstanceInfo Instance, string GameDirectory);
