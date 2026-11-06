# Hydrogen 主题公共契约

由 theme-contract.json 自动生成。协议 v1，首个支持版本 0.2.7。

## 语义变量

| 变量 | 浅色默认 | 深色默认 |
| --- | --- | --- |
| `--h-surface-canvas` | `#ffffff` | `#191d25` |
| `--h-surface-sidebar` | `#f2f3f6` | `#12161e` |
| `--h-surface-card` | `#ffffff` | `#252b36` |
| `--h-surface-popup` | `#fafbfc` | `#252b36` |
| `--h-surface-input` | `#ffffff` | `#1b202a` |
| `--h-text-primary` | `#17191c` | `#edf1f7` |
| `--h-text-secondary` | `#62666e` | `#b7c0ce` |
| `--h-text-disabled` | `#747982` | `#9aa6b7` |
| `--h-border` | `#cdd1d6` | `#566273` |
| `--h-control-hover` | `#eceff3` | `#333d4d` |
| `--h-control-active` | `#dce2e9` | `#405068` |
| `--h-selection` | `#e5e8ed` | `#344157` |
| `--h-primary` | `#292c32` | `#d6e2f4` |
| `--h-on-primary` | `#ffffff` | `#151c27` |
| `--h-primary-hover` | `#41454d` | `#b9cce6` |
| `--h-focus-ring` | `#34373c` | `#a9cfff` |
| `--h-control-border` | `#8b8f98` | `#697586` |
| `--h-danger` | `#b42318` | `#ffb4a9` |
| `--h-danger-surface` | `#fff5f4` | `#431f24` |
| `--h-success` | `#37613c` | `#a5ddb0` |
| `--h-progress` | `#34373c` | `#bbd6ff` |
| `--h-scrollbar` | `#69717f99` | `#a1adc099` |
| `--h-font-body` | `'Hydrogen MiSans','Microsoft YaHei UI',sans-serif` | `'Hydrogen MiSans','Microsoft YaHei UI',sans-serif` |
| `--h-font-size` | `16px` | `16px` |
| `--h-font-weight` | `400` | `400` |
| `--h-line-height` | `1.5` | `1.5` |
| `--h-space` | `12px` | `12px` |
| `--h-page-padding` | `24px` | `24px` |
| `--h-radius` | `8px` | `8px` |
| `--h-card-radius` | `12px` | `12px` |
| `--h-shadow` | `0 2px 4px #18202b12` | `0 2px 6px #0004` |
| `--h-sidebar-width` | `239px` | `239px` |
| `--h-sidebar-collapsed-width` | `72px` | `72px` |
| `--h-titlebar-height` | `47px` | `47px` |

## 公共部位和状态

- **parts**：`titlebar`、`brand`、`brand-image`、`instance-icon`、`sidebar`、`nav-item`、`content`、`page`、`page-heading`、`card`、`button`、`input-shell`
- **variants**：`primary`、`launch`、`danger`、`secondary`、`icon`
- **scopes**：`popup`
- **pages**：`home`、`versions`、`instance`、`profiles`、`settings`、`downloads`、`crash`、`resources`、`plugin`
- **sidebar**：`expanded`、`collapsed`
- **slots**：`window.background`、`content.background`、`sidebar.background`、`titlebar.decoration`、`sidebar.decoration`、`content.decoration`
- **atRules**：`media`、`supports`、`font-face`、`keyframes`

主根标记为 `data-h-theme-root`；面板根为 `data-h-panel-root`，由宿主管理。
主题 CSS 使用 `:root` 和 `data-h-part`、`data-h-layer`、`data-h-variant`、`data-h-scope`、`data-h-page`、`data-h-sidebar`。
支持普通元素、伪类/伪元素，以及 disabled、hidden、type、role、aria-selected、aria-expanded、aria-checked、aria-current、aria-disabled 状态。
CSS 不接受私有类、应用 ID、html/body、@import、!important、远程/data URL 或未支持的资源函数。
详细写法见 [主题作者指南](docs/THEMES.md)。
