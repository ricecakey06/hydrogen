[简体中文](README.md) | **English**

<div align="center">

<img src="src-tauri/icons/icon.png" alt="Hydrogen icon" width="96" height="96">

# Hydrogen

A desktop launcher for Minecraft: Java Edition, built with Tauri 2, Rust and Svelte.

A simple interface · Everything is a plugin

[![madewithlove](https://img.shields.io/badge/made_with-%E2%9D%A4-red?style=for-the-badge&labelColor=orange)](https://github.com/ricecakey06/hydrogen)
[![Licence](https://img.shields.io/badge/LICENSE-MIT-green.svg?style=for-the-badge)](LICENSE)
[![Build and test](https://github.com/ricecakey06/hydrogen/actions/workflows/ci.yml/badge.svg)](https://github.com/ricecakey06/hydrogen/actions/workflows/ci.yml)

[Download and usage](#download-and-usage) · [Build from source](#build-from-source) · [Plugin development](#project-layout-and-plugin-development)

</div>

Hydrogen organizes its interface, accounts, game instances and platform features into independent plugins. The host manages dependencies, permissions and lifetimes. The current version is **0.2.7**, targeting Windows x64 desktop distribution.

The project is under development and primarily written with AI assistance. Long-term usability and stability still require ongoing validation.

## Features

- A Svelte interface using system WebView2, with native business logic in Rust. Distribution requires no Node.js, Rust or .NET runtime.
- Profiles for Microsoft accounts and offline players, cached names and skin avatars, and background session refresh.
- Vanilla and Fabric installation, existing `.minecraft` directories, and downloads with validation and repair of game files, libraries and assets.
- Local mod search, import, drag selection, batch enable/disable and deletion, and locally editable Chinese names.
- Launch and download progress, game logs, and crash analysis and report export through an independent plugin.
- Cordis-inspired dependency and lifetime management, JavaScript and native ABI plugins, and local `.hplugin` package management.

## Platforms and requirements

| Platform | Distribution | Requirements |
| --- | --- | --- |
| Windows x64 | Portable `Launcher.exe` | Microsoft Edge WebView2 Runtime 110 or later; compatible Java to launch the game |

The launcher uses system WebView2 and does not bundle a browser engine or automatically install WebView2 or Java. Shared Rust code is tested on Windows, macOS and Linux in CI; macOS/Linux desktop releases are not yet available.

## Download and usage

Download `Launcher.exe` from [Releases](https://github.com/ricecakey06/hydrogen/releases) and run it from a writable directory. Use `SHA256SUMS.txt` on the release page to verify the file. Unreleased builds are available as `Hydrogen-win-x64` artifacts in [Actions](https://github.com/ricecakey06/hydrogen/actions/workflows/ci.yml).

### First launch

1. Check Java, memory and the game directory in settings, or select an existing `.minecraft`.
2. Add a Microsoft account or offline player in Profiles and select the profile to use. Offline profiles require the preinstalled Offline Launch plugin, which is enabled by default.
3. Add a game version, choose vanilla or Fabric components, install it and select the instance to launch.

New installations support official Minecraft releases from 1.18 onward. Fabric and Fabric API versions can be selected explicitly, and failed or cancelled installations can be retried. Downloads and preparation continue after navigating away. Launching existing games validates and fills missing assets and dependencies.

The mods page shows local metadata and cached information. Click or drag to select mods and use the selection bar to enable, disable, delete or clear the selection. Import `.jar` / `.jar.disabled` files and edit local Chinese names. File modifications are restricted while the game or related tasks are active.

After an abnormal game exit, the Crash Analysis plugin can show possible causes, log evidence and suggestions, and export a ZIP containing log snapshots. Common tokens and local paths are redacted; review content written by mods before sharing. Analysis depends on available logs and does not cover every crash.

### Data and accounts

By default, `HydrogenTauriData/` beside the EXE stores settings, credentials, plugins, caches and logs. Game files use the adjacent `.minecraft/`, standard `versions/<name>/` directories, shared `libraries/` and `assets/`. Global settings support automatic, shared or per-version game directories. Deleting a version preserves shared worlds and assets.

Profiles are stored in `profiles.json`. Microsoft credentials are stored separately per profile and encrypted with the current Windows user's DPAPI. Microsoft sign-in uses the system browser; publishers can set the default application Client ID through `HYDROGEN_MICROSOFT_CLIENT_ID`.

Legacy data is previewed and copied through Import legacy data in settings, leaving the source unchanged. Account credentials and old C# DLL plugins are not imported. To use a separate data directory:

```powershell
.\Launcher.exe --data-dir D:\HydrogenData
```

## Build from source

Run all commands from the repository root containing `package.json` and `src-tauri/`.

Install Node.js 24, Rust 1.99.1 (pinned by `rust-toolchain.toml`), MSVC x64, Windows SDK and WebView2 Runtime. `scripts/native.ps1` prefers a local toolchain in the adjacent `.tools/tauri/` directory, otherwise using the installed toolchain. These tools are not distributed with the repository.

### Install dependencies, build and test

```powershell
npm ci
npm run build
npm test
.\scripts\native.ps1 -Action test -Extra @('--features','desktop','--locked')
.\scripts\native.ps1 -Action lint
```

`npm run build` checks Svelte/TypeScript and builds plugin UI assets and the host bootstrap page. The native script builds independent plugins before testing or building the host. `npm test` runs all frontend unit tests. Manual tests requiring a populated game directory or a prepared JDK simulation are skipped by default.

For cross-platform host tests without desktop dependencies:

```sh
npm ci
npm run build
cargo build --manifest-path official-plugins/Cargo.toml --locked
cargo test --manifest-path official-plugins/Cargo.toml --workspace --locked
cargo test --manifest-path official-plugins/Cargo.toml --locked -p hydrogen-plugin-sdk -p hydrogen-plugin-services
cargo test --manifest-path src-tauri/Cargo.toml --locked --no-default-features
```

### Development and Windows release builds

```powershell
npm run desktop
npm run release
```

Development uses a local Vite server. Release output is `src-tauri/target/release/Launcher.exe`, with the host frontend and preinstalled plugins embedded. Plugins are extracted into the data directory at runtime; no separate plugin directory is needed. Release builds do not start Vite.

```powershell
$check = Start-Process -FilePath .\src-tauri\target\release\Launcher.exe -ArgumentList '--check' -WindowStyle Hidden -Wait -PassThru
$check.ExitCode
```

`--check` only checks WebView2 availability, returning `0` on success without opening a window or accessing user data. CI separately starts the UI with isolated data, waits for readiness and checks every bundled plugin entry.

## Project layout and plugin development

| Path | Purpose |
| --- | --- |
| `src/` | Generic frontend bootstrap page |
| `src-tauri/src/` | Tauri entry point, IPC, tasks and business service adapters |
| `src-tauri/src/plugins/` | Package installation, validation, dependencies, runtimes and lifetimes |
| `official-plugins/` | Independent native plugins and Svelte UI sources |
| `official-plugins/distribution.json` | Bundled plugin distribution index |
| `plugin-sdk/` | JavaScript types, native ABI and shared service SDK |
| `examples/instance-tools/` | JavaScript command and isolated panel example |
| `tests/`, `src-tauri/tests/` | Frontend and native integration tests |
| `scripts/` | Native builds and controlled Java simulation tools |

A `.hplugin` package is a ZIP containing `plugin.json`, an entry point and assets. See [launcher.d.ts](plugin-sdk/launcher.d.ts) for the JavaScript API and [instance-tools](examples/instance-tools/) for a complete example.

```powershell
.\scripts\native.ps1 cli plugin init artifacts/my-plugin
.\scripts\native.ps1 cli plugin validate examples/instance-tools --json
.\scripts\native.ps1 cli plugin test examples/instance-tools --json
.\scripts\native.ps1 cli plugin pack examples/instance-tools --out artifacts/instance-tools.hplugin
.\src-tauri\target\release\Launcher.exe --dev-plugin D:\MyPlugin
```

The UI supports installation, upgrades, enable/disable, uninstall and validated reload. Restarting the same distribution does not restore plugins the user uninstalled. JavaScript runs in restricted QuickJS without Node.js, DOM or arbitrary filesystem/process access. Native plugins have process-level privileges; install only trusted plugins. Legacy .NET plugins are incompatible.

## GitHub Actions

The [CI workflow](.github/workflows/ci.yml) runs on pushes, pull requests and manual dispatch. It builds and tests the frontend and Rust code across three platforms, with additional desktop integration tests on Windows. Once all tests pass, it builds the Windows x64 EXE, checks native and UI startup, and uploads a `Hydrogen-win-x64` artifact retained for 14 days.

Pushes and manual runs on `main` also use the [Release Drafter configuration](.github/release-drafter.yml) to generate release notes and publish the tested `Launcher.exe` and `SHA256SUMS.txt`. The first release uses the version in `package.json`; Release Drafter resolves subsequent versions. The release job uses the repository's `GITHUB_TOKEN`.

## Feedback and contributions

Issues and pull requests are welcome. Include the launcher version or commit, OS, Java version, reproduction steps and logs with personal information and credentials removed. For game launch issues, also include the game version and external plugins.

Run frontend builds, unit tests and native tests before submitting changes. Changes to packaging or plugin loading also need a Windows Release build, `--check` and actual UI startup checks.

## License

This project is licensed under the [MIT License](LICENSE). Refer to the licenses accompanying third-party components for their terms.

## Statistics

![Repobeats](https://repobeats.axiom.co/api/embed/81ab72994aeb53a1b2d0c7f31b64f2b5cbc30e1b.svg "Repobeats analytics image")
[![Star History Chart](https://api.star-history.com/chart?repos=ricecakey06/hydrogen&type=date&legend=top-left)](https://www.star-history.com/?repos=ricecakey06%2Fhydrogen&type=date&legend=top-left)

## Contributors

[![Contributors](https://contrib.rocks/image?repo=ricecakey06/hydrogen)](https://github.com/ricecakey06/hydrogen/graphs/contributors)
