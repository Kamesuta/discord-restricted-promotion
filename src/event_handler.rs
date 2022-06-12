use anyhow::{anyhow, Context as _, Result};
use chrono_tz::Tz::Japan;
use futures::future::{join_all, try_join_all};
use tokio::time::{sleep, Duration};

use crate::app_config::AppConfig;
use crate::history_log::{HistoryKeyType, HistoryLog, HistoryRecord};
use crate::invite_finder::{DiscordInviteLink, InviteFinder};

use serenity::async_trait;
use serenity::model::channel::Message;
use serenity::prelude::*;

/// イベント受信リスナー
pub struct Handler {
    /// 設定
    app_config: AppConfig,
    /// 履歴
    history: HistoryLog,
}

impl Handler {
    /// コンストラクタ
    pub fn new(app_config: AppConfig) -> Result<Self> {
        Ok(Self {
            history: HistoryLog::new(app_config.discord.ban_period_days)?,
            app_config,
        })
    }

    /// 警告を一定時間後に削除する
    async fn wait_and_delete_message(
        &self,
        ctx: &Context,
        msg: &Message,
        reply: &Message,
    ) -> Result<()> {
        // 一定時間待つ
        sleep(Duration::from_secs(self.app_config.discord.alert_sec)).await;

        // 警告メッセージを削除
        reply
            .delete(&ctx)
            .await
            .with_context(|| format!("警告メッセージの削除に失敗: {:?}", reply))?;
        // 該当メッセージを削除
        msg.delete(&ctx.http)
            .await
            .with_context(|| format!("対象メッセージの削除に失敗: {:?}", msg))?;

        Ok(())
    }

    /// 招待コードを検証する
    async fn check_invite_links<'t>(
        &self,
        ctx: &Context,
        msg: &Message,
        invites: &Vec<DiscordInviteLink<'t>>,
    ) -> Result<Option<Message>> {
        // 無期限の招待コードを除外
        let expirable_invites = invites
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
            .await
            .context("警告メッセージの構築に失敗")?;

        Ok(Some(reply))
    }

    /// 過去ログに同じリンクがないかを検証
    async fn check_invite_history<'t>(
        &self,
        ctx: &Context,
        msg: &Message,
        invites: Vec<HistoryKeyType>,
    ) -> Result<Option<Message>> {
        // 過去ログに同じリンクがないかを検証
        let invites = invites.into_iter().map(|invite_key| async {
            let result = self.history.validate(&msg.channel_id, &invite_key).await;
            let records = match result {
                Ok(records) if !records.is_empty() => records,
                _ => return None,
            };

            // 空だったらNoneを返す
            if records.is_empty() {
                return None;
            }

            Some((invite_key, records))
        });
        let invites = join_all(invites).await;
        let invites: Vec<(HistoryKeyType, Vec<HistoryRecord>)> =
            invites.into_iter().filter_map(|f| f).collect::<Vec<_>>();
        if invites.is_empty() {
            return Ok(None); // 過去に送信されたリンクが無い
        }

        // ギルドIDを取得
        let guild_id = msg.guild_id.ok_or(anyhow!("ギルドIDの取得に失敗"))?;

        // 警告メッセージを構築
        let reply = msg
            .channel_id
            .send_message(&ctx.http, |m| {
                m.reference_message(msg);
                m.embed(|e| {
                    e.title("宣伝済みの招待リンク");
                    e.description("同じ鯖の招待リンクは送信できません");
                    e.field(
                        "以前に宣伝されたメッセージ",
                        invites
                            .iter()
                            .flat_map(move |(_invite_key, records)| records.iter())
                            .map(|record| {
                                format!(
                                    "[メッセージリンク](https://discord.com/channels/{}/{}/{})",
                                    guild_id.to_string(),
                                    record.channel_id.to_string(),
                                    record.message_id.to_string(),
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("\n"),
                        false,
                    );
                    e
                })
            })
            .await
            .context("警告メッセージの構築に失敗")?;

        Ok(Some(reply))
    }

    /// 説明文が書かれているかどうかを検証する
    async fn check_invite_message<'t>(
        &self,
        ctx: &Context,
        msg: &Message,
        finder: &InviteFinder<'t>,
    ) -> Result<Option<Message>> {
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
                        "説明文の長さが短すぎます\n説明文でサーバーをアピールしましょう!",
                    );
                    e
                })
            })
            .await
            .context("警告メッセージの構築に失敗")?;

        Ok(Some(reply))
    }

    async fn check_invite<'t>(&self, ctx: &Context, msg: &Message) -> Result<Option<Message>> {
        // 招待リンクをパース
        let finder = InviteFinder::new(msg.content.as_str());

        // メッセージを検証
        match self
            .check_invite_message(&ctx, &msg, &finder)
            .await
            .context("メッセージ長さの検証に失敗")?
        {
            Some(reply) => return Ok(Some(reply)),
            None => (), // 検証に失敗
        };

        // メッセージが過去に送信された招待リンクを検証 (招待リンク)
        let invite_codes = finder
            .invite_codes
            .clone()
            .into_iter()
            .map(|f| HistoryKeyType::InviteCode(f.invite_code.to_string()))
            .collect::<Vec<_>>();
        match self
            .check_invite_history(&ctx, &msg, invite_codes)
            .await
            .context("過去の招待コードの検証に失敗")?
        {
            Some(reply) => return Ok(Some(reply)),
            None => (), // 検証に失敗
        };

        // 招待コードリストを取得
        let invites = finder
            .get_invite_list()
            .await
            .context("招待リンク情報の取得に失敗")?;

        // 招待コードを検証
        match self
            .check_invite_links(&ctx, &msg, &invites)
            .await
            .context("招待コード期限の検証に失敗")?
        {
            Some(reply) => return Ok(Some(reply)),
            None => (), // 検証に失敗
        };

        // メッセージが過去に送信された招待リンクを検証 (ギルドID)
        let invite_guilds = invites
            .clone()
            .into_iter()
            .filter_map(|f| f.guild_id)
            .map(|guild_id| HistoryKeyType::InviteGuildId(guild_id))
            .collect::<Vec<_>>();
        match self
            .check_invite_history(&ctx, &msg, invite_guilds)
            .await
            .context("過去の招待サーバーの検証に失敗")?
        {
            Some(reply) => return Ok(Some(reply)),
            None => (), // 検証に失敗
        };

        // 警告がない場合、履歴に登録
        let invite_result = invites.iter().map(|invite| async {
            if let Some(guild_id) = invite.guild_id {
                return self
                    .history
                    .insert(HistoryRecord {
                        invite_code: invite.invite_code.to_string(),
                        invite_guild_id: guild_id,
                        channel_id: msg.channel_id,
                        message_id: msg.id,
                        timestamp: msg.timestamp.unix_timestamp(),
                    })
                    .await;
            }
            Ok(())
        });
        try_join_all(invite_result)
            .await
            .context("履歴の登録に失敗")?;

        Ok(None)
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

        // チェック&警告
        let reply = match self.check_invite(&ctx, &msg).await {
            Ok(Some(reply)) => reply, // 警告あり
            Ok(None) => return,       // 警告なし
            Err(why) => {
                // エラー
                println!("検証に失敗: {:?}", why);
                return;
            }
        };

        // 一定時間後に警告メッセージを削除
        if let Err(why) = self.wait_and_delete_message(&ctx, &msg, &reply).await {
            println!("警告メッセージの削除に失敗: {}", why);
            return;
        }
    }
}
