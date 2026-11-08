**简体中文** | [English](README-EN.md)

<div align="center">

<img src="src-tauri/icons/icon.png" alt="Hydrogen 图标" width="96" height="96">

# Hydrogen

一个 Minecraft: Java 版桌面启动器。

简洁界面 · 一切皆可插件

[![madewithlove](https://img.shields.io/badge/made_with-%E2%9D%A4-red?style=for-the-badge&labelColor=orange)](https://github.com/ricecakey06/hydrogen)
[![Licence](https://img.shields.io/badge/LICENSE-MIT-green.svg?style=for-the-badge)](LICENSE)

[下载与使用](#下载与使用) · [数据与账号](#数据与账号) · [插件管理](#插件管理)

</div>

Hydrogen 使用插件提供界面、账号和游戏管理等功能。

项目仍在开发中，主要由 AI 编写，开发者本人不保证长期使用的稳定性。
## 功能

- 正版账号登录，正版和离线档案的管理与游戏启动；
- 一键安装原版、Fabric、Forge 与 NeoForge；
- Modrinth和Curseforge整合包的一键安装；
- 可通过插件来高度定制启动器功能；
- 没了，也许会有别的

## 平台与运行要求

| 平台 | 执行文件 |
| --- | --- | 
| Windows x64 |  `Launcher.exe` | 
## 下载与使用

从 [Releases](https://github.com/ricecakey06/hydrogen/releases) 下载 `Launcher.exe`并启动。
测试版构建可以去Actions里寻找。
因Release发版不规律不频繁，优先推荐下载Actions内版本。

### 启动

1. 在启动器设置中检查 Java 路径、内存和游戏目录；
2. 在主界面登录微软账号，或在“档案管理”添加离线玩家，并选择要使用的档案。
3. 在“版本管理”添加游戏版本，此时可以选择安装三种加载器当中的任意一种，或是拖入整合包文件来自动安装；
4. 启动游戏。

## 插件管理

在启动器的插件管理中可以安装、升级、启用、停用、卸载以及校验并重载插件。对于相同发行包，重启启动器不会重新安装已卸载的插件。
原生插件具有进程级权限，请只安装可信来源的插件。

[插件编写说明](docs/PLUGINS.md) · [主题编写说明](docs/THEMES.md)

## 反馈

遇到问题时，欢迎提交 Issue。请附上启动器版本、操作系统、Java 版本、复现步骤和日志，并先从日志中移除个人信息和凭据。涉及游戏启动时，请注明游戏版本及使用的外置插件。

## 许可证

本项目采用 [MIT 许可证](LICENSE)。第三方组件的许可信息见各组件随附的许可证。
## 状态
![Alt](https://repobeats.axiom.co/api/embed/81ab72994aeb53a1b2d0c7f31b64f2b5cbc30e1b.svg "Repobeats analytics image")
[![Star History Chart](https://api.star-history.com/chart?repos=ricecakey06/hydrogen&type=date&legend=top-left)](https://www.star-history.com/?repos=ricecakey06%2Fhydrogen&type=date&legend=top-left)
## 贡献者们
[![Contributors](https://contrib.rocks/image?repo=ricecakey06/hydrogen)](https://github.com/ricecakey06/hydrogen/graphs/contributors)
