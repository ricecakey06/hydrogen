**简体中文** | [English](README-EN.md)

<div align="center">

<img src="src/Launcher.UI/Assets/hydrogen-icon.png" alt="Hydrogen 图标" width="96" height="96">

# Hydrogen

一个基于 .NET 10 和 Avalonia 的 Minecraft: Java 版桌面启动器。

简洁界面 · 一切皆插件

[![madewithlove](https://img.shields.io/badge/made_with-%E2%9D%A4-red?style=for-the-badge&labelColor=orange
)](https://github.com/ricecakey06/hydrogen)
[![Licence](https://img.shields.io/badge/LICENSE-MIT-green.svg?style=for-the-badge)](https://github.com/ricecakey06/hydrogen/blob/main/LICENSE)

[下载与使用](#下载与使用) · [从源码构建](#从源码构建) · [插件开发](docs/plugins.md)

</div>

Hydrogen 将界面、账号、游戏实例和平台功能组织为插件，由宿主管理依赖与生命周期。

项目仍在开发中。

本项目主要由 AI 编写，因此不保证长期可用性和稳定性。

## 功能

- 基于 Avalonia，有着一定的跨平台兼容性；
- 参考了Cordis的理念，启动器的一切功能皆为插件，也可通过内置的插件管理安装新的插件，带来了高度的可拓展性；
- 也许会有更多？


## 平台与运行要求

| 平台 | 当前交付方式 | 说明 |
| --- | --- | --- |
| Windows x64 | 自包含单文件 `Launcher.exe` | 无需另外安装 .NET 运行时；启动游戏仍需兼容的 Java |


## 使用

### 首次启动

1. 在启动器设置中检查 Java 路径、内存和游戏目录。
2. 使用微软账号登录；微软登录需要有效的应用 Client ID，配置方式见下文。也可在插件管理中启用离线模式，重启后设置玩家名称。
3. 添加游戏版本并创建实例，启动器会自动下载和校验所需文件，准备完成后即可选择实例启动。

Windows 默认在 EXE 同级的 `LauncherData/` 保存设置、实例记录和插件，在 `.minecraft/` 保存游戏文件及实例游戏目录。切换数据目录后，以设置页显示的实际路径为准；已有数据保留在原位置。

## 从源码构建

以下命令均在仓库根目录运行，即包含 `Launcher.slnx` 的目录。

### 还原、编译与测试

```sh
dotnet restore Launcher.slnx --locked-mode
dotnet build Launcher.slnx --configuration Release --no-restore
dotnet test Launcher.slnx --configuration Release --no-build --no-restore
```

### 运行与启动检查

```sh
dotnet run --project src/Launcher.Bootstrap --configuration Release --no-build --no-restore
dotnet run --project src/Launcher.Bootstrap --configuration Release --no-build --no-restore -- --check
```

`--check` 会初始化界面与插件后退出，成功时返回 `0`。Linux 无图形会话时，使用 `xvfb-run -a` 执行启动检查。

### 发布 Windows x64 单文件

```sh
dotnet publish src/Launcher.Bootstrap --configuration Release -p:PublishProfile=win-x64 -p:RestoreLockedMode=true
```

输出为 `artifacts/publish/win-x64/Launcher.exe`。运行时释放的官方插件已嵌入 EXE，无需另行分发插件目录。


## 项目结构与插件开发

| 路径 | 用途 |
| --- | --- |
| `src/Launcher.Bootstrap/` | 程序入口、官方插件目录与发布配置 |
| `src/Launcher.Runtime/` | 插件依赖解析、作用域及资源生命周期 |
| `src/Launcher.Contracts/` | 插件共享的业务与服务契约 |
| `src/Launcher.Plugins/` | 插件包安装、加载、升级与管理 |
| `src/Launcher.Features/` | 账号、实例、Java、存储和游戏启动功能 |
| `src/Launcher.UI/` | Avalonia 界面、主题与图标资源 |
| `src/Launcher.Platform.Windows/` | Windows 平台集成、凭据和窗口效果 |
| `tests/` | 运行时、插件管理与游戏业务测试 |

插件包格式、入口示例和生效规则见 [插件开发文档](docs/plugins.md)。插件在启动器进程中运行，应仅安装可信来源的插件。

## 反馈与贡献

欢迎提交 Issue 或 Pull Request。问题反馈请附上启动器版本或提交、操作系统、Java 版本、复现步骤，以及移除个人信息和凭据后的日志。涉及游戏启动时，请注明游戏版本及使用的外置插件。

提交代码前，请运行编译与测试命令；修改发布或插件加载逻辑时，也请执行 `--check` 和 Windows 发布检查。

## 许可证

本项目采用 [MIT 许可证](LICENSE)。第三方组件的许可信息见各组件随附的许可证。

## 状态
![Alt](https://repobeats.axiom.co/api/embed/81ab72994aeb53a1b2d0c7f31b64f2b5cbc30e1b.svg "Repobeats analytics image")
[![Star History Chart](https://api.star-history.com/chart?repos=ricecakey06/hydrogen&type=date&legend=top-left)](https://www.star-history.com/?repos=ricecakey06%2Fhydrogen&type=date&legend=top-left)
## 贡献者们
[![](https://contrib.rocks/image?repo=ricecakey06/hydrogen)](https://github.com/ricecakey06/hydrogen/graphs/contributors)