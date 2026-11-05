mod common;
use hydrogen::launcher::{minecraft, net};
use serde_json::json;

#[tokio::test]
async fn metadata_rules_and_network_boundaries() {
    let root = tempfile::tempdir().unwrap();
    let host = common::plugin_host(root.path());
    let client = net::client(&host, &hydrogen::launcher::storage::Settings::default()).unwrap();
    for id in [
        "1.0",
        "1.7.10",
        "1.12.2",
        "1.17.1",
        "1.18",
        "1.21.1",
        "26.1",
        "24w14a",
        "1.18-pre1",
        "1.18-rc1",
    ] {
        assert!(minecraft::supported(&client, id).unwrap());
    }
    for id in ["../1.21.1", "", "..", "a/b", "a\\b"] {
        assert!(!minecraft::supported(&client, id).unwrap());
    }
    assert!(
        !minecraft::allows(
            &client,
            &json!({"rules":[{"action":"allow","features":{"is_demo_user":true}}]}),
            "x64",
            false
        )
        .unwrap()
    );
    assert!(
        minecraft::allows(
            &client,
            &json!({"rules":[{"action":"allow","features":{"has_custom_resolution":true}}]}),
            "x64",
            true
        )
        .unwrap()
    );
    for url in [
        "http://libraries.minecraft.net/file",
        "https://localhost/",
        "https://127.0.0.1/a",
        "https://user@libraries.minecraft.net/file",
        "https://libraries.minecraft.net:444/file",
    ] {
        assert!(!net::trusted(&client, &reqwest::Url::parse(url).unwrap()).unwrap());
    }
    assert_eq!(
        net::source_url(
            &client,
            "https://resources.download.minecraft.net/aa/hash",
            true
        )
        .unwrap(),
        "https://bmclapi2.bangbang93.com/assets/aa/hash"
    );
    assert_eq!(
        net::source_url(
            &client,
            "https://libraries.minecraft.net/org/example.jar",
            true
        )
        .unwrap(),
        "https://bmclapi2.bangbang93.com/maven/org/example.jar"
    );
}

#[cfg(feature = "desktop")]
#[tokio::test]
async fn oauth_callback_requires_exact_state_host_and_unique_parameters() {
    let root = tempfile::tempdir().unwrap();
    let host = common::plugin_host(root.path());
    let callback =
        |request: &str, state: &str, port: u16| -> hydrogen::contracts::Result<Option<String>> {
            Ok(serde_json::from_value(host.call_service(
                &hydrogen::launcher::storage::Settings::default(),
                "account.microsoft",
                "account.microsoft",
                "callback",
                serde_json::json!({"request":request,"state":state,"port":port}),
            )?)?)
        };
    let valid = "GET /?state=nonce&code=secret-code HTTP/1.1\r\nHost: localhost:12345\r\n\r\n";
    assert_eq!(
        callback(valid, "nonce", 12345).unwrap(),
        Some("secret-code".into())
    );
    assert!(callback(valid, "wrong", 12345).is_err());
    assert!(
        callback(
            &valid.replace("state=nonce&", "state=nonce&state=wrong&"),
            "nonce",
            12345
        )
        .is_err()
    );
    assert!(
        callback(
            &valid.replace("localhost:12345", "attacker.example"),
            "nonce",
            12345
        )
        .is_err()
    );
    assert!(callback(&valid.replace("/?state", "/other?state"), "nonce", 12345).is_err());
}
