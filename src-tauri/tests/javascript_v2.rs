mod common;
use hydrogen::{
    contracts::{ApiError, prototype_instances},
    launcher::storage::Settings,
    plugins::{
        host::PluginHost,
        manifest::{self, Plugin},
        panel::Panels,
    },
};
use serde_json::{Value, json};
use std::{fs, path::Path, sync::Arc, time::Duration};

fn plugin(
    root: &Path,
    id: &str,
    source: &str,
    permissions: &[&str],
    dependencies: Value,
    provides: Value,
) -> Plugin {
    let root = root.join(id);
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("main.js"), source).unwrap();
    fs::write(root.join("plugin.json"),serde_json::to_vec(&json!({
        "schemaVersion":3,"apiVersion":2,"id":id,"name":id,"version":"1.0.0","runtime":"javascript",
        "entry":"main.js","permissions":permissions,"dependencies":dependencies,"provides":provides,
        "contributes":{"commands":[{"id":"test","title":"Test"}]}
    })).unwrap()).unwrap();
    manifest::load(&root).unwrap()
}
fn host(plugins: Vec<Plugin>) -> PluginHost {
    let host = PluginHost::new(tokio::runtime::Handle::current(), prototype_instances());
    host.set_plugins(plugins);
    host
}
#[tokio::test]
async fn parameters_services_and_versions_use_the_real_engine() {
    let root = tempfile::tempdir().unwrap();
    let provider = plugin(
        root.path(),
        "example.provider",
        "export function activate(ctx){ctx.services.register('example.math',{double:({value})=>({value:value*2})});}",
        &[],
        json!([]),
        json!([{"id":"example.math","version":"2.0.0","methods":["double"]}]),
    );
    let consumer = plugin(
        root.path(),
        "example.consumer",
        "export function activate(ctx){ctx.commands.register('test',p=>ctx.services.call({provider:'example.provider',service:'example.math',version:p.version,method:'double',params:p}));}",
        &["services.call"],
        json!([{"id":"example.provider","version":"^1"}]),
        json!([]),
    );
    let host = host(vec![provider, consumer.clone()]);
    assert_eq!(
        host.run_params(
            consumer.clone(),
            "test".into(),
            json!({"value":21,"version":"^2"}),
            false
        )
        .await
        .unwrap()
        .result,
        json!({"value":42})
    );
    assert!(
        host.run_params(
            consumer,
            "test".into(),
            json!({"value":1,"version":"^1"}),
            false
        )
        .await
        .is_err()
    );
    host.stop_all().await.unwrap();
}

#[tokio::test]
async fn javascript_proxy_cannot_elevate_business_api_permissions() {
    let root = tempfile::tempdir().unwrap();
    let proxy = plugin(
        root.path(),
        "example.proxy",
        "export function activate(ctx){ctx.services.register('example.proxy',{list:()=>ctx.instances.list()});}",
        &["instances.read"],
        json!([]),
        json!([{"id":"example.proxy","version":"1.0.0","methods":["list"]}]),
    );
    let caller = plugin(
        root.path(),
        "example.caller",
        "export function activate(ctx){ctx.commands.register('test',async()=>{try{return await ctx.services.call({provider:'example.proxy',service:'example.proxy',version:'^1',method:'list'})}catch(e){return e.code}});}",
        &["services.call"],
        json!(["example.proxy"]),
        json!([]),
    );
    let host = host(vec![proxy, caller.clone()]);
    assert_eq!(
        host.run(caller, "test".into(), false).await.unwrap().result,
        "PERMISSION_DENIED"
    );
    host.stop_all().await.unwrap();
}
#[tokio::test]
async fn waiting_requests_yield_and_stop_revokes_pending_responses() {
    let root = tempfile::tempdir().unwrap();
    let slow = plugin(
        root.path(),
        "example.slow",
        "export function activate(ctx){ctx.commands.register('test',()=>ctx.http.request({url:'https://example.test'}));}",
        &["http"],
        json!([]),
        json!([]),
    );
    let fast = plugin(
        root.path(),
        "example.fast",
        "export function activate(ctx){ctx.commands.register('test',p=>p.value);}",
        &[],
        json!([]),
        json!([]),
    );
    let host = host(vec![slow.clone(), fast.clone()]);
    let (started, received) = tokio::sync::oneshot::channel();
    let started = Arc::new(std::sync::Mutex::new(Some(started)));
    let (release, wait) = std::sync::mpsc::channel();
    let wait = Arc::new(std::sync::Mutex::new(wait));
    host.set_operations(Arc::new(move |_, _, method, _| {
        if method == "lifecycle.stop" {
            return Ok(Value::Null);
        }
        if let Some(started) = started.lock().unwrap().take() {
            let _ = started.send(());
        }
        wait.lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        Ok(json!("old response"))
    }));
    let running = {
        let host = host.clone();
        tokio::spawn(async move {
            host.run_params(slow, "test".into(), Value::Null, false)
                .await
        })
    };
    received.await.unwrap();
    let result = tokio::time::timeout(
        Duration::from_millis(500),
        host.run_params(fast, "test".into(), json!({"value":42}), false),
    )
    .await;
    host.stop_plugins(vec!["example.slow".into()])
        .await
        .unwrap();
    release.send(()).unwrap();
    assert_eq!(result.unwrap().unwrap().result, 42);
    assert_eq!(running.await.unwrap().unwrap_err().code, "SCRIPT_STOPPED");
    host.stop_all().await.unwrap();
}
#[tokio::test]
async fn native_command_can_call_a_javascript_service() {
    let root = tempfile::tempdir().unwrap();
    let provider = plugin(
        root.path(),
        "example.provider",
        "export function activate(ctx){ctx.services.register('example.provider',{session:p=>({echo:p.name})});}",
        &[],
        json!([]),
        json!([{"id":"example.provider","version":"1.0.0","methods":["session"]}]),
    );
    let host = common::plugin_host(root.path());
    let source = root.path().join("consumer");
    fs::create_dir_all(source.join("bin")).unwrap();
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).join("../official-plugins");
    let mut descriptor: Value =
        serde_json::from_slice(&fs::read(project.join("example.services/plugin.json")).unwrap())
            .unwrap();
    descriptor["dependencies"] = json!([{"id":"example.provider","version":"^1"}]);
    descriptor["contributes"] = json!({"commands":[{"id":"forward","title":"Forward"}]});
    fs::copy(
        project.join(format!(
            "target/debug/{}hydrogen_example_services{}",
            std::env::consts::DLL_PREFIX,
            std::env::consts::DLL_SUFFIX
        )),
        source.join("bin/plugin.dll"),
    )
    .unwrap();
    fs::write(
        source.join("plugin.json"),
        serde_json::to_vec(&descriptor).unwrap(),
    )
    .unwrap();
    let native = manifest::load(&source).unwrap();
    host.set_plugins(vec![native.clone(), provider]);
    let report=host.run_params(native,"forward".into(),json!({"provider":"example.provider","service":"example.provider","method":"session","params":{"name":"hello"}}),false).await.unwrap();
    assert_eq!(report.result, json!({"echo":"hello"}));
    host.stop_all().await.unwrap();
}
#[tokio::test]
async fn services_cannot_inherit_proxy_permissions_and_cycles_are_rejected() {
    let root = tempfile::tempdir().unwrap();
    let native_host = common::plugin_host(root.path());
    let mut installed = hydrogen::plugins::manager::discover(root.path(), &[]).unwrap();
    let proxy = plugin(
        root.path(),
        "example.proxy",
        "export function activate(ctx){ctx.services.register('example.proxy',{forward:p=>ctx.services.call(p)});}",
        &["services.call", "java.read"],
        json!(["java.runtime", "example.consumer"]),
        json!([{"id":"example.proxy","version":"1.0.0","methods":["forward"]}]),
    );
    let consumer = plugin(
        root.path(),
        "example.consumer",
        "export function activate(ctx){ctx.commands.register('test',async p=>{try{return await ctx.services.call({provider:'example.proxy',service:'example.proxy',version:'^1',method:'forward',params:p})}catch(e){return e.code}});}",
        &["services.call"],
        json!(["example.proxy"]),
        json!([]),
    );
    // Dependency cycles are rejected even before execution.
    installed.extend([proxy.clone(), consumer.clone()]);
    native_host.set_plugins(installed);
    assert!(
        native_host
            .call_service(
                &Settings::default(),
                "example.proxy",
                "example.proxy",
                "forward",
                json!({})
            )
            .is_err()
    );
    let mut value: Value =
        serde_json::from_slice(&fs::read(proxy.root.join("plugin.json")).unwrap()).unwrap();
    value["dependencies"] = json!(["java.runtime"]);
    fs::write(
        proxy.root.join("plugin.json"),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
    let proxy = manifest::load(&proxy.root).unwrap();
    let mut installed = hydrogen::plugins::manager::discover(root.path(), &[]).unwrap();
    installed.extend([proxy, consumer.clone()]);
    native_host.set_plugins(installed);
    let result=native_host.run_params(consumer,"test".into(),json!({"provider":"java.runtime","service":"java.runtime","version":"^1","method":"parse","params":{"path":"java","output":""}}),false).await.unwrap().result;
    assert_eq!(result, "PERMISSION_DENIED");
    native_host.stop_all().await.unwrap();
}
#[tokio::test]
async fn hook_reentry_retains_the_waiting_command_and_validates_its_result() {
    let root = tempfile::tempdir().unwrap();
    let p = plugin(
        root.path(),
        "example.hook",
        "export function activate(ctx){ctx.hooks.register('beforeInstall',()=>({deny:'hold'}));ctx.commands.register('test',async()=>{try{return await ctx.instances.repair('instance')}catch(e){return e.code}});}",
        &["instances.repair", "hooks"],
        json!([]),
        json!([]),
    );
    let host = host(vec![p.clone()]);
    let routed = host.clone();
    host.set_operations(Arc::new(move |_, _, method, _| {
        if method == "lifecycle.stop" {
            return Ok(Value::Null);
        }
        routed.hooks(
            &Settings::default(),
            "beforeInstall",
            json!({"instanceId":"instance"}),
        )?;
        Ok(Value::Null)
    }));
    assert_eq!(
        host.run(p, "test".into(), false).await.unwrap().result,
        "HOOK_DENIED"
    );
    host.stop_all().await.unwrap();
}
#[test]
fn js_v1_is_rejected_and_panel_sessions_do_not_revoke_each_other() {
    let root = tempfile::tempdir().unwrap();
    let p = plugin(
        root.path(),
        "example.version",
        "export function activate(){}",
        &[],
        json!([]),
        json!([]),
    );
    let mut descriptor: Value =
        serde_json::from_slice(&fs::read(p.root.join("plugin.json")).unwrap()).unwrap();
    descriptor["schemaVersion"] = 2.into();
    descriptor["apiVersion"] = 1.into();
    fs::write(
        p.root.join("plugin.json"),
        serde_json::to_vec(&descriptor).unwrap(),
    )
    .unwrap();
    assert_eq!(
        manifest::load(&p.root).unwrap_err().code,
        "INCOMPATIBLE_PLUGIN"
    );
    let p =
        manifest::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/instance-tools"))
            .unwrap();
    let panels = Panels::default();
    let first = panels.open(p.clone(), "overview", 1).unwrap();
    let second = panels.open(p, "overview", 1).unwrap();
    panels.close(&first.session_id);
    assert!(panels.plugin(&second.session_id, 1).is_ok());
}

#[tokio::test]
async fn launch_hook_patches_conflict_and_hook_cleanup_keeps_its_identity() {
    let root = tempfile::tempdir().unwrap();
    let first = plugin(
        root.path(),
        "example.first",
        "export function activate(ctx){ctx.hooks.register('beforeLaunch',()=>({patch:{maxMemoryMib:4096}}));}",
        &["hooks"],
        json!([]),
        json!([]),
    );
    let second = plugin(
        root.path(),
        "example.second",
        "export function activate(ctx){ctx.hooks.register('beforeLaunch',()=>({patch:{maxMemoryMib:2048}}));}",
        &["hooks"],
        json!([]),
        json!([]),
    );
    let host = host(vec![first.clone()]);
    let cleaned = Arc::new(std::sync::Mutex::new(Vec::new()));
    let observed = cleaned.clone();
    host.set_operations(Arc::new(move |plugin, generation, method, _| {
        if method == "lifecycle.stop" {
            observed
                .lock()
                .unwrap()
                .push((plugin.manifest.id.clone(), generation));
        }
        Ok(Value::Null)
    }));
    assert_eq!(
        host.hooks(
            &Settings::default(),
            "beforeLaunch",
            json!({"instanceId":"test"})
        )
        .unwrap(),
        json!({"maxMemoryMib":4096})
    );
    host.set_plugins(vec![second, first]);
    assert_eq!(
        host.hooks(&Settings::default(), "beforeLaunch", json!({}))
            .unwrap_err()
            .code,
        "HOOK_CONFLICT"
    );
    host.stop_all().await.unwrap();
    let cleaned = cleaned.lock().unwrap();
    assert_eq!(cleaned.len(), 2);
    assert!(cleaned.iter().all(|(_, generation)| *generation > 0));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn instance_events_record_failures_and_continue_other_subscribers_after_a_hook() {
    let root = tempfile::tempdir().unwrap();
    let p = plugin(
        root.path(),
        "example.events",
        "export function activate(ctx){let count=0;ctx.hooks.register('beforeLaunch',async()=>{try{await ctx.storage.set('bad',true)}catch(e){return e.code==='HOOK_SIDE_EFFECT'?null:{deny:e.code}}});ctx.events.on('instance.changed',()=>{throw new Error('broken listener')});ctx.events.on('instance.changed',async()=>{await ctx.storage.set('seen',true);count++});ctx.commands.register('test',()=>count);}",
        &["hooks", "events", "storage"],
        json!([]),
        json!([]),
    );
    let host = host(vec![p.clone()]);
    host.set_operations(Arc::new(|_, _, _, _| Ok(Value::Null)));
    host.hooks(&Settings::default(), "beforeLaunch", json!({}))
        .unwrap();
    host.set_instances(vec![]);
    for _ in 0..100 {
        if !host.diagnostics(&p.manifest.id).is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        host.diagnostics(&p.manifest.id),
        vec!["instance.changed: broken listener"]
    );
    let mut count = 0;
    for _ in 0..100 {
        if let Ok(report) = host.run(p.clone(), "test".into(), false).await {
            count = report.result.as_i64().unwrap();
            if count == 1 {
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(count, 1);
    host.stop_all().await.unwrap();
}
#[tokio::test]
async fn activation_cannot_write_before_a_reload_commit() {
    let root = tempfile::tempdir().unwrap();
    let p = plugin(
        root.path(),
        "example.prepare",
        "export async function activate(ctx){await ctx.storage.set('x',1);}",
        &["storage"],
        json!([]),
        json!([]),
    );
    let host = host(vec![p.clone()]);
    host.set_operations(Arc::new(|_, _, _, _| {
        Err(ApiError::new("UNEXPECTED", "should not be reached"))
    }));
    assert!(host.activate(p).await.is_err());
}

fn contribute(mut plugin: Plugin, kind: &str, id: &str, hosts: &[&str]) -> Plugin {
    let path = plugin.root.join("plugin.json");
    let mut value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    value["contributes"]["providers"] =
        json!([{"id":id,"kind":kind,"title":id,"service":plugin.manifest.id,"version":"1.0.0"}]);
    if !hosts.is_empty() {
        value["scopes"] = json!({"http":hosts});
    }
    fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
    plugin = manifest::load(&plugin.root).unwrap();
    plugin
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn account_provider_is_consumed_by_native_profiles_without_receiving_host_paths() {
    let root = tempfile::tempdir().unwrap();
    let host = common::plugin_host(root.path());
    let provider = plugin(
        root.path(),
        "example.account",
        "export function activate(ctx){const check=p=>{if('root'in p||'settings'in p)throw Error('private context leaked');};ctx.services.register('example.account',{view:async p=>{check(p);let cycle;try{await ctx.accounts.list()}catch(e){cycle=e.code}if(cycle!=='SERVICE_CYCLE')throw Error('account reentry was not rejected');return{status:'signedIn',playerName:'Player',detail:null,skin:null}},signIn:p=>{check(p);return null},session:p=>{check(p);return{name:'Player',uuid:'00000000000000000000000000000001',access_token:'test',xuid:'',client_id:'',expires:0,offline:true}},signOut:()=>null});}",
        &["accounts.read"],
        json!([]),
        json!([{"id":"example.account","version":"1.0.0","methods":["view","signIn","session","signOut"]}]),
    );
    let provider = contribute(provider, "accountProvider", "account", &[]);
    let mut installed = hydrogen::plugins::manager::discover(root.path(), &[]).unwrap();
    installed.push(provider);
    host.set_plugins(installed);
    let mut settings = Settings::default();
    let profile=host.call_service(&settings,"account.manager","account.manager","profiles.addProvider",json!({"root":root.path(),"settings":settings,"provider":"example.account:account","name":"Example"})).unwrap();
    settings.account_mode = "example.account:account".into();
    host.call_service(
        &settings,
        "account.manager",
        "account.manager",
        "profiles.select",
        json!({"root":root.path(),"settings":settings,"id":profile["id"]}),
    )
    .unwrap();
    let view = host
        .call_service(
            &settings,
            "account.manager",
            "account.manager",
            "view",
            json!({"root":root.path(),"settings":settings}),
        )
        .unwrap();
    assert_eq!(view["mode"], "example.account:account");
    assert_eq!(view["status"], "signedIn");
    let session = host
        .call_service(
            &settings,
            "account.manager",
            "account.manager",
            "session",
            json!({"root":root.path(),"settings":settings}),
        )
        .unwrap();
    assert_eq!(session["result"]["name"], "Player");
    assert!(
        host.providers(&settings, "accountProvider")
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == "example.account:account")
    );
    host.stop_all().await.unwrap();
}
#[tokio::test]
async fn resource_provider_downloads_verified_bytes_and_imports_into_mods_with_cancellation() {
    use hydrogen::launcher::{mods, net, tasks::Tasks};
    use sha1::{Digest, Sha1};
    use std::io::{Read, Write};
    let root = tempfile::tempdir().unwrap();
    let host = common::plugin_host(root.path());
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/example.jar", listener.local_addr().unwrap());
    let data = b"isolated plugin resource fixture";
    let hash = format!("{:x}", Sha1::digest(data));
    let plan = json!([{"filename":"example.jar","url":url,"sha1":hash,"size":data.len()}]);
    let provider = plugin(
        root.path(),
        "example.resources",
        &format!(
            "export function activate(ctx){{ctx.services.register('example.resources',{{filters:()=>({{categories:[],gameVersions:[]}}),search:()=>({{hits:[{{project_id:'example',title:'Example'}}],total_hits:1}}),versions:()=>[{{id:'1',number:'1'}}],dependencies:()=>({{dependencies:[]}}),downloadPlan:()=>({plan})}});}}"
        ),
        &["http"],
        json!([]),
        json!([{"id":"example.resources","version":"1.0.0","methods":["filters","search","versions","dependencies","downloadPlan"]}]),
    );
    let provider = contribute(provider, "resourceSource", "source", &["127.0.0.1"]);
    let mut installed = hydrogen::plugins::manager::discover(root.path(), &[]).unwrap();
    installed.push(provider);
    host.set_plugins(installed);
    let settings = Settings::default();
    let search = host
        .provider_call(
            &settings,
            "resourceSource",
            "example.resources:source",
            "search",
            json!({}),
        )
        .await
        .unwrap();
    assert_eq!(search["hits"][0]["project_id"], "example");
    let plan = host
        .provider_call(
            &settings,
            "resourceSource",
            "example.resources:source",
            "downloadPlan",
            json!({"project":"example","versionId":"1","gameVersion":"1.20.1","loader":"fabric"}),
        )
        .await
        .unwrap();
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = [0; 4096];
        let _ = socket.read(&mut request).unwrap();
        write!(
            socket,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            data.len()
        )
        .unwrap();
        socket.write_all(data).unwrap();
    });
    let client = net::client(&host, &settings).unwrap();
    let task = Tasks::default().start("resource", None).unwrap();
    let result = net::task(
        &client,
        "minecraft.resources",
        "downloadPlan",
        json!({"root":root.path(),"plan":plan,"allowedHosts":["127.0.0.1"]}),
        Some(&task),
    )
    .await
    .unwrap();
    server.join().unwrap();
    let paths: Vec<std::path::PathBuf> = serde_json::from_value(result).unwrap();
    let game = root.path().join("game");
    fs::create_dir(&game).unwrap();
    mods::import(&host, &settings, &game, &paths, &task)
        .await
        .unwrap();
    assert_eq!(fs::read(game.join("mods/example.jar")).unwrap(), data);
    let cancelled = Tasks::default().start("resource", None).unwrap();
    cancelled.cancel.cancel();
    assert!(
        net::task(
            &client,
            "minecraft.resources",
            "downloadPlan",
            json!({"root":root.path(),"plan":plan,"allowedHosts":["127.0.0.1"]}),
            Some(&cancelled)
        )
        .await
        .is_err()
    );
    host.stop_all().await.unwrap();
}
