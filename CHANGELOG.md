# 更新日志

## 0.3.0 — 2026-11-07

本次发布汇总 2026 年 11 月 4 日至 7 日 Tauri 主线的功能更新与修复。下面先列出 v0.2.12 发布之后的改动，再回顾近期已经交付的主要功能；每项均附对应提交链接。

### 本次新增

- 接入 MCIM 国内镜像，支持 Modrinth / CurseForge 的资源查询及文件下载。设置中默认启用；关闭后使用源站，镜像失败时自动回源。[15cba5f](https://github.com/ricecakey06/hydrogen/commit/15cba5fcc947526a6a03ac5dc540f3d1b7273861)
- 在版本选择和版本设置中显示实例图标，优先使用整合包自己的图片；默认图标使用 Minecraft Wiki 草方块、铁砧以及 NeoForge / Fabric 官方 Logo。主题可以替换默认图标，不能覆盖整合包自带图标。[15cba5f](https://github.com/ricecakey06/hydrogen/commit/15cba5fcc947526a6a03ac5dc540f3d1b7273861)
- 关于页增加 MCIM 鸣谢与站点图标，补充图标来源和第三方许可证说明。[15cba5f](https://github.com/ricecakey06/hydrogen/commit/15cba5fcc947526a6a03ac5dc540f3d1b7273861)
- 将插件开发和主题作者指南完整改为英文，明确 API、权限、资源预算、生命周期、CLI 命令和代码示例，方便开发者及 agent 使用。[9cdbde4](https://github.com/ricecakey06/hydrogen/commit/9cdbde4dbcdcb88ea091f1dcf18b347f2649d740)

### 下载与安装优化

- 整合包最多同时处理 16 个文件，总下载连接数限制为 32；中小模组采用单连接，较大文件按大小分配最多 4 个分段，减少连接争抢。[8238b15](https://github.com/ricecakey06/hydrogen/commit/8238b152e06bdb5875a4ad6aa1557b8fea16d9a2)
- 批量传递已经到达的网络分片，降低插件消息与轮询开销。这项通用传输优化也覆盖原版客户端、依赖库和游戏资源。[8238b15](https://github.com/ricecakey06/hydrogen/commit/8238b152e06bdb5875a4ad6aa1557b8fea16d9a2)
- 使用连接超时和空闲超时，移除整文件固定倒计时，避免仍在正常传输的文件被中断后重新下载。镜像故障或限流后快速回源并短暂避开该镜像；统一处理 CurseForge 的新旧 CDN 地址。[8238b15](https://github.com/ricecakey06/hydrogen/commit/8238b152e06bdb5875a4ad6aa1557b8fea16d9a2)
- 解包时按协作任务预算分批处理小文件，减少逐文件让出执行造成的等待，同时继续检查取消状态。[8238b15](https://github.com/ricecakey06/hydrogen/commit/8238b152e06bdb5875a4ad6aa1557b8fea16d9a2)
- 本机实测：`Construct Technological Innovation 0.46` 的 267 个下载文件约 633 MB，从读取清单到下载、校验和解包共约 138 秒；前次测试在 600 秒时仍未完成。同一组 12 个文件约 18.6 MB，镜像下载与校验从约 90 秒降到约 5.7 秒，源站从约 144 秒降到约 6.4 秒。整合包数据不包含 Minecraft / Forge 的首次下载，实际耗时随网络变化。[8238b15](https://github.com/ricecakey06/hydrogen/commit/8238b152e06bdb5875a4ad6aa1557b8fea16d9a2)

### 本次修复

- 修复模组下载页在输入法组合输入或自动填充后，点击搜索及按回车不更新结果的问题；提交搜索时读取当前可见输入值，并验证请求先后顺序。[84c10b1](https://github.com/ricecakey06/hydrogen/commit/84c10b1f60803011d4c9992e3b3895ce549e4b4e)
- 修复大型 CurseForge 整合包长时间停留在“读取清单”的问题：支持 gzip 元数据响应，使用有上限的并发请求，并显示批次进度；为元数据和备用来源请求设置等待上限。[9d08ab7](https://github.com/ricecakey06/hydrogen/commit/9d08ab78b4a24e6cbf53db4a963582184665f998)
- 完善 CurseForge 受限下载文件的处理，按哈希和大小匹配备用资源；无法自动获取时，可从整合包 ZIP 同目录补充同名文件，校验通过后再安装。[15cba5f](https://github.com/ricecakey06/hydrogen/commit/15cba5fcc947526a6a03ac5dc540f3d1b7273861)
- 修复基准启动失败时等待错误弹窗的问题，改为记录对应错误日志并退出，便于自动检查发现失败。[84c10b1](https://github.com/ricecakey06/hydrogen/commit/84c10b1f60803011d4c9992e3b3895ce549e4b4e)

### 近期功能汇总

- 建立 Tauri 主线和插件化桌面启动器，提供微软 / 离线档案、Java 探测、实例及本地模组管理、启动与下载进度、崩溃分析，以及外置插件安装、升级和重载等基础能力。[2b85f54](https://github.com/ricecakey06/hydrogen/commit/2b85f549558b5946edde7c8ea27dec3b7a56c370) · [f6a2ea9](https://github.com/ricecakey06/hydrogen/commit/f6a2ea9194b7d17a94bb0dc640d19811df5d5528)
- 增加 Forge / NeoForge 安装支持，完善原版、Fabric 和旧版本的准备流程，以及正式版、快照、预发布版和具体加载器版本的选择。[aa67cbf](https://github.com/ricecakey06/hydrogen/commit/aa67cbf581f12595bfd2022464b32eee4c287623)
- 支持拖入 Modrinth `.mrpack` 或 CurseForge `.zip` 安装独立实例，并按目录或子目录选择内容导出整合包。[1ce0e65](https://github.com/ricecakey06/hydrogen/commit/1ce0e650a1ea8b111a40802bd1866b24dfa909d8)
- 增加 Modrinth / CurseForge 资源搜索、版本选择、依赖安装与本地模组更新工作流；完善批量操作、文件冲突检查和更新失败回滚。[1ce0e65](https://github.com/ricecakey06/hydrogen/commit/1ce0e650a1ea8b111a40802bd1866b24dfa909d8)
- 增加简体中文 / English 切换和自定义语言包，并同步 Minecraft 默认语言，同时保留已有游戏语言设置；调整设置、版本、资源与下载页面，内置 MiSans 字体和第三方声明。[aa67cbf](https://github.com/ricecakey06/hydrogen/commit/aa67cbf581f12595bfd2022464b32eee4c287623) · [1ce0e65](https://github.com/ricecakey06/hydrogen/commit/1ce0e650a1ea8b111a40802bd1866b24dfa909d8)
- 增加完整外观主题系统，支持 CSS、图片、字体、浅色 / 深色变体、主题预览与回退、20 秒自动撤销、`Ctrl+Alt+T` 恢复入口及 `--safe-theme` 启动。[6f5a36e](https://github.com/ricecakey06/hydrogen/commit/6f5a36ed6e1a198c7c5b590c5e29ce062d2f93c2)
- 升级 JavaScript 插件协议至 schema v3 / API v2，提供隔离面板与 bridge、任务、事件、安装 / 启动 Hook、UI 扩展，以及资源、账号、加载器和下载源提供者；完善权限边界与运行时生命周期。[6f5a36e](https://github.com/ricecakey06/hydrogen/commit/6f5a36ed6e1a198c7c5b590c5e29ce062d2f93c2)
- 集成 LWJGL Unsafe Agent 2.0，在满足 Java 与 LWJGL 版本条件时加入启动参数，改善相关新 Java 环境兼容性。[eee8d19](https://github.com/ricecakey06/hydrogen/commit/eee8d1980d39ab4a1c9f0c018f13f47d9644e1ec)

### 近期修复与工程改进

- 修复 Tauri 整合包导入、Forge 安装和安装重试；失败或取消后可重新安装，并正确替换原失败任务，下载任务在页面切换后继续执行。[3a92504](https://github.com/ricecakey06/hydrogen/commit/3a92504c05ec01f6b5ee0ba58908d6b7982d9dc6)
- 恢复模组下载页中的 Modrinth 和 CurseForge 来源图标。[615d0c1](https://github.com/ricecakey06/hydrogen/commit/615d0c11cff88891c3d4271c71ce045425d36947)
- 修复插件打包和安装事务的根路径标准化，正确处理嵌套文件，并补充对应回归检查。[58cda7d](https://github.com/ricecakey06/hydrogen/commit/58cda7d2b96a69cbde6af6c794f74401e35a20b4) · [2658ec1](https://github.com/ricecakey06/hydrogen/commit/2658ec1d821a0e7a03ed18eb80028ae3b2642394)
- 完善跨平台构建：Windows 合成器按目标平台编译，测试使用系统对应的动态库后缀，并使用物理临时目录避免链接路径影响文件系统检查。[8faa45f](https://github.com/ricecakey06/hydrogen/commit/8faa45fd25e34dbebe3a55074f5b672b27f72628) · [7c42bfb](https://github.com/ricecakey06/hydrogen/commit/7c42bfb808f49d820ea2961016fdb5fcd489de96) · [48d19ec](https://github.com/ricecakey06/hydrogen/commit/48d19ecb0c614325b893570df3620060072505a2)
- 稳定崩溃日志时间相关测试，并完善 Windows 打包启动检查、退出码采集及启动失败诊断。[a657395](https://github.com/ricecakey06/hydrogen/commit/a6573950a81d4da4947d03ec5d231815767e73df) · [c1d9c56](https://github.com/ricecakey06/hydrogen/commit/c1d9c56a1d6b74edeadd8a698feafb12a2f71687)
- 移除 Release Drafter 和自动发布任务；CI 保留跨平台测试与 Windows 构建产物，Release 改由维护者手动选择提交、标签、说明和文件后发布。[15cba5f](https://github.com/ricecakey06/hydrogen/commit/15cba5fcc947526a6a03ac5dc540f3d1b7273861)

### 发行文件

- `Launcher.exe`：Windows x64 便携式启动器，需要 Microsoft Edge WebView2 Runtime 110 或更新版本。
- `hydrogen-dev.exe`：配套插件 / 主题开发 CLI，可创建、校验、测试、打包插件并导出 SDK。
- `SHA256SUMS.txt`：上述两个文件的 SHA-256 摘要。

[v0.2.12 到 v0.3.0 的完整提交比较](https://github.com/ricecakey06/hydrogen/compare/v0.2.12...v0.3.0)

## 2026-11-06

- 添加了主题系统，并优化了 JavaScript 插件系统。[6f5a36e](https://github.com/ricecakey06/hydrogen/commit/6f5a36ed6e1a198c7c5b590c5e29ce062d2f93c2)
