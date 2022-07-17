use anyhow::{Context as _, Error, Result};
use chrono::{DateTime, Duration, NaiveDateTime, Utc};
use chrono_tz::Tz::{self, Japan};
use futures::future::{join_all, try_join_all};
use serenity::model::event::MessageUpdateEvent;
use serenity::model::gateway::Ready;
use serenity::model::id::{ChannelId, GuildId, MessageId};
use tokio::time::sleep;

use crate::app_config::AppConfig;
use crate::history_log::{HistoryFindKey, HistoryLog, HistoryRecord};
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
            history: HistoryLog::new(app_config.discord.ban_period.clone())?,
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
        sleep(tokio::time::Duration::from_secs(
            self.app_config.discord.alert_sec,
        ))
        .await;

        // 警告メッセージを削除
        reply
            .delete(ctx)
            .await
            .with_context(|| format!("警告メッセージの削除に失敗: {}", reply.id))?;
        // 該当メッセージを削除
        msg.channel_id
            .delete_message(ctx, msg.id)
            .await
            .with_context(|| format!("対象メッセージの削除に失敗: {}", msg.id))?;

        Ok(())
    }

    /// 招待コードを検証する
    async fn check_invite_links<'t>(
        &self,
        ctx: &Context,
        msg: &Message,
        invites: &[DiscordInviteLink<'t>],
    ) -> Result<Option<Message>> {
        // 無効な招待コードを集める
        let invalid_invites = invites
            .iter()
            .filter(|x| !x.guild_id.is_some())
            .collect::<Vec<_>>();
        // 無効なリンクがある
        if !invalid_invites.is_empty() {
            // 警告メッセージを構築
            let reply = msg
                .channel_id
                .send_message(ctx, |m| {
                    m.reference_message(msg);
                    m.embed(|e| {
                        e.title("無効な招待リンク");
                        e.description("有効な招待リンクのみ宣伝できます");
                        e.fields(
                            invalid_invites
                                .iter()
                                .map(|x| ("招待コード", format!("`{}`", x.invite_code), false)),
                        );
                        e
                    })
                })
                .await
                .context("警告メッセージの構築に失敗")?;

            return Ok(Some(reply));
        }

        // 期限付きの招待コードを集める
        let expirable_invites = invites
            .iter()
            .filter(|x| x.expires_at.is_some())
            .collect::<Vec<_>>();
        // 期限付きのリンクがある
        if !expirable_invites.is_empty() {
            // 警告メッセージを構築
            let reply = msg
                .channel_id
                .send_message(ctx, |m| {
                    m.reference_message(msg);
                    m.embed(|e| {
                        e.title(format!(
                            "{0}宣伝できない招待リンク{0}",
                            self.app_config.discord.alert_emoji
                        ));
                        e.description("招待リンクは無期限のものだけ使用できます");
                        e.fields(
                            expirable_invites
                                .iter()
                                .filter_map(|x| {
                                    Some((
                                        x,
                                        x.expires_at?
                                            .with_timezone(&Japan)
                                            .format("%Y年%m月%d日 %H時%M分%S秒"),
                                    ))
                                })
                                .map(|(x, expires_at)| {
                                    (format!("`{}` の有効期限", x.invite_code), expires_at, false)
                                }),
                        );
                        e
                    })
                })
                .await
                .context("警告メッセージの構築に失敗")?;

            return Ok(Some(reply));
        }

        Ok(None)
    }

    /// 過去ログに同じリンクがないかを検証
    async fn check_invite_history<'t>(
        &self,
        ctx: &Context,
        msg: &Message,
        invites: Vec<HistoryFindKey>,
    ) -> Result<Option<Message>> {
        // 過去ログに同じリンクがないかを検証
        type RecordLink = Vec<(HistoryRecord, String)>;
        let invites: Vec<Option<(HistoryFindKey, RecordLink)>> =
            try_join_all(invites.into_iter().map(|invite_key| async {
                // 履歴データベースから検索
                let records = self
                    .history
                    .validate(&msg.id, &msg.channel_id, &msg.author.id, &invite_key)
                    .await?;

                let ban_period_user_start =
                    (Utc::now() - Duration::minutes(self.app_config.discord.ban_period.min_per_user_start)).timestamp();

                    // メッセージが有効なのか検証する
                let records = try_join_all(
                    records
                    .into_iter()
                    .map(|record| async {
                        // メッセージをDiscordから取得する
                        let result = record.channel_id.message(ctx, record.message_id).await;

                        match result {
                            Ok(message) if record.user_id == msg.author.id && record.timestamp > ban_period_user_start => {
                                // min_per_user_start分以内のメッセージであれば前のメッセージを消す
                                message.channel_id.delete_message(ctx, message.id).await?;
                                Ok(None)
                            },
                            Ok(_message) => Ok(Some(record)), // メッセージが取得できたら残す
                            Err(_err) if record.deleted => Ok(Some(record)),
                            Err(_err) => {
                                println!(
                                    "メッセージが削除されているためデータベースから削除します: message_id={}, guild_id={}, invite_code={}",
                                    record.message_id,
                                    record.invite_guild_id,
                                    record.invite_code
                                );

                                // データベースから削除
                                self.history.delete(&record.message_id).await?;

                                // async closureは型を明示できないので、Okのときに型を明示する
                                // https://rust-lang.github.io/async-book/07_workarounds/02_err_in_async_blocks.html
                                Ok::<Option<HistoryRecord>, Error>(None)
                            }
                        }
                    })
                )
                .await?;
                let records = records
                    .into_iter()
                    .filter_map(|record| record)
                    .collect::<Vec<HistoryRecord>>();

                // 空だったらNoneを返す
                if records.is_empty() {
                    return Ok(None);
                }

                // リンク取得
                let records: RecordLink = join_all(records.into_iter().map(|record| async {
                    let invite_link = record
                        .message_id
                        .link_ensured(ctx, record.channel_id, None)
                        .await;
                    (record, invite_link)
                }))
                .await;

                // async closureは型を明示できないので、Okのときに型を明示する
                // https://rust-lang.github.io/async-book/07_workarounds/02_err_in_async_blocks.html
                Ok::<Option<(HistoryFindKey, RecordLink)>, Error>(Some((invite_key, records)))
            }))
            .await?;
        let invites = invites.into_iter().flatten().collect::<Vec<_>>();
        if invites.is_empty() {
            // 過去に送信されたリンクが無い
            return Ok(None);
        }

        // 警告メッセージを構築
        let reply = msg
            .channel_id
            .send_message(ctx, |m| {
                m.reference_message(msg);
                m.embed(|e| {
                    e.title(format!("{0}最近宣伝された鯖は宣伝できません{0}", self.app_config.discord.alert_emoji));
                    e.description(format!("直近{}日間に他人が宣伝した鯖、及び直近{}日間に自分が宣伝した鯖は宣伝できません\n自分が宣伝した鯖は30分以内であれば再投稿できます", self.app_config.discord.ban_period.day, self.app_config.discord.ban_period.day_per_user));
                    let history = invites
                        .iter()
                        .flat_map(move |(_invite_key, records)| records.iter())
                        .filter(|(record, _invite_link)| !record.deleted)
                        .collect::<Vec<&(HistoryRecord, String)>>();
                    if !history.is_empty() {
                        // 同じサーバーの宣伝
                        e.field(
                            "以前に宣伝されたメッセージ",
                            history
                                .iter()
                                .map(|(_record, invite_link)| {
                                    format!("[メッセージリンク]({})", invite_link)
                                })
                                .collect::<Vec<_>>()
                                .join("\n"),
                            false,
                        );
                    } else {
                        // 直近の自分が宣伝したサーバー (削除済みメッセージ)
                        let recent = invites
                            .iter()
                            .flat_map(move |(_invite_key, records)| records.iter())
                            .filter(|(record, _invite_link)| record.deleted)
                            .max_by_key(|(_record, _invite_link)| _record.timestamp);
                        if let Some((record, _invite_link)) = recent {
                            let date: DateTime<Tz> = DateTime::<Utc>::from_utc(NaiveDateTime::from_timestamp(record.timestamp, 0), Utc).with_timezone(&Japan);
                            e.field(
                                format!("直近{}日間に自分がこのサーバーを宣伝しています", self.app_config.discord.ban_period.day_per_user),
                                format!(
                                    "{} ({}日前)に宣伝",
                                    date.format("%Y年%m月%d日 %H時%M分%S秒"),
                                    (Utc::now().with_timezone(&Japan) - date).num_days(),
                                ),
                                false,
                            );
                        }
                    }
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
            .map(|invite_link| invite_link.invite_link.chars().count())
            .sum::<usize>();
        // メッセージを全体の長さを取得
        let message_length = msg.content.chars().count();
        // 説明文の長さを計算
        let desc_length = message_length - link_total_length;
        // 長さが足りているかどうかを検証
        if desc_length > self.app_config.discord.required_message_length {
            return Ok(None);
        }

        // 警告メッセージを構築
        let reply = msg
            .channel_id
            .send_message(ctx, |m| {
                m.reference_message(msg);
                m.embed(|e| {
                    e.title(format!("{0}説明文不足{0}", self.app_config.discord.alert_emoji));
                    e.description(
                        format!(
                            "説明文の長さが短すぎます\n少なくとも{}文字は説明文が必要です\n説明文でサーバーをアピールしましょう!",
                            self.app_config.discord.required_message_length,
                        ),
                    );
                    e
                })
            })
            .await
            .context("警告メッセージの構築に失敗")?;

        Ok(Some(reply))
    }

    /// 招待リンクが含まれるか検証する
    async fn check_has_invite<'t>(
        &self,
        ctx: &Context,
        msg: &Message,
        finder: &InviteFinder<'t>,
    ) -> Result<Option<Message>> {
        // 招待リンクが含まれるか検証する
        if !finder.invite_codes.is_empty() {
            return Ok(None);
        }

        // 警告メッセージを構築
        let reply = msg
            .channel_id
            .send_message(ctx, |m| {
                m.reference_message(msg);
                m.embed(|e| {
                    e.title(format!("{0}Discord鯖の宣伝のみ許可されています{0}", self.app_config.discord.alert_emoji));
                    e.description("ここはDiscord鯖の宣伝する為のチャンネルです\n少なくとも1つ以上のDiscord招待リンクが必要です");
                    e
                })
            })
            .await
            .context("警告メッセージの構築に失敗")?;

        Ok(Some(reply))
    }

    /// 招待メッセージの検証をすべて実行する
    async fn check_invite<'t>(&self, ctx: &Context, msg: &Message) -> Result<Option<Message>> {
        // 招待リンクをパース
        let finder = InviteFinder::new(msg.content.as_str())?;

        // メッセージに招待リンクが含まれているか検証
        match self
            .check_has_invite(ctx, msg, &finder)
            .await
            .context("招待リンクが含むかの検証に失敗")?
        {
            Some(reply) => return Ok(Some(reply)),
            None => (), // 検証に失敗
        };

        // メッセージを検証
        match self
            .check_invite_message(ctx, msg, &finder)
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
            .map(|f| HistoryFindKey::InviteCode(f.invite_code.to_string()))
            .collect::<Vec<_>>();
        match self
            .check_invite_history(ctx, msg, invite_codes)
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
            .check_invite_links(ctx, msg, &invites)
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
            .map(HistoryFindKey::InviteGuildId)
            .collect::<Vec<_>>();
        match self
            .check_invite_history(ctx, msg, invite_guilds)
            .await
            .context("過去の招待サーバーの検証に失敗")?
        {
            Some(reply) => return Ok(Some(reply)),
            None => (), // 検証に失敗
        };

        // 警告がない場合、履歴に登録
        self.history
            .delete(&msg.id)
            .await
            .context("履歴の更新に失敗")?;
        let invite_result = invites.iter().map(|invite| async {
            // 招待の中からサーバーIDが取れたものを選ぶ
            if let Some(guild_id) = invite.guild_id {
                // 招待コードを履歴に登録
                return self
                    .history
                    .insert(HistoryRecord {
                        invite_code: invite.invite_code.to_string(),
                        invite_guild_id: guild_id,
                        channel_id: msg.channel_id,
                        message_id: msg.id,
                        user_id: msg.author.id,
                        timestamp: msg.timestamp.unix_timestamp(), // 現在の時間
                        deleted: false,
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
    /// 準備完了時に呼ばれる
    async fn ready(&self, _ctx: Context, _data_about_bot: Ready) {
        println!("Bot準備完了");
    }

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
            println!("警告メッセージの削除に失敗: {:?}", why);
            return;
        }
    }

    // メッセージが更新された時に呼び出される
    async fn message_update(
        &self,
        ctx: Context,
        _old_if_available: Option<Message>,
        _new: Option<Message>,
        event: MessageUpdateEvent,
    ) {
        // メッセージIDからメッセージを取得
        let message = match event.channel_id.message(&ctx, event.id).await {
            Ok(message) => message,
            Err(why) => {
                println!("編集されたメッセージの取得に失敗: {:?}", why);
                return;
            }
        };

        // メッセージ投稿時と同じ処理を行う
        self.message(ctx, message).await;
    }

    /// メッセージが削除された時に呼び出される
    async fn message_delete(
        &self,
        _ctx: Context,
        _channel_id: ChannelId,
        deleted_message_id: MessageId,
        _guild_id: Option<GuildId>,
    ) {
        // メッセージIDに対応する履歴を削除
        match self.history.delete(&deleted_message_id).await {
            Ok(_) => (),
            Err(why) => {
                println!("履歴の削除に失敗: {:?}", why);
                return;
            }
        }
    }

    /// 一括削除時に呼び出される (BAN等)
    async fn message_delete_bulk(
        &self,
        _ctx: Context,
        _channel_id: ChannelId,
        multiple_deleted_messages_ids: Vec<MessageId>,
        _guild_id: Option<GuildId>,
    ) {
        // それぞれのメッセージIDに対応する履歴を削除
        match try_join_all(
            multiple_deleted_messages_ids
                .iter()
                .map(|message| self.history.delete(message)),
        )
        .await
        {
            Ok(_) => (),
            Err(why) => {
                println!("履歴の削除に失敗: {:?}", why);
                return;
            }
        }
    }
}
