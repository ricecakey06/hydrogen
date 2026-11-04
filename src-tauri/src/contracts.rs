use serde::{Deserialize, Serialize};

pub type Result<T> = std::result::Result<T, ApiError>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    pub correlation_id: String,
}

impl ApiError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable: false,
            correlation_id: uuid::Uuid::new_v4().to_string(),
        }
    }
}
impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for ApiError {}
impl From<std::io::Error> for ApiError {
    fn from(_: std::io::Error) -> Self {
        Self::new("IO_ERROR", "文件操作失败，请检查路径、权限与剩余空间。")
    }
}
impl From<serde_json::Error> for ApiError {
    fn from(_: serde_json::Error) -> Self {
        Self::new("INVALID_JSON", "JSON 格式不正确或字段不兼容。")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceInfo {
    pub id: String,
    pub name: String,
    pub game_version: String,
    pub is_installed: bool,
}

// P0 fixtures are deliberately separate from real LauncherData and never marked installed.
pub fn prototype_instances() -> Vec<InstanceInfo> {
    [
        ("prototype-vanilla", "Minecraft 原版", "1.21.1"),
        ("prototype-fabric", "Fabric 实例（布局样本）", "1.21.1"),
    ]
    .map(|(id, name, version)| InstanceInfo {
        id: id.into(),
        name: name.into(),
        game_version: version.into(),
        is_installed: false,
    })
    .to_vec()
}
