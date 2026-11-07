# Hydrogen Plugin Development

JavaScript plugins use **schemaVersion 3 / API 2**. The previous JavaScript protocol is unsupported. Native plugins use ABI v1 and accept manifest schema 2 or 3. External plugins are distributed as `.hplugin` packages and can be installed without rebuilding the launcher.

**Complete appearance themes** use declarative CSS, images, and fonts. See the [theme authoring guide](THEMES.md) and [public variables and parts](../THEME-CONTRACT.md). Standalone themes do not require `ui.host`, and the host does not execute their JavaScript entry point. The theme contract declares support starting with version 0.2.7.

## Getting started without a build toolchain

Use the `hydrogen-dev.exe` CLI supplied with the release to create, validate, test, and package plugins. Rust, Node.js, and npm are not required.

~~~powershell
hydrogen-dev.exe plugin init ./instance-tools
hydrogen-dev.exe plugin validate ./instance-tools
hydrogen-dev.exe plugin test ./instance-tools
hydrogen-dev.exe plugin test ./instance-tools --command show-count
hydrogen-dev.exe plugin pack ./instance-tools --out ./instance-tools.hplugin
hydrogen-dev.exe api export --out ./sdk
~~~

By default, `plugin test` validates activation and cleanup only. Add `--command` and optionally `--params` to execute a command. The CLI uses an isolated instance list and does not connect to user accounts or game directories. Validate real repairs, installations, tasks, and UI behavior with `Launcher.exe --data-dir ./test-data --dev-plugin ./plugin-dir`.

The template contains `plugin.json`, `main.js`, `launcher.d.ts`, and a complete `ui/` directory. After editing JavaScript or HTML, choose **Verify and reload** in the plugin details. Adding permissions, domains, or process scopes requires reinstalling and confirming the new permissions; reloading cannot expand permissions.

## Business logic and panels

See the [instance tools manifest](../examples/instance-tools/plugin.json), [business script](../examples/instance-tools/main.js), and [complete type declarations](../plugin-sdk/launcher.d.ts).

~~~js
export function activate(ctx) {
  ctx.commands.register('repair', async ({ instanceId }) => {
    return ctx.instances.repair(instanceId); // { id: taskId }
  });
}
~~~

Commands accept JSON parameters and may return JSON or a Promise. Repair, creation, launch, resource installation, and Java scanning return host task handles. Use `ctx.tasks.get(id)` to read progress and `ctx.tasks.cancel(id)` to cancel. Task access belongs to the creating plugin and runtime generation. The commit phase cannot be canceled.

Panels run in isolated iframes without access to Tauri commands. Copy [bridge.js](../plugin-sdk/bridge.js) into `ui/` and import it:

~~~js
import { bridge } from './bridge.js';
await bridge.ready;
const task = await bridge.call('repair', bridge.context);
const subscription = bridge.tasks.watch(task.id, task => {
  document.querySelector('#status').textContent = task.message;
});
await bridge.tasks.cancel(task.id);
subscription.dispose();
~~~

The bridge provides structured errors, context/language/theme notifications, progress queries, and cancellation. `bridge.resize(height)` requests a height between 120 and 900 px. Commands execute only in the panel's owning plugin. Closing or navigating the view, or stopping its plugin, revokes the view session. At most 16 independent sessions may be mounted concurrently. Panel resources cannot load remote scripts, make network connections, or navigate through forms.

## Manifests and UI contributions

~~~json
{
  "schemaVersion": 3,
  "apiVersion": 2,
  "id": "example.tools",
  "name": "Instance tools",
  "version": "1.0.0",
  "runtime": "javascript",
  "entry": "main.js",
  "permissions": ["instances.repair", "tasks.read", "tasks.cancel", "ui.contribute", "ui.panel"],
  "contributes": {
    "commands": [{ "id": "repair", "title": "Repair this instance" }],
    "panels": [{ "id": "overview", "title": "Instance tools", "entry": "ui/index.html" }],
    "views": [{ "id": "tools", "location": "sidebar.main", "panel": "overview" }],
    "menus": [{ "id": "repair-instance", "location": "instance.toolbar", "command": "repair", "when": "instanceSelected" }]
  }
}
~~~

Supported locations are `sidebar.main`, `sidebar.footer`, `home.widgets`, `home.actions`, `settings.sections`, `instance.tabs`, `instance.toolbar`, `instance.contextMenu`, `mods.toolbar`, `mods.contextMenu`, `resources.actions`, `profiles.actions`, `downloads.taskActions`, `titlebar.actions`, `status.items`, and `commands.palette`. Home widgets mount panels directly; instance tabs must reference a panel. Other custom areas mount a panel when opened. Instance and mod lists provide context menus, also accessible with `Shift+F10`; the host renders simple actions as buttons. `group`, `order`, and the complete contribution ID determine stable ordering. `when` supports only `instanceSelected`, `modsSelected`, and `taskActive`.

The host passes page context such as `instanceId`, `selectedModIds` / `selectedMods`, `taskId`, and `taskActive`. Use `ctx.ui.update(id, { title, badge, enabled, visible })` to update your own contribution. State is scoped to the plugin digest and runtime generation and is cleared with them.

`commands.palette` is currently a reserved manifest location and is hidden in the main sidebar. Plugin commands can still be executed from plugin details.

## Permissions and scopes

| Capability | Permissions | Resource boundaries |
| --- | --- | --- |
| List, edit, repair, and launch instances | `instances.read`, `instances.write`, `instances.repair`, `instances.launch` | The host validates instance IDs, busy state, directories, and configuration |
| Profiles and Java | `accounts.read`, `accounts.write`, `java.read`, `java.scan` | Host account credentials are not exposed; scanning returns a task |
| Resources, mods, and modpacks | `resources.read`, `resources.install`, `mods.read`, `mods.write`, `modpacks.read`, `modpacks.write` | Installation and import use host transactions |
| Settings | `settings.read`, `settings.write` | Field patches only; cannot change plugin enablement, game directories, or account identity |
| Private storage and files | `storage`, `files` | Restricted to `plugin-data/<pluginId>/`; path escapes and links are rejected |
| Network | `http` | Exact hostnames in `scopes.http`; redirects must stay within the scope |
| Processes | `process` | Exact absolute executable paths in `scopes.process`; tasks support cancellation |
| Tasks, events, hooks, and logging | `tasks.read`, `tasks.cancel`, `events`, `hooks`, `log` | Tasks belong to a runtime generation; hooks execute in a read-only phase |
| UI | `ui.contribute`, `ui.panel` | Restricted to manifest references and supported locations |
| Services | `services.call` | Dependencies and versions must be declared; native low-level methods require business adapters |

HTTP requests return bounded UTF-8 text. Use `http.download` for large files; each download is limited to 1 GiB. The file API does not provide arbitrary system directory access. Process standard input/output is not transported through the message bridge; the task reports completion or failure.

## Services and providers

Service registration must match the service ID and complete method list declared in `provides`:

~~~js
ctx.services.register('example.tools', {
  summarize: async ({ instanceId }) => ({ instanceId })
});
const value = await ctx.services.call({
  provider: 'example.tools', service: 'example.tools',
  version: '^1', method: 'summarize', params: { instanceId: '...' }
});
~~~

`dependencies` declares plugin version constraints; `request.version` declares the service version constraint. These are checked independently. Call chains reject cycles and depths exceeding 32. Cross-service calls continue to enforce the original JavaScript caller's permissions. Business adapters intersect the caller's and provider's network/process scopes and reject callbacks into a native provider already waiting on the current JavaScript runtime. `ctx.services.list()` lists methods and availability. The [official method coverage catalog](../plugin-sdk/services.json) identifies each method as directly available, adapted, or native-only.

Declare providers in `contributes.providers` as `{ id, kind, title, service, version: "1.0.0" }`. The complete provider ID is `pluginId:providerId`:

| `kind` | JavaScript service methods and results | Business entry points |
| --- | --- | --- |
| `resourceSource` | `filters`, `search`, `versions`, `dependencies`; `downloadPlan` returns `{filename,url,sha1,size}[]` | Resource source selection, version details, installation |
| `accountProvider` | `view` returns an account view; `signIn` / `session` return a session; `signOut` clears state | Add a profile, select an account, sign in, launch |
| `loaderProvider` | `versions` returns `{version,stable?}[]`; `installPlan` returns `mainClass`, `libraries`, and `arguments` patches | Loader selection and installation when creating an instance |
| `downloadSource` | `resolve({url})` returns `{url}` | Download source settings and file transfer |

Resource results use the resource page DTOs: `search` returns `hits` / `total_hits`, and `versions` returns the version list. A download plan may contain at most 128 JARs and must provide SHA-1 and size declarations; its domains come from the provider's `scopes.http`. Loader `libraries` may contain at most 256 entries, each including URL, SHA-1, and size in `downloads.artifact`. Existing host installation services handle path validation, download, and publication. Supported download-domain restrictions also apply to low-level content services. Download source providers must return HTTPS URLs whose domains are within `scopes.http`.

Custom account providers receive only public parameters such as `profileId` and `restore`, not host account directories, settings, or another provider's tokens. `view` must at least return `status`, `playerName`, `detail`, and `skin`; `session` follows the account service `Session` contract. Users explicitly select a custom provider. The host does not forward Microsoft credentials to it.

## Lifecycle, events, and limits

`activate(ctx)` registers handlers and reads data only. Put work that must run after a successful commit in `ctx.onReady(handler)`; failed candidate validation leaves the previous runtime available. `ctx.onDispose(handler)` provides time-bounded cleanup; a cleanup failure does not prevent other resources from being released.

Commands, views, and services activate on demand. `activation: ["startup"]` explicitly enables startup activation. Supported events are `instance.changed`, `task.updated`, and `game.exited`. Instance/task changes are deduplicated against host snapshots and cover both main UI and plugin operations. Subscriptions and UI state are cleared with their runtime generation; event queues are bounded when a plugin is busy. Subscriber errors are recorded in plugin runtime diagnostics, keeping at most the latest 32 entries without affecting other subscribers.

Hooks support `beforeLaunch` and `beforeInstall`, execute in plugin ID order, and may return `null`, an empty object, or `{deny:"reason"}`. `beforeLaunch` may also return `{patch:{maxMemoryMib:4096,jvmArguments:"-XX:+UseG1GC"}}`; the patch affects only that launch and does not save instance configuration. Patches use existing physical-memory and JVM-argument validation. Different plugins setting the same field to different values produce `HOOK_CONFLICT`. `beforeInstall` may only allow or deny the operation. Hooks are read-only. Unknown result fields, errors, or timeouts terminate the operation.

Each QuickJS runtime is limited to 8 MiB, with a total budget of 64 MiB. A single JavaScript execution slice is limited to 100 ms; command execution to 30 s; host requests to 25 s; and activation to 3 s. Promises waiting for I/O yield to the scheduler. Each plugin may have at most 8 in-flight host requests, with a 256 KiB message limit. Long-running business tasks do not consume the JavaScript command's wait budget. Synchronous native requests run on worker threads. The native ABI cannot forcibly interrupt synchronous system calls, and related runtimes cannot be replaced until in-flight leases finish.

## Native development and repository builds

The [native ABI v1 SDK](../plugin-sdk/native/src/lib.rs) and [C header](../plugin-sdk/native/hydrogen_plugin.h) retain their existing ABI. Native manifests declare `native.code` and a platform library under `bin/`. Native code has the launcher's process privileges; ordinary JavaScript plugins do not need native code.

Repository developers use `scripts/native.ps1 check` to check desktop Rust, `scripts/native.ps1 test` to run tests, `npm run check` to check Svelte, and `npm run build` to build the frontend. New native distribution plugins must be registered in `official-plugins/distribution.json`. External JavaScript plugins can be installed directly as `.hplugin` packages.

Packages do not execute installation scripts, parse TS/JSX, or install npm dependencies. TypeScript and Svelte authors must compile to package-local JavaScript and static assets before packaging. A manifest is limited to 64 KiB; both the archive and extracted package to 256 MiB; file count to 4096; and all JavaScript sources combined to 2 MiB. Absolute paths, path traversal, links, and filename case collisions are rejected.
