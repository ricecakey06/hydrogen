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

Hydrogen 使用插件提供界面、账号和游戏管理等功能。当前版本为 **0.3.0**。

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

## 下载与使用

从 [Releases](https://github.com/ricecakey06/hydrogen/releases) 下载 `Launcher.exe`，放入具有写入权限的独立目录后运行。发布页的 `SHA256SUMS.txt` 用于核对文件摘要；未发布的构建可从 [Actions](https://github.com/ricecakey06/hydrogen/actions/workflows/ci.yml) 下载 `Hydrogen-win-x64` 产物。

### 启动

1. 在启动器设置中检查 Java 路径、内存和游戏目录，也可直接选择已有的 `.minecraft`。
2. 在“档案管理”添加微软账号或离线玩家，并选择要使用的档案。离线档案需要启用默认预装的“离线启动”插件。
3. 在“版本管理”添加游戏版本，选择原版、Fabric、Forge 或 NeoForge 组件并安装；完成后选择实例启动。

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
