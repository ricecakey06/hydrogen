mod profiles;
use hydrogen_plugin_services::{
    PluginHost, Settings,
    contracts::{ApiError, Result},
    invoke, storage, text,
};
use profiles::{Profiles, account_root};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::PathBuf};
struct Pending {
    handle: String,
    root: PathBuf,
    id: String,
    adding: bool,
    session: bool,
    settings: Settings,
}
#[derive(Default)]
struct State {
    tasks: BTreeMap<String, Pending>,
}
fn microsoft(method: &str, params: Value) -> Result<Value> {
    invoke("account.microsoft", "account.microsoft", method, params)
}
fn custom(id: &str, method: &str, params: Value) -> Result<Value> {
    invoke(
        "host.providers",
        "accountProvider",
        method,
        json!({"id":id,"params":params}),
    )
}
fn provider_available(settings: &Settings, id: &str) -> bool {
    if id.contains(':') {
        custom(id, "__available", Value::Null).is_ok_and(|v| v == true)
    } else {
        PluginHost.available(settings, id)
    }
}
fn profile_views(
    root: &std::path::Path,
    settings: &Settings,
    profiles: &Profiles,
) -> Result<Value> {
    let available = PluginHost.available(settings, "account.microsoft");
    profiles
        .profiles
        .iter()
        .map(|profile| {
            let mut view = serde_json::to_value(profile)?;
            // Read each identity's cached skin through its provider, without starting a login.
            view["skin"] = if available && profile.kind == "microsoft" {
                account_root(root, &profile.id)
                    .and_then(|directory| {
                        microsoft("view", json!({"root":directory,"settings":settings}))
                    })
                    .map(|account| account["skin"].clone())
                    .unwrap_or(Value::Null)
            } else {
                Value::Null
            };
            Ok(view)
        })
        .collect::<Result<Vec<_>>>()
        .map(Value::Array)
}
fn cleanup(root: &std::path::Path, id: &str) -> Result<()> {
    let directory = account_root(root, id)?;
    let path = storage::within(&directory, "credentials.bin")?;
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    let _ = microsoft("forget", json!({"root":directory}));
    // Remove only an empty per-account directory, never game or shared data.
    if directory != root {
        let _ = std::fs::remove_dir(directory);
    }
    Ok(())
}
impl State {
    fn finish_login(&self, pending: &Pending) -> Result<()> {
        let directory = account_root(&pending.root, &pending.id)?;
        let view = microsoft(
            "view",
            json!({"root":directory,"settings":pending.settings}),
        )?;
        if view["status"] != "signedIn" {
            return Ok(());
        }
        let uuid = text(&view, "uuid")?.to_owned();
        let name = text(&view, "playerName")?.to_owned();
        let mut profiles = Profiles::load(&pending.root, &pending.settings)?;
        let duplicate = profiles
            .profiles
            .iter()
            .find(|p| {
                p.kind == "microsoft" && p.uuid.as_deref() == Some(&uuid) && p.id != pending.id
            })
            .map(|p| p.id.clone());
        if let Some(id) = &duplicate {
            profiles.remove(id)?;
        }
        if let Some(profile) = profiles.profiles.iter_mut().find(|p| p.id == pending.id) {
            profile.name = name;
            profile.uuid = Some(uuid);
        } else {
            profiles.check_limit()?;
            profiles.profiles.push(profiles::Profile {
                id: pending.id.clone(),
                kind: "microsoft".into(),
                name,
                uuid: Some(uuid),
            });
        }
        profiles.select(&pending.id)?;
        profiles.save(&pending.root)?;
        if let Some(id) = duplicate {
            let _ = cleanup(&pending.root, &id);
        }
        Ok(())
    }
    fn call(&mut self, request: &mut Value) -> Result<Value> {
        let method = request["method"].as_str().unwrap_or("").to_owned();
        let p = &mut request["params"];
        if method == "plugin.describe" {
            return Ok(json!({"id":"account.manager","abiVersion":1}));
        }
        if method == "reset" || method == "plugin.stop" {
            for (_, task) in std::mem::take(&mut self.tasks) {
                let _ = microsoft("cancel", json!({"handle":task.handle}));
                if task.adding {
                    let _ = cleanup(&task.root, &task.id);
                }
            }
            return microsoft("reset", Value::Null);
        }
        if method == "poll" || method == "cancel" {
            let id = text(p, "handle")?.to_owned();
            let task = self
                .tasks
                .get(&id)
                .ok_or_else(|| ApiError::new("UNKNOWN_TASK", "账号任务已结束。"))?;
            let result = microsoft(&method, json!({"handle":task.handle}));
            if method == "cancel" || result.as_ref().map_or(true, |v| v["ready"] == true) {
                let task = self.tasks.remove(&id).unwrap();
                if method == "poll" && result.is_ok() && !task.session {
                    if let Err(error) = self.finish_login(&task) {
                        if task.adding {
                            let _ = cleanup(&task.root, &task.id);
                        }
                        return Err(error);
                    }
                } else if task.adding {
                    cleanup(&task.root, &task.id)?;
                }
            }
            return result;
        }
        let settings: Settings = serde_json::from_value(p["settings"].clone()).unwrap_or_default();
        let root = PathBuf::from(text(p, "root")?);
        let mut profiles = Profiles::load(&root, &settings)?;
        let offline = settings.offline_selected();
        let selected = profiles.selected_mode(&settings).cloned();
        let provider = if settings.account_mode.contains(':') {
            settings.account_mode.as_str()
        } else if offline {
            "account.offline"
        } else {
            "account.microsoft"
        };
        let available = provider_available(&settings, provider);
        match method.as_str() {
            "profiles.list" => Ok(
                json!({"profiles":profile_views(&root, &settings, &profiles)?,"selectedProfileId":selected.map(|p|p.id)}),
            ),
            "profiles.hasCredentials" => Ok(json!(profiles.selected(false).is_some_and(|p| {
                account_root(&root, &p.id)
                    .and_then(|r| storage::within(&r, "credentials.bin"))
                    .is_ok_and(|p| p.is_file())
            }))),
            "profiles.addProvider" => {
                let id = text(p, "provider")?;
                if !provider_available(&settings, id) {
                    return Err(ApiError::new("UNKNOWN_PROVIDER", "账号提供者不可用。"));
                }
                let profile = profiles.add_provider(id, text(p, "name")?)?;
                profiles.save(&root)?;
                Ok(serde_json::to_value(profile)?)
            }
            "profiles.addOffline" => {
                if !PluginHost.available(&settings, "account.offline") {
                    return Err(ApiError::new("PLUGIN_DISABLED", "请先启用离线启动插件。"));
                }
                let profile = profiles.add_offline(text(p, "name")?)?;
                profiles.save(&root)?;
                Ok(serde_json::to_value(profile)?)
            }
            "profiles.select" => {
                let profile = profiles.select(text(p, "id")?)?;
                let id = if profile.kind.contains(':') {
                    profile.kind.clone()
                } else {
                    format!("account.{}", profile.kind)
                };
                if !provider_available(&settings, &id) {
                    return Err(ApiError::new("PLUGIN_DISABLED", "此档案的账号插件未启用。"));
                }
                profiles.save(&root)?;
                if profile.kind == "offline" {
                    invoke(&id, &id, "signIn", json!({"name":profile.name}))?;
                }
                Ok(serde_json::to_value(profile)?)
            }
            "profiles.move" => {
                let id = text(p, "id")?;
                let index = profiles
                    .profiles
                    .iter()
                    .position(|profile| profile.id == id)
                    .ok_or_else(|| ApiError::new("PROFILE_NOT_FOUND", "此档案已不存在。"))?;
                let target = text(p, "target")?;
                let anchor = profiles
                    .profiles
                    .iter()
                    .position(|profile| profile.id == target)
                    .ok_or_else(|| ApiError::new("PROFILE_NOT_FOUND", "此档案已不存在。"))?;
                let destination =
                    anchor + usize::from(p["after"] == true) - usize::from(index < anchor);
                if index != anchor && index != destination {
                    let profile = profiles.profiles.remove(index);
                    profiles.profiles.insert(destination, profile);
                    profiles.save(&root)?;
                }
                Ok(Value::Null)
            }
            "profiles.delete" => {
                let profile = profiles.get(text(p, "id")?)?.clone();
                if text(p, "confirmation")? != profile.name {
                    return Err(ApiError::new(
                        "CONFIRMATION_REQUIRED",
                        "请确认要删除的档案。",
                    ));
                }
                let credential = if profile.kind == "microsoft" {
                    let directory = account_root(&root, &profile.id)?;
                    let path = storage::within(&directory, "credentials.bin")?;
                    let saved = if path.exists() {
                        Some(std::fs::read(&path)?)
                    } else {
                        None
                    };
                    cleanup(&root, &profile.id)?;
                    Some((path, saved))
                } else {
                    None
                };
                profiles.remove(&profile.id)?;
                if let Err(error) = profiles.save(&root) {
                    if let Some((path, Some(saved))) = credential {
                        storage::atomic_write(&path, &saved)?;
                    }
                    return Err(error);
                }
                Ok(Value::Null)
            }
            "view" => {
                let mut view = if !available {
                    json!({"status":"unavailable","playerName":null,"detail":"所选账号提供者不可用。","skin":null})
                } else if provider.contains(':') {
                    if let Some(profile) = &selected {
                        custom(provider, "view", json!({"profileId":profile.id}))?
                    } else {
                        json!({"status":"signedOut","playerName":null,"detail":null,"skin":null})
                    }
                } else if let Some(profile) = &selected {
                    invoke(
                        provider,
                        provider,
                        "view",
                        if offline {
                            json!({"name":profile.name})
                        } else {
                            json!({"settings":settings,"root":account_root(&root,&profile.id)?,"signingIn":p["signingIn"]})
                        },
                    )?
                } else {
                    json!({"status":"signedOut","playerName":null,"detail":null,"skin":null})
                };
                if offline {
                    view["status"] = if view["playerName"].is_string() {
                        "signedIn"
                    } else if available {
                        "signedOut"
                    } else {
                        "unavailable"
                    }
                    .into();
                    view["skin"] = Value::Null;
                }
                // Legacy credentials have no cached session/skin yet; reuse the verified profile name.
                if view["status"] == "cached" && view["playerName"].is_null() {
                    if let Some(profile) = &selected {
                        if profile.uuid.is_some() {
                            view["playerName"] = profile.name.clone().into();
                        }
                    }
                }
                if p["signingIn"] == true {
                    view["status"] = "signingIn".into();
                }
                view["mode"] = settings.account_mode.clone().into();
                view["offlineAvailable"] =
                    PluginHost.available(&settings, "account.offline").into();
                view["selectedProfileId"] = json!(selected.map(|p| p.id));
                view["profiles"] = profile_views(&root, &settings, &profiles)?;
                Ok(view)
            }
            "failure" => {
                if let Some(profile) = profiles.selected(false) {
                    microsoft(
                        "failure",
                        json!({"root":account_root(&root,&profile.id)?,"message":p["message"]}),
                    )?;
                }
                Ok(Value::Null)
            }
            "signOut" => {
                if let Some(profile) = selected {
                    if provider.contains(':') {
                        return custom(provider, "signOut", json!({"profileId":profile.id}));
                    }
                    invoke(
                        provider,
                        provider,
                        "signOut",
                        if offline {
                            json!({})
                        } else {
                            json!({"root":account_root(&root,&profile.id)?,"settings":settings})
                        },
                    )?;
                }
                Ok(Value::Null)
            }
            "signIn" | "session" => {
                if !available {
                    return Err(ApiError::new("PLUGIN_DISABLED", "账号插件未启用。"));
                }
                if provider.contains(':') {
                    let profile = selected
                        .ok_or_else(|| ApiError::new("ACCOUNT_REQUIRED", "请先添加档案。"))?;
                    let result = custom(
                        provider,
                        &method,
                        json!({"profileId":profile.id,"restore":p["restore"]}),
                    )?;
                    return Ok(json!({"ready":true,"result":result}));
                }
                if offline {
                    let profile = selected.ok_or_else(|| {
                        ApiError::new("ACCOUNT_REQUIRED", "请在档案管理中添加离线档案。")
                    })?;
                    return Ok(
                        json!({"ready":true,"result":invoke(provider,provider,&method,json!({"name":profile.name}))?}),
                    );
                }
                if self
                    .tasks
                    .values()
                    .any(|task| method != "session" || !task.session)
                {
                    return Err(ApiError::new("ACCOUNT_BUSY", "请等待当前登录完成。"));
                }
                let adding = method == "signIn" && (p["newProfile"] == true || selected.is_none());
                if adding {
                    profiles.check_limit()?;
                }
                let id = if adding {
                    uuid::Uuid::new_v4().to_string()
                } else if let Some(profile) = selected {
                    profile.id
                } else {
                    return if p["restore"] == true {
                        Ok(json!({"ready":true,"result":null}))
                    } else {
                        Err(ApiError::new("ACCOUNT_REQUIRED", "请先添加正版档案。"))
                    };
                };
                let handle = microsoft(
                    &method,
                    json!({"root":account_root(&root,&id)?,"settings":settings,"restore":p["restore"]}),
                )?;
                let task_id = uuid::Uuid::new_v4().to_string();
                self.tasks.insert(
                    task_id.clone(),
                    Pending {
                        handle: handle
                            .as_str()
                            .ok_or_else(|| ApiError::new("INVALID_RESPONSE", "无效账号任务。"))?
                            .into(),
                        root,
                        id,
                        adding,
                        session: method == "session",
                        settings,
                    },
                );
                Ok(json!({"ready":false,"handle":task_id}))
            }
            _ => Err(ApiError::new("UNKNOWN_METHOD", "未知账号管理方法。")),
        }
    }
}
fn dispatch(state: &mut State, request: &mut Value) -> hydrogen_plugin_sdk::Result {
    state.call(request).map_err(|e| (e.code, e.message))
}
hydrogen_plugin_sdk::export_plugin!(State, dispatch);
