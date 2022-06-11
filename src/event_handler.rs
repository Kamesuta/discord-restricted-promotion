use chrono_tz::Tz::Japan;
use futures::future::{join_all, try_join_all};
use std::error::Error;
use tokio::time::{sleep, Duration};

use crate::app_config::AppConfig;
use crate::history_log::{HistoryKey, HistoryKeyType, HistoryLog, HistoryRecord};
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
    pub fn new(app_config: AppConfig) -> Result<Self, Box<dyn Error>> {
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
        invites: &Vec<DiscordInviteLink<'t>>,
    ) -> Result<Option<Message>, Box<dyn Error>> {
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
            .await?;

        Ok(Some(reply))
    }

    /// 過去ログに同じリンクがないかを検証
    async fn check_invite_history<'t>(
        &self,
        ctx: &Context,
        msg: &Message,
        invites: Vec<HistoryKeyType>,
    ) -> Result<Option<Message>, Box<dyn Error>> {
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
        let guild_id = msg.guild_id.ok_or("ギルドIDの取得に失敗")?;

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
                                    record.key.channel_id.to_string(),
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
                        "説明文の長さが短すぎます\n説明文でサーバーをアピールしましょう!",
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

        // メッセージが過去に送信された招待リンクを検証 (招待リンク)
        let invite_codes = finder
            .invite_codes
            .clone()
            .into_iter()
            .map(|f| HistoryKeyType::InviteCode(f.invite_link.to_string()))
            .collect::<Vec<_>>();
        match self.check_invite_history(&ctx, &msg, invite_codes).await {
            Ok(reply) => match reply {
                Some(reply) => replies.push(reply),
                None => (), // 検証に失敗
            },
            Err(why) => {
                println!("検証に失敗: {}", why);
                return;
            }
        };

        // 招待コードリストを取得
        let invites = match finder.get_invite_list().await {
            Ok(invites) => invites,
            Err(why) => {
                println!("招待リンク情報の取得に失敗: {}", why);
                return;
            }
        };

        // 招待コードを検証
        match self.check_invite_links(&ctx, &msg, &invites).await {
            Ok(reply) => match reply {
                Some(reply) => replies.push(reply),
                None => (), // 検証に失敗
            },
            Err(why) => {
                println!("招待リンクの検証に失敗: {}", why);
                return;
            }
        };

        // メッセージが過去に送信された招待リンクを検証 (ギルドID)
        let invite_guilds = invites
            .clone()
            .into_iter()
            .filter_map(|f| f.guild_id)
            .map(|guild_id| HistoryKeyType::InviteGuildId(guild_id))
            .collect::<Vec<_>>();
        match self.check_invite_history(&ctx, &msg, invite_guilds).await {
            Ok(reply) => match reply {
                Some(reply) => replies.push(reply),
                None => (), // 検証に失敗
            },
            Err(why) => {
                println!("検証に失敗: {}", why);
                return;
            }
        };

        // 登録
        let invite_result = invites.iter().map(|invite| async {
            if let Some(guild_id) = invite.guild_id {
                return self
                    .history
                    .insert(HistoryRecord {
                        key: HistoryKey {
                            invite_code: invite.invite_code.to_string(),
                            invite_guild_id: guild_id,
                            channel_id: msg.channel_id,
                        },
                        message_id: msg.id,
                    })
                    .await;
            }
            Ok(())
        });
        match try_join_all(invite_result).await {
            Ok(_) => (),
            Err(why) => {
                println!("履歴の登録に失敗: {}", why);
                return;
            }
        };

        // 一定時間後に警告メッセージを削除
        if let Err(why) = self.wait_and_delete_message(&ctx, &msg, &replies).await {
            println!("警告メッセージの削除に失敗: {}", why);
        }
    }
}
