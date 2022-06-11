use std::sync::{Arc, Mutex};

use crate::app_config::AppConfig;
use chrono::Duration;
use rusqlite::{params, Connection, Result};
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
    /// 設定
    app_config: AppConfig,
}

impl HistoryLog {
    pub fn new(app_config: AppConfig) -> Result<HistoryLog> {
        let conn = Connection::open("history_log.db")?;

        conn.execute(
            "CREATE TABLE history (
                id               INTEGER PRIMARY KEY AUTO_INCREMENT,
                invite_code      VARCHAR(20) NOT NULL UNIQUE,
                invite_guild_id  VARCHAR(20) NOT NULL,
                channel_id       VARCHAR(20) NOT NULL,
                message_id       VARCHAR(20) NOT NULL UNIQUE,
                timestamp        TIMESTAMP NOT NULL,
            )",
            params!(),
        )?;

        Ok(HistoryLog { conn: Arc::new(Mutex::new(conn)), app_config })
    }

    pub fn insert<'t>(&self, record: &HistoryRecord) -> Result<()> {
        self.conn.lock().unwrap().execute(
            "REPLACE INTO history (invite_code, invite_guild_id, channel_id, message_id, timestamp) VALUES (?1, ?2, ?3, ?4 ?5)",
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

    pub fn validate<'t>(
        &self,
        channel_id: &ChannelId,
        key: &HistoryKeyType,
    ) -> Result<Vec<HistoryRecord>> {
        let collect: Vec<HistoryRecord> = {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT
                        invite_code,
                        invite_guild_id,
                        channel_id,
                        message_id
                    FROM history
                    WHERE channel_id = ?1 AND ?2 = ?3 AND timestamp > ?4",
            )?;
            let timestamp = (chrono::Utc::now() + Duration::weeks(1)).timestamp();
            let (search_key, search_value) = match key {
                HistoryKeyType::InviteCode(invite_code) => ("invite_code", invite_code.to_owned()),
                HistoryKeyType::InviteGuildId(invite_guild_id) => ("invite_guild_id", invite_guild_id.to_string()),
            };
            let result = stmt.query_map(
                params!(
                    channel_id.to_string(),
                    search_key,
                    search_value,
                    timestamp
                ),
                |row| {
                    Ok(HistoryRecord {
                        key: HistoryKey {
                            invite_code: row.get(0)?,
                            invite_guild_id: GuildId(row.get(1)?),
                            channel_id: ChannelId(row.get(2)?),
                        },
                        message_id: MessageId(row.get(3)?),
                    })
                },
            )?;
            result.filter_map(|x| x.ok()).collect()
        };
        Ok(collect)
    }
}
