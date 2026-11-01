using System.IO.Compression;
using System.Text.Json;
using Launcher.Contracts;
using Launcher.Plugins.Java;
using Launcher.Plugins.Minecraft;
using Launcher.Plugins.Storage;

namespace Launcher.Launch.Tests;

public sealed class PlanTests
{
    [Theory]
    [InlineData("org.lwjgl:lwjgl:3.4.1:natives-windows", "windows", "x64", true)]
    [InlineData("org.lwjgl:lwjgl:3.4.1:natives-windows-arm64", "windows", "x64", false)]
    [InlineData("org.lwjgl:lwjgl:3.4.1:natives-windows-arm64", "windows", "arm64", true)]
    [InlineData("org.lwjgl:lwjgl:3.4.1:natives-linux", "linux", "arm64", false)]
    [InlineData("org.lwjgl:lwjgl:3.4.1:natives-macos-arm64", "osx", "arm64", true)]
    public void Native_classifiers_must_match_the_java_architecture(string name, string os, string arch, bool expected)
        => Assert.Equal(expected, ContentService.MatchesNativeClassifier(name, new(os, arch, "10.0")));

    [Theory]
    [InlineData("windows", "x64", ';', false)]
    [InlineData("linux", "x64", ':', false)]
    [InlineData("osx", "arm64", ':', true)]
    public void Launch_plans_preserve_arguments_paths_and_platform_rules(string os, string arch, char separator, bool firstThread)
    {
        var root = Path.Combine(Path.GetTempPath(), "中文 游戏 data");
        var game = StubContent.Prepared(StubContent.Metadata(), root);
        var java = new JavaRuntimeInfo(Path.Combine(root, "Java 25", "java"), "25.0.1", arch);
        var settings = LauncherStorage.Defaults with { MaxMemoryMiB = 2048, Window = new(GameWindowMode.Custom, 1280, 720), JvmArguments = "-Dlabel=\"two words\"" };
        var target = new PlatformTarget(os, arch, "10.0.22631");
        var gameDirectory = Path.Combine(root, "instance a", ".minecraft");
        var plan = LaunchPlanBuilder.Build(game, java, StubAccount.Session, settings, target, gameDirectory, Path.Combine(root, "natives session"));
        Assert.Equal(gameDirectory, plan.WorkingDirectory);
        Assert.Equal(java.ExecutablePath, plan.Executable);
        Assert.Contains(string.Join(separator, game.Classpath.Append(game.ClientPath)), plan.Arguments);
        Assert.Contains(gameDirectory, plan.Arguments);
        Assert.Contains("-Dlabel=two words", plan.Arguments);
        Assert.Contains("1280", plan.Arguments);
        Assert.Contains("-Xmx2048M", plan.Arguments);
        Assert.Equal(firstThread, plan.Arguments.Contains("-XstartOnFirstThread"));
        Assert.Contains(StubAccount.Session.AccessToken, plan.Arguments);
        Assert.DoesNotContain(plan.Arguments, argument => argument.Contains("${", StringComparison.Ordinal));
    }

    [Theory]
    [InlineData("-Xmx8G")]
    [InlineData("-XX:MaxHeapSize=8G")]
    [InlineData("--class-path=other.jar")]
    [InlineData("-cp other.jar")]
    [InlineData("@options.txt")]
    [InlineData("-Duser.dir=somewhere")]
    [InlineData("-Djava.library.path=elsewhere")]
    [InlineData("-Dlog4j.configurationFile=elsewhere")]
    [InlineData("-jar other.jar")]
    [InlineData("net.other.Main")]
    [InlineData("--add-opens")]
    [InlineData("-Dlabel=\"unclosed")]
    public void Managed_or_malformed_custom_arguments_are_rejected(string arguments)
        => Assert.Throws<FormatException>(() => SettingsValidation.ParseJvmArguments(arguments));

    [Fact]
    public void Custom_jvm_arguments_keep_quoted_values_and_literal_windows_separators()
    {
        var result = SettingsValidation.ParseJvmArguments("-Dpath=\"C:\\Program Files\\Java\\conf\" -XX:+UseG1GC --add-opens java.base/java.lang=ALL-UNNAMED");
        Assert.Equal([@"-Dpath=C:\Program Files\Java\conf", "-XX:+UseG1GC", "--add-opens", "java.base/java.lang=ALL-UNNAMED"], result);
    }

    [Fact]
    public void Rules_use_order_features_and_the_java_architecture()
    {
        using var document = JsonDocument.Parse("""{"rules":[{"action":"allow","os":{"name":"windows"}},{"action":"disallow","os":{"arch":"x86"}}]}""");
        Assert.True(MetadataRules.Allows(document.RootElement, new("windows", "x64", "10.0")));
        Assert.False(MetadataRules.Allows(document.RootElement, new("windows", "x86", "10.0")));
        Assert.False(MetadataRules.Allows(document.RootElement, new("linux", "x64", "6.0")));
        using var feature = JsonDocument.Parse("""{"rules":[{"action":"allow","features":{"is_demo_user":true}}]}""");
        Assert.False(MetadataRules.Allows(feature.RootElement, new("windows", "x64", "10.0")));
    }

    [Theory]
    [InlineData("1.8.0_432", "amd64", 8, "x64")]
    [InlineData("17.0.16", "x86", 17, "x86")]
    [InlineData("25.0.1", "aarch64", 25, "arm64")]
    public void Java_probe_parses_vendor_properties(string version, string arch, int major, string expectedArch)
    {
        var runtime = JavaRuntimeService.ParseSettings("java", $"    java.version = {version}\n    os.arch = {arch}\n");
        Assert.NotNull(runtime);
        Assert.Equal(major, JavaRuntimeService.MajorVersion(runtime.Version));
        Assert.Equal(expectedArch, runtime.Architecture);
        Assert.Null(JavaRuntimeService.ParseSettings("java", "not a Java executable"));
    }

    [Fact]
    public async Task Native_extraction_excludes_metadata_and_rejects_path_traversal()
    {
        await using var test = await TestScope.CreateAsync();
        Directory.CreateDirectory(test.Root);
        var archive = Path.Combine(test.Root, "natives.zip");
        using (var zip = ZipFile.Open(archive, ZipArchiveMode.Create))
        {
            await using (var content = zip.CreateEntry("native.dll").Open()) await content.WriteAsync(new byte[] { 1, 2, 3 });
            zip.CreateEntry("META-INF/MANIFEST.MF");
        }
        var target = Path.Combine(test.Root, "extract");
        await LaunchPlanBuilder.ExtractNativesAsync([new(archive, [])], target, default);
        Assert.True(File.Exists(Path.Combine(target, "native.dll")));
        Assert.False(Directory.Exists(Path.Combine(target, "META-INF")));
        using (var zip = ZipFile.Open(archive, ZipArchiveMode.Update)) zip.CreateEntry("../escape.txt");
        await Assert.ThrowsAsync<IOException>(() => LaunchPlanBuilder.ExtractNativesAsync([new(archive, [])], target, default));
        Assert.False(File.Exists(Path.Combine(test.Root, "escape.txt")));
    }
}
