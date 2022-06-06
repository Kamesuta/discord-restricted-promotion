use chrono::prelude::*;
use chrono_tz::Tz::Japan;
use config::Config;
use regex::Regex;
use std::{env, error::Error};
use tokio::time::{sleep, Duration};

use serenity::async_trait;
use serenity::framework::standard::StandardFramework;
use serenity::model::channel::Message;
use serenity::prelude::*;

#[derive(Debug, Default, serde::Deserialize, PartialEq)]
struct DiscordConfig {
    channels: Vec<String>,
}

#[derive(Debug, Default, serde::Deserialize, PartialEq)]
struct AppConfig {
    discord: DiscordConfig,
}

#[derive(Debug, Default, serde::Deserialize, PartialEq)]
struct DiscordInvite {
    expires_at: Option<String>,
}

struct DiscordInviteCode<'t> {
    invite_code: &'t str,
    expires_at: Option<DateTime<FixedOffset>>,
}

struct InviteFinder<'t> {
    invite_codes: Vec<&'t str>,
}

impl<'t> InviteFinder<'t> {
    fn new(message: &'t str) -> InviteFinder<'t> {
        // 正規表現パターンを準備
        let invite_regex = Regex::new(r"(?:https?://)?discord\.gg/(\w+)").unwrap();

        // 招待コードリストを取得
        let invite_codes: Vec<&'t str> = invite_regex
            .captures_iter(message)
            .map(|c| c.get(1).unwrap().as_str())
            .collect();

        InviteFinder { invite_codes }
    }

    async fn get_invite_list(
        &self,
    ) -> Result<Vec<DiscordInviteCode<'t>>, Box<dyn Error>> {
        futures::future::try_join_all(self.invite_codes.iter().map(|invite_code| async move {
            // APIリクエストを構築
            let invite_url = format!("https://discordapp.com/api/v9/invites/{}", invite_code);
            let invite_response = reqwest::get(&invite_url).await?;
            let invite_result = invite_response.json::<DiscordInvite>().await?;
            let expires_at = invite_result
                .expires_at
                .map(|expires_at| DateTime::parse_from_rfc3339(&expires_at).unwrap());
            Ok(DiscordInviteCode {
                invite_code,
                expires_at,
            })
        }))
        .await
    }
}

struct Handler {
    app_config: AppConfig,
}

impl Handler {
    // 招待コードを検証する
    async fn check_invite_links(&self, ctx: Context, msg: &Message) -> Result<(), Box<dyn Error>> {
        // 招待リンクをパース
        let finder = InviteFinder::new(msg.content.as_str());

        // 招待コードリストを取得
        let invite_data = match finder.get_invite_list().await {
            Ok(invite_data) => invite_data,
            Err(_) => return Ok(()), // 取得に失敗
        };

        // 無期限の招待コードを除外
        let expirable_invites = invite_data
            .iter()
            .filter(|x| x.expires_at.is_some())
            .collect::<Vec<_>>();
        if expirable_invites.is_empty() {
            return Ok(()); // 有効期限のあるリンクが無い
        }

        // 警告メッセージを構築
        let reply = msg
            .channel_id
            .send_message(&ctx.http, |m| {
                m.reference_message(msg);
                m.embed(|e| {
                    e.title("宣伝できない招待リンク");
                    e.description("招待リンクは無期限のものだけ使用できます");
                    e.fields(expirable_invites.iter().map(move |x| {
                        (
                            format!("`{}` の有効期限", x.invite_code),
                            x.expires_at
                                .as_ref()
                                .unwrap()
                                .with_timezone(&Japan)
                                .format("%Y年%m月%d日 %H時%M分%S秒"),
                            false,
                        )
                    }));
                    e
                })
            })
            .await?;

        // 30秒待つ
        sleep(Duration::from_secs(5)).await;

        // 警告メッセージを削除
        reply.delete(&ctx.http).await?;
        // 該当メッセージを削除
        msg.delete(&ctx.http).await?;

        Ok(())
    }
}

#[async_trait]
impl EventHandler for Handler {
    async fn message(&self, ctx: Context, msg: Message) {
        // コンフィグで指定されたチャンネルのメッセージのみ処理する
        if !self.app_config.discord.channels.contains(&msg.channel_id.to_string()) {
            return; // チャンネルが違う
        }

        // 招待コードを検証
        if let Err(why) = self.check_invite_links(ctx, &msg).await {
            println!("検証に失敗: {}", why);
        }
    }
}

#[tokio::main]
async fn main() {
    // フレームワークを初期化
    let framework = StandardFramework::new().configure(|c| c.prefix("~"));

    // 設定ファイルを読み込む
    let config = Config::builder()
        // Add in `./Settings.toml`
        .add_source(config::File::with_name("config.toml"))
        // Add in settings from the environment (with a prefix of APP)
        // Eg.. `APP_DEBUG=1 ./target/app` would set the `debug` key
        .add_source(config::Environment::with_prefix("APP"))
        .build()
        .unwrap();
    // 設定ファイルをパース
    let app_config = config.try_deserialize::<AppConfig>().unwrap();

    // イベント受信リスナーを構築
    let handler = Handler {
        app_config,
    };

    // 環境変数のトークンを使用してDiscord APIを初期化
    let token = env::var("DISCORD_TOKEN").expect("token");
    let intents = GatewayIntents::non_privileged() | GatewayIntents::MESSAGE_CONTENT;
    let mut client = Client::builder(token, intents)
        .event_handler(handler)
        .framework(framework)
        .await
        .expect("Error creating client");

    // イベント受信を開始
    if let Err(why) = client.start().await {
        println!("Bot動作中にエラーが発生しました: {:?}", why);
    }
}
