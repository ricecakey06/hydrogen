use hydrogen_plugin_sdk::contracts::{ApiError, Result};
use reqwest::{Client, Url};
use std::time::Duration;
pub fn trusted(url: &Url) -> bool {
    url.scheme() == "https"
        && url.port().is_none_or(|p| p == 443)
        && url.username().is_empty()
        && url.password().is_none()
        && url.host_str().is_some_and(|h| {
            [
                "piston-meta.mojang.com",
                "piston-data.mojang.com",
                "launchermeta.mojang.com",
                "launcher.mojang.com",
                "libraries.minecraft.net",
                "resources.download.minecraft.net",
                "textures.minecraft.net",
                "meta.fabricmc.net",
                "maven.fabricmc.net",
                "maven.minecraftforge.net",
                "files.minecraftforge.net",
                "maven.neoforged.net",
                "maven.quiltmc.org",
                "repo.maven.apache.org",
                "repo1.maven.org",
                "libraries.optifine.net",
                "api.modrinth.com",
                "cdn.modrinth.com",
                "cdn.jsdelivr.net",
                "raw.githubusercontent.com",
                "bmclapi2.bangbang93.com",
                "mirror.nju.edu.cn",
                "login.microsoftonline.com",
                "user.auth.xboxlive.com",
                "xsts.auth.xboxlive.com",
                "api.minecraftservices.com",
            ]
            .contains(&h)
                || h.ends_with(".bmclapi.com")
                || h.ends_with(".bmclapi.cn")
        })
}
pub fn client() -> Result<Client> {
    Client::builder()
        .user_agent(concat!("Hydrogen/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            let authentication = attempt.previous().iter().any(|u| {
                u.host_str().is_some_and(|h| {
                    [
                        "login.microsoftonline.com",
                        "user.auth.xboxlive.com",
                        "xsts.auth.xboxlive.com",
                        "api.minecraftservices.com",
                    ]
                    .contains(&h)
                })
            });
            if authentication || attempt.previous().len() >= 8 || !trusted(attempt.url()) {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(|_| ApiError::new("NETWORK_INIT", "无法初始化 HTTPS。"))
}
