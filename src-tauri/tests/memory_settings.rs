use hydrogen::launcher::{net, storage::Store};
use serde_json::{Value, json};

#[test]
fn gc_presets_reach_launch_arguments_without_changing_heap_or_game_arguments() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(root.path().join("data")).unwrap();
    let client = net::client(&store.plugins, &store.settings).unwrap();
    let mut params = json!({
        "game": {
            "metadata": {"id":"test", "type":"release", "mainClass":"net.minecraft.client.main.Main",
                "arguments":{"jvm":["-cp", "${classpath}"], "game":["--username", "${auth_player_name}"]}},
            "classpath":[root.path().join("client.jar")], "natives":[], "assets":root.path(),
            "libraries":root.path(), "asset_index":"test", "logging":null
        },
        "java":{"path":"unused", "version":"8", "major":8, "architecture":"x64"},
        "session":{"name":"Player", "uuid":"test", "access_token":"0", "xuid":"", "client_id":"", "expires":0, "offline":true},
        "settings":store.settings, "directory":root.path(), "natives":root.path()
    });
    let arguments = |params: &Value| -> Vec<String> {
        serde_json::from_value(
            client
                .service("minecraft.launch", "arguments", params.clone())
                .unwrap(),
        )
        .unwrap()
    };
    for mode in ["g1", "tunedG1", "preferZgc", "preferGenerationalZgc"] {
        params["settings"]["gcMode"] = json!(mode);
        let args = arguments(&params);
        let main = args
            .iter()
            .position(|arg| arg == "net.minecraft.client.main.Main")
            .unwrap();
        assert_eq!(
            args.iter()
                .filter(|arg| arg.as_str() == "-XX:+UseG1GC")
                .count(),
            1
        );
        assert!(args[..main].contains(&"-XX:+UseG1GC".into()));
        assert!(args[..main].contains(&format!("-Xmx{}M", store.settings.max_memory_mib)));
        assert_eq!(&args[main + 1..], ["--username", "Player"]);
        assert_eq!(
            args.contains(&"-XX:MaxGCPauseMillis=100".into()),
            mode == "tunedG1"
        );
    }
    params["settings"]["jvmArguments"] = json!("-XX:+UseParallelGC");
    let args = arguments(&params);
    assert!(!args.contains(&"-XX:+UseG1GC".into()));
    assert!(args.contains(&"-XX:+UseParallelGC".into()));
    params["settings"]["jvmArguments"] = json!("");

    // Optional real-runtime smoke check, using the platform's path-list separator.
    if let Some(paths) = std::env::var_os("HYDROGEN_TEST_JAVA") {
        for path in std::env::split_paths(&paths) {
            params["java"] = client
                .service("java.runtime", "inspect", json!({"path":path}))
                .unwrap();
            let args = arguments(&params);
            assert!(
                args.contains(&"-XX:+UseZGC".into()),
                "ZGC unavailable in {}",
                path.display()
            );
            if params["java"]["major"].as_u64().unwrap() >= 24 {
                assert!(!args.iter().any(|arg| arg.contains("ZGenerational")));
            }
            // A Java 17 binary advertised as 21 must reject the flag and fall back to regular ZGC.
            if params["java"]["major"] == 17 {
                params["java"]["major"] = json!(21);
                let args = arguments(&params);
                assert!(args.contains(&"-XX:+UseZGC".into()));
                assert!(!args.contains(&"-XX:+ZGenerational".into()));
            }
        }
    }
}
