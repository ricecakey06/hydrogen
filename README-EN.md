[简体中文](README.md) | **English**

<div align="center">

<img src="src/Launcher.UI/Assets/hydrogen-icon.png" alt="Hydrogen icon" width="96" height="96">

# Hydrogen

A desktop launcher for Minecraft: Java Edition, built with .NET 10 and Avalonia.

A simple interface · Everything is a plugin

[![madewithlove](https://img.shields.io/badge/made_with-%E2%9D%A4-red?style=for-the-badge&labelColor=orange
)](https://github.com/ricecakey06/hydrogen)
[![Licence](https://img.shields.io/badge/LICENSE-MIT-green.svg?style=for-the-badge)](https://github.com/ricecakey06/hydrogen/blob/main/LICENSE)

[Usage](#usage) · [Build from source](#build-from-source) · [Plugin development](docs/plugins.md)

</div>

Hydrogen organizes its interface, accounts, game instances, and platform features as plugins. The host manages their dependencies and lifetimes.

The project is under development.

This project is primarily written by AI, so its long-term usability and stability are not guaranteed.

## Features

- Built on Avalonia, offering a degree of cross-platform compatibility.
- Inspired by Cordis, every launcher feature is a plugin. The built-in plugin manager can install additional plugins, making the launcher highly extensible.
- More to come, perhaps?

## Platforms and requirements

| Platform | Current distribution | Notes |
| --- | --- | --- |
| Windows x64 | Self-contained single-file `Launcher.exe` | No separate .NET runtime installation required; the game still needs a compatible Java runtime |

## Usage

### First launch

1. Check the Java path, memory allocation, and game directory in launcher settings.
2. Sign in with a Microsoft account. Microsoft sign-in requires a valid application Client ID. You can also enable offline mode in plugin management, restart, and set a player name.
3. Add a game version and create an instance. The launcher automatically downloads and verifies the required files. Once preparation is complete, select the instance to launch.

On Windows, settings, instance records, and plugins default to `LauncherData/` beside the EXE. Game files and instance game directories are stored in the adjacent `.minecraft/` directory. After switching the data directory, use the actual paths shown in settings; existing data remains at its previous location.

## Build from source

Run all commands from the repository root, which contains `Launcher.slnx`.

### Restore, build, and test

```sh
dotnet restore Launcher.slnx --locked-mode
dotnet build Launcher.slnx --configuration Release --no-restore
dotnet test Launcher.slnx --configuration Release --no-build --no-restore
```

### Run and check startup

```sh
dotnet run --project src/Launcher.Bootstrap --configuration Release --no-build --no-restore
dotnet run --project src/Launcher.Bootstrap --configuration Release --no-build --no-restore -- --check
```

`--check` initializes the UI and plugins, then exits with code `0` on success. On Linux without a graphical session, prefix the startup check command with `xvfb-run -a`.

### Publish a Windows x64 executable

```sh
dotnet publish src/Launcher.Bootstrap --configuration Release -p:PublishProfile=win-x64 -p:RestoreLockedMode=true
```

The output is `artifacts/publish/win-x64/Launcher.exe`. Official plugins are embedded in the EXE and extracted at runtime; no separate plugin directory needs to be distributed.

## Project layout and plugin development

| Path | Purpose |
| --- | --- |
| `src/Launcher.Bootstrap/` | Entry point, official plugin catalog, and publishing configuration |
| `src/Launcher.Runtime/` | Plugin dependency resolution, scopes, and resource lifetimes |
| `src/Launcher.Contracts/` | Shared business and service contracts |
| `src/Launcher.Plugins/` | Plugin package installation, loading, upgrades, and management |
| `src/Launcher.Features/` | Accounts, instances, Java, storage, and game launching |
| `src/Launcher.UI/` | Avalonia interface, themes, and icon assets |
| `src/Launcher.Platform.Windows/` | Windows integration, credentials, and window effects |
| `tests/` | Runtime, plugin management, and game feature tests |

See the [plugin development guide](docs/plugins.md) (Chinese) for package formats, entry point examples, and activation rules. Plugins run inside the launcher process; install plugins only from sources you trust.

## Feedback and contributions

Issues and pull requests are welcome. Include the launcher version or commit, OS, Java version, reproduction steps, and logs with personal information and credentials removed. For game launch issues, also include the game version and external plugins in use.

Run the build and test commands before submitting changes. For publishing or plugin loading changes, also run `--check` and the Windows package checks.

## License

This project is licensed under the [MIT License](LICENSE). Refer to the licenses accompanying third-party components for their terms.

## Statistic
![Alt](https://repobeats.axiom.co/api/embed/81ab72994aeb53a1b2d0c7f31b64f2b5cbc30e1b.svg "Repobeats analytics image")
[![Star History Chart](https://api.star-history.com/chart?repos=ricecakey06/hydrogen&type=date&legend=top-left)](https://www.star-history.com/?repos=ricecakey06%2Fhydrogen&type=date&legend=top-left)
## Contributors
[![](https://contrib.rocks/image?repo=ricecakey06/hydrogen)](https://github.com/ricecakey06/hydrogen/graphs/contributors)