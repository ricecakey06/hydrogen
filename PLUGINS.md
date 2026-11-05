# Hydrogen 插件开发指南

本指南面向为 Hydrogen 编写插件的 agent。路径和命令以包含 `src-tauri/`、`plugin-sdk/` 和 `official-plugins/` 的工程根目录为基准。清单和 API 以当前 checkout 的实现为准，分别见 [清单校验](src-tauri/src/plugins/manifest.rs)、[JavaScript SDK](plugin-sdk/launcher.d.ts)、[Rust SDK](plugin-sdk/native/src/lib.rs) 和 [C ABI](plugin-sdk/native/hydrogen_plugin.h)。

## 选择扩展入口

| 需求 | 使用的入口 |
| --- | --- |
| 读取实例列表，提供一个可点击的操作 | QuickJS 后端，声明 `contributes.commands`，在 `activate(ctx)` 注册处理函数。 |
| 提供表单、按钮、结果列表等交互界面 | 声明 `contributes.panels`，用包内 HTML、CSS 和 JavaScript 构建隔离面板。面板可通过消息通道运行本插件的命令。 |
| 调用或提供启动器服务，或者需要本地文件、网络、进程等系统能力 | 原生 ABI v1 后端。Rust 用 `hydrogen-plugin-sdk`，其他语言可按 C ABI 导出。 |
| 修改主侧栏、首页、设置页或窗口外壳 | 在 `ui.shell` 等受信任的宿主前端中显式接入组件，并随其构建。外置插件可使用的 UI 扩展点是详情页的命令和面板。 |

## 清单与目录

- 包根目录放 `plugin.json`。必填字段为 `schemaVersion: 2`、`apiVersion: 1`、`id`、`name`、`version`、`runtime` 和 `entry`。`description` 可选。清单拒绝未知字段，插件版本使用三段数字，例如 `1.0.0`。
- 插件、命令、面板和服务 ID 以小写 ASCII 字母开头，可含小写字母、数字、`.` 和 `-`，最长 100 字节。禁止连续的点或以点结尾。同一个插件内，命令和面板 ID 也不能重名。外置插件应选用自己的命名空间，例如 `acme.instance-tools`。
- `runtime: "javascript"` 的入口是包内 `.js` ES 模块。`runtime: "native"` 需要 `native.code`，入口在 `bin/` 下，后缀随目标平台使用 `.dll`、`.dylib` 或 `.so`。当前桌面发行版使用 Windows x64 DLL。
- 清单中的文件路径用 `/` 分隔，指向包内文件。打包目录只放运行所需的产物和资源，源工程、`node_modules/`、`target/` 和输出包放在目录外。
- `dependencies` 使用提供者的**插件 ID**。`"java.runtime"` 表示任意版本，`{"id":"java.runtime","version":"^1"}` 约束插件版本。必需依赖必须已安装、已启用、版本匹配，且依赖图无环。`optional: true` 会跳过必需依赖的排序与激活，实际调用前仍需判断提供者是否可用。安装包不会自动获取依赖。
- `activation` 是当前清单接受的字段。加载器按命令和服务调用等入口激活插件，当前实现没有根据 `activation` 字符串订阅启动事件或其他事件。

| `permissions` 值 | 作用 |
| --- | --- |
| `instances.read` | 允许 QuickJS 调用 `ctx.instances.list()`。 |
| `ui.contribute` | 允许 QuickJS 调用 `ctx.ui.notify({ message })`。命令注册本身按 `contributes.commands` 校验。 |
| `ui.panel` | 允许打开 `contributes.panels` 声明的隔离面板。 |
| `native.code` | 允许加载原生库。原生库拥有启动器进程的系统权限。 |
| `ui.host` | 允许宿主加载受信任的 `frontend` 资源。实际挂载由外壳决定。 |

## JavaScript 命令与 UI 控件

以下四个文件组成一个可打包的示例。`main.js` 运行在 QuickJS，`ui/main.js` 运行在面板的浏览器环境。

```text
instance-tools/
  plugin.json
  main.js
  ui/
    index.html
    main.js
```

**`plugin.json`**

```json
{
  "schemaVersion": 2,
  "apiVersion": 1,
  "id": "acme.instance-tools",
  "name": "实例工具",
  "version": "1.0.0",
  "runtime": "javascript",
  "entry": "main.js",
  "permissions": ["instances.read", "ui.contribute", "ui.panel"],
  "contributes": {
    "commands": [{ "id": "show-count", "title": "查看实例数量" }],
    "panels": [{ "id": "overview", "title": "实例概览", "entry": "ui/index.html" }]
  }
}
```

**`main.js`**

```js
export function activate(ctx) {
  ctx.commands.register('show-count', async () => {
    const instances = await ctx.instances.list();
    const count = instances.length;
    await ctx.ui.notify({ message: `共有 ${count} 个实例` });
    return { count };
  });
}
```

`activate(ctx)` 可异步。每个命令 ID 必须已在清单声明，且每次激活只注册一次。命令处理函数不接收参数，可以返回普通 JSON 值或解析为该值的 Promise。`ctx.instances.list()` 返回 `{ id, name, gameVersion, isInstalled }` 列表。`ctx.ui.notify` 的消息进入本次命令的 `RunReport.notifications`，插件详情页运行命令时会显示它。命令返回值位于 `RunReport.result`。

QuickJS 的宿主能力以 `launcher.d.ts` 为准。它没有浏览器 DOM、Node.js、`fetch`、文件系统或原生服务调用接口。运行时只解析包内以 `./` 开头的 `.js` 导入，TypeScript 或第三方库需在包外构建成兼容的 JavaScript。宿主请求最多同时等待 8 个，命令返回的 JSON 上限为 256 KiB。请等待宿主调用完成后再返回，避免阻塞执行或跨重载依赖内存状态。

### 注册命令按钮与隔离面板

在「启动器设置 → 插件 → 插件详情」，`contributes.commands` 的每项会渲染为「插件命令」下的按钮，按钮使用声明的 `title`，点击后调用同 ID 的处理函数。`contributes.panels` 的每项会渲染为「自定义面板」下的按钮，点击后打开 `entry` 指定的页面。面板内的控件直接写成 HTML，或者使用构建后的前端组件，事件由面板脚本绑定。当前清单没有主侧栏、设置项、工具栏或通用控件注册字段。见 [插件管理页](official-plugins/plugins.manager/frontend/Plugins.svelte)。

**`ui/index.html`**

```html
<!doctype html>
<html lang="zh-CN">
  <head>
    <meta charset="utf-8">
    <title>实例概览</title>
  </head>
  <body>
    <button id="count" type="button" disabled>读取实例数量</button>
    <p id="result" role="status">正在连接启动器…</p>
    <script src="main.js"></script>
  </body>
</html>
```

**`ui/main.js`**

```js
const button = document.querySelector('#count');
const status = document.querySelector('#result');
let nonce;
let port;
let requestId = 0;

window.addEventListener('message', (event) => {
  if (event.source !== parent || port) return;
  const data = event.data;
  if (data?.type === 'hydrogen:hello' && typeof data.nonce === 'string') {
    nonce = data.nonce;
    parent.postMessage({ type: 'hydrogen:ready', nonce }, '*');
  } else if (data?.type === 'hydrogen:port' && nonce && data.nonce === nonce && event.ports.length === 1) {
    port = event.ports[0];
    port.onmessage = ({ data: reply }) => {
      if (reply.id !== requestId) return;
      button.disabled = false;
      status.textContent = reply.ok ? `共有 ${reply.result.result.count} 个实例` : reply.error;
    };
    button.disabled = false;
    status.textContent = '已连接';
  }
});

button.addEventListener('click', () => {
  if (!port) return;
  button.disabled = true;
  status.textContent = '正在读取…';
  port.postMessage({ id: ++requestId, command: 'show-count' });
});
```

- 宿主发送 `hydrogen:hello`，面板向父页面回传带相同 `nonce` 的 `hydrogen:ready`，宿主随后通过 `hydrogen:port` 转移一个 `MessagePort`。命令请求形如 `{ id, command }`，`id` 使用安全整数。成功响应的 `result` 是完整的 `RunReport`，失败响应是 `{ id, ok: false, error }`。命令只可来自本插件的清单，桥接协议不传业务参数，也不提供任意 Tauri IPC 或服务调用。
- iframe 使用 `sandbox="allow-scripts"`。HTML 引用包内 `ui/` 下的外部脚本、样式和图片。资源类型为 HTML、JS、CSS、PNG、SVG、WOFF2，每个文件最多 1 MiB。CSP 禁止内联脚本、网络请求和表单提交，面板也不能访问父页面 DOM 或使用 `localStorage`。
- 面板会话一次只保留一个。离开面板、面板导航或重载插件会撤销旧会话。重新打开时重新握手。更多示例见 [instance-tools](examples/instance-tools)，协议和资源约束见 [PluginPanel.svelte](official-plugins/ui.shell/frontend/components/PluginPanel.svelte) 和 [panel.rs](src-tauri/src/plugins/panel.rs)。

### 接入主窗口页面

维护宿主界面时，可参考 `ui.launcher/frontend/pages/` 的 Svelte 组件。页面由 [`ui.shell/frontend/App.svelte`](official-plugins/ui.shell/frontend/App.svelte) 显式导入、接入 `Page` 和导航，并在对应的页面分支渲染。复用 [`ui.shell/frontend/lib/api.ts`](official-plugins/ui.shell/frontend/lib/api.ts) 的 `invoke`、`action` 和类型，分别在 [`desktop.rs`](src-tauri/src/desktop.rs) 核对可用的宿主命令和 `LauncherAction`。

受信任前端的清单声明 `ui.host` 和 `frontend: { "entry": "assets/entry.js", "styles": ["assets/main.css"] }`。[`src/main.ts`](src/main.ts) 从 `ui.shell` 的依赖图取得样式和入口信息，加载样式后只导入 `ui.shell` 的入口。其余组件由外壳的模块依赖加载。前端启动过程也会检查依赖图中标记为 `optional` 的依赖。安装一个声明了 `frontend` 的外置插件，不会把它的控件自动插入主窗口。现有构建由 [`vite.plugins.config.ts`](vite.plugins.config.ts) 和 [`scripts/build-plugin-ui.mjs`](scripts/build-plugin-ui.mjs) 将产物分配给 `ui.shell`、`ui.launcher` 和 `plugins.manager`。宿主资源走 `hydrogen-ui` 协议，隔离面板走 `hydrogen-plugin` 协议。两者的资源白名单分别见 [frontend.rs](src-tauri/src/plugins/frontend.rs) 和 [panel.rs](src-tauri/src/plugins/panel.rs)。

## 原生插件与跨插件服务

1. 从 [example.counter](official-plugins/example.counter) 起步。把 Rust 库设为 `crate-type = ["cdylib"]`，将 `hydrogen-plugin-sdk` 的路径指向本 checkout 的 `plugin-sdk/native`，用 `hydrogen_plugin_sdk::export_plugin!(State, dispatch)` 导出。`State` 实现 `Default`，`dispatch` 接收 `&mut State` 和 `&mut serde_json::Value`，返回 SDK 的 `Result`。
2. `dispatch` 读取 `{ "method": "...", "params": ... }`。必须实现 `plugin.describe`，返回 `{ "id": "与清单完全相同的 ID", "abiVersion": 1 }`。业务成功返回 JSON 值，失败返回 `(code, message)`，由 SDK 包成 ABI 响应。使用其他语言时，按 [C 头文件](plugin-sdk/native/hydrogen_plugin.h) 导出 `hydrogen_plugin_v1`，同步调用响应回调一次。
3. 默认在工作线程串行处理同一实例的调用。确实依赖 OS UI 线程的实现可声明 `executionThread: "main"`，并在桌面宿主中验证。`destroy` 返回前须释放句柄、取消或收尾所有自有工作。请求缓冲区和服务回调指针只在本次调用期间有效，服务回调还绑定当前线程。基础消息上限为 256 KiB，双方支持 `hydrogen_plugin_large_services_v1` 时可用 64 MiB 的原生传输。SDK 宏已经导出该扩展。
4. 原生命令在 `contributes.commands` 声明，`dispatch` 按命令 ID 分支即可，收到的 `params` 为 `null`。发布服务则在 `provides` 声明服务 ID、版本和方法，如 `[{ "id": "acme.counter", "version": "1.0.0", "methods": ["increment"] }]`。服务版本和插件版本分别校验，当前宿主调用 `^1` 的服务契约。服务的方法名支持 ASCII 字母、数字和 `.`，`plugin.*` 保留给宿主。

原生消费者将提供者写入 `dependencies`。在**有路由上下文的服务处理函数**中调用 `hydrogen_plugin_sdk::call_service(provider, service, method, params)`。例如声明 `java.runtime` 为依赖后，可用 `parse` 解析 `java -XshowSettings:properties -version` 的属性输出。下面的示例返回 `major: 21` 和 `architecture: "x64"`。`provider` 和 `service` 是两个字段，官方服务通常用相同的 ID。

```rust
let parsed = hydrogen_plugin_sdk::call_service(
    "java.runtime",
    "java.runtime",
    "parse",
    json!({"path": "java.exe", "output": "java.version = 21.0.2\nos.arch = amd64"}),
).map_err(|error| (error.code, error.message))?;
```

宿主会核对直接依赖、提供者状态和版本，以及目标服务与方法是否声明，并阻止循环调用。详情页和隔离面板触发的原生命令都走普通命令通道，`plugin.describe` 也走无路由的调用路径。在这些入口调用其他插件的服务会返回 `SERVICE_UNAVAILABLE`。测试服务用 [`service_plugins.rs`](src-tauri/tests/service_plugins.rs) 的 `PluginHost::call_service` 路径，最小消费者见 [example.services](official-plugins/example.services)。不要在后台线程继续使用当前调用的服务回调。

### 官方服务索引

下表用于定位能力。实际方法名和服务版本查各目录的 `plugin.json` → `provides`，参数、返回值、句柄与错误处理查 `src/lib.rs` 的分发逻辑及现有调用点。声明可选依赖时，可参考 [`hydrogen-plugin-services`](plugin-sdk/services/src/lib.rs) 的 `PluginHost::available`；网络、存储和任务可复用同 crate 的 `http`、`net`、`storage` 和 `jobs`。例如使用 `jobs::service_task` 前，先确认目标服务遵循该 helper 的 `handle`、`poll` 和 `cancel` 协议。

| 能力 | 提供者 |
| --- | --- |
| 启动器配置、实际生效的 Java / 内存 / JVM 参数、语言包 | [`storage`](official-plugins/storage/plugin.json)、[`i18n`](official-plugins/i18n/plugin.json) |
| 实例扫描、实例记录与目录、旧数据迁移 | [`minecraft.instances`](official-plugins/minecraft.instances/plugin.json) |
| Minecraft 版本目录、元数据与平台规则，以及本机 Java 扫描和验证 | [`minecraft.content`](official-plugins/minecraft.content/plugin.json)、[`java.runtime`](official-plugins/java.runtime/plugin.json) |
| 实例安装与修复、客户端和资源准备、加载器配置 | [`minecraft.install`](official-plugins/minecraft.install/plugin.json)、[`minecraft.preinstall`](official-plugins/minecraft.preinstall/plugin.json)、[`minecraft.fabric`](official-plugins/minecraft.fabric/plugin.json)、[`minecraft.forge`](official-plugins/minecraft.forge/plugin.json) |
| HTTP 请求、下载源、文件传输和哈希校验 | [`network`](official-plugins/network/plugin.json)、[`minecraft.download`](official-plugins/minecraft.download/plugin.json)、[`minecraft.files`](official-plugins/minecraft.files/plugin.json) |
| 本地模组管理、Modrinth 和 CurseForge 资源搜索与下载、整合包导入导出 | [`minecraft.mods`](official-plugins/minecraft.mods/plugin.json)、[`minecraft.resources`](official-plugins/minecraft.resources/plugin.json)、[`minecraft.modpacks`](official-plugins/minecraft.modpacks/plugin.json) |
| 档案、微软和离线会话、Windows 凭据加解密 | [`account.manager`](official-plugins/account.manager/plugin.json)、[`account.microsoft`](official-plugins/account.microsoft/plugin.json)、[`account.offline`](official-plugins/account.offline/plugin.json)、[`credentials`](official-plugins/credentials/plugin.json) |
| 游戏启动、进程监控、日志与崩溃报告 | [`minecraft.launch`](official-plugins/minecraft.launch/plugin.json)、[`process`](official-plugins/process/plugin.json)、[`diagnostics`](official-plugins/diagnostics/plugin.json)、[`minecraft.crash`](official-plugins/minecraft.crash/plugin.json) |
| Windows 文件选择器、浏览器和文件夹、磁盘空间、窗口材质和几何 | [`platform.windows.environment`](official-plugins/platform.windows.environment/plugin.json)、[`platform.windows.window`](official-plugins/platform.windows.window/plugin.json) |

## 构建、调试和打包

下列 PowerShell 命令从工程根目录运行。先用 `npm ci` 安装前端构建依赖。仓库的 [Windows 构建脚本](scripts/native.ps1) 会先构建预装插件及前端资源，并在存在配套工具时加载本地 Rust 和 MSVC 环境。

```powershell
# 将上面的四个文件存入 ./instance-tools 后，构建 CLI 并校验。
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/native.ps1 cli plugin validate ./instance-tools

# 运行并打包示例。输出目录须已存在，目标文件须是新文件。
$dev = '.\src-tauri\target\debug\hydrogen-dev.exe'
& $dev plugin test ./instance-tools
& $dev plugin pack ./instance-tools --out ./instance-tools.hplugin
```

`plugin init <目录>` 可在新目录或空目录生成无面板的 JavaScript 模板。`plugin test` 用隔离的样本实例激活插件、运行清单命令并清理运行时。上面的示例应返回 `result.count = 2`。面板握手和有路由的服务调用需另外验证。`api export --out <目录>` 可导出 `launcher.d.ts`。

原生插件先编译，再把 DLL 复制到单独的打包目录。以已有示例为例，以下命令假设 Rust 和 MSVC 已在当前终端可用。

```powershell
cargo build --manifest-path official-plugins/Cargo.toml -p hydrogen-example-counter --locked
New-Item -ItemType Directory -Path ./counter-package/bin -Force | Out-Null
Copy-Item official-plugins/example.counter/plugin.json ./counter-package/plugin.json
Copy-Item official-plugins/target/debug/hydrogen_example_counter.dll ./counter-package/bin/plugin.dll
& $dev plugin validate ./counter-package
& $dev plugin test ./counter-package
& $dev plugin pack ./counter-package --out ./counter.hplugin
```

- **开发目录调试。** 用 `Launcher.exe --data-dir <独立测试目录> --dev-plugin <插件目录>` 加载本地插件目录，可多次传 `--dev-plugin`。同 ID 的开发目录在该会话优先于已安装包。更改文件后，在插件详情点「校验并重载」或重启。若新增权限，开发目录可重启后重新加载，已安装包应重新打包并安装。
- **安装包调试。** 在「启动器设置 → 插件 → 安装插件」选择 `.hplugin` 并核对权限与依赖。`.hplugin` 是以 `plugin.json` 为根级条目的 ZIP 包，`plugin pack` 会处理格式和内容校验。安装通过 `<数据目录>/plugins/<id>/` 和摘要记录管理，直接拷入已安装目录会缺少对应的校验记录。发布升级时保留插件 ID 并提高版本号。
- **打包限制。** 清单最多 64 KiB，压缩包及解压总量各最多 256 MiB，最多 4096 个文件，所有 `.js` 文件合计最多 2 MiB。包不接受绝对路径、`..`、符号链接、目录联接或大小写冲突的文件名。
- **随启动器预装。** 仅在明确需要加入官方发行时，把原生 crate 加入 [`official-plugins/Cargo.toml`](official-plugins/Cargo.toml) workspace，并在 [`distribution.json`](official-plugins/distribution.json) 登记插件 ID 和库名。[`src-tauri/build.rs`](src-tauri/build.rs) 用相同构建 profile 的原生库、清单和 `assets/` 生成预装包。普通外置插件直接交付 `.hplugin`。

交付前，用独立数据目录安装产物，确认插件详情显示正确的按钮和面板，点击命令和面板控件都得到预期结果。改动服务时额外检查依赖缺失、停用或版本不匹配时的行为，并验证停用或重载后资源已释放。可对照 [脚本与面板测试](src-tauri/tests/plugins.rs)、[原生插件测试](src-tauri/tests/native_plugins.rs) 和 [服务路由测试](src-tauri/tests/service_plugins.rs)。
