using System.Text.Json;
using System.Text.RegularExpressions;
using Launcher.Contracts;
using Launcher.Plugins.Storage;

namespace Launcher.Plugins.Minecraft;

public sealed class ContentService(IGameFileService files, ILauncherStorage storage) : IVersionCatalog, IGameContentService
{
    private readonly SemaphoreSlim _catalogGate = new(1, 1);
    private JsonElement? _catalog;
    private DateTimeOffset _loaded;
    private string? _catalogRoot;

    // The first release range has modern argument metadata and Java requirements. Older launch formats stay hidden.
    public static bool IsSupportedVersion(string id)
    {
        if (string.IsNullOrEmpty(id) || !Regex.IsMatch(id, "^[0-9]+\\.[0-9]+(?:\\.[0-9]+)?$", RegexOptions.CultureInvariant)) return false;
        var parts = id.Split('.');
        return int.TryParse(parts[0], out var major) && int.TryParse(parts[1], out var minor) && (major == 1 && minor >= 18 || major >= 26);
    }

    public async Task<IReadOnlyList<GameVersionInfo>> GetVersionsAsync(CancellationToken cancellationToken)
    {
        using var usage = storage.AcquireUsage();
        var catalog = await GetCatalogAsync(cancellationToken).ConfigureAwait(false);
        return catalog.GetProperty("versions").EnumerateArray()
            .Where(version => version.GetProperty("type").GetString() == "release")
            .Select(version => version.GetProperty("id").GetString()!)
            .Where(IsSupportedVersion).Select(id => new GameVersionInfo(id)).ToArray();
    }

    public async Task<GameMetadata> ResolveAsync(string version, CancellationToken cancellationToken)
    {
        if (!IsSupportedVersion(version)) throw new GameLaunchException(LaunchFailureKind.PlatformUnsupported, "支持范围为 1.18 起的官方正式版。");
        using var usage = storage.AcquireUsage();
        var catalog = await GetCatalogAsync(cancellationToken).ConfigureAwait(false);
        var entry = catalog.GetProperty("versions").EnumerateArray().FirstOrDefault(item => item.GetProperty("id").GetString() == version && item.GetProperty("type").GetString() == "release");
        if (entry.ValueKind == JsonValueKind.Undefined) throw new GameLaunchException(LaunchFailureKind.FileVerification, "官方版本目录中找不到所选版本。");
        var hash = entry.GetProperty("sha1").GetString()!;
        var destination = JsonFiles.Within(storage.GameRoot, Path.Combine("versions", version, version + ".json"));
        await files.EnsureFileAsync(new(new Uri(entry.GetProperty("url").GetString()!), destination, hash, null), cancellationToken).ConfigureAwait(false);
        var document = await ReadDocumentAsync(destination, cancellationToken).ConfigureAwait(false);
        if (document.TryGetProperty("minimumLauncherVersion", out var minimum) && minimum.GetInt32() > 21)
            throw new GameLaunchException(LaunchFailureKind.PlatformUnsupported, "此版本需要更新的启动格式，请更新启动器后重试。");
        if (document.GetProperty("id").GetString() != version || !document.TryGetProperty("arguments", out _) || !document.TryGetProperty("javaVersion", out var java))
            throw new GameLaunchException(LaunchFailureKind.PlatformUnsupported, "此版本的启动格式尚未支持。");
        return new GameMetadata(version, java.GetProperty("majorVersion").GetInt32(), document);
    }

    public async Task<PreparedGame> PrepareAsync(GameMetadata metadata, PlatformTarget target, string dataRoot,
        IProgress<LaunchProgress>? progress, CancellationToken cancellationToken)
    {
        if (target.OperatingSystem is not ("windows" or "linux" or "osx") || target.Architecture is not ("x64" or "arm64" or "x86"))
            throw new GameLaunchException(LaunchFailureKind.PlatformUnsupported, "没有可用的游戏原生依赖。");
        var root = metadata.Document;
        var downloads = new List<DownloadFile>();
        var classpath = new List<string>();
        var natives = new List<NativeArchive>();
        var client = Download(root.GetProperty("downloads").GetProperty("client"), JsonFiles.Within(dataRoot, Path.Combine("versions", metadata.Id, metadata.Id + ".jar")));
        downloads.Add(client);
        var assetIndex = root.GetProperty("assetIndex");
        var indexId = assetIndex.GetProperty("id").GetString()!;
        if (!Regex.IsMatch(indexId, "^[a-zA-Z0-9_.-]{1,100}$")) throw new InvalidDataException("Invalid asset index ID.");
        var assets = JsonFiles.Within(dataRoot, "assets");
        var indexFile = Download(assetIndex, JsonFiles.Within(assets, Path.Combine("indexes", indexId + ".json")));
        var indexName = Path.GetRelativePath(dataRoot, indexFile.Destination);
        progress?.Report(new(0, null, indexName, ActiveFiles: [indexName]));
        await files.EnsureFileAsync(indexFile with { Progress = new ByteProgress(value => progress?.Report(new(value, null, indexName, ActiveFiles: [indexName]))) }, cancellationToken).ConfigureAwait(false);
        downloads.Add(indexFile);
        var index = await ReadDocumentAsync(indexFile.Destination, cancellationToken).ConfigureAwait(false);
        if (index.TryGetProperty("virtual", out var virtualAssets) && virtualAssets.GetBoolean()
            || index.TryGetProperty("map_to_resources", out var mapped) && mapped.GetBoolean())
            throw new GameLaunchException(LaunchFailureKind.PlatformUnsupported, "此资源布局尚未支持。");
        foreach (var asset in index.GetProperty("objects").EnumerateObject())
        {
            var hash = asset.Value.GetProperty("hash").GetString()!;
            if (hash.Length != 40 || hash.Any(c => !Uri.IsHexDigit(c))) throw new InvalidDataException("Invalid asset hash.");
            downloads.Add(new(new Uri($"https://resources.download.minecraft.net/{hash[..2]}/{hash}"), JsonFiles.Within(assets, Path.Combine("objects", hash[..2], hash)), hash, asset.Value.GetProperty("size").GetInt64()));
        }
        var lwjglModules = new HashSet<string>(StringComparer.Ordinal);
        var nativeModules = new HashSet<string>(StringComparer.Ordinal);
        var nativeRequired = root.GetProperty("libraries").EnumerateArray().Where(library => MetadataRules.Allows(library, target) &&
            (library.GetProperty("name").GetString()!.Contains(":natives-", StringComparison.Ordinal) || library.TryGetProperty("natives", out _)))
            .Select(library => Module(library.GetProperty("name").GetString()!)).ToHashSet(StringComparer.Ordinal);
        foreach (var library in root.GetProperty("libraries").EnumerateArray())
        {
            if (!MetadataRules.Allows(library, target)) continue;
            var name = library.GetProperty("name").GetString()!;
            if (name.Contains(":natives-", StringComparison.Ordinal) && !MatchesNativeClassifier(name, target)) continue;
            var module = Module(name);
            var libraryDownloads = library.GetProperty("downloads");
            if (libraryDownloads.TryGetProperty("artifact", out var artifact))
            {
                var file = Download(artifact, JsonFiles.Within(dataRoot, Path.Combine("libraries", artifact.GetProperty("path").GetString()!)));
                downloads.Add(file);
                classpath.Add(file.Destination);
            }
            if (name.StartsWith("org.lwjgl:", StringComparison.Ordinal)) lwjglModules.Add(module);
            if (name.Contains(":natives-", StringComparison.Ordinal)) nativeModules.Add(module);
            if (!library.TryGetProperty("natives", out var nativeMap)) continue;
            if (!nativeMap.TryGetProperty(target.OperatingSystem, out var classifier))
                throw new GameLaunchException(LaunchFailureKind.PlatformUnsupported, "缺少当前系统所需的原生库。");
            // Legacy classifiers distinguish 32/64-bit Intel only, never reinterpret them as ARM binaries.
            if (target.Architecture == "arm64") throw new GameLaunchException(LaunchFailureKind.PlatformUnsupported, "此版本未提供 ARM64 原生库。");
            var key = classifier.GetString()!.Replace("${arch}", target.Architecture == "x86" ? "32" : "64", StringComparison.Ordinal);
            if (!libraryDownloads.TryGetProperty("classifiers", out var classifiers) || !classifiers.TryGetProperty(key, out var native))
                throw new GameLaunchException(LaunchFailureKind.PlatformUnsupported, "缺少当前架构所需的原生库。");
            var archive = Download(native, JsonFiles.Within(dataRoot, Path.Combine("libraries", native.GetProperty("path").GetString()!)));
            downloads.Add(archive);
            IReadOnlyList<string> excludes = library.TryGetProperty("extract", out var extract) && extract.TryGetProperty("exclude", out var excluded)
                ? excluded.EnumerateArray().Select(item => item.GetString()!).ToArray() : [];
            natives.Add(new(archive.Destination, excludes));
            nativeModules.Add(module);
        }
        if (lwjglModules.Contains("org.lwjgl:lwjgl") && !nativeModules.Contains("org.lwjgl:lwjgl")
            || lwjglModules.Any(module => nativeRequired.Contains(module) && !nativeModules.Contains(module)))
            throw new GameLaunchException(LaunchFailureKind.PlatformUnsupported, "此版本没有完整匹配系统与架构的 LWJGL 原生库。");
        string? loggingArgument = null;
        if (root.TryGetProperty("logging", out var logging) && logging.TryGetProperty("client", out var clientLogging))
        {
            var logFile = clientLogging.GetProperty("file");
            var file = Download(logFile, JsonFiles.Within(dataRoot, Path.Combine("assets", "log_configs", logFile.GetProperty("sha1").GetString()! + ".xml")));
            downloads.Add(file);
            loggingArgument = clientLogging.GetProperty("argument").GetString()!.Replace("${path}", file.Destination, StringComparison.Ordinal);
        }
        var unique = downloads.GroupBy(file => file.Destination, StringComparer.Ordinal).Select(group => group.First()).ToArray();
        var missingBytes = unique.Where(file => !File.Exists(file.Destination) || file.Size is { } size && new FileInfo(file.Destination).Length != size).Sum(file => file.Size ?? 0);
        var drive = new DriveInfo(Path.GetPathRoot(dataRoot)!);
        if (drive.IsReady && drive.AvailableFreeSpace < missingBytes + 64 * 1024 * 1024)
            throw new GameLaunchException(LaunchFailureKind.InsufficientSpace, $"至少需要 {missingBytes / (1024 * 1024) + 64} MB 可用空间。");
        var tracker = new ContentProgress(unique, dataRoot, progress);
        tracker.Report(force: true);
        await Parallel.ForEachAsync(unique, new ParallelOptions { MaxDegreeOfParallelism = 32, CancellationToken = cancellationToken }, async (file, token) =>
        {
            tracker.Start(file);
            await files.EnsureFileAsync(file with { Progress = new ByteProgress(value => tracker.Update(file, value)) }, token).ConfigureAwait(false);
            tracker.Complete(file);
        }).ConfigureAwait(false);
        return new PreparedGame(metadata, client.Destination, classpath.Distinct(StringComparer.Ordinal).ToArray(), natives, assets, indexId, loggingArgument);
    }

    private async Task<JsonElement> GetCatalogAsync(CancellationToken token)
    {
        await _catalogGate.WaitAsync(token).ConfigureAwait(false);
        try
        {
            if (_catalog is { } current && _catalogRoot == storage.Root && DateTimeOffset.UtcNow - _loaded < TimeSpan.FromMinutes(15)) return current;
            var manifest = await files.GetCatalogAsync(token).ConfigureAwait(false);
            _catalog = manifest;
            _catalogRoot = storage.Root;
            _loaded = DateTimeOffset.UtcNow;
            return manifest;
        }
        finally { _catalogGate.Release(); }
    }

    public static bool MatchesNativeClassifier(string name, PlatformTarget target)
    {
        var parts = name.Split(':');
        if (parts.Length < 4 || !parts[3].StartsWith("natives-", StringComparison.Ordinal)) return true;
        var classifier = parts[3];
        var os = target.OperatingSystem == "osx" ? "macos" : target.OperatingSystem;
        var expected = "natives-" + os + (target.Architecture switch { "x64" => "", "arm64" => "-arm64", "x86" => "-x86", _ => "-unsupported" });
        return classifier == expected;
    }

    private static string Module(string name) => string.Join(':', name.Split(':').Take(2));

    private static DownloadFile Download(JsonElement file, string destination) => new(new Uri(file.GetProperty("url").GetString()!), destination,
        file.GetProperty("sha1").GetString()!, file.TryGetProperty("size", out var size) ? size.GetInt64() : null);

    private static async Task<JsonElement> ReadDocumentAsync(string path, CancellationToken token)
    {
        await using var stream = File.OpenRead(path);
        if (stream.Length > 32 * 1024 * 1024) throw new InvalidDataException("Metadata is too large.");
        using var document = await JsonDocument.ParseAsync(stream, cancellationToken: token).ConfigureAwait(false);
        return document.RootElement.Clone();
    }
    private sealed class ByteProgress(Action<long> report) : IProgress<long>
    { public void Report(long value) => report(value); }

    private sealed class ContentProgress(DownloadFile[] files, string root, IProgress<LaunchProgress>? progress)
    {
        private readonly object _sync = new();
        private readonly Dictionary<string, long> _bytes = new(StringComparer.Ordinal);
        private readonly HashSet<string> _active = new(StringComparer.Ordinal);
        private int _completed;
        private DateTime _reported;
        private readonly long? _total = files.All(file => file.Size is not null) ? files.Sum(file => file.Size!.Value) : null;
        public void Start(DownloadFile file) { lock (_sync) { _active.Add(file.Destination); Report(); } }
        public void Update(DownloadFile file, long value)
        {
            lock (_sync)
            {
                _bytes[file.Destination] = Math.Clamp(value, 0, file.Size ?? long.MaxValue);
                Report();
            }
        }
        public void Complete(DownloadFile file)
        {
            lock (_sync)
            {
                _active.Remove(file.Destination);
                _bytes[file.Destination] = file.Size ?? new FileInfo(file.Destination).Length;
                _completed++;
                Report(force: _completed == files.Length);
            }
        }
        public void Report(bool force = false)
        {
            lock (_sync)
            {
                if (progress is null || !force && DateTime.UtcNow - _reported < TimeSpan.FromMilliseconds(100)) return;
                _reported = DateTime.UtcNow;
                var active = _active.Select(path => Path.GetRelativePath(root, path)).ToArray();
                progress.Report(new(_bytes.Values.Sum(), _total, active.FirstOrDefault(), _completed, files.Length, active));
            }
        }
    }
}
