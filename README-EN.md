[简体中文](README.md) | **English**

<div align="center">

<img src="src-tauri/icons/icon.png" alt="Hydrogen icon" width="96" height="96">

# Hydrogen

A desktop launcher for Minecraft: Java Edition.

A simple interface · Everything is a plugin

[![madewithlove](https://img.shields.io/badge/made_with-%E2%9D%A4-red?style=for-the-badge&labelColor=orange)](https://github.com/ricecakey06/hydrogen)
[![Licence](https://img.shields.io/badge/LICENSE-MIT-green.svg?style=for-the-badge)](LICENSE)
[Download and usage](#download-and-usage) · [Data and accounts](#data-and-accounts) · [Plugin management](#plugin-management)

</div>

Hydrogen uses plugins to provide its interface, account management and game management features. The current version is **0.3.5**.

The project is under development and primarily written with AI. Long-term usability and stability still require ongoing validation.

## Features

- Profiles for Microsoft accounts and offline players, cached names and skin avatars, and background session refresh.
- Install vanilla Minecraft, Fabric, Forge and NeoForge; manage existing `.minecraft` directories; and download, validate and repair game files, libraries and assets.
- Local mod search, import, drag selection, batch enable/disable and deletion, and locally editable Chinese names.
- Launch and download progress, game logs, and crash analysis and report export through an independent plugin.
- Install and manage local `.hplugin` packages.

## Platforms and requirements

| Platform | Distribution | Requirements |
| --- | --- | --- |
| Windows x64 | Portable `Launcher.exe` | Microsoft Edge WebView2 Runtime 110 or later; compatible Java to launch the game |

The launcher uses system WebView2 and does not automatically install WebView2 or download Java. Desktop releases for macOS and Linux are not yet available.

## Download and usage

Download `Launcher.exe` from [Releases](https://github.com/ricecakey06/hydrogen/releases), place it in a separate writable directory, and run it. Use `SHA256SUMS.txt` on the release page to verify the file. Unreleased builds are available as `Hydrogen-win-x64` artifacts in [Actions](https://github.com/ricecakey06/hydrogen/actions/workflows/ci.yml).

Maintainers publish releases manually. CI builds, tests and retains the Windows artifacts; maintainers choose the commit and version tag, write the release notes, and upload `Launcher.exe` and `SHA256SUMS.txt` when publishing.

### First launch

1. Check Java, memory and the game directory in settings, or select an existing `.minecraft`.
2. Add a Microsoft account or offline player in Profiles and select the profile to use. Offline profiles require you to enable the preinstalled Offline Launch plugin.
3. Add a game version, choose vanilla, Fabric, Forge, or NeoForge components, install it and select the instance to launch.

New installations support all official Minecraft releases (including versions below 1.18), snapshots, and pre-releases, with version search and type filters. You can choose specific versions of Fabric, Forge, NeoForge, and Fabric API. Choose one loader; Fabric API only works with Fabric. Failed or cancelled installations can be retried. Downloads and preparation continue after navigating away. Launching existing games validates and fills missing assets and dependencies.

The mods page shows local metadata and cached information. Click or drag to select mods and use the selection bar to enable, disable, delete or clear the selection. Import `.jar` / `.jar.disabled` files and edit local Chinese names. File modifications are restricted while the game or related tasks are active.

Drop a Modrinth `.mrpack` or CurseForge `.zip` on the launcher to install it as an isolated instance. Progress appears in Downloads. To export, open **Version settings → Export modpack**, select folders or subfolders, then choose a format and destination folder. The mod catalog supports both Modrinth and CurseForge.

Switch between English and Simplified Chinese under **Settings → General**. Saving also sets Minecraft's default language and preserves language choices made in the game. To add a custom language, use the [English pack](official-plugins/i18n/assets/locales/en-US.json) as a template, save it as `<locale>.json` in the launcher data directory under `language-packs`, and restart the launcher.

After an abnormal game exit, the Crash Analysis plugin can show possible causes, log evidence and suggestions, and export a ZIP containing log snapshots. Common tokens and local paths are redacted; review private content written by mods before sharing. Analysis depends on the logs and is not guaranteed to cover every cause of a crash.

### Data and accounts

By default, `HydrogenTauriData/` beside the EXE stores settings, credentials, plugins, caches and logs. Game files use the adjacent `.minecraft/`, standard `versions/<name>/` directories, shared `libraries/` and `assets/`. Global settings support automatic, shared or per-version game directories. Deleting a version preserves shared worlds and assets.

Profiles are stored in `profiles.json`. Microsoft sign-in uses the system browser. Credentials are stored separately per profile and encrypted with the current Windows user's DPAPI.

Legacy data is previewed and copied through Import legacy data in settings, leaving the source unchanged. Account credentials and old C# DLL plugins are not imported. To use a separate data directory:

```powershell
.\Launcher.exe --data-dir D:\HydrogenData
```

## Plugin management

You can install, upgrade, enable, disable, uninstall, and validate and reload plugins in the launcher. Restarting the same distribution does not restore plugins you have uninstalled. Native plugins have process-level privileges, so install plugins only from trusted sources.

[Plugin development guide](docs/PLUGINS.md) · [Theme author guide](docs/THEMES.md)

## Feedback

If you encounter a problem, please open an issue. Include the launcher version, operating system, Java version, steps to reproduce the problem, and logs with personal information and credentials removed. For game launch issues, also include the game version and any external plugins.

## License

This project is licensed under the [MIT License](LICENSE). Refer to the licenses accompanying third-party components for their terms.

## Statistics

![Repobeats](https://repobeats.axiom.co/api/embed/81ab72994aeb53a1b2d0c7f31b64f2b5cbc30e1b.svg "Repobeats analytics image")
[![Star History Chart](https://api.star-history.com/chart?repos=ricecakey06/hydrogen&type=date&legend=top-left)](https://www.star-history.com/?repos=ricecakey06%2Fhydrogen&type=date&legend=top-left)

## Contributors

[![Contributors](https://contrib.rocks/image?repo=ricecakey06/hydrogen)](https://github.com/ricecakey06/hydrogen/graphs/contributors)
