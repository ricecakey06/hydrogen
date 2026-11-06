**简体中文** | [English](README-EN.md)

<div align="center">

<img src="src-tauri/icons/icon.png" alt="Hydrogen 图标" width="96" height="96">

# Hydrogen

一个 Minecraft: Java 版桌面启动器。

简洁界面 · 一切皆插件

[![madewithlove](https://img.shields.io/badge/made_with-%E2%9D%A4-red?style=for-the-badge&labelColor=orange)](https://github.com/ricecakey06/hydrogen)
[![Licence](https://img.shields.io/badge/LICENSE-MIT-green.svg?style=for-the-badge)](LICENSE)

[下载与使用](#下载与使用) · [数据与账号](#数据与账号) · [插件管理](#插件管理)

</div>

Hydrogen 使用插件提供界面、账号和游戏管理等功能。当前版本为 **0.2.12**。

项目仍在开发中，主要由 AI 编写，长期可用性和稳定性仍需持续验证。

## 功能

- 通过档案管理保存、切换微软账号和离线玩家，缓存用户名与皮肤头像，后台刷新过期会话。
- 安装原版、Fabric、Forge 与 NeoForge，管理已有 `.minecraft` 目录，下载、校验和修复客户端、依赖库及资源。
- 管理本地模组，支持搜索、导入、框选、批量启停与删除，以及本地中文名编辑。
- 显示游戏启动与下载进度，收集游戏日志，并通过独立插件分析和导出崩溃报告。
- 安装和管理本地 `.hplugin` 插件包。

## 平台与运行要求

| 平台 | 当前交付方式 | 运行要求 |
| --- | --- | --- |
| Windows x64 | 便携式 `Launcher.exe` | Microsoft Edge WebView2 Runtime 110 或更新版本；启动游戏仍需兼容的 Java |

启动器使用系统 WebView2，不会自动安装 WebView2 或下载 Java。目前尚未提供 macOS 和 Linux 桌面发行版。

## 下载与使用

从 [Releases](https://github.com/ricecakey06/hydrogen/releases) 下载 `Launcher.exe`，放入具有写入权限的独立目录后运行。发布页的 `SHA256SUMS.txt` 用于核对文件摘要；未发布的构建可从 [Actions](https://github.com/ricecakey06/hydrogen/actions/workflows/ci.yml) 下载 `Hydrogen-win-x64` 产物。

Release 由维护者手动发布。CI 负责构建、测试并保存 Windows 产物；发布时由维护者选择提交和版本标签、填写说明并上传 `Launcher.exe` 与 `SHA256SUMS.txt`。

### 首次启动

1. 在启动器设置中检查 Java 路径、内存和游戏目录，也可直接选择已有的 `.minecraft`。
2. 在“档案管理”添加微软账号或离线玩家，并选择要使用的档案。离线档案需要启用默认预装的“离线启动”插件。
3. 在“版本管理”添加游戏版本，选择原版、Fabric、Forge 或 NeoForge 组件并安装；完成后选择实例启动。

新安装列表支持全部 Minecraft 官方正式版（含 1.18 以下）、快照与预发布版，并提供版本搜索和类型筛选。Fabric、Forge、NeoForge 和 Fabric API 可选择具体版本；加载器互斥，Fabric API 仅适用于 Fabric，安装失败或取消后可重试。下载与准备任务在离开页面后继续执行；启动已有游戏时会校验并补齐缺失资源和依赖。

实例的模组页面显示本地元数据和已缓存信息。点击或拖动框选模组，通过底部操作栏批量启停、删除或清除选择；支持导入 `.jar` / `.jar.disabled` 文件及编辑本地中文名。游戏运行或相关任务执行时限制文件修改。

将 Modrinth 的 `.mrpack` 或 CurseForge 的 `.zip` 拖入启动器即可安装为独立实例，进度在“下载管理”查看。“版本设置 → 导出整合包”支持按目录或子目录勾选内容，然后选择格式和保存目录。“资源下载 → 模组下载”可切换 Modrinth 和 CurseForge。

在「启动器设置 → 常规」可以切换简体中文和 English，保存后也会设置 Minecraft 的默认语言。已有的游戏语言选择会保留。自定义语言包可参考[英文语言包](official-plugins/i18n/assets/locales/en-US.json)，将 `<locale>.json` 放入启动器数据目录的 `language-packs` 文件夹，重新打开启动器后即可选择。

游戏异常退出时，“崩溃分析”插件可提供可能原因、日志依据和建议，并导出包含日志快照的 ZIP。常见令牌及本机路径会被脱敏，分享前仍请检查模组自行输出的私人内容。分析结果依赖日志，不保证覆盖所有崩溃原因。

### 数据与账号

默认在 EXE 旁的 `HydrogenTauriData/` 保存设置、账号凭据、插件、缓存和日志，在 `.minecraft/` 保存游戏文件。`versions/<版本名>/` 使用标准版本目录，库和资源分别共享 `libraries/` 与 `assets/`。全局设置可选择自动识别、共享游戏目录或每个版本独立目录；删除版本会保留共享存档与资源。

档案列表保存在 `profiles.json`。微软登录使用系统浏览器授权，登录凭据按档案存储，并由当前 Windows 用户的 DPAPI 加密。

旧版数据通过设置中的“导入旧版数据”预览后复制，原目录保持不变；账号凭据及旧版 C# DLL 插件不会导入。也可通过启动参数选择独立数据目录：

```powershell
.\Launcher.exe --data-dir D:\HydrogenData
```

## 插件管理

在启动器中可以安装、升级、启用、停用、卸载以及校验并重载插件。对于相同发行包，重启启动器不会重新安装已卸载的插件。原生插件具有进程级权限，请只安装可信来源的插件。

[插件开发指南](docs/PLUGINS.md) · [主题作者指南](docs/THEMES.md)

## 反馈

遇到问题时，欢迎提交 Issue。请附上启动器版本、操作系统、Java 版本、复现步骤和日志，并先从日志中移除个人信息和凭据。涉及游戏启动时，请注明游戏版本及使用的外置插件。

## 许可证

本项目采用 [MIT 许可证](LICENSE)。第三方组件的许可信息见各组件随附的许可证。
## 状态
![Alt](https://repobeats.axiom.co/api/embed/81ab72994aeb53a1b2d0c7f31b64f2b5cbc30e1b.svg "Repobeats analytics image")
[![Star History Chart](https://api.star-history.com/chart?repos=ricecakey06/hydrogen&type=date&legend=top-left)](https://www.star-history.com/?repos=ricecakey06%2Fhydrogen&type=date&legend=top-left)
## 贡献者们

[![Contributors](https://contrib.rocks/image?repo=ricecakey06/hydrogen)](https://github.com/ricecakey06/hydrogen/graphs/contributors)
