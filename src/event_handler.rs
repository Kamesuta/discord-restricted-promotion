use chrono_tz::Tz::Japan;
use std::error::Error;
use std::sync::Arc;
use tokio::time::{sleep, Duration};

use crate::app_config::AppConfig;
use crate::history_log::{HistoryKeyType, HistoryLog};
use crate::invite_finder::InviteFinder;

use serenity::async_trait;
use serenity::model::channel::Message;
use serenity::prelude::*;

/// イベント受信リスナー
pub struct Handler {
    /// 設定
    app_config: AppConfig,
    /// 履歴
    history: Arc<Mutex<HistoryLog>>,
}

impl Handler {
    /// コンストラクタ
    pub fn new(
        app_config: AppConfig,
    ) -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            history: Arc::new(Mutex::new(HistoryLog::new(app_config.clone())?)),
            app_config,
        })
    }

    /// 警告を一定時間後に削除する
    async fn wait_and_delete_message(
        &self,
        ctx: &Context,
        msg: &Message,
        replies: &Vec<Message>,
    ) -> Result<(), Box<dyn Error>> {
        // 警告がない場合は処理を行わない
        if replies.is_empty() {
            return Ok(());
        }

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

    /// 招待コードを検証する
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

    /// 説明文が書かれているかどうかを検証する
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
    /// メッセージが送信された時に呼び出される
    async fn message(&self, ctx: Context, msg: Message) {
        // Botの投稿を無視
        if msg.author.bot {
            return;
        }

        // コンフィグで指定されたチャンネルのメッセージのみ処理する
        if !self.app_config.discord.channels.contains(&msg.channel_id) {
            return; // チャンネルが違う
        }

        // 無視するロールを持っているかどうかを検証
        let manage_channels = msg.member.as_ref().map(|member| {
            self.app_config
                .discord
                .ignore_roles
                .iter()
                .any(|f| member.roles.contains(f))
        });
        if manage_channels.unwrap_or(false) {
            return;
        }

        // 招待リンクをパース
        let finder = InviteFinder::new(msg.content.as_str());

        // 警告リプライ
        let mut replies: Vec<Message> = Vec::new();

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

        // 過去ログに同じリンクがないかを検証
        //self.history.lock().await.insert(msg.content.clone());
        let history_log = self.history.lock().await;
        let invites = finder.invite_codes.iter().filter_map(|invite_link| {
            let records = match history_log.validate(
                &msg.channel_id,
                &HistoryKeyType::InviteCode(invite_link.invite_link.to_string()),
            ) {
                Ok(records) if !records.is_empty() => records,
                _ => return None,
            };
            // let message_ids = records
            //     .iter()
            //     .map(|f| f.message_id)
            //     .map(|id| msg.channel_id.message(ctx, id));
            // let messages = futures::future::try_join_all(message_ids).await;
            // let messages = messages.iter().flat_map(|f| f);

            // !records.is_empty()
            Some((invite_link, records))
        });
        // if invites.any(|f| f.1.is_empty()) {
        //     return;
        // }

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

        // #TODO 過去ログに同じリンクがないかを検証

        // 一定時間後に警告メッセージを削除
        if let Err(why) = self.wait_and_delete_message(&ctx, &msg, &replies).await {
            println!("警告メッセージの削除に失敗: {}", why);
        }
    }
}
