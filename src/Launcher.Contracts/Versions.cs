namespace Launcher.Contracts;

/// <summary>Official game versions that this launcher supports. Provided by <c>minecraft.content</c>.</summary>
public interface IVersionCatalog
{
    Task<IReadOnlyList<GameVersionInfo>> GetVersionsAsync(CancellationToken cancellationToken);
}

public sealed record GameVersionInfo(string Id);
