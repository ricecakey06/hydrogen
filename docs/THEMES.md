# Hydrogen Theme Authoring Guide

Themes are distributed as `.hplugin` packages and use the existing plugin installation, authorization, disablement, uninstallation, and development reload workflows. The contract declares the first supported version as **0.2.7**. Packages use manifest schema v3 / JavaScript API v2; the theme data protocol is independently versioned as v1.

## Creating and validating a theme

```powershell
hydrogen-dev.exe plugin init ./my-theme --template theme
hydrogen-dev.exe plugin validate ./my-theme
hydrogen-dev.exe plugin test ./my-theme
hydrogen-dev.exe plugin pack ./my-theme --out ./my-theme.hplugin
hydrogen-dev.exe api export --out ./sdk
Launcher.exe --data-dir ./theme-test-data --dev-plugin ./my-theme
```

The template requires neither npm, a frontend build, nor Rust. `validate` checks the manifest, variants, CSS AST, resources, and budgets. For a standalone theme, `test` performs 100 prepare/preview/revert/release cycles in an isolated host and checks that released URLs become invalid. It does not modify the current desktop, user accounts, or game directories. Validate actual layout, image composition, and readability in the launcher.

Installation adds the theme to the appearance catalog. Select it under **Launcher Settings → Themes** to preview it, then click **Keep** within 20 seconds to save. **Revert**, leaving the Themes tab, or timeout restores the last confirmed theme. Unsaved general settings and open panels are preserved. `Ctrl+Alt+T` opens a separate recovery entry point. `Launcher.exe --safe-theme` uses the built-in theme and opaque native material for that run while preserving saved settings and data.

## Package structure and manifest

`plugin init --template theme` generates a manifest, JavaScript entry point, and light/dark variants. Variant JSON paths are relative to the package root. Resource paths inside JSON are relative to that JSON file; CSS `url()` paths are relative to the CSS file.

```json
{
  "schemaVersion": 3,
  "apiVersion": 2,
  "id": "example.my-theme",
  "name": "My theme",
  "version": "1.0.0",
  "runtime": "javascript",
  "entry": "main.js",
  "permissions": ["theme.contribute"],
  "contributes": {
    "themes": [{
      "id": "paper",
      "title": "Paper",
      "themeVersion": 1,
      "variants": {"light": "themes/light.json", "dark": "themes/dark.json"},
      "preview": "themes/preview.png"
    }]
  }
}
```

`preview` is optional. A complete theme ID is `pluginId:themeId`; `builtin:default` is reserved for the host. Supply at least one variant. If the variant for the current color scheme is missing, the host uses the corresponding built-in appearance, displays the reason, and preserves the user's selection.

Standalone themes use the JavaScript package format; `main.js` can contain only `export function activate() {}`. Discovery, selection, and candidate reload skip QuickJS for a package declaring only `theme.contribute` with no services, business contributions, frontend, or activation events. Mixed-function packages continue to use the business plugin permission and lifecycle rules.

## Variants and decoration

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

`scheme` must match its manifest key, and `extends` may only reference the fixed built-in base for the same scheme. `styles` override built-in semantic variables in declaration order. `panelStyles` apply only to isolated HTML panels. Without a declared logo, the host shows its built-in wordmark; a declared logo replaces the complete visible branding. Layers are noninteractive images created by the host; see the slots in the [public contract](../THEME-CONTRACT.md). A single composite background can be defined directly in CSS.

## Instance icons

The version selection page first uses a modpack/instance custom icon. If no usable custom icon exists, it uses the current variant's `instanceIcons`, then the built-in icons: a grass block for vanilla, an anvil for Forge, and the official NeoForge/Fabric logos. Light and dark variants can specify different icons; omitted types use their built-in icon:

```json
"instanceIcons": {
  "vanilla": {"source": "assets/grass.png"},
  "forge": {"source": "assets/anvil.png"},
  "neoforge": {"source": "assets/neoforge.png"},
  "fabric": {"source": "assets/fabric.png"}
}
```

Paths are relative to the variant JSON. Only static PNG/JPEG/WebP images are accepted, with a 2 MiB file limit and a maximum dimension of 1600 px, and they count toward the variant image budget. Icons are prepared, previewed, reverted, and released together with the theme. A theme cannot replace a modpack's own icon. Use `[data-h-part="instance-icon"]` to adjust the displayed size.

Importing a `.mrpack` or CurseForge `.zip` preserves package-root `icon`, `pack`, and `logo` images in PNG/JPEG/WebP format. Instance directories also support `.hydrogen/instance-icon.<extension>`, `PCL/Logo.png`, and root-level `icon`, `pack`, or `logo` images. Damaged, oversized, or linked files are skipped in favor of the next usable icon. Icons are read only from the corresponding version directory; no instance configuration changes are required.

## Writing CSS

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

Use semantic variables and public parts from [theme-contract.json](../theme-contract.json). Author-defined `:root` selectors are compiled to the current visual lease's main or panel root, and other selectors are scoped within that root. Themes can adjust fonts, font sizes, spacing, control dimensions, sidebar width, titlebar height, local layout, and responsive rules. Themes cannot create, delete, or replace business pages.

The host validates CSS with the Lightning CSS AST and CSS tokens, rather than approximating CSS with regular expressions. Supported at-rules are `@media`, `@supports`, restricted `@font-face`, and `@keyframes`. Font families, keyframes, and their references are namespaced. Package-local backgrounds, masks, `border-image`, and `image-set` are supported. Remote URLs, `data:`, `@import`, private classes, application IDs, global `html`/`body` selectors, escaping to root siblings, `!important`, CSS nesting, and unknown at-rules are rejected. Specific properties and pseudo-classes depend on parser support; validation errors identify the source file.

Official default CSS belongs to the `hydrogen.defaults` cascade layer. Theme CSS is applied afterward within its valid scope. Final host protection rules preserve the recovery entry point, noninteractive decoration, window operations, `forced-colors`, and `reduced-motion`. Themes must not obscure functionality, errors, or focus indicators.

Resources must use package-local relative paths. Normalized `..` segments are allowed only when the final path remains within the package root. Links, junctions, encoded paths, absolute paths, and network paths are rejected. Preparation snapshots resource bytes and validates the package digest; existing URLs never read replacement files subsequently written to disk.

## Resource budgets

| Resource | Limit |
| --- | --- |
| Themes per package | 16; at most 256 candidates across the catalog |
| Styles per variant | 8 main/panel styles combined; 512 KiB of CSS in total |
| Variant JSON | 64 KiB |
| Layers | 12; layer IDs must be unique |
| Images | Static PNG/JPEG/WebP; at most 32; 8 MiB each; maximum dimension 4096 px; at most 8 million pixels |
| Logo / preview / instance icon | 2 MiB; maximum dimension 1600 px |
| Fonts | WOFF2; at most 2; 4 MiB each; declared expanded size at most 32 MiB |
| Theme sources and resources per package | 32 MiB |
| Decoded images per variant | 64 MiB; switching decode peak 128 MiB |

PNG/JPEG/WebP resources are checked for extension, container, dimensions, and successful full decoding. Animated PNG/WebP files are rejected. The CLI validates WOFF2 headers, table counts, lengths, and additional-data boundaries. WebView preparation then loads the font; a full decoding failure prevents the theme from being applied. The CLI header check is not an independent complete font decoder.

## Lifecycle, panels, and state

Only one skin is active at a time. The main document first loads styles and decodes images/fonts, then commits CSS, logo, layers, and root tokens together. Switching does not reload the WebView, rebuild routes, or recreate forms. A new request cancels the previous preparation, and late results cannot be reinserted. Scroll restoration uses a visible content anchor; font or dimension changes may cause reflow. Rollback revisions increase monotonically.

Theme resource leases are independent of QuickJS generations. Isolated panel manifests default to `appearance: "inherit"`; `"independent"` is also supported. After the bridge receives the actual scheme, ID, mode, revision, and `highContrast`, an inheriting panel loads tokens and `panelStyles` through its own session protocol. Main-shell scenes and branding do not enter the panel. The host binds the panel's permitted theme lease and keeps the previous styles until the new set is confirmed. A panel that fails to synchronize within 5 seconds is closed, an explanation is shown, and rollback is requested. The bridge also supports cancellation and timeouts. CSP does not gain arbitrary inline-style permissions.

`bridge.theme` retains its light/dark semantics. `bridge.appearance` and `bridge.onTheme(listener)` expose complete appearance information and updates. Panel CSS can inherit colors through public variables such as `var(--h-text-primary)` and declare public parts with `data-h-part`. A third-party panel with fixed colors does not automatically receive complete theme styling.

`ctx.ui.theme()` returns the actual appearance summary (`theme`/`id`/`mode`/`revision`/`highContrast`/`locale`). `theme.changed` notifies only activated subscribers with `events` and either `ui.contribute` or `theme.contribute`. It exposes neither resource paths nor leases and does not activate every plugin just to deliver the event.

Switching a standalone theme does not wait for games or downloads. Disabling or uninstalling the active theme first restores the built-in appearance; a later persistence or deletion failure restores the previously confirmed state. After development edits, use **Verify and reload**. Invalid candidates leave the previous package and snapshots in place.

## Visual validation

Check Home, Settings, Instances, Resources, Profiles, Downloads, Plugins, Crash Reports, menus, dialogs, and error states. Include the minimum window size, collapsed sidebar, long text, Chinese and English, and different DPI settings. In the launcher, verify contrast with actual images, transparency composition, and font rendering.
