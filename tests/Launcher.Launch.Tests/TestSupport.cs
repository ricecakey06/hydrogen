using System.Net;
using System.Text;
using System.Text.Json;
using Launcher.Contracts;
using Launcher.Plugins.Minecraft;
using Launcher.Plugins.Storage;
using Launcher.Runtime;

namespace Launcher.Launch.Tests;

internal sealed class TestScope : IAsyncDisposable
{
    public string Root { get; } = Path.Combine(Path.GetTempPath(), "hydrogen-launch-tests", Guid.NewGuid().ToString("N"), "中文 路径");
    public PluginHost Host { get; }
    public Context Context { get; private set; } = null!;
    public LauncherStorage Storage { get; private set; } = null!;
    public InstanceService Instances { get; private set; } = null!;
    private TestScope() => Host = new PluginHost([new PluginDefinition("test.context", (context, _) => { Context = context; return ValueTask.CompletedTask; })]);
    public static async Task<TestScope> CreateAsync(string? root = null)
    {
        var test = new TestScope();
        await test.Host.StartAsync();
        test.Storage = new LauncherStorage(test.Context, root ?? test.Root);
        await test.Storage.InitializeAsync(default);
        test.Instances = new InstanceService(test.Context, test.Storage, new FixedCatalog(), new StubPlatform());
        test.Context.Subscribe<DataDirectoryChanged>((_, token) => new ValueTask(test.Instances.LoadAsync(token)));
        await test.Instances.LoadAsync(default);
        return test;
    }
    public async Task CompleteInstanceAsync(string instanceId)
    {
        using var lease = Instances.AcquireLaunch(instanceId);
        await lease.MarkInstalledAsync(default);
    }
    public async ValueTask DisposeAsync()
    {
        await Host.DisposeAsync();
        var owned = Path.GetDirectoryName(Root)!;
        var expected = Path.GetFullPath(Path.Combine(Path.GetTempPath(), "hydrogen-launch-tests")) + Path.DirectorySeparatorChar;
        if (!Path.GetFullPath(owned).StartsWith(expected, OperatingSystem.IsWindows() ? StringComparison.OrdinalIgnoreCase : StringComparison.Ordinal)) throw new InvalidOperationException();
        if (Directory.Exists(owned)) Directory.Delete(owned, true);
    }
}

internal sealed class FixedCatalog : IVersionCatalog
{
    public Task<IReadOnlyList<GameVersionInfo>> GetVersionsAsync(CancellationToken cancellationToken)
        => Task.FromResult<IReadOnlyList<GameVersionInfo>>([new("26.1.2"), new("1.21.1"), new("1.18.2")]);
}

internal sealed class StubPlatform : IPlatformEnvironment
{
    public PlatformTarget Target { get; init; } = new("windows", "x64", "10.0.22631");
    public string DefaultDataDirectory { get; init; } = Path.GetTempPath();
    public List<string> JavaPaths { get; } = [];
    public Func<Uri, CancellationToken, Task>? OnBrowser { get; set; }
    public IEnumerable<string> FindJavaCandidates() => JavaPaths;
    public Task OpenBrowserAsync(Uri uri, CancellationToken cancellationToken) => OnBrowser?.Invoke(uri, cancellationToken) ?? Task.CompletedTask;
}

internal sealed class ScriptedHandler(Func<HttpRequestMessage, CancellationToken, Task<HttpResponseMessage>> respond) : HttpMessageHandler
{
    protected override Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken cancellationToken) => respond(request, cancellationToken);
    public static HttpResponseMessage Json(string json, HttpStatusCode status = HttpStatusCode.OK) => new(status) { Content = new StringContent(json, Encoding.UTF8, "application/json") };
    public static HttpResponseMessage Bytes(byte[] bytes) => new(HttpStatusCode.OK) { Content = new ByteArrayContent(bytes) };
}

internal sealed class StubJava : IJavaRuntimeService
{
    public JavaRuntimeInfo Runtime { get; set; } = new(Path.Combine(Path.GetTempPath(), "java home", "bin", "java"), "25.0.1", "x64");
    public IReadOnlyList<JavaRuntimeInfo> KnownRuntimes => [Runtime];
    public Task<IReadOnlyList<JavaRuntimeInfo>> ScanAsync(CancellationToken cancellationToken) => Task.FromResult(KnownRuntimes);
    public Task<JavaRuntimeInfo?> InspectAsync(string executablePath, CancellationToken cancellationToken) => Task.FromResult<JavaRuntimeInfo?>(Runtime);
}

internal sealed class StubAccount : IGameAccountService
{
    public static GameAccountSession Session { get; } = new("TestPlayer", "123456781234123412341234567890ab", "secret-game-access", "secret-xuid", "11111111-2222-3333-4444-555555555555", DateTimeOffset.UtcNow.AddHours(1));
    public AccountFailureKind? Failure { get; set; }
    public Task<GameAccountSession> GetSessionAsync(CancellationToken cancellationToken) => Failure is { } kind
        ? Task.FromException<GameAccountSession>(new AccountAuthenticationException(kind)) : Task.FromResult(Session);
}

internal sealed class StubContent : IGameContentService
{
    public Func<CancellationToken, Task>? OnPrepare { get; set; }
    public IProgress<LaunchProgress>? LastProgress { get; private set; }
    public int RequiredJava { get; set; } = 25;
    public Task<GameMetadata> ResolveAsync(string version, CancellationToken cancellationToken) => Task.FromResult(Metadata(version, RequiredJava));
    public async Task<PreparedGame> PrepareAsync(GameMetadata metadata, PlatformTarget target, string dataRoot, IProgress<LaunchProgress>? progress, CancellationToken cancellationToken)
    {
        LastProgress = progress;
        if (OnPrepare is not null) await OnPrepare(cancellationToken);
        progress?.Report(new(10, 10));
        return Prepared(metadata, dataRoot);
    }
    public static GameMetadata Metadata(string version = "26.1.2", int java = 25) => new(version, java, JsonDocument.Parse("""
        {"id":"26.1.2","type":"release","mainClass":"net.minecraft.client.main.Main","arguments":{
        "jvm":["-Djava.library.path=${natives_directory}","-cp","${classpath}",
        {"rules":[{"action":"allow","os":{"name":"osx"}}],"value":"-XstartOnFirstThread"}],
        "game":["--username","${auth_player_name}","--accessToken","${auth_access_token}","--userType","${user_type}","--uuid","${auth_uuid}",
        "--clientId","${clientid}","--xuid","${auth_xuid}","--gameDir","${game_directory}",
        {"rules":[{"action":"allow","features":{"has_custom_resolution":true}}],"value":["--width","${resolution_width}","--height","${resolution_height}"]}]}}
        """).RootElement.Clone());
    public static PreparedGame Prepared(GameMetadata metadata, string root) => new(metadata, Path.Combine(root, "cache", "client.jar"),
        [Path.Combine(root, "cache", "a library.jar")], [], Path.Combine(root, "assets"), "test-index", null);
}

internal sealed class StubProcesses : IProcessService
{
    public ProcessRequest? LastRequest { get; private set; }
    public StubProcess? Process { get; private set; }
    public Func<ProcessOutput, ValueTask>? Output { get; private set; }
    public IRunningProcess Start(ProcessRequest request, Func<ProcessOutput, ValueTask>? output = null)
    { LastRequest = request; Output = output; return Process = new StubProcess(); }
}

internal sealed class StubProcess : IRunningProcess
{
    public TaskCompletionSource<int> Exit { get; } = new(TaskCreationOptions.RunContinuationsAsynchronously);
    public int Id => int.MaxValue;
    public bool WasTerminated { get; private set; }
    public bool WasDisposed { get; private set; }
    public Task<int> WaitForExitAsync(CancellationToken cancellationToken) => Exit.Task.WaitAsync(cancellationToken);
    public void Terminate() { WasTerminated = true; Exit.TrySetResult(-1); }
    public ValueTask DisposeAsync() { WasDisposed = true; return ValueTask.CompletedTask; }
}
