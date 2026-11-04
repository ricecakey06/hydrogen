**简体中文** | [English](README-EN.md)

<div align="center">

<img src="src-tauri/icons/icon.png" alt="Hydrogen 图标" width="96" height="96">

# Hydrogen

一个基于 Tauri 2、Rust 和 Svelte 的 Minecraft: Java 版桌面启动器。

简洁界面 · 一切皆插件

[![madewithlove](https://img.shields.io/badge/made_with-%E2%9D%A4-red?style=for-the-badge&labelColor=orange)](https://github.com/ricecakey06/hydrogen)
[![Licence](https://img.shields.io/badge/LICENSE-MIT-green.svg?style=for-the-badge)](LICENSE)
[![Build and test](https://github.com/ricecakey06/hydrogen/actions/workflows/ci.yml/badge.svg)](https://github.com/ricecakey06/hydrogen/actions/workflows/ci.yml)

[下载与使用](#下载与使用) · [从源码构建](#从源码构建) · [插件开发](#项目结构与插件开发)

</div>

Hydrogen 将界面、账号、游戏实例和平台功能组织为独立插件，由宿主管理依赖、权限及生命周期。当前版本为 **0.2.7**，桌面发行目标为 Windows x64。

项目仍在开发中，主要由 AI 辅助编写，长期可用性和稳定性仍需持续验证。

## 功能

- 使用系统 WebView2 显示 Svelte 界面，Rust 承担原生业务；发行包无需 Node.js、Rust 或 .NET 运行时。
- 通过档案管理保存、切换微软账号和离线玩家，缓存用户名与皮肤头像，后台刷新过期会话。
- 安装原版与 Fabric，管理已有 `.minecraft` 目录，下载、校验和修复客户端、依赖库及资源。
- 管理本地模组，支持搜索、导入、框选、批量启停与删除，以及本地中文名编辑。
- 显示游戏启动与下载进度，收集游戏日志，并通过独立插件分析和导出崩溃报告。
- 参考 Cordis 的依赖与生命周期理念，提供 JavaScript 插件、原生 ABI 插件及本地 `.hplugin` 包管理。

## 平台与运行要求

| 平台 | 当前交付方式 | 运行要求 |
| --- | --- | --- |
| Windows x64 | 便携式 `Launcher.exe` | Microsoft Edge WebView2 Runtime 110 或更新版本；启动游戏仍需兼容的 Java |

启动器使用系统 WebView2，不携带浏览器内核，也不会自动安装 WebView2 或下载 Java。共享 Rust 代码在 Windows、macOS 和 Linux 上运行 CI 测试；macOS/Linux 桌面发行尚未提供。

## 下载与使用

从 [Releases](https://github.com/ricecakey06/hydrogen/releases) 下载 `Launcher.exe`，放入具有写入权限的独立目录后运行。发布页的 `SHA256SUMS.txt` 用于核对文件摘要；未发布的构建可从 [Actions](https://github.com/ricecakey06/hydrogen/actions/workflows/ci.yml) 下载 `Hydrogen-win-x64` 产物。

### 首次启动

1. 在启动器设置中检查 Java 路径、内存和游戏目录，也可直接选择已有的 `.minecraft`。
2. 在“档案管理”添加微软账号或离线玩家，并选择要使用的档案。离线档案需要启用默认预装的“离线启动”插件。
3. 在“版本管理”添加游戏版本，选择原版或 Fabric 组件并安装；完成后选择实例启动。

新安装列表支持 Minecraft 1.18 起的官方正式版。Fabric 和 Fabric API 可选择具体版本，安装失败或取消后可重试。下载与准备任务在离开页面后继续执行；启动已有游戏时会校验并补齐缺失资源和依赖。

实例的模组页面显示本地元数据和已缓存信息。点击或拖动框选模组，通过底部操作栏批量启停、删除或清除选择；支持导入 `.jar` / `.jar.disabled` 文件及编辑本地中文名。游戏运行或相关任务执行时限制文件修改。

游戏异常退出时，“崩溃分析”插件可提供可能原因、日志依据和建议，并导出包含日志快照的 ZIP。常见令牌及本机路径会被脱敏，分享前仍请检查模组自行输出的私人内容。分析结果依赖日志，不保证覆盖所有崩溃原因。

### 数据与账号

默认在 EXE 旁的 `HydrogenTauriData/` 保存设置、账号凭据、插件、缓存和日志，在 `.minecraft/` 保存游戏文件。`versions/<版本名>/` 使用标准版本目录，库和资源分别共享 `libraries/` 与 `assets/`。全局设置可选择自动识别、共享游戏目录或每个版本独立目录；删除版本会保留共享存档与资源。

档案列表保存在 `profiles.json`，微软登录凭据按档案存储，并由当前 Windows 用户的 DPAPI 加密。微软登录使用系统浏览器授权；发行者可用 `HYDROGEN_MICROSOFT_CLIENT_ID` 配置默认应用 Client ID。

旧版数据通过设置中的“导入旧版数据”预览后复制，原目录保持不变；账号凭据及旧版 C# DLL 插件不会导入。也可通过启动参数选择独立数据目录：

```powershell
.\Launcher.exe --data-dir D:\HydrogenData
```

## 从源码构建

以下命令均在包含 `package.json` 和 `src-tauri/` 的仓库根目录执行。

需要 Node.js 24、Rust 1.99.1（由 `rust-toolchain.toml` 固定）、MSVC x64、Windows SDK 和 WebView2 Runtime。`scripts/native.ps1` 优先使用相邻 `.tools/tauri/` 中的本地工具链，否则使用已安装的工具链；这些工具不随仓库分发。

### 安装依赖、编译与测试

```powershell
npm ci
npm run build
npm test
.\scripts\native.ps1 -Action test -Extra @('--features','desktop','--locked')
.\scripts\native.ps1 -Action lint
```

`npm run build` 检查 Svelte/TypeScript，构建插件界面资源与宿主引导页。原生脚本先构建独立插件，再测试或构建宿主。`npm test` 运行全部前端单元测试；需要本地游戏目录或 JDK 模拟环境的手动测试默认跳过。

跨平台的无桌面依赖测试可使用：

```sh
npm ci
npm run build
cargo build --manifest-path official-plugins/Cargo.toml --locked
cargo test --manifest-path official-plugins/Cargo.toml --workspace --locked
cargo test --manifest-path official-plugins/Cargo.toml --locked -p hydrogen-plugin-sdk -p hydrogen-plugin-services
cargo test --manifest-path src-tauri/Cargo.toml --locked --no-default-features
```

### 开发运行与 Windows 发布构建

```powershell
npm run desktop
npm run release
```

开发模式使用本地 Vite 服务。发布输出为 `src-tauri/target/release/Launcher.exe`，内嵌宿主前端和预装插件，运行时将插件释放到数据目录；无需另行分发插件目录。发布构建不启动 Vite。

```powershell
$check = Start-Process -FilePath .\src-tauri\target\release\Launcher.exe -ArgumentList '--check' -WindowStyle Hidden -Wait -PassThru
$check.ExitCode
```

`--check` 仅检查 WebView2 是否可用，成功返回 `0`，不创建窗口或读写用户数据。CI 另行启动隔离数据目录，等待界面就绪并检查全部预装插件入口。

## 项目结构与插件开发

| 路径 | 用途 |
| --- | --- |
| `src/` | 通用前端引导页 |
| `src-tauri/src/` | Tauri 入口、IPC、任务与业务服务适配 |
| `src-tauri/src/plugins/` | 包安装、校验、依赖、运行时与生命周期 |
| `official-plugins/` | 独立原生插件及 Svelte 界面源码 |
| `official-plugins/distribution.json` | 预装插件发行索引 |
| `plugin-sdk/` | JavaScript 类型定义、原生 ABI 和共享服务 SDK |
| `examples/instance-tools/` | JavaScript 命令和隔离面板示例 |
| `tests/`、`src-tauri/tests/` | 前端与原生集成测试 |
| `scripts/` | 原生构建与可控 Java 模拟工具 |

插件包使用 ZIP 格式的 `.hplugin` 容器，包含 `plugin.json`、入口及资源。JavaScript 插件 API 见 [launcher.d.ts](plugin-sdk/launcher.d.ts)，完整示例见 [instance-tools](examples/instance-tools/)。开发命令如下：

```powershell
.\scripts\native.ps1 cli plugin init artifacts/my-plugin
.\scripts\native.ps1 cli plugin validate examples/instance-tools --json
.\scripts\native.ps1 cli plugin test examples/instance-tools --json
.\scripts\native.ps1 cli plugin pack examples/instance-tools --out artifacts/instance-tools.hplugin
.\src-tauri\target\release\Launcher.exe --dev-plugin D:\MyPlugin
```

界面可安装、升级、启停、卸载及校验重载插件；相同发行包不会在重启时补回用户已卸载的插件。JavaScript 插件运行于受限 QuickJS 环境，不提供 Node.js、DOM、任意文件或进程接口。原生插件具有进程级权限，仅安装可信来源的插件；旧版 .NET 插件不兼容。

## GitHub Actions

[CI 工作流](.github/workflows/ci.yml) 在 push、Pull Request 和手动触发时执行三平台 Rust 测试、前端构建与测试，Windows 额外运行桌面集成测试。全部通过后构建 Windows x64 EXE，执行原生与界面启动检查，并上传保留 14 天的 `Hydrogen-win-x64` 产物。

`main` 分支的 push 或手动运行还会使用 [Release Drafter 配置](.github/release-drafter.yml) 生成发布说明，发布经过检查的 `Launcher.exe` 和 `SHA256SUMS.txt`。首个 Release 使用 `package.json` 中的版本，之后由 Release Drafter 解析下一版本；发布作业使用仓库的 `GITHUB_TOKEN`。

## 反馈与贡献

欢迎提交 Issue 或 Pull Request。反馈请附上启动器版本或提交、操作系统、Java 版本、复现步骤，以及移除个人信息和凭据后的日志。涉及游戏启动时，请注明游戏版本及外置插件。

提交代码前运行前端构建、单元测试与原生测试；修改发布或插件加载逻辑时，还请检查 Windows Release 构建、`--check` 和实际界面启动。

## 许可证

本项目采用 [MIT 许可证](LICENSE)。第三方组件的许可信息见各组件随附的许可证。

## 状态

![Repobeats](https://repobeats.axiom.co/api/embed/81ab72994aeb53a1b2d0c7f31b64f2b5cbc30e1b.svg "Repobeats analytics image")
[![Star History Chart](https://api.star-history.com/chart?repos=ricecakey06/hydrogen&type=date&legend=top-left)](https://www.star-history.com/?repos=ricecakey06%2Fhydrogen&type=date&legend=top-left)

## 贡献者们

[![Contributors](https://contrib.rocks/image?repo=ricecakey06/hydrogen)](https://github.com/ricecakey06/hydrogen/graphs/contributors)
