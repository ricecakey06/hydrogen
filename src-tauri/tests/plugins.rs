use hydrogen::{
    contracts::prototype_instances,
    plugins::{
        builtin, manager,
        manifest::{self, Plugin},
        package,
        panel::Panels,
        runtime::ScriptHost,
    },
};
use std::{fs, path::Path};

fn fixture(source: &str, permissions: &[&str]) -> (tempfile::TempDir, Plugin) {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("main.js"), source).unwrap();
    fs::write(
        dir.path().join("plugin.json"),
        serde_json::to_vec(&serde_json::json!({
            "schemaVersion":2,"apiVersion":1,"id":"example.test","name":"Test","version":"1.0.0",
            "runtime":"javascript","entry":"main.js","permissions":permissions,
            "contributes":{"commands":[{"id":"test","title":"Test"}]}
        }))
        .unwrap(),
    )
    .unwrap();
    let plugin = manifest::load(dir.path()).unwrap();
    (dir, plugin)
}

#[test]
fn builtin_manifests_are_extracted_once_loaded_and_protected() {
    let dir = tempfile::tempdir().unwrap();
    manager::initialize(dir.path()).unwrap();
    assert!(
        dir.path()
            .join("plugins/account.offline/plugin.json")
            .is_file()
    );
    assert!(!dir.path().join("plugin").exists());
    let loaded = manager::load(dir.path(), &[]).unwrap();
    assert_eq!(loaded.len(), builtin::PACKAGES.len());
    for plugin in &loaded {
        assert_eq!(plugin.manifest.runtime, "native");
        if builtin::migrated(&plugin.manifest.id) {
            assert!(
                manifest::within(&plugin.root, &plugin.manifest.entry)
                    .unwrap()
                    .is_file()
            );
            assert!(!plugin.manifest.provides.is_empty() || plugin.manifest.frontend.is_some());
        } else {
            assert_eq!(plugin.manifest.entry, plugin.manifest.id);
        }
        assert!(plugin.root.starts_with(dir.path().join("plugins")));
    }
    let path = dir.path().join("plugins/storage/plugin.json");
    let before = fs::read(&path).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    manager::initialize(dir.path()).unwrap();
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    assert_eq!(fs::read(&path).unwrap(), before);
    // Unchanged application distribution never overwrites an independently modified package.
    let mut changed: serde_json::Value = serde_json::from_slice(&before).unwrap();
    changed["version"] = "1.1.0".into();
    let revision = serde_json::to_vec(&changed).unwrap();
    fs::write(&path, &revision).unwrap();
    manager::initialize(dir.path()).unwrap();
    assert_eq!(fs::read(&path).unwrap(), revision);
    changed["entry"] = "injected.dll".into();
    let damaged = serde_json::to_vec(&changed).unwrap();
    fs::write(&path, &damaged).unwrap();
    manager::initialize(dir.path()).unwrap();
    assert_eq!(fs::read(path).unwrap(), damaged);
    assert!(builtin::load_all(dir.path()).is_err());
}

#[tokio::test]
async fn installed_plugin_reloads_without_a_command_and_persists_the_receipt() {
    let (source, _) = fixture(
        "export function activate(ctx) { ctx.commands.register('test', () => 1); }",
        &[],
    );
    let data = tempfile::tempdir().unwrap();
    let archive = data.path().join("example.hplugin");
    package::pack(source.path(), &archive).unwrap();
    let candidate = manager::prepare(data.path(), &archive).unwrap();
    let installed = manager::install(data.path(), &candidate, None).unwrap();
    assert_eq!(installed.root, data.path().join("plugins/example.test"));
    let host = host();
    host.activate(installed.clone()).await.unwrap();
    fs::write(
        installed.root.join("main.js"),
        "export function activate(ctx) { ctx.commands.register('test', () => 42); }",
    )
    .unwrap();
    let checked = manager::reload_candidate(&installed).unwrap();
    let root = data.path().to_owned();
    let replacement = checked.clone();
    host.reload_with_commit(checked.clone(), move || {
        manager::approve_reload(&root, &installed, &replacement)
    })
    .await
    .unwrap();
    assert_eq!(
        host.run(checked.clone(), "test".into(), false)
            .await
            .unwrap()
            .result,
        42
    );
    assert_eq!(
        manager::discover(data.path(), &[]).unwrap()[0].digest,
        checked.digest
    );
    // Reload is not permission consent: new capabilities still require package installation.
    let path = checked.root.join("plugin.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    manifest["permissions"] = serde_json::json!(["instances.read"]);
    fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    assert_eq!(
        manager::reload_candidate(&checked).unwrap_err().code,
        "PLUGIN_PERMISSION_CHANGED"
    );
    host.stop_all().await.unwrap();
}

#[tokio::test]
async fn failed_reload_commit_keeps_the_old_runtime() {
    let (dir, previous) = fixture(
        "export function activate(ctx) { ctx.commands.register('test', () => 7); }",
        &[],
    );
    let host = host();
    let before = host.activate(previous.clone()).await.unwrap();
    fs::write(
        dir.path().join("main.js"),
        "export function activate(ctx) { ctx.commands.register('test', () => 8); }",
    )
    .unwrap();
    let checked = manager::reload_candidate(&previous).unwrap();
    assert!(
        host.reload_with_commit(checked, || Err(hydrogen::contracts::ApiError::new(
            "IO_ERROR",
            "failed receipt"
        )))
        .await
        .is_err()
    );
    let after = host.run(previous, "test".into(), false).await.unwrap();
    assert_eq!(after.generation, before.generation);
    assert_eq!(after.result, 7);
    let stats = host.stop_all().await.unwrap();
    assert_eq!(stats.created, stats.destroyed);
}

#[tokio::test]
async fn invalidating_a_provider_stops_its_transitive_consumers_only() {
    let (_dir, provider) = fixture(
        "export function activate(ctx) { let calls=0; ctx.commands.register('test', () => ++calls); }",
        &[],
    );
    let mut consumer = provider.clone();
    consumer.manifest.id = "example.consumer".into();
    consumer.manifest.dependencies =
        serde_json::from_value(serde_json::json!(["example.test"])).unwrap();
    let mut indirect = provider.clone();
    indirect.manifest.id = "example.indirect".into();
    indirect.manifest.dependencies =
        serde_json::from_value(serde_json::json!(["example.consumer"])).unwrap();
    let mut independent = provider.clone();
    independent.manifest.id = "example.independent".into();
    let plugins = vec![indirect, consumer, provider, independent.clone()];
    let host = host();
    for plugin in &plugins {
        host.activate(plugin.clone()).await.unwrap();
    }
    let affected = manager::affected(&plugins, "example.test");
    assert_eq!(affected.len(), 3);
    let stats = host.stop_plugins(affected).await.unwrap();
    assert_eq!(stats.active, 1);
    assert_eq!(stats.registrations, 1);
    assert_eq!(
        host.run(independent, "test".into(), false)
            .await
            .unwrap()
            .result,
        1
    );
    let stats = host.stop_all().await.unwrap();
    assert_eq!(stats.created, stats.destroyed);
}
fn host() -> ScriptHost {
    ScriptHost::new(tokio::runtime::Handle::current(), prototype_instances())
}

#[test]
fn paths_and_reserved_ids_are_rejected() {
    for path in [
        "../secret",
        "/root",
        "C:/foo",
        "foo\\bar",
        "a/../b",
        "a//b",
        "a/./b",
        "%2e%2e/x",
        "NUL.txt",
        "a/COM1.js",
        "x.",
    ] {
        assert!(!manifest::safe_relative(path), "accepted {path}");
    }
    let (dir, _) = fixture("export function activate() {}", &[]);
    let path = dir.path().join("plugin.json");
    let text = fs::read_to_string(&path)
        .unwrap()
        .replace("example.test", "storage");
    fs::write(path, text).unwrap();
    assert_eq!(manifest::load(dir.path()).unwrap().manifest.id, "storage");
}

#[tokio::test]
async fn real_engine_calls_async_host_and_cleans_up() {
    let (_dir, plugin) = fixture(
        "export function activate(ctx) { ctx.commands.register('test', async () => { const rows = await ctx.instances.list(); await ctx.ui.notify({message:'count '+rows.length}); return rows.length; }); }",
        &["instances.read", "ui.contribute"],
    );
    let host = host();
    let report = host.run(plugin, "test".into(), false).await.unwrap();
    assert_eq!(report.result, 2);
    assert_eq!(report.notifications, ["count 2"]);
    assert!(report.heap_bytes < 8 * 1024 * 1024);
    let stats = host.stop_all().await.unwrap();
    assert_eq!(stats.active, 0);
    assert_eq!(stats.registrations, 0);
    assert_eq!(stats.created, stats.destroyed);
}

#[tokio::test]
async fn permission_denial_is_enforced_by_rust() {
    let (_dir, plugin) = fixture(
        "export function activate(ctx) { ctx.commands.register('test', async () => { try { await ctx.instances.list(); } catch(e) { return e.code; } }); }",
        &[],
    );
    assert_eq!(
        host()
            .run(plugin, "test".into(), false)
            .await
            .unwrap()
            .result,
        "PERMISSION_DENIED"
    );
}

#[tokio::test]
async fn infinite_loop_is_interrupted() {
    let (_dir, plugin) = fixture(
        "export function activate(ctx) { ctx.commands.register('test', () => { while(true) {} }); }",
        &[],
    );
    let started = std::time::Instant::now();
    assert!(host().run(plugin, "test".into(), false).await.is_err());
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
}

#[tokio::test]
async fn heap_exhaustion_is_stopped() {
    let (_dir, plugin) = fixture(
        "export function activate(ctx) { ctx.commands.register('test', () => { let a=[]; for(let i=0;i<100000;i++) a.push(new Array(1024).fill(i)); return a.length; }); }",
        &[],
    );
    assert!(host().run(plugin, "test".into(), false).await.is_err());
}

#[tokio::test]
async fn rejected_promises_and_oversized_results_fail_without_hanging() {
    for source in [
        "export function activate(ctx) { ctx.commands.register('test', async () => { throw new Error('rejected'); }); }",
        "export function activate(ctx) { ctx.commands.register('test', () => 'x'.repeat(300000)); }",
    ] {
        let (_dir, plugin) = fixture(source, &[]);
        assert!(host().run(plugin, "test".into(), false).await.is_err());
    }
}

#[tokio::test]
async fn ninth_pending_host_request_is_rejected() {
    let (_dir, plugin) = fixture(
        "export function activate(ctx) { ctx.commands.register('test', async () => { const p = Array.from({length:9}, () => ctx.instances.list()); const r = await Promise.allSettled(p); return r.filter(x => x.status === 'rejected').length; }); }",
        &["instances.read"],
    );
    assert_eq!(
        host()
            .run(plugin, "test".into(), false)
            .await
            .unwrap()
            .result,
        1
    );
}

#[tokio::test]
async fn modified_directory_requires_explicit_reload() {
    let (dir, plugin) = fixture(
        "export function activate(ctx) { ctx.commands.register('test', () => 1); }",
        &[],
    );
    let host = host();
    host.run(plugin, "test".into(), false).await.unwrap();
    fs::write(
        dir.path().join("main.js"),
        "export function activate(ctx) { ctx.commands.register('test', () => 2); }",
    )
    .unwrap();
    let changed = manifest::load(dir.path()).unwrap();
    assert_eq!(
        host.run(changed.clone(), "test".into(), false)
            .await
            .unwrap_err()
            .code,
        "PLUGIN_CHANGED"
    );
    assert_eq!(
        host.run(changed, "test".into(), true).await.unwrap().result,
        2
    );
}

#[tokio::test]
async fn imports_cannot_access_node_or_escape_package() {
    for name in ["node:fs", "https://example.com/code.js", "../outside.js"] {
        let (_dir, plugin) = fixture(
            &format!(
                "import x from '{name}'; export function activate(ctx) {{ ctx.commands.register('test', () => 1); }}"
            ),
            &[],
        );
        assert!(
            host().run(plugin, "test".into(), false).await.is_err(),
            "accepted {name}"
        );
    }
}

#[tokio::test]
async fn relative_import_works_in_same_engine() {
    let (dir, _) = fixture(
        "import {n} from './lib.js'; export function activate(ctx) { ctx.commands.register('test', () => n); }",
        &[],
    );
    fs::write(dir.path().join("lib.js"), "export const n = 42;").unwrap();
    let plugin = manifest::load(dir.path()).unwrap();
    assert_eq!(
        host()
            .run(plugin, "test".into(), false)
            .await
            .unwrap()
            .result,
        42
    );
}

#[tokio::test]
async fn failed_reload_preserves_previous_runtime() {
    let (dir, plugin) = fixture(
        "export function activate(ctx) { ctx.commands.register('test', () => 7); }",
        &[],
    );
    let host = host();
    let before = host
        .run(plugin.clone(), "test".into(), false)
        .await
        .unwrap();
    fs::write(
        dir.path().join("main.js"),
        "export function activate( { syntax error",
    )
    .unwrap();
    let changed = manifest::load(dir.path()).unwrap();
    assert!(host.run(changed, "test".into(), true).await.is_err());
    let after = host.run(plugin, "test".into(), false).await.unwrap();
    assert_eq!(before.generation, after.generation);
    assert_eq!(after.result, 7);
}

#[tokio::test]
async fn hundred_reloads_release_every_runtime_and_registration() {
    let (_dir, plugin) = fixture(
        "export function activate(ctx) { ctx.commands.register('test', () => 1); }",
        &[],
    );
    let host = host();
    let mut last_generation = 0;
    for _ in 0..100 {
        let report = host.run(plugin.clone(), "test".into(), true).await.unwrap();
        assert!(report.generation > last_generation);
        last_generation = report.generation;
    }
    let stats = host.stop_all().await.unwrap();
    assert_eq!(stats.created, 100);
    assert_eq!(stats.destroyed, 100);
    assert_eq!(stats.active, 0);
    assert_eq!(stats.registrations, 0);
}

#[test]
fn panel_sessions_are_bound_and_revoked() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/instance-tools");
    let plugin = manifest::load(&root).unwrap();
    let panels = Panels::default();
    let session = panels.open(plugin, "overview", 4).unwrap();
    assert!(panels.plugin(&session.session_id, 3).is_err());
    assert!(panels.plugin("forged", 4).is_err());
    assert!(
        panels
            .resource(&format!("/{}/../main.js", session.session_id))
            .is_err()
    );
    assert!(
        panels
            .resource(&format!("/{}/%2e%2e/main.js", session.session_id))
            .is_err()
    );
    assert!(
        panels
            .resource(&format!("/{}/index.html", session.session_id))
            .is_ok()
    );
    panels.close(&session.session_id);
    assert!(panels.plugin(&session.session_id, 4).is_err());
    assert!(
        panels
            .resource(&format!("/{}/index.html", session.session_id))
            .is_err()
    );
}

#[tokio::test]
async fn ten_plugins_really_activate_with_independent_runtimes() {
    let (_dir, plugin) = fixture(
        "export function activate(ctx) { let calls=0; ctx.commands.register('test', async () => { await ctx.instances.list(); return ++calls; }); }",
        &["instances.read"],
    );
    let host = host();
    let mut total_heap = 0;
    for index in 0..10 {
        let mut candidate = plugin.clone();
        candidate.manifest.id = format!("example.plugin-{index}");
        let report = host.run(candidate, "test".into(), false).await.unwrap();
        assert_eq!(report.result, 1);
        total_heap += report.heap_bytes;
    }
    assert!(total_heap < 20 * 1024 * 1024);
    let stats = host.stop_all().await.unwrap();
    assert_eq!(stats.created, 10);
    assert_eq!(stats.destroyed, 10);
    assert_eq!(stats.registrations, 0);
}

#[test]
fn packaging_keeps_only_safe_relative_entries_and_refuses_overwrite() {
    let (dir, _) = fixture("export function activate() {}", &[]);
    let output_dir = tempfile::tempdir().unwrap();
    let output = output_dir.path().join("test.hplugin");
    package::pack(dir.path(), &output).unwrap();
    let mut zip = zip::ZipArchive::new(fs::File::open(&output).unwrap()).unwrap();
    assert_eq!(zip.len(), 2);
    assert_eq!(
        zip.by_name("main.js").unwrap().size(),
        fs::metadata(dir.path().join("main.js")).unwrap().len()
    );
    assert!(package::pack(dir.path(), &output).is_err());
}
