fn main() {
    bundle_plugins();
    #[cfg(feature = "desktop")]
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "app_snapshot",
            "instance_select",
            "plugin_run",
            "plugin_reload",
            "panel_open",
            "panel_call",
            "panel_close",
            "ui_ready",
            "ui_bootstrap",
            "launcher_action",
        ]),
    ))
    .expect("Tauri build configuration");
}

fn bundle_plugins() {
    use std::{fs, io::Write, path::PathBuf};
    let root =
        PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../official-plugins");
    let output = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let profile = std::env::var("PROFILE").unwrap();
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap();
    let (prefix, suffix) = if os == "windows" {
        ("", "dll")
    } else if os == "macos" {
        ("lib", "dylib")
    } else {
        ("lib", "so")
    };
    let mut generated = String::from("pub const PACKAGES: &[(&str, &[u8])] = &[\n");
    let index_path = root.join("distribution.json");
    println!("cargo:rerun-if-changed={}", index_path.display());
    let distribution: std::collections::BTreeMap<String, String> =
        serde_json::from_slice(&fs::read(index_path).unwrap()).unwrap();
    for (id, library) in distribution {
        assert!(
            !id.contains(['/', '\\']) && !library.contains(['/', '\\']),
            "Invalid distribution path"
        );
        let manifest_path = root.join(&id).join("plugin.json");
        let binary = root
            .join("target")
            .join(&profile)
            .join(format!("{prefix}{library}.{suffix}"));
        println!("cargo:rerun-if-changed={}", manifest_path.display());
        println!("cargo:rerun-if-changed={}", binary.display());
        let bytes = fs::read(&binary).unwrap_or_else(|_| panic!("Build official plugins first: cargo build --manifest-path ../official-plugins/Cargo.toml{}", if profile == "release" { " --release" } else { "" }));
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(manifest_path).unwrap()).unwrap();
        let entry = format!("bin/plugin.{suffix}");
        manifest["entry"] = entry.clone().into();
        let archive_path = output.join(format!("{id}.hplugin"));
        let mut archive = zip::ZipWriter::new(fs::File::create(&archive_path).unwrap());
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        archive.start_file("plugin.json", options).unwrap();
        archive
            .write_all(&serde_json::to_vec_pretty(&manifest).unwrap())
            .unwrap();
        archive.start_file(entry, options).unwrap();
        archive.write_all(&bytes).unwrap();
        fn assets(
            base: &std::path::Path,
            directory: &std::path::Path,
            archive: &mut zip::ZipWriter<fs::File>,
            options: zip::write::SimpleFileOptions,
        ) {
            if !directory.exists() {
                return;
            }
            println!("cargo:rerun-if-changed={}", directory.display());
            let mut entries: Vec<_> = fs::read_dir(directory)
                .unwrap()
                .map(|e| e.unwrap())
                .collect();
            entries.sort_by_key(|e| e.file_name());
            for entry in entries {
                let kind = entry.file_type().unwrap();
                assert!(!kind.is_symlink(), "Linked bundled asset");
                let path = entry.path();
                if kind.is_dir() {
                    assets(base, &path, archive, options);
                } else if kind.is_file() {
                    let name = path
                        .strip_prefix(base)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/");
                    archive.start_file(name, options).unwrap();
                    archive.write_all(&fs::read(path).unwrap()).unwrap();
                }
            }
        }
        assets(
            &root.join(&id),
            &root.join(&id).join("assets"),
            &mut archive,
            options,
        );
        archive.finish().unwrap();
        generated.push_str(&format!(
            "({id:?}, include_bytes!({:?})),\n",
            archive_path.to_str().unwrap()
        ));
    }
    generated.push_str("];\n");
    fs::write(output.join("bundled-plugins.rs"), generated).unwrap();
}
