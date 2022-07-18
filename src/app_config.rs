use anyhow::{Context as _, Result};
use config::Config;
use serenity::model::id::{ChannelId, RoleId};

/// 同じ鯖の宣伝を禁止する設定
#[derive(Debug, Default, serde::Deserialize, PartialEq, Clone)]
pub struct BanPeriodConfig {
    /// 同じ鯖の宣伝を禁止する日数
    pub day: i64,
    /// 同じユーザーが同じ鯖の宣伝を禁止する日数
    pub day_per_user: i64,
    /// 同じユーザーが同じ鯖の宣伝を再投稿できる分数
    pub min_per_user_start: i64,
}

#[derive(Debug, Default, serde::Deserialize, PartialEq, Clone)]
pub struct MessageConfig {
    /// 言語
    pub lang: String,
    /// 警告の絵文字
    pub alert_emoji: String,
    /// 無期限招待リンクの作成方法紹介ページURL
    pub no_expiration_invite_link_guide: String,
}

#[derive(Debug, Default, serde::Deserialize, PartialEq, Clone)]
pub struct DiscordConfig {
    /// Botが動作するチャンネルID
    pub channels: Vec<ChannelId>,
    /// 警告を表示する秒数
    pub alert_sec: u64,
    /// 必要なメッセージの長さ
    pub required_message_length: usize,
    /// 警告を無視するロールID
    pub ignore_roles: Vec<RoleId>,
}

/// アプリケーションの設定
#[derive(Debug, Default, serde::Deserialize, PartialEq, Clone)]
pub struct AppConfig {
    /// Discordの設定
    pub discord: DiscordConfig,
    /// 同じ鯖の宣伝を禁止する設定
    pub ban_period: BanPeriodConfig,
    /// メッセージ
    pub message: MessageConfig,
}

impl AppConfig {
    /// 設定を読み込む
    pub fn load_config() -> Result<AppConfig> {
        // 設定ファイルを読み込む
        let config = Config::builder()
            // Add in `./Settings.toml`
            .add_source(config::File::with_name("bot/config.toml"))
            // Add in settings from the environment (with a prefix of APP)
            // Eg.. `APP_DEBUG=1 ./target/app` would set the `debug` key
            .add_source(config::Environment::with_prefix("APP"))
            .build()?;
        // 設定ファイルをパース
        let app_config = config
            .try_deserialize::<AppConfig>()
            .context("設定ファイルの読み込みに失敗")?;
        Ok(app_config)
    }
}
