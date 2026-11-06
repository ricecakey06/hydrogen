# Hydrogen 插件开发

JavaScript 插件使用 **schemaVersion 3 / API 2**，不支持旧版 JS 协议。原生插件继续使用 ABI v1，清单支持 schema 2 或 3。外置插件仍以 .hplugin 分发，无需重新编译启动器。

**完整外观主题**使用声明式 CSS、图片和字体，见 [主题作者指南](THEMES.md) 与 [公共变量/部位目录](../THEME-CONTRACT.md)。独立主题无需 `ui.host`，宿主不执行它的 JavaScript 入口。支持从 0.2.7 起。

## 零构建起步

使用发行版附带的 hydrogen-dev.exe 即可创建、校验、测试和打包插件，不需要 Rust、Node.js 或 npm。

~~~powershell
hydrogen-dev.exe plugin init ./instance-tools
hydrogen-dev.exe plugin validate ./instance-tools
hydrogen-dev.exe plugin test ./instance-tools
hydrogen-dev.exe plugin test ./instance-tools --command show-count
hydrogen-dev.exe plugin pack ./instance-tools --out ./instance-tools.hplugin
hydrogen-dev.exe api export --out ./sdk
~~~

plugin test 默认只校验激活及清理；指定 --command 和可选 --params 执行命令。CLI 使用隔离的实例列表，不连接用户账号和游戏目录。真实修复、安装、任务与界面在 Launcher.exe --data-dir 测试目录 --dev-plugin 插件目录 中验证。

模板包含 plugin.json、main.js、launcher.d.ts 和完整 ui 目录。修改 JS / HTML 后在插件详情执行“校验并重载”。增加权限、域名或程序范围需要重新安装确认；重载不会扩权。

## 业务脚本与面板

参考 [实例工具清单](../examples/instance-tools/plugin.json)、[业务脚本](../examples/instance-tools/main.js) 和 [完整类型声明](../plugin-sdk/launcher.d.ts)。

~~~js
export function activate(ctx) {
  ctx.commands.register('repair', async ({ instanceId }) => {
    return ctx.instances.repair(instanceId); // { id: taskId }
  });
}
~~~

命令接收 JSON 参数，可返回 JSON 或 Promise。修复、创建、启动、资源安装、Java 扫描等返回宿主任务句柄。ctx.tasks.get(id) 查询进度，ctx.tasks.cancel(id) 取消；任务权限只授予创建插件及该运行时代次。提交阶段不可取消。

面板运行在隔离 iframe，没有 Tauri 调用权限。复制 [bridge.js](../plugin-sdk/bridge.js) 到 ui/ 并导入：

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

桥接提供结构化错误、上下文/语言/主题通知、进度查询和取消。bridge.resize(height) 可调整视图高度，宿主限制在 120–900 px。命令只在所属插件执行。关闭、导航或停止所属插件会撤销该视图；最多同时挂载 16 个独立会话。面板资源不允许远程脚本、网络连接或表单导航。

## 清单与 UI 扩展

~~~json
{
  "schemaVersion": 3,
  "apiVersion": 2,
  "id": "example.tools",
  "name": "实例工具",
  "version": "1.0.0",
  "runtime": "javascript",
  "entry": "main.js",
  "permissions": ["instances.repair", "tasks.read", "tasks.cancel", "ui.contribute", "ui.panel"],
  "contributes": {
    "commands": [{ "id": "repair", "title": "修复此实例" }],
    "panels": [{ "id": "overview", "title": "实例工具", "entry": "ui/index.html" }],
    "views": [{ "id": "tools", "location": "sidebar.main", "panel": "overview" }],
    "menus": [{ "id": "repair-instance", "location": "instance.toolbar", "command": "repair", "when": "instanceSelected" }]
  }
}
~~~

支持 sidebar.main、sidebar.footer、home.widgets、home.actions、settings.sections、instance.tabs、instance.toolbar、instance.contextMenu、mods.toolbar、mods.contextMenu、resources.actions、profiles.actions、downloads.taskActions、titlebar.actions、status.items、commands.palette。首页卡片直接挂载面板，实例标签页必须引用面板，其他自定义区点击后挂载。实例和模组列表提供右键菜单，也可用 Shift+F10 打开；简单动作由宿主按钮渲染。group、order 和完整贡献 ID 决定稳定顺序。when 只支持 instanceSelected、modsSelected、taskActive。

宿主传递 instanceId、selectedModIds / selectedMods、taskId、taskActive 等页面上下文。ctx.ui.update(id, { title, badge, enabled, visible }) 更新自己的入口；状态按插件摘要和代次清理。

commands.palette 暂保留清单位置标识，主侧栏隐藏此入口；插件命令仍可在插件详情中执行。

## 权限和范围

| 能力 | 权限 | 资源限制 |
| --- | --- | --- |
| 实例列表、编辑、修复、启动 | instances.read / write / repair / launch | 宿主验证实例 ID、占用、目录和配置 |
| 档案、Java | accounts.read / write、java.read / scan | 不返回宿主账号凭据；扫描返回任务 |
| 资源、模组、整合包 | resources.read / install、mods.read / write、modpacks.read / write | 安装和导入使用宿主事务 |
| 设置 | settings.read / write | 字段补丁；不能修改插件启用、游戏目录、账号身份 |
| 私有存储与文件 | storage、files | 仅 plugin-data/插件ID/；拒绝路径越界和链接 |
| 网络 | http | scopes.http 的确切主机名，重定向也须在范围内 |
| 进程 | process | scopes.process 的确切绝对程序路径；任务支持取消 |
| 任务、事件、Hook、日志 | tasks.read / cancel、events、hooks、log | 任务按代次归属；Hook 在读取阶段执行 |
| UI | ui.contribute、ui.panel | 限定清单引用和固定位置 |
| 服务 | services.call | 声明依赖与版本；原生低层方法须经业务适配 |

HTTP 请求返回有限的 UTF-8 文本；大文件使用 http.download，单次最大 1 GiB。文件 API 不提供任意系统目录访问。进程标准输入输出不通过消息桥接传输，任务报告完成或失败。

## 服务及提供者

服务注册必须与 provides 的服务 ID 和全部方法对应：

~~~js
ctx.services.register('example.tools', {
  summarize: async ({ instanceId }) => ({ instanceId })
});
const value = await ctx.services.call({
  provider: 'example.tools', service: 'example.tools',
  version: '^1', method: 'summarize', params: { instanceId: '...' }
});
~~~

dependencies 声明插件版本，request.version 声明服务版本，两者独立检查。调用链拒绝循环及超过 32 层的调用，跨服务继续检查最初 JS 调用者的权限；业务适配使用双方网络与程序授权范围的交集，并拦截回调正在等待当前 JS 的原生提供者。ctx.services.list() 列出方法及开放状态。[官方方法覆盖清单](../plugin-sdk/services.json) 逐项记录直接开放、业务适配或原生限定的方法。

在 contributes.providers 中声明 { id, kind, title, service, version: "1.0.0" }，完整提供者 ID 为 pluginId:providerId：

| kind | JS 服务方法及结果 | 业务入口 |
| --- | --- | --- |
| resourceSource | filters、search、versions、dependencies；downloadPlan 返回 {filename,url,sha1,size}[] | 资源来源选择、版本详情、安装 |
| accountProvider | view 返回账号视图；signIn / session 返回会话；signOut 清理 | 添加档案、选择、登录与启动 |
| loaderProvider | versions 返回 {version,stable?}[]；installPlan 返回 mainClass、libraries、arguments 补丁 | 创建实例中的加载器选择和安装 |
| downloadSource | resolve({url}) 返回 {url} | 下载源设置与实际传输 |

资源结果沿用资源页面 DTO（search 返回 hits / total_hits，versions 返回版本列表）；下载计划最多 128 个 JAR，提供 SHA-1 和大小，域名来自提供者 scopes.http。加载器 libraries 最多 256 项，必须含 downloads.artifact 的 URL、SHA-1 和大小；路径校验、下载和发布由宿主已有安装服务处理。低层内容服务的受支持下载域名约束仍适用。下载源返回 HTTPS 地址且域名必须在 scopes.http 内。

自定义账号提供者只收到 profileId 和 restore 等公共参数，不接收宿主账号目录、设置或其他提供者令牌。view 至少返回 status、playerName、detail、skin；session 使用账号服务 Session 契约。用户自行选择自定义提供者，宿主不会把微软凭据传给它。

## 生命周期、事件与限制

activate(ctx) 只注册处理函数和读取数据。需要提交后执行的工作放入 ctx.onReady(handler)；候选验证失败时旧运行时继续可用。ctx.onDispose(handler) 提供有期限的清理，失败不阻止其他资源回收。

命令、视图和服务按需激活；activation: ["startup"] 显式启用启动激活。事件支持 instance.changed、task.updated、game.exited；实例变化和任务变化根据宿主快照去重后投递，覆盖主界面及插件操作。订阅和 UI 状态随代次清理，繁忙时事件队列有界。订阅者错误记录在插件详情的运行诊断中，最多保留最近 32 条，不影响其他订阅者。

Hook 支持 beforeLaunch 和 beforeInstall，按插件 ID 排序，可返回 null、空对象或 {deny:"原因"}。beforeLaunch 还可返回 {patch:{maxMemoryMib:4096,jvmArguments:"-XX:+UseG1GC"}}，仅影响本次启动，不保存实例配置。补丁复用宿主的物理内存与 JVM 参数校验；不同插件设置同一字段为不同值时返回 HOOK_CONFLICT。beforeInstall 只允许放行或拒绝。Hook 内只允许读取；未知返回字段、错误或超时终止流程。

每个 QuickJS Runtime 上限 8 MiB，总预算 64 MiB；单个 JS 执行片段 100 ms，命令总时长 30 s，宿主请求超时 25 s，激活限时 3 s。等待 I/O 的 Promise 让出调度。每个插件最多 8 个在途宿主请求，消息上限 256 KiB；业务长任务不占用 JS 命令的等待时限。同步原生请求在工作线程执行。原生 ABI 无法强制中断同步系统调用，在途租约结束前禁止替换相关运行时。

## 原生开发

[原生 ABI v1 SDK](../plugin-sdk/native/src/lib.rs) 和 [C 头文件](../plugin-sdk/native/hydrogen_plugin.h) 保持不变。原生清单声明 native.code 与 bin/ 下的平台库。原生代码拥有启动器进程权限；普通 JS 插件无需使用它。

仓库开发者用 scripts/native.ps1 check 检查桌面 Rust，scripts/native.ps1 test 运行测试，npm run check 检查 Svelte，npm run build 构建前端。新增原生发行插件仍需登记 official-plugins/distribution.json；外置 JS 插件直接安装 .hplugin。

包不执行安装脚本，不解析 TS/JSX，不安装 npm 依赖。TypeScript 和 Svelte 作者自行构建为包内 JavaScript / 静态资源后打包。清单最多 64 KiB，包与解压总量各最多 256 MiB，最多 4096 个文件，所有 JS 合计最多 2 MiB；禁止绝对路径、路径穿越、链接和文件名大小写冲突。
