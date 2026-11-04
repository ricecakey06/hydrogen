use hydrogen_plugin_sdk::{
    Result,
    serde_json::{Value, json},
};
use md5::{Digest, Md5};

struct Account {
    signed_in: bool,
}
impl Default for Account {
    fn default() -> Self {
        Self { signed_in: true }
    }
}

fn dispatch(account: &mut Account, request: &mut Value) -> Result {
    let params = &request["params"];
    let name = params["name"].as_str().unwrap_or("");
    match request["method"].as_str().unwrap_or("") {
        "plugin.describe" => Ok(json!({"id":"account.offline","abiVersion":1})),
        "signIn" => {
            account.signed_in = true;
            Ok(Value::Null)
        }
        "signOut" => {
            account.signed_in = false;
            Ok(Value::Null)
        }
        "view" => Ok(json!({"playerName":account.signed_in.then_some(name)})),
        "session" => {
            if !account.signed_in {
                return Err(("ACCOUNT_REQUIRED".into(), "请先登录离线账号。".into()));
            }
            if !(3..=16).contains(&name.len())
                || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            {
                return Err((
                    "INVALID_PLAYER_NAME".into(),
                    "离线名称需要 3–16 个英文字母、数字或下划线。".into(),
                ));
            }
            let mut hash: [u8; 16] = Md5::digest(format!("OfflinePlayer:{name}").as_bytes()).into();
            hash[6] = (hash[6] & 15) | 0x30;
            hash[8] = (hash[8] & 63) | 0x80;
            Ok(
                json!({"name":name,"uuid":uuid::Uuid::from_bytes(hash).simple().to_string(),
                "access_token":"0","xuid":"","client_id":"","expires":u64::MAX,"offline":true}),
            )
        }
        _ => Err(("UNKNOWN_METHOD".into(), "Unknown account method".into())),
    }
}

hydrogen_plugin_sdk::export_plugin!(Account, dispatch);
