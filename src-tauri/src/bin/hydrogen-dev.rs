use hydrogen::{
    contracts::{ApiError, Result, prototype_instances},
    plugins::{builtin, host::PluginHost, manifest, package},
};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
};

async fn execute(args: &[String]) -> Result<serde_json::Value> {
    match args.first().map(String::as_str) {
        Some("check") => {
            #[cfg(feature = "desktop")]
            {
                Ok(
                    json!({"webviewVersion": tauri::webview_version().map_err(|_| ApiError::new("WEBVIEW_MISSING", "未找到 WebView2 Runtime。"))?, "stage":"P0"}),
                )
            }
            #[cfg(not(feature = "desktop"))]
            {
                Ok(
                    json!({"stage":"P0", "webviewVersion":null,"note":"此 CLI 构建不链接桌面运行时。Launcher.exe --check 执行原生启动检查。"}),
                )
            }
        }
        Some("plugin") => {
            let operation = args.get(1).map(String::as_str).unwrap_or("");
            let root = PathBuf::from(args.get(2).ok_or_else(usage)?);
            match operation {
                "bundled" => {
                    if builtin::PACKAGES
                        .iter()
                        .any(|(id, _)| root.join(format!("{id}.hplugin")).exists())
                    {
                        return Err(ApiError::new(
                            "OUTPUT_EXISTS",
                            "输出目录已有同名插件包，请选择新目录。",
                        ));
                    }
                    fs::create_dir_all(&root)?;
                    for &(id, bytes) in builtin::PACKAGES {
                        use std::io::Write;
                        fs::OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .open(root.join(format!("{id}.hplugin")))?
                            .write_all(bytes)?;
                    }
                    Ok(
                        json!({"directory":root,"packages":builtin::PACKAGES.iter().map(|(id,_)| id).collect::<Vec<_>>()}),
                    )
                }
                "init" => {
                    if root.exists() && fs::read_dir(&root)?.next().is_some() {
                        return Err(ApiError::new(
                            "DIRECTORY_NOT_EMPTY",
                            "模板只写入新目录或空目录。",
                        ));
                    }
                    fs::create_dir_all(&root)?;
                    if args
                        .iter()
                        .position(|a| a == "--template")
                        .and_then(|i| args.get(i + 1))
                        .is_some_and(|t| t == "theme")
                    {
                        hydrogen::plugins::themes::write_template(&root)?;
                        return Ok(json!({"directory":root,"created":true,"template":"theme"}));
                    }
                    // The template is self-contained and requires no package installation or compiler.
                    let manifest: serde_json::Value = serde_json::from_str(include_str!(
                        "../../../examples/instance-tools/plugin.json"
                    ))?;
                    fs::write(
                        root.join("plugin.json"),
                        serde_json::to_vec_pretty(&manifest)?,
                    )?;
                    fs::write(
                        root.join("main.js"),
                        include_str!("../../../examples/instance-tools/main.js")
                            .replace("../../plugin-sdk/launcher", "./launcher"),
                    )?;
                    fs::write(
                        root.join("launcher.d.ts"),
                        include_str!("../../../plugin-sdk/launcher.d.ts"),
                    )?;
                    fs::create_dir_all(root.join("ui"))?;
                    for (name, contents) in [
                        (
                            "index.html",
                            include_str!("../../../examples/instance-tools/ui/index.html"),
                        ),
                        (
                            "main.js",
                            include_str!("../../../examples/instance-tools/ui/main.js"),
                        ),
                        (
                            "style.css",
                            include_str!("../../../examples/instance-tools/ui/style.css"),
                        ),
                        ("bridge.js", include_str!("../../../plugin-sdk/bridge.js")),
                    ] {
                        fs::write(root.join("ui").join(name), contents)?;
                    }
                    Ok(json!({"directory":root,"created":true}))
                }
                "validate" => {
                    let plugin = manifest::load(&root)?;
                    Ok(
                        json!({"valid":true,"id":plugin.manifest.id,"digest":plugin.digest,"permissions":plugin.manifest.permissions}),
                    )
                }
                "test" => {
                    let plugin = manifest::load(&root)?;
                    let host =
                        PluginHost::new(tokio::runtime::Handle::current(), prototype_instances());
                    if hydrogen::plugins::themes::standalone(&plugin) {
                        return hydrogen::plugins::themes::self_test(&host, &plugin);
                    }
                    host.activate(plugin.clone()).await?;
                    let mut reports = Vec::new();
                    if let Some(index) = args.iter().position(|a| a == "--command") {
                        let command = args.get(index + 1).ok_or_else(usage)?;
                        let params = args
                            .iter()
                            .position(|a| a == "--params")
                            .map(|i| args.get(i + 1).ok_or_else(usage))
                            .transpose()?
                            .map(|v| serde_json::from_str(v))
                            .transpose()?
                            .unwrap_or(serde_json::Value::Null);
                        reports.push(
                            host.run_params(plugin.clone(), command.clone(), params, false)
                                .await?,
                        );
                    }
                    let cleanup = host.stop_all().await?;
                    Ok(
                        json!({"passed":true,"reports":reports,"cleanup":cleanup,"fixtures":"isolated instance list; business operations require Launcher.exe"}),
                    )
                }
                "pack" => {
                    let index = args.iter().position(|a| a == "--out").ok_or_else(usage)?;
                    let output = PathBuf::from(args.get(index + 1).ok_or_else(usage)?);
                    package::pack(&root, &output)?;
                    Ok(json!({"package": output, "bytes": fs::metadata(output)?.len()}))
                }
                _ => Err(usage()),
            }
        }
        Some("api") if args.get(1).map(String::as_str) == Some("export") => {
            let index = args.iter().position(|a| a == "--out").ok_or_else(usage)?;
            let output = Path::new(args.get(index + 1).ok_or_else(usage)?);
            fs::create_dir_all(output)?;
            fs::write(
                output.join("launcher.d.ts"),
                include_str!("../../../plugin-sdk/launcher.d.ts"),
            )?;
            fs::write(
                output.join("bridge.js"),
                include_str!("../../../plugin-sdk/bridge.js"),
            )?;
            fs::write(
                output.join("services.json"),
                include_str!("../../../plugin-sdk/services.json"),
            )?;
            fs::write(
                output.join("theme-contract.json"),
                hydrogen::plugins::themes::CONTRACT,
            )?;
            fs::write(
                output.join("theme.d.ts"),
                include_str!("../../../plugin-sdk/theme.d.ts"),
            )?;
            fs::write(
                output.join("theme-schema.json"),
                include_str!("../../../theme-schema.json"),
            )?;
            fs::write(
                output.join("theme-defaults.css"),
                include_str!("../../../official-plugins/ui.shell/frontend/theme-defaults.css"),
            )?;
            fs::write(
                output.join("THEMES.md"),
                include_str!("../../../docs/THEMES.md").replace("](../", "]("),
            )?;
            fs::write(
                output.join("THEME-CONTRACT.md"),
                include_str!("../../../THEME-CONTRACT.md")
                    .replace("](docs/THEMES.md)", "](THEMES.md)"),
            )?;
            Ok(json!({"apiVersion":2,"schemaVersion":3,"themeVersion":1,"out":output}))
        }
        _ => Err(usage()),
    }
}
fn usage() -> ApiError {
    ApiError::new(
        "USAGE",
        "hydrogen-dev plugin <init|validate|test|pack|bundled> <directory> [--template theme] [--out package.hplugin] [--command id --params JSON] [--json] | api export --out <directory> | check",
    )
}

#[tokio::main]
async fn main() {
    let args: Vec<_> = std::env::args().skip(1).filter(|s| s != "--json").collect();
    match execute(&args).await {
        Ok(result) => println!("{}", serde_json::to_string_pretty(&result).unwrap()),
        Err(error) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({"error":error})).unwrap()
            );
            std::process::exit(1);
        }
    }
}
