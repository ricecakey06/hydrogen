use super::{
    core::Launcher,
    net, repository,
    storage::{self, Components, Instance},
    tasks::{InstallationRetry, Job},
};
use crate::contracts::{ApiError, Result};
use serde_json::{Value, json};
use std::{fs, path::PathBuf, sync::Arc};

const SERVICE: &str = "minecraft.modpacks";

struct Stage(PathBuf);
impl Drop for Stage {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

impl Launcher {
    pub async fn modpack_directories(&self, id: &str) -> Result<Value> {
        self.require(SERVICE)?;
        let _lease = self.lease();
        let (instance, game) = {
            let store = self.store.lock().unwrap();
            (store.instance(id)?, store.game_dir(id)?)
        };
        net::task(
            &self.client,
            SERVICE,
            "list",
            json!({"game":game,"directory":instance.directory_name}),
            None,
        )
        .await
    }

    pub fn export_modpack(
        self: &Arc<Self>,
        id: &str,
        destination: PathBuf,
        options: Value,
    ) -> Result<String> {
        self.require(SERVICE)?;
        let lease = self.lease();
        let lock = self.instance_lock(id)?;
        let (instance, game) = {
            let store = self.store.lock().unwrap();
            (store.instance(id)?, store.game_dir(id)?)
        };
        if !instance.info.is_installed {
            return Err(ApiError::new("NOT_INSTALLED", "此实例尚未安装完成。"));
        }
        let directory = instance.directory_name.as_deref().unwrap_or(id);
        let loader = if let Some(components) = &instance.components {
            let kind = if components.kind.is_empty() {
                "fabric"
            } else {
                &components.kind
            };
            json!({"kind":kind,"loader":components.loader})
        } else if instance.loader_name.is_empty() {
            Value::Null
        } else {
            let version = repository::load(&self.client, &lease.game_root, directory)?;
            let kind = version.loader.to_ascii_lowercase();
            if !matches!(kind.as_str(), "fabric" | "forge" | "neoforge" | "quilt")
                || version.loader_version.is_none()
            {
                return Err(ApiError::new(
                    "INVALID_MODPACK",
                    "无法确定可导出的模组加载器版本。",
                ));
            }
            json!({"kind":kind,"loader":version.loader_version})
        };
        let job = self.tasks.start("modpackExport", Some(id))?;
        let task_id = label(&job, &instance.info.name, &instance.info.game_version);
        let client = self.client.clone();
        let params = json!({"game":game,"directory":directory,"destination":destination,"gameVersion":instance.info.game_version,"components":loader,"options":options});
        tauri::async_runtime::spawn(async move {
            let (_lease, _lock) = (lease, lock);
            match net::task(&client, SERVICE, "export", params, Some(&job)).await {
                Ok(output) => job.phase("completed", output.as_str().unwrap_or_default()),
                Err(error) => job.finish(Err(error)),
            }
        });
        Ok(task_id)
    }

    pub async fn import_modpack(self: &Arc<Self>, source: PathBuf) -> Result<String> {
        self.start_modpack_import(source, None).await
    }

    pub async fn retry_installation(self: &Arc<Self>, id: &str) -> Result<String> {
        let _lease = self.lease();
        match self.tasks.installation_retry(id)? {
            InstallationRetry::Instance(instance) => self.install(instance),
            InstallationRetry::Modpack(source) => self.start_modpack_import(source, Some(id)).await,
        }
    }

    async fn start_modpack_import(
        self: &Arc<Self>,
        source: PathBuf,
        retry: Option<&str>,
    ) -> Result<String> {
        self.require(SERVICE)?;
        self.require("minecraft.install")?;
        let lease = self.lease();
        let preview = self
            .client
            .service_async(SERVICE, "preview", json!({"source":source}))
            .await?;
        let version: String = serde_json::from_value(preview["gameVersion"].clone())?;
        let components: Option<Components> = serde_json::from_value(preview["components"].clone())?;
        if let Some(components) = &components {
            self.require(if matches!(components.kind.as_str(), "fabric" | "quilt") {
                "minecraft.fabric"
            } else {
                "minecraft.forge"
            })?;
        }
        let name = {
            let store = self.store.lock().unwrap();
            store.writable()?;
            import_name(&store, super::text(&preview, "name")?)?
        };
        let job = self.tasks.start_modpack_import(source.clone(), retry)?;
        let task_id = label(&job, &name, &version);
        let core = self.clone();
        tauri::async_runtime::spawn(async move {
            let result = async {
                let stage_id = uuid::Uuid::new_v4().simple().to_string();
                let path = storage::within(&lease.game_root, &format!(".hydrogen-pack-{stage_id}"))?;
                fs::create_dir_all(&lease.game_root)?;
                fs::create_dir(&path)?;
                let stage = Stage(path);
                job.phase("resolving", "正在读取整合包清单…");
                net::task(&core.client, SERVICE, "prepare", json!({
                    "source":source,"stage":stage.0,"directory":name,"optional":false,
                    "downloadsEnabled":lease.settings.enabled("minecraft.download"),"preview":preview
                }), Some(&job)).await?;
                let state = core.store.lock().unwrap().state();
                // Once the payload is committed, a failed game install can be retried in Downloads.
                job.begin_commit("正在创建整合包实例…")?;
                let instance: Instance = serde_json::from_value(net::task(&core.client, "minecraft.install", "create",
                    json!({"state":state,"version":version,"name":name,"components":components,"packStage":stage_id}), None).await?)?;
                let id = instance.info.id.clone();
                job.state.lock().unwrap().instance_id = Some(id.clone());
                let _lock = core.instance_lock(&id)?;
                core.install_instance(&instance, &job).await
            }.await;
            job.finish(result);
        });
        Ok(task_id)
    }
}

fn import_name(store: &storage::Store, title: &str) -> Result<String> {
    // Both components of versions/<name>/<name>.json share the 240-byte path limit.
    const MAX_NAME_BYTES: usize = (240 - "versions//.json".len()) / 2;
    let mut title: String = title
        .trim()
        .chars()
        .take(64)
        .map(|ch| {
            if ch.is_control() || r#"<>:"/\|?*%"#.contains(ch) {
                '-'
            } else {
                ch
            }
        })
        .collect();
    title.truncate(title.floor_char_boundary(MAX_NAME_BYTES));
    let title = title.trim_end_matches(['.', ' ']);
    let stem = title
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end()
        .to_uppercase();
    let special_device = matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || (stem.starts_with("COM") || stem.starts_with("LPT"))
        && stem[3..].chars().count() == 1
        && stem[3..].contains(|ch: char| ch.is_ascii_digit() || "¹²³".contains(ch));
    let title = if crate::plugins::manifest::safe_relative(title)
        && !special_device
        && !title.starts_with(".hydrogen-")
    {
        title
    } else {
        "Modpack"
    };
    let instances = store.instances()?;
    for index in 1..=instances.len() + 100 {
        let name = if index == 1 {
            title.to_owned()
        } else {
            let suffix = format!(" ({index})");
            let end = title.floor_char_boundary(MAX_NAME_BYTES - suffix.len());
            let shortened: String = title[..end].chars().take(64 - suffix.len()).collect();
            format!("{}{suffix}", shortened.trim_end_matches(['.', ' ']))
        };
        if !instances
            .iter()
            .any(|instance| instance.info.name.eq_ignore_ascii_case(&name))
            && !storage::within(&store.game_root, &format!("versions/{name}"))?.exists()
        {
            return Ok(name);
        }
    }
    Err(ApiError::new(
        "DUPLICATE_NAME",
        "无法为整合包分配实例名称。",
    ))
}

fn label(job: &Job, name: &str, version: &str) -> String {
    let mut state = job.state.lock().unwrap();
    state.instance_name = Some(name.into());
    state.game_version = Some(version.into());
    state.id.clone()
}
