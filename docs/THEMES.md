# Hydrogen 主题作者指南

主题以 `.hplugin` 分发，复用现有插件安装、授权、停用、卸载和开发重载。首个支持版本为 **0.2.7**；清单为 schema v3 / JavaScript API v2，主题数据协议单独为 v1。

## 创建和验证

```powershell
hydrogen-dev.exe plugin init ./my-theme --template theme
hydrogen-dev.exe plugin validate ./my-theme
hydrogen-dev.exe plugin test ./my-theme
hydrogen-dev.exe plugin pack ./my-theme --out ./my-theme.hplugin
hydrogen-dev.exe api export --out ./sdk
Launcher.exe --data-dir ./theme-test-data --dev-plugin ./my-theme
```

模板无需 npm、前端编译或 Rust。`validate` 检查清单、变体、CSS AST、资源和预算；独立主题的 `test` 在隔离宿主中执行 100 次准备、预览、撤销和释放，并检查已释放 URL 失效。它不会改动当前桌面、用户账号或游戏目录。真实排版、图片合成和可读性需要在启动器中验证。

安装只把主题加入外观目录。在“启动器设置 → 主题”选择后预览，20 秒内点击“保留”才保存；“撤销”、离开主题标签页或超时恢复已确认的主题。常规设置中的未保存输入和已打开的面板会保留。`Ctrl+Alt+T` 使用独立恢复入口。`Launcher.exe --safe-theme` 本次使用内置主题和不透明原生材质，保留原设置与数据。

## 包结构与清单

`plugin init --template theme` 会生成清单、JavaScript 入口和浅色、深色变体。每个变体的 JSON 路径相对包根；JSON 内的资源相对 JSON，CSS `url()` 相对 CSS 文件。

```json
{
  "schemaVersion": 3,
  "apiVersion": 2,
  "id": "example.my-theme",
  "name": "我的主题",
  "version": "1.0.0",
  "runtime": "javascript",
  "entry": "main.js",
  "permissions": ["theme.contribute"],
  "contributes": {
    "themes": [{
      "id": "paper",
      "title": "纸面",
      "themeVersion": 1,
      "variants": {"light": "themes/light.json", "dark": "themes/dark.json"},
      "preview": "themes/preview.png"
    }]
  }
}
```

`preview` 可省略。完整主题 ID 是 `pluginId:themeId`；`builtin:default` 保留给宿主。至少提供一个变体，缺少当前明暗变体时使用对应内置外观并显示原因，保留用户选择。

独立主题沿用 JS 包格式，`main.js` 写 `export function activate() {}` 即可。仅有 `theme.contribute`、没有服务、业务贡献、frontend 或激活事件的包，发现、选择和候选重载均跳过 QuickJS。混合功能包继续按业务插件的权限和生命周期运行。

## 变体与装饰

```json
{
  "themeVersion": 1,
  "scheme": "light",
  "extends": "builtin.light.v1",
  "styles": ["styles/common.css", "styles/light.css"],
  "panelStyles": ["styles/panels.css"],
  "branding": {"logo": {"source": "assets/logo.png"}},
  "layers": [{"id": "figure-left", "slot": "content.background", "source": "assets/figure.png"}]
}
```

`scheme` 必须与清单键一致，`extends` 只能使用相同 scheme 的固定内置基础。`styles` 按声明顺序覆盖内置语义变量；`panelStyles` 仅进入隔离 HTML 面板。未声明 logo 时显示内置字标，声明后替换整份可见品牌。图层为宿主创建的非交互图片，槽位见 [公共契约](../THEME-CONTRACT.md)。单张合成背景直接写在 CSS 中即可。

## 实例图标

版本选择页先显示整合包/实例的自定义图标；没有可用的自定义图标时，使用当前变体的 `instanceIcons`，最后使用内置图标（原版草方块、Forge 铁砧、NeoForge 与 Fabric 官方 Logo）。可在浅色、深色变体中分别指定，未指定的类型保留内置图标：

```json
"instanceIcons": {
  "vanilla": {"source": "assets/grass.png"},
  "forge": {"source": "assets/anvil.png"},
  "neoforge": {"source": "assets/neoforge.png"},
  "fabric": {"source": "assets/fabric.png"}
}
```

资源路径相对变体 JSON，只接受静态 PNG/JPEG/WebP，单张最多 2 MiB、边长 1600，并计入变体图片预算。它们与主题一起准备、预览、撤销和释放。主题不能替换整合包自身的图标。可用 `[data-h-part="instance-icon"]` 调整图标显示尺寸。

导入 `.mrpack` / CurseForge `.zip` 时，会保存包根目录的 `icon`、`pack`、`logo` 图片（PNG/JPEG/WebP）。实例目录也支持 `.hydrogen/instance-icon.<扩展名>`、`PCL/Logo.png` 及根目录的 `icon`、`pack`、`logo` 图片；损坏、超限或链接文件会被跳过，继续使用下一个可用图标。图标只从对应版本目录读取，无需修改实例配置。

## CSS 写法

```css
@font-face {
  font-family: "Paper Display";
  src: url("../assets/display.woff2") format("woff2");
  font-display: swap;
}
:root {
  --h-text-primary: #24342c;
  --h-surface-card: #fffdf5;
  --h-font-size: 17px;
  --h-sidebar-width: 240px;
}
[data-h-part="page-heading"] { font-family: "Paper Display", serif; }
[data-h-part="content"] { background: url("../assets/scene.png") center / cover no-repeat; }
[data-h-part="card"] { padding: 20px; border-radius: 14px; }
[data-h-layer="figure-left"] { position: absolute; left: 0; bottom: 0; height: 65vh; }
@media (max-width: 900px) { [data-h-layer="figure-left"] { opacity: .25; } }
```

使用 [theme-contract.json](../theme-contract.json) 的语义变量与公共部位。作者 `:root` 会编译成当前视觉租约的主根或面板根，其他选择器限定在根内部。可调整字体、字号、间距、控件尺寸、侧栏宽度、标题栏高度、局部排版和响应式规则。主题不创建、删除或替换业务页面。

宿主用 Lightning CSS AST 和 CSS token 校验，不按正则模拟 CSS。支持 `@media`、`@supports`、受限 `@font-face`、`@keyframes`；字体族与关键帧及引用经过命名空间处理。支持包内背景、mask、border-image 和 image-set；拒绝远程地址、`data:`、`@import`、私有类、应用 ID、全局 html/body、根节点同级逃逸、`!important`、CSS nesting 和未知 at-rule。具体属性/伪类由解析器支持情况决定，失败信息指出源文件。

默认官方 CSS 统一位于 `hydrogen.defaults` cascade layer，主题位于其后的有效作用域中。宿主最终保护规则保持恢复入口、装饰层非交互、窗口操作、forced-colors 和 reduced-motion 可用。主题不得遮住功能、错误提示或焦点。

资源只接受包内相对路径。规范化后的 `..` 可用，最终不能越过包根；链接、junction、编码路径和绝对/网络路径拒绝。准备时保存字节快照并校验包摘要；旧 URL 不会读到磁盘上后来替换的文件。

## 资源预算

| 项目 | 限制 |
| --- | --- |
| 每包主题 | 16 个；目录总计最多 256 个候选 |
| 每变体样式 | 主样式与面板样式合计 8 个、CSS 合计 512 KiB |
| 单个变体 JSON | 64 KiB |
| 图层 | 12 个；图层 ID 唯一 |
| 图片 | 静态 PNG/JPEG/WebP，最多 32 张，每张 8 MiB，边长 4096、像素 800 万 |
| logo / 预览 / 实例图标 | 2 MiB、边长 1600 |
| 字体 | WOFF2，最多 2 个，每个 4 MiB，声明展开尺寸最多 32 MiB |
| 每包主题源码及资源 | 32 MiB |
| 单变体图片解码 | 64 MiB；切换解码峰值 128 MiB |

PNG/JPEG/WebP 会检查扩展名、容器、尺寸并完整解码，动画 PNG/WebP 拒绝。CLI 对 WOFF2 检查头、表数量、长度和附加数据边界；WebView 准备阶段再实际加载字体，完整字体解码失败不会应用主题。不能把 CLI 的头校验当成独立完整字体解码器。

## 生命周期、面板与状态

同时只有一个活动皮肤。主文档先加载样式、解码图片和字体，再一起提交 CSS、logo、图层和根令牌；不刷新 WebView、不重建路由或表单。新请求取消旧准备，迟到结果不能重新插入。滚动恢复可见内容锚点，字体和尺寸变化可引起重排。回退 revision 单调增长。

主题资源租约独立于 QuickJS generation。隔离面板的清单 `appearance` 默认为 `inherit`，也可选 `independent`。继承面板在 bridge 收到实际 scheme、ID、mode、revision、highContrast 后，通过自己的会话协议加载 tokens 与 panelStyles；主外壳场景、logo 不进入面板。宿主绑定该面板允许的主题租约，保留旧样式到新组确认；5 秒未同步会关闭失同步面板、提示并请求回退，bridge 自身也有取消与超时。CSP 不增加任意内联样式许可。

`bridge.theme` 保留明暗语义，新增 `bridge.appearance`、`bridge.onTheme(listener)`。面板 CSS 可通过 `var(--h-text-primary)` 等公共变量继承主题配色，并用 `data-h-part` 声明公共部位。第三方面板的固定配色不会自动获得完整主题效果。

`ctx.ui.theme()` 返回实际外观摘要（theme/id/mode/revision/highContrast/locale）；`theme.changed` 只通知已激活并有 `events` 及 `ui.contribute` 或 `theme.contribute` 权限的订阅者，不公开资源路径或租约，也不会为了事件激活全部插件。

纯主题切换不等待游戏和下载。停用/卸载当前主题先回到内置外观；后续持久化或删除失败会恢复先前确认状态。开发修改后使用已有“校验并重载”，候选不合法时保留旧包与旧快照。

## 验证主题

检查首页、设置、实例、资源、账号、下载、插件、崩溃、菜单、弹窗与错误状态，包含最小窗口、折叠侧栏、长文本、中英文和不同 DPI。在启动器里确认图片、透明叠加和实际字体渲染的对比度。
