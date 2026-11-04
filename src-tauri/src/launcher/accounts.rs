//! Account DTOs and cancellable service transport. Provider state lives in installed plugins.
use super::{net, storage::Settings, tasks::Job};
use crate::{
    contracts::{ApiError, Result},
    plugins::host::PluginHost,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{path::Path, sync::Mutex};
#[derive(Clone, Serialize, Deserialize)]
pub struct Session {
    pub name: String,
    pub uuid: String,
    pub access_token: String,
    pub xuid: String,
    pub client_id: String,
    pub expires: u64,
    pub offline: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub uuid: Option<String>,
    #[serde(default)]
    pub skin: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountView {
    #[serde(default)]
    pub profiles: Vec<Profile>,
    #[serde(default)]
    pub selected_profile_id: Option<String>,
    pub status: String,
    pub player_name: Option<String>,
    pub mode: String,
    pub offline_available: bool,
    pub detail: Option<String>,
    pub skin: Option<String>,
}

impl AccountView {
    pub fn needs_restore(&self) -> bool {
        self.status != "signedIn" || self.skin.is_none()
    }
}

pub struct Accounts {
    plugins: PluginHost,
    settings: Mutex<Settings>,
}
impl Accounts {
    pub fn new(plugins: PluginHost) -> Self {
        Self {
            plugins,
            settings: Mutex::new(Settings::default()),
        }
    }
    fn call(&self, settings: &Settings, method: &str, mut params: Value) -> Result<Value> {
        *self.settings.lock().unwrap() = settings.clone();
        params["settings"] = serde_json::to_value(settings)?;
        self.plugins.call_service(
            settings,
            "account.manager",
            "account.manager",
            method,
            params,
        )
    }
    pub fn view(&self, root: &Path, settings: &Settings, signing_in: bool) -> AccountView {
        self.call(
            settings,
            "view",
            json!({"root":root,"signingIn":signing_in}),
        )
        .and_then(|v| Ok(serde_json::from_value(v)?))
        .unwrap_or_else(|e| AccountView {
            status: "unavailable".into(),
            profiles: vec![],
            selected_profile_id: None,
            player_name: None,
            mode: settings.account_mode.clone(),
            offline_available: false,
            detail: Some(e.message),
            skin: None,
        })
    }
    pub fn reset(&self) {
        let settings = self.settings.lock().unwrap().clone();
        let _ = self.call(&settings, "reset", json!({}));
    }
    pub fn failure(&self, root: &Path, settings: &Settings, message: &str) {
        let _ = self.call(settings, "failure", json!({"root":root,"message":message}));
    }
    pub fn sign_out(&self, root: &Path, settings: &Settings) -> Result<()> {
        self.call(settings, "signOut", json!({"root":root}))?;
        Ok(())
    }
    async fn task(
        &self,
        root: &Path,
        settings: &Settings,
        method: &str,
        restore: bool,
        new_profile: bool,
        job: Option<&Job>,
    ) -> Result<Value> {
        if let Some(job) = job {
            job.check()?;
        }
        let begin = self.call(
            settings,
            method,
            json!({"root":root,"restore":restore,"newProfile":new_profile}),
        )?;
        if begin["ready"] == true {
            return Ok(begin["result"].clone());
        }
        struct Pending<'a> {
            accounts: &'a Accounts,
            settings: &'a Settings,
            id: String,
        }
        impl Drop for Pending<'_> {
            fn drop(&mut self) {
                let _ = self
                    .accounts
                    .call(self.settings, "cancel", json!({"handle":self.id}));
            }
        }
        let pending = Pending {
            accounts: self,
            settings,
            id: begin["handle"]
                .as_str()
                .ok_or_else(|| ApiError::new("INVALID_RESPONSE", "无效账号任务。"))?
                .into(),
        };
        loop {
            if let Some(job) = job {
                job.check()?;
            }
            let response = self.call(settings, "poll", json!({"handle":pending.id}))?;
            if response["ready"] == true {
                return Ok(response["result"].clone());
            }
            if let Some(job) = job
                && let (Some(phase), Some(message)) = (
                    response["progress"]["phase"].as_str(),
                    response["progress"]["message"].as_str(),
                )
            {
                job.phase(phase, message);
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }
    pub async fn sign_in(
        &self,
        _client: &net::Client,
        root: &Path,
        settings: &Settings,
        job: &Job,
        restore: bool,
    ) -> Result<()> {
        self.task(root, settings, "signIn", restore, false, Some(job))
            .await?;
        Ok(())
    }
    pub fn profiles(
        &self,
        root: &Path,
        settings: &Settings,
        method: &str,
        mut fields: Value,
    ) -> Result<Value> {
        fields["root"] = json!(root);
        self.call(settings, method, fields)
    }
    pub async fn add_microsoft(&self, root: &Path, settings: &Settings, job: &Job) -> Result<()> {
        self.task(root, settings, "signIn", false, true, Some(job))
            .await?;
        Ok(())
    }
    pub async fn session(
        &self,
        _client: &net::Client,
        root: &Path,
        settings: &Settings,
    ) -> Result<Session> {
        Ok(serde_json::from_value(
            self.task(root, settings, "session", false, false, None)
                .await?,
        )?)
    }
}
