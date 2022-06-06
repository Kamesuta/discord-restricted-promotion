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
    alert_sec: u64,
    required_message_length: usize,
}

#[derive(Debug, Default, serde::Deserialize, PartialEq)]
struct AppConfig {
    discord: DiscordConfig,
}

#[derive(Debug, Default, serde::Deserialize, PartialEq)]
struct DiscordInvite {
    expires_at: Option<String>,
}

struct DiscordInviteLink<'t> {
    invite_link: &'t str,
    invite_code: &'t str,
    expires_at: Option<DateTime<FixedOffset>>,
}

struct InviteFinder<'t> {
    invite_codes: Vec<DiscordInviteLink<'t>>,
}

impl<'t> InviteFinder<'t> {
    fn new(message: &'t str) -> InviteFinder<'t> {
        // 正規表現パターンを準備
        let invite_regex = Regex::new(r"(?:https?://)?discord\.gg/(\w+)").unwrap();

        // 招待コードリストを取得
        let invite_codes = invite_regex
            .captures_iter(message)
            .map(|c| DiscordInviteLink {
                invite_link: c.get(0).unwrap().as_str(),
                invite_code: c.get(1).unwrap().as_str(),
                expires_at: None,
            })
            .collect::<Vec<_>>();

        InviteFinder { invite_codes }
    }

    async fn get_invite_list(&self) -> Result<Vec<DiscordInviteLink<'t>>, Box<dyn Error>> {
        futures::future::try_join_all(self.invite_codes.iter().map(|invite_link| async move {
            // APIリクエストを構築
            let invite_url = format!(
                "https://discordapp.com/api/v9/invites/{}",
                invite_link.invite_code
            );
            // APIリクエストを実行
            let invite_response = reqwest::get(&invite_url).await?;
            // 招待リンク情報をパース
            let invite_result = invite_response.json::<DiscordInvite>().await?;
            // 招待リンクの有効期限を抽出
            let expires_at = invite_result
                .expires_at
                .map(|expires_at| DateTime::parse_from_rfc3339(&expires_at).unwrap());

            // 有効期限をセットした構造体を返す
            Ok(DiscordInviteLink {
                expires_at,
                ..*invite_link
            })
        }))
        .await
    }
}

struct Handler {
    app_config: AppConfig,
}

impl Handler {
    // 警告を一定時間後に削除する
    async fn wait_and_delete_message(
        &self,
        ctx: &Context,
        msg: &Message,
        replies: &Vec<Message>,
    ) -> Result<(), Box<dyn Error>> {
        // 一定時間待つ
        sleep(Duration::from_secs(self.app_config.discord.alert_sec)).await;

        // 警告メッセージを削除
        for reply in replies {
            reply.delete(&ctx).await?;
        }
        // 該当メッセージを削除
        msg.delete(&ctx.http).await?;

        Ok(())
    }

    // 招待コードを検証する
    async fn check_invite_links<'t>(
        &self,
        ctx: &Context,
        msg: &Message,
        finder: &InviteFinder<'t>,
    ) -> Result<Option<Message>, Box<dyn Error>> {
        // 招待コードリストを取得
        let invite_data = match finder.get_invite_list().await {
            Ok(invite_data) => invite_data,
            Err(_) => return Ok(None), // 取得に失敗
        };

        // 無期限の招待コードを除外
        let expirable_invites = invite_data
            .iter()
            .filter(|x| x.expires_at.is_some())
            .collect::<Vec<_>>();
        if expirable_invites.is_empty() {
            return Ok(None); // 有効期限のあるリンクが無い
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

        Ok(Some(reply))
    }

    // 説明文が書かれているかどうかを検証する
    async fn check_invite_message<'t>(
        &self,
        ctx: &Context,
        msg: &Message,
        finder: &InviteFinder<'t>,
    ) -> Result<Option<Message>, Box<dyn Error>> {
        // リンクの合計の長さを取得
        let link_total_length = finder
            .invite_codes
            .iter()
            .map(|invite_link| invite_link.invite_link.len())
            .sum::<usize>();
        // メッセージを全体の長さを取得
        let message_length = msg.content.len();
        // 説明文の長さを計算
        let desc_length = message_length - link_total_length;
        // 長さが足りているかどうかを検証
        if desc_length > self.app_config.discord.required_message_length {
            return Ok(None);
        }

        // 警告メッセージを構築
        let reply = msg
            .channel_id
            .send_message(&ctx.http, |m| {
                m.reference_message(msg);
                m.embed(|e| {
                    e.title("説明文不足");
                    e.description(
                        "説明文の長さが短すぎます\n説明文でサーバーをアピールしましょう！",
                    );
                    e
                })
            })
            .await?;

        Ok(Some(reply))
    }
}

#[async_trait]
impl EventHandler for Handler {
    async fn message(&self, ctx: Context, msg: Message) {
        // Botの投稿を無視
        if msg.author.bot {
            return;
        }

        // コンフィグで指定されたチャンネルのメッセージのみ処理する
        if !self
            .app_config
            .discord
            .channels
            .contains(&msg.channel_id.to_string())
        {
            return; // チャンネルが違う
        }

        // 無視するロールを持っているかどうかを検証
        let manage_channels = msg
            .member
            .as_ref()
            .and_then(|member| member.has_role(&ctx.http, self.app_config.discord.manage_channels));
        if manage_channels.unwrap_or(false) {
            return;
        }

        // 招待リンクをパース
        let finder = InviteFinder::new(msg.content.as_str());

        // 警告リプライ
        let mut replies: Vec<Message> = Vec::new();

        // 招待コードを検証
        match self.check_invite_links(&ctx, &msg, &finder).await {
            Ok(reply) => match reply {
                Some(reply) => replies.push(reply),
                None => (), // 検証に失敗
            },
            Err(why) => {
                println!("招待リンクの検証に失敗: {}", why);
                return;
            }
        };

        // メッセージを検証
        match self.check_invite_message(&ctx, &msg, &finder).await {
            Ok(reply) => match reply {
                Some(reply) => replies.push(reply),
                None => (), // 検証に失敗
            },
            Err(why) => {
                println!("検証に失敗: {}", why);
                return;
            }
        };

        // #TODO 過去ログに同じリンクがないかを検証

        // 一定時間後に警告メッセージを削除
        if let Err(why) = self.wait_and_delete_message(&ctx, &msg, &replies).await {
            println!("警告メッセージの削除に失敗: {}", why);
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
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
    let app_config = match config.try_deserialize::<AppConfig>() {
        Ok(config) => config,
        Err(why) => {
            println!("設定ファイルのパースに失敗: {}", why);
            return Err(why.into());
        }
    };

    // イベント受信リスナーを構築
    let handler = Handler { app_config };

    // 環境変数のトークンを使用してDiscord APIを初期化
    let token = env::var("DISCORD_TOKEN").expect("token");
    let intents = GatewayIntents::non_privileged() | GatewayIntents::MESSAGE_CONTENT;
    let mut client = match Client::builder(token, intents)
        .event_handler(handler)
        .framework(framework)
        .await
    {
        Ok(client) => client,
        Err(why) => {
            println!("Botの初期化に失敗しました: {:?}", why);
            return Err(why.into());
        }
    };

    // イベント受信を開始
    match client.start().await {
        Ok(_) => (),
        Err(why) => {
            println!("Bot動作中にエラーが発生しました: {:?}", why);
            return Err(why.into());
        }
    };

    Ok(())
}
