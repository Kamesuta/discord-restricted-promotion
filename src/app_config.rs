use config::Config;
use serenity::model::id::{ChannelId, RoleId};
use std::error::Error;

#[derive(Debug, Default, serde::Deserialize, PartialEq)]
pub struct DiscordConfig {
    pub channels: Vec<ChannelId>,
    pub alert_sec: u64,
    pub required_message_length: usize,
    pub ignore_roles: Vec<RoleId>,
}

#[derive(Debug, Default, serde::Deserialize, PartialEq)]
pub struct AppConfig {
    pub discord: DiscordConfig,
}

impl AppConfig {
    pub fn load_config() -> Result<AppConfig, Box<dyn Error>> {
        // 設定ファイルを読み込む
        let config = Config::builder()
            // Add in `./Settings.toml`
            .add_source(config::File::with_name("config.toml"))
            // Add in settings from the environment (with a prefix of APP)
            // Eg.. `APP_DEBUG=1 ./target/app` would set the `debug` key
            .add_source(config::Environment::with_prefix("APP"))
            .build()?;
        // 設定ファイルをパース
        let app_config = config.try_deserialize::<AppConfig>()?;
        Ok(app_config)
    }
}
