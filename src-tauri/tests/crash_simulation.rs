mod common;

use hydrogen::launcher::{net, storage::Settings};
use serde_json::{Value, json};
use std::{fs, io::Read, path::PathBuf, time::Duration};

/// Uses a real JVM child and all installed launch/process/diagnostic/crash providers.
/// Prepare the isolated repository with scripts/prepare-crash-simulation.ps1 first.
#[tokio::test]
#[ignore = "requires an explicitly prepared isolated Java simulation repository"]
async fn simulated_game_processes_exercise_the_complete_launch_and_report_pipeline() {
    let manifest_file = PathBuf::from(
        std::env::var_os("HYDROGEN_CRASH_SIMULATION")
            .expect("set HYDROGEN_CRASH_SIMULATION to simulation.json"),
    );
    let fixture: Value = serde_json::from_slice(&fs::read(manifest_file).unwrap()).unwrap();
    let root = PathBuf::from(fixture["data"].as_str().unwrap());
    let game = PathBuf::from(fixture["game"].as_str().unwrap());
    let config: Value =
        serde_json::from_slice(&fs::read(root.join("settings.json")).unwrap()).unwrap();
    let settings: Settings = serde_json::from_value(config["settings"].clone()).unwrap();
    let host = common::plugin_host(&root);
    let client = net::client(&host, &settings).unwrap();
    let normal = fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["scenario"] == "normal")
        .unwrap();
    let instance = json!({"id":normal["id"],"name":normal["name"],"directoryName":normal["name"],"gameVersion":"1.20.1","isInstalled":true,"overrides":{},"components":null});
    let start = |task: String| {
        let client = &client;
        let params = json!({"root":root,"gameRoot":game,"settings":settings,"instance":instance,"java":[],"taskId":task,"minimized":false});
        let markers = root.join(format!(
            "instances/{}/processes",
            normal["id"].as_str().unwrap()
        ));
        async move {
            let mut overlapping = false;
            let result = net::task_observed(
                client,
                "minecraft.launch",
                "launch",
                params,
                None,
                |progress| {
                    if progress["phase"] == "running" {
                        let own = markers.join(format!("{task}.json"));
                        assert!(own.is_file());
                        overlapping |= fs::read_dir(&markers)?
                            .filter_map(Result::ok)
                            .filter(|entry| {
                                entry
                                    .path()
                                    .extension()
                                    .is_some_and(|extension| extension == "json")
                            })
                            .count()
                            == 2;
                    }
                    Ok(())
                },
            )
            .await
            .unwrap();
            assert_eq!(result["exitCode"], 0);
            assert!(!markers.join(format!("{task}.json")).exists());
            overlapping
        }
    };
    let (first, second) = tokio::time::timeout(Duration::from_secs(20), async {
        tokio::join!(
            start(uuid::Uuid::new_v4().simple().to_string()),
            start(uuid::Uuid::new_v4().simple().to_string())
        )
    })
    .await
    .expect("concurrent launches timed out");
    assert!(
        first && second,
        "both JVMs must be running at the same time"
    );
    let mut results = Vec::new();
    for case in fixture["cases"].as_array().unwrap() {
        let scenario = case["scenario"].as_str().unwrap();
        let task = uuid::Uuid::new_v4().simple().to_string();
        let instance = json!({"id":case["id"],"name":case["name"],"directoryName":case["name"],"gameVersion":"1.20.1","isInstalled":true,
            "overrides":{"java":null,"maxMemoryMib":null,"jvmArguments":null},"components":null});
        let mut terminal = Value::Null;
        let mut running = false;
        tokio::time::timeout(Duration::from_secs(20), net::task_observed(&client, "minecraft.launch", "launch", json!({
            "root":root,"gameRoot":game,"settings":settings,"instance":instance,"java":[],"taskId":task,"minimized":false
        }), None, |progress| {
            running |= progress["phase"] == "running";
            if ["failed", "exited"].contains(&progress["phase"].as_str().unwrap_or("")) { terminal = progress.clone(); }
            Ok(())
        })).await.expect("simulated launch timed out").unwrap();
        assert!(!terminal.is_null(), "no terminal progress for {scenario}");
        let console = fs::read_to_string(root.join(format!("logs/game-{task}.log"))).unwrap();
        assert!(
            !root
                .join(format!(
                    "instances/{}/processes/{task}.json",
                    case["id"].as_str().unwrap()
                ))
                .exists()
        );
        if scenario == "normal" {
            assert_eq!(terminal["phase"], "exited");
            assert_eq!(terminal["exitCode"], 0);
            assert!(terminal["crashReport"].is_null());
            assert!(!root.join(format!("crash-reports/{task}")).exists());
            assert!(console.contains("HYDROGEN_SIMULATION_NORMAL_EXIT"));
            results.push(json!({"scenario":scenario,"passed":true,"exitCode":0,"taskId":task}));
            continue;
        }
        let expected = match scenario {
            "heap" => "heap-memory",
            "native-memory" => "native-memory",
            "metaspace" => "metaspace",
            "java-version" => "java-version",
            "graphics" => "graphics",
            "dependencies" => "mod-dependencies",
            "mixin" => "mixin",
            "library" => "missing-library",
            "native" => "native-crash",
            "jvm-arguments" => "jvm-arguments",
            _ => "unknown",
        };
        assert_eq!(terminal["phase"], "failed");
        assert_eq!(terminal["exitCode"], 1);
        assert!(
            terminal["crashError"].is_null(),
            "{}",
            terminal["crashError"]
        );
        let report = &terminal["crashReport"];
        assert!(
            report["causes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|cause| cause["id"] == expected),
            "{scenario}: {report}"
        );
        if scenario != "jvm-arguments" {
            assert!(running, "running state was never observed: {scenario}");
            assert!(console.contains(&format!("HYDROGEN_SIMULATION_EXIT_1 {scenario}")));
        } else {
            assert!(console.contains("Unrecognized VM option"));
        }
        if scenario == "native" {
            assert!(report["logPath"].as_str().unwrap().contains("hs_err_pid"));
        }
        let exported = host
            .call_service(
                &settings,
                "minecraft.crash",
                "minecraft.crash",
                "export",
                json!({"root":root,"taskId":task,"destination":fixture["root"]}),
            )
            .unwrap();
        let mut archive =
            zip::ZipArchive::new(fs::File::open(exported["path"].as_str().unwrap()).unwrap())
                .unwrap();
        let mut all = String::new();
        for index in 0..archive.len() {
            archive
                .by_index(index)
                .unwrap()
                .read_to_string(&mut all)
                .unwrap();
        }
        for secret in [
            "SIMULATED_ACCESS_SECRET_123",
            "SIMULATED_REFRESH_SECRET_456",
            "SIMULATED_BEARER_SECRET_789",
        ] {
            assert!(
                !all.contains(secret),
                "export leaked fixture token in {scenario}"
            );
        }
        assert!(
            !all.contains(case["directory"].as_str().unwrap()),
            "export leaked game path"
        );
        results.push(json!({"scenario":scenario,"passed":true,"exitCode":1,"cause":expected,"taskId":task,"archive":exported["path"]}));
        println!("PASS {scenario}: {expected}");
    }
    fs::write(
        PathBuf::from(fixture["root"].as_str().unwrap()).join("results.json"),
        serde_json::to_vec_pretty(&results).unwrap(),
    )
    .unwrap();
    host.stop_all().await.unwrap();
    assert_eq!(results.len(), 12);
}
