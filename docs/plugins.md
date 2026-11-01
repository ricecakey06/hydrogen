# 本地 Hydrogen 插件

插件面向 .NET 10，使用 `Launcher.Runtime` 的 `IPluginModule` 和 `PluginDefinition`，通过 `Launcher.Contracts` 引用共享业务契约。它们与内置插件使用相同的依赖解析、Context 资源归属和关闭语义。

## 官方插件释放与升级

项目只维护一个正式发行组合，内置 `minecraft.download`（游戏下载）插件并默认启用，支持官方源与 BMCLAPI；来源在常规设置中选择。用户可以在插件管理中停用，重启后只复用本地缓存。外置插件也可通过 `IGameDownloadRegistry` 注册 `IGameDownloadProvider`，同一时刻只允许一个提供者。

发行物仍是自包含的单个 `Launcher.exe`，无需联网下载组件。官方插件的程序集以资源形式随 EXE 分发；首次启动先将每个插件的 `plugin.json` 和入口 DLL 释放到 `LauncherData/Plugins/<插件 ID>/`，再通过与外置包相同的清单、`IPluginModule` 入口和运行时加载。账号、游戏业务实现位于 `Launcher.Features`，不再与宿主的插件管理器编译在同一程序集。

每次启动会校验官方文件。文件完整且与当前 EXE 一致时直接复用；更换 EXE、文件缺失或被修改时，在加载程序集前通过临时目录和备份目录替换或修复。中断的目录交换会在下次启动恢复。插件目录只保存插件程序文件，用户数据应保存在目录之外。

`plugins.json` 独立保存启停设置，释放和升级不会重写该文件。已有插件保持用户的选择，新插件才使用发行默认值；离线模式仍默认关闭。当前发行未包含的官方插件目录会被跳过，启停设置保留供之后恢复该组件时使用。

“官方内置”来源由 EXE 中的受信任目录确定，不能通过外置清单中的作者或自定义字段冒充。主界面及其传递依赖禁止停用和卸载；官方可选插件可以停用，文件由宿主维护，不能用外置包覆盖。最小插件管理器本身属于宿主基础设施，不需要释放。官方程序集从插件目录加载到共享上下文，以保持界面资源和服务类型身份一致；外置插件继续使用私有加载上下文。

## 包结构

`.hplugin` 是 ZIP 文件的自定义后缀，`.zip` 同样可以安装。根目录必须包含以下文件，不能再套一层目录：

```text
plugin.json
Example.Plugin.dll
Example.Plugin.deps.json   （使用私有依赖时建议附带）
其他私有依赖文件           （如有）
```

`plugin.json` 示例：

```json
{
  "schemaVersion": 1,
  "apiVersion": 1,
  "id": "example.clock",
  "name": "示例时钟",
  "version": "1.0.0",
  "description": "演示 Hydrogen 插件的激活与清理。",
  "author": "Example",
  "entryAssembly": "Example.Plugin.dll",
  "entryType": "Example.Plugin.ClockModule",
  "dependencies": []
}
```

ID 使用小写字母、数字和 `.`、`-`、`_`，例如 `example.clock`。`dependencies` 列出其他必需插件的 ID；共享服务仍需在 `PluginDefinition.Requires` 中以契约类型声明。不要把启动器的 Runtime、Contracts、Avalonia 或系统程序集复制进包；这些类型与宿主共享，私有依赖随自己的 DLL 分发。

包最大为 256 MB，解压后同样最多 256 MB、4096 项；清单最多 64 KB。不接收越界路径、符号链接、目录联接、重复文件或当前 API 不兼容的清单。版本为 `1.0.0` 之类的三段格式，可带预发布或构建标记。

## 模块入口

项目引用与启动器同版本的 `Launcher.Runtime`；需要业务服务时再引用 `Launcher.Contracts`。入口类必须公开、非抽象并有公共无参构造函数，返回的定义 ID 必须与清单一致：

```csharp
using Launcher.Runtime;

namespace Example.Plugin;

public sealed class ClockModule : IPluginModule
{
    public PluginDefinition CreateDefinition() => new(
        "example.clock",
        (context, cancellationToken) =>
        {
            context.Run(async stopping =>
            {
                using var timer = new PeriodicTimer(TimeSpan.FromSeconds(1));
                while (await timer.WaitForNextTickAsync(stopping))
                {
                    // 执行插件自己的工作，响应 stopping。
                }
            });
            return ValueTask.CompletedTask;
        });
}
```

构建后将入口 DLL、清单和私有依赖一起压缩，在“启动器设置 → 插件 → 安装插件”中选择该包并重启。安装和清单浏览不会调用构造函数或 `CreateDefinition`；重启后才加载入口。不要在构造函数或 `CreateDefinition` 中启动工作，副作用应归属激活获得的 Context。

插件是用户选择的受信任本地代码，与启动器处于同一进程，不是脚本沙箱。加载器共享宿主程序集的类型身份，为插件保留私有加载上下文；关闭全部插件后才释放这些上下文。插件自己的持久数据应保存在包目录之外，卸载只管理包目录。

## 管理与生效时机

启停偏好写入便携目录 `LauncherData/Plugins/plugins.json`，正在运行的组合保持到程序关闭；重启时再应用。卸载先写入待移除状态，用户可以撤销，下一次启动加载程序集前才删除包目录。其他已启用插件的声明依赖和已加载的服务依赖都会阻止直接停用或卸载其提供者。

内置插件不能被外置包覆盖。官方插件随 EXE 自动同步；外置插件当前没有原地更新操作，需要先卸载旧包、重启，再安装新包。外置插件加载失败时可查看条目并停用或卸载，其他插件继续运行。没有 UI 扩展槽位 API；本版本的包入口负责宿主功能和共享服务。
