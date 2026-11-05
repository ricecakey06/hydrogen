#![cfg(feature = "desktop")]

use hydrogen::launcher::storage::Settings;
use serde_json::json;

#[tokio::test]
async fn locale_persists_and_custom_language_packs_use_english_fallback() {
    use hydrogen::launcher::core::Launcher;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("data");
    let core = Launcher::open(Some(root.clone())).unwrap();
    assert_eq!(core.store.lock().unwrap().settings.locale, "zh-CN");
    let mut settings = core.store.lock().unwrap().settings.clone();
    settings.locale = "en-US".into();
    core.save_settings(settings.clone()).unwrap();
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("settings.json")).unwrap()).unwrap();
    assert_eq!(saved["settings"]["locale"], "en-US");
    let mut old_settings = saved["settings"].clone();
    old_settings.as_object_mut().unwrap().remove("locale");
    assert_eq!(
        serde_json::from_value::<Settings>(old_settings)
            .unwrap()
            .locale,
        "zh-CN"
    );
    settings.locale = "not-installed".into();
    assert!(core.save_settings(settings).is_err());
    std::fs::create_dir_all(root.join("language-packs")).unwrap();
    std::fs::write(root.join("language-packs/en-GB.json"),json!({"locale":"en-GB","name":"English (UK)","minecraftLanguage":"en_gb","messages":{"启动游戏":"Play"}}).to_string()).unwrap();
    let pack = core
        .plugins
        .call_service(
            &core.lease().settings,
            "i18n",
            "i18n",
            "bundle",
            json!({"root":root,"locale":"en-GB"}),
        )
        .unwrap();
    assert_eq!(pack["bundle"]["messages"]["启动游戏"], "Play");
    assert_eq!(pack["bundle"]["messages"]["保存"], "Save");
    assert_eq!(pack["languages"].as_array().unwrap().len(), 3);
    let game = root.join("separate game");
    core.plugins
        .call_service(
            &core.lease().settings,
            "i18n",
            "i18n",
            "applyGameLanguage",
            json!({"root":root,"directory":game,"locale":"en-GB","version":"1.20.1"}),
        )
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(game.join("options.txt")).unwrap(),
        "lang:en_gb\n"
    );
}
