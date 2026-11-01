using Launcher.Plugins;

namespace Launcher.Bootstrap;

internal static class BuiltInPlugins
{
    // This catalog is trusted host metadata only. No product implementation is invoked here.
    private static readonly Dictionary<string, (string Name, string Description)> Metadata = new()
        {
            ["ui.shell"] = ("窗口与外壳", "管理主窗口、缩放和窗口生命周期。"),
            ["ui.launcher"] = ("启动器界面", "提供主页、账号交互、版本选择和设置页面。"),
            ["platform.windows.window"] = ("Windows 窗口效果", "为 Windows 提供圆角裁剪和背景模糊。"),
            ["platform.windows.environment"] = ("Windows 平台", "提供便携数据目录、系统浏览器和 Java 安装位置探测。"),
            ["platform.environment"] = ("桌面平台", "提供系统浏览器、平台信息和 Java 安装位置探测。"),
            ["credentials"] = ("安全凭据", "Windows 使用当前用户的 DPAPI 加密凭据，其他平台仅保留会话登录。"),
            ["storage"] = ("配置与存储", "原子保存启动器设置，管理数据目录并保护不兼容配置。"),
            ["network"] = ("网络服务", "为微软认证及其他服务提供 HTTPS 请求。"),
            ["process"] = ("进程服务", "通过参数数组创建进程，读取输出并监测退出。"),
            ["diagnostics"] = ("本地诊断", "保存经过凭据脱敏的启动和游戏进程日志。"),
            ["account.microsoft"] = ("微软账号", "通过系统浏览器登录，刷新凭据并检查 Minecraft 游戏资格。"),
            ["account.manager"] = ("账号管理", "选择当前登录方式，为界面和游戏启动提供当前账号。"),
            ["account.offline"] = ("离线模式", "提供本地离线账号，在主页显示正版与离线切换。启停在重启后生效。"),
            ["minecraft.content"] = ("官方游戏内容", "读取版本目录和元数据，处理客户端、依赖、资源及平台规则。"),
            ["minecraft.files"] = ("游戏缓存", "校验并复用本地游戏文件，为下载插件提供接入点。"),
            ["minecraft.download"] = ("游戏下载", "通过官方源或 BMCLAPI 获取版本目录，下载或修复客户端、依赖和资源。可在常规设置中选择来源；停用后只使用已校验的本地缓存。"),
            ["minecraft.instances"] = ("隔离游戏实例", "每个版本使用独立目录，保存版本选择和实例设置。"),
            ["minecraft.install"] = ("游戏下载任务", "添加版本时准备游戏文件，显示总体进度和当前文件；离开下载页面后任务继续。"),
            ["java.runtime"] = ("Java 运行时", "扫描并执行探测已安装的 Java，校验版本与架构。"),
            ["minecraft.launch"] = ("游戏启动", "检查环境、准备文件和原生库，启动并监测游戏进程。"),
        };

    // Keep retired IDs here too: a stale official directory must never become a user extension.
    public static IEnumerable<string> ReservedIds => Metadata.Keys;

    public static IReadOnlyList<BundledPlugin> Create()
    {
        var entries = new Dictionary<string, (string Assembly, string Type)>
        {
            ["ui.shell"] = ("Launcher.UI", "Launcher.UI.ShellModule"),
            ["ui.launcher"] = ("Launcher.UI", "Launcher.UI.LauncherModule"),
            ["storage"] = ("Launcher.Features", "Launcher.Plugins.StorageModule"),
            ["network"] = ("Launcher.Features", "Launcher.Plugins.NetworkModule"),
            ["process"] = ("Launcher.Features", "Launcher.Plugins.ProcessModule"),
            ["diagnostics"] = ("Launcher.Features", "Launcher.Plugins.DiagnosticsModule"),
            ["account.microsoft"] = ("Launcher.Features", "Launcher.Plugins.MicrosoftModule"),
            ["account.manager"] = ("Launcher.Features", "Launcher.Plugins.AccountModule"),
            ["account.offline"] = ("Launcher.Features", "Launcher.Plugins.OfflineModule"),
            ["minecraft.content"] = ("Launcher.Features", "Launcher.Plugins.ContentModule"),
            ["minecraft.files"] = ("Launcher.Features", "Launcher.Plugins.FilesModule"),
            ["minecraft.instances"] = ("Launcher.Features", "Launcher.Plugins.InstancesModule"),
            ["minecraft.install"] = ("Launcher.Features", "Launcher.Plugins.InstallModule"),
            ["java.runtime"] = ("Launcher.Features", "Launcher.Plugins.JavaModule"),
            ["minecraft.launch"] = ("Launcher.Features", "Launcher.Plugins.LaunchModule"),
            ["minecraft.download"] = ("Launcher.Downloads", "Launcher.Downloads.DownloadModule"),
        };
        if (OperatingSystem.IsWindows())
        {
            entries["platform.windows.environment"] = ("Launcher.Platform.Windows", "Launcher.Platform.Windows.EnvironmentModule");
            entries["credentials"] = ("Launcher.Platform.Windows", "Launcher.Platform.Windows.CredentialsModule");
            entries["platform.windows.window"] = ("Launcher.Platform.Windows", "Launcher.Platform.Windows.WindowModule");
        }
        else
        {
            entries["platform.environment"] = ("Launcher.Features", "Launcher.Plugins.PlatformModule");
            entries["credentials"] = ("Launcher.Features", "Launcher.Plugins.SessionCredentialsModule");
        }
        var assembly = typeof(BuiltInPlugins).Assembly;
        var version = assembly.GetName().Version?.ToString(3) ?? "0.1.0";
        var bytes = new Dictionary<string, byte[]>();
        return entries.Select(entry =>
        {
            var (name, type) = entry.Value;
            if (!bytes.TryGetValue(name, out var contents))
            {
                using var resource = assembly.GetManifestResourceStream("OfficialPlugins." + name + ".dll")
                    ?? throw new InvalidOperationException("Missing embedded plugin: " + name);
                using var output = new MemoryStream();
                resource.CopyTo(output);
                bytes.Add(name, contents = output.ToArray());
            }
            return new BundledPlugin(entry.Key, Metadata[entry.Key].Name, Metadata[entry.Key].Description,
                name + ".dll", type, contents, version, EnabledByDefault: entry.Key != "account.offline");
        }).ToArray();
    }
}
