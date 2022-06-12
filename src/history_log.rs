use std::error::Error;
use std::sync::Arc;

use chrono::Duration;
use futures::lock::Mutex;
use rusqlite::{params, Connection, Result, Row};
use serenity::model::id::{ChannelId, GuildId, MessageId};

pub struct HistoryKey {
    /// 招待コード
    pub invite_code: String,
    /// 招待コードのギルドID
    pub invite_guild_id: GuildId,
    /// メッセージのチャンネルID
    pub channel_id: ChannelId,
}

pub struct HistoryRecord {
    /// 招待キー
    pub key: HistoryKey,
    /// メッセージID
    pub message_id: MessageId,
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
        let conn = Connection::open("history_log.db")?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS history (
                id               INTEGER PRIMARY KEY AUTOINCREMENT,
                invite_code      VARCHAR(20) NOT NULL UNIQUE,
                invite_guild_id  VARCHAR(20) NOT NULL,
                channel_id       VARCHAR(20) NOT NULL,
                message_id       VARCHAR(20) NOT NULL UNIQUE,
                timestamp        TIMESTAMP NOT NULL
            )",
            params!(),
        )?;

        Ok(HistoryLog {
            conn: Arc::new(Mutex::new(conn)),
            ban_period_days,
        })
    }

    pub async fn insert<'t>(&self, record: HistoryRecord) -> Result<()> {
        self.conn.lock().await.execute(
            "REPLACE INTO history (invite_code, invite_guild_id, channel_id, message_id, timestamp) VALUES (?1, ?2, ?3, ?4, ?5)",
            params!(
                record.key.invite_code,
                record.key.invite_guild_id.to_string(),
                record.key.channel_id.to_string(),
                record.message_id.to_string(),
                chrono::Utc::now().timestamp(),
            ),
        )?;

        Ok(())
    }

    pub async fn validate<'t>(
        &self,
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
                format!("SELECT invite_code, invite_guild_id, channel_id, message_id FROM history WHERE channel_id = ?1 AND {} = ?2 AND timestamp > ?3", search_key);
            let mut stmt = conn.prepare(&query)?;
            let timestamp = (chrono::Utc::now() - Duration::days(self.ban_period_days)).timestamp();
            let result = stmt
                .query_map(
                    params!(channel_id.to_string(), search_value, timestamp),
                    |row| {
                        let invite_code: String = row.get(0)?;
                        let invite_guild_id: String = row.get(1)?;
                        let channel_id: String = row.get(2)?;
                        let message_id: String = row.get(3)?;
                        Ok((invite_code, invite_guild_id, channel_id, message_id))
                    },
                )?
                .map(|row| -> Result<HistoryRecord, Box<dyn Error>> {
                    let (invite_code, invite_guild_id, channel_id, message_id) = row?;
                    Ok(HistoryRecord {
                        key: HistoryKey {
                            invite_code,
                            invite_guild_id: GuildId(invite_guild_id.parse()?),
                            channel_id: ChannelId(channel_id.parse()?),
                        },
                        message_id: MessageId(message_id.parse()?),
                    })
                })
                .filter_map(|row| row.ok())
                .collect::<Vec<_>>();
            result
        };
        Ok(collect)
    }
}
