use anyhow::{Context as _, Result};
use std::sync::Arc;

use chrono::Duration;
use futures::lock::Mutex;
use rusqlite::{params, Connection};
use serenity::model::id::{ChannelId, GuildId, MessageId};

#[derive(Debug, Default, serde::Deserialize, PartialEq, Clone)]
pub struct HistoryRecord {
    /// 招待コード
    pub invite_code: String,
    /// 招待コードのギルドID
    pub invite_guild_id: GuildId,
    /// メッセージのチャンネルID
    pub channel_id: ChannelId,
    /// メッセージID
    pub message_id: MessageId,
    /// タイムスタンプ
    pub timestamp: i64,
}

pub enum HistoryKeyType {
    /// 招待コード
    InviteCode(String),
    /// 招待コードのギルドID
    InviteGuildId(GuildId),
}

pub struct HistoryLog {
    /// sql接続情報
    conn: Arc<Mutex<Connection>>,
    /// 同じ鯖の宣伝を禁止する日数
    pub ban_period_days: i64,
}

impl HistoryLog {
    pub fn new(ban_period_days: i64) -> Result<HistoryLog> {
        let conn =
            Connection::open("history_log.db").context("履歴データベースのオープンに失敗")?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS history (
                id               INTEGER PRIMARY KEY AUTOINCREMENT,
                invite_code      VARCHAR(20) NOT NULL UNIQUE,
                invite_guild_id  VARCHAR(20) NOT NULL,
                channel_id       VARCHAR(20) NOT NULL,
                message_id       VARCHAR(20) NOT NULL,
                timestamp        TIMESTAMP NOT NULL
            )",
            params!(),
        )
        .context("履歴データベースの作成に失敗")?;

        Ok(HistoryLog {
            conn: Arc::new(Mutex::new(conn)),
            ban_period_days,
        })
    }

    pub async fn insert<'t>(&self, record: HistoryRecord) -> Result<()> {
        self.conn.lock().await.execute(
            "REPLACE INTO history (invite_code, invite_guild_id, channel_id, message_id, timestamp) VALUES (?1, ?2, ?3, ?4, ?5)",
            params!(
                record.invite_code,
                record.invite_guild_id.to_string(),
                record.channel_id.to_string(),
                record.message_id.to_string(),
                record.timestamp,
            ),
        ).with_context(|| format!("履歴データベースへの書き込みに失敗: {:?}", record))?;

        Ok(())
    }

    pub async fn delete<'t>(&self, message_id: &MessageId) -> Result<()> {
        self.conn
            .lock()
            .await
            .execute(
                "DELETE FROM history WHERE message_id = ?1",
                params!(message_id.to_string(),),
            )
            .with_context(|| format!("履歴データベースからの削除に失敗: {:?}", message_id))?;

        Ok(())
    }

    pub async fn validate<'t>(
        &self,
        event_message_id: &MessageId,
        channel_id: &ChannelId,
        key: &HistoryKeyType,
    ) -> Result<Vec<HistoryRecord>> {
        let collect: Vec<HistoryRecord> = {
            let conn = self.conn.lock().await;
            let (search_key, search_value) = match key {
                HistoryKeyType::InviteCode(invite_code) => ("invite_code", invite_code.to_owned()),
                HistoryKeyType::InviteGuildId(invite_guild_id) => {
                    ("invite_guild_id", invite_guild_id.to_string())
                }
            };
            let query =
                format!("SELECT invite_code, invite_guild_id, channel_id, message_id, timestamp FROM history WHERE message_id != ?1 AND channel_id = ?2 AND {} = ?3 AND timestamp > ?4", search_key);
            let mut stmt = conn
                .prepare(&query)
                .with_context(|| format!("SQL文の構築に失敗: {}", query))?;
            let timestamp = (chrono::Utc::now() - Duration::days(self.ban_period_days)).timestamp();
            let result = stmt
                .query_map(
                    params!(
                        event_message_id.to_string(),
                        channel_id.to_string(),
                        search_value,
                        timestamp
                    ),
                    |row| {
                        let invite_code: String = row.get(0)?;
                        let invite_guild_id: String = row.get(1)?;
                        let channel_id: String = row.get(2)?;
                        let message_id: String = row.get(3)?;
                        let timestamp: i64 = row.get(4)?;
                        Ok((
                            invite_code,
                            invite_guild_id,
                            channel_id,
                            message_id,
                            timestamp,
                        ))
                    },
                )
                .context("履歴データベースの読み込みに失敗")?
                .map(|row| -> Result<HistoryRecord> {
                    let (invite_code, invite_guild_id, channel_id, message_id, timestamp) = row?;
                    Ok(HistoryRecord {
                        invite_code,
                        invite_guild_id: GuildId(invite_guild_id.parse()?),
                        channel_id: ChannelId(channel_id.parse()?),
                        message_id: MessageId(message_id.parse()?),
                        timestamp,
                    })
                })
                .filter_map(|row| row.ok())
                .collect::<Vec<_>>();
            result
        };
        Ok(collect)
    }
}
