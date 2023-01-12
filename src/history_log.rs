use anyhow::{Context as _, Result};
use std::sync::Arc;

use chrono::{Duration, Utc};
use futures::lock::Mutex;
use rusqlite::{params, Connection, Rows};
use serenity::model::id::{ChannelId, GuildId, MessageId, UserId};

use crate::app_config::BanPeriodConfig;

/// 履歴のレコード
#[derive(Debug, Default, serde::Deserialize, PartialEq, Clone)]
pub struct HistoryRecord {
    /// 招待コード
    pub invite_code: String,
    /// 招待コードのギルドID
    pub invite_guild_id: GuildId,
    /// メッセージのギルドID
    pub guild_id: Option<GuildId>,
    /// メッセージのチャンネルID
    pub channel_id: ChannelId,
    /// メッセージID
    pub message_id: MessageId,
    /// 投稿者のID
    pub user_id: UserId,
    /// タイムスタンプ
    pub timestamp: i64,
    /// 削除済み
    pub deleted: bool,
}

/// 履歴を探すキー
pub enum HistoryFindKey {
    /// 招待コード
    InviteCode(String),
    /// 招待コードのギルドID
    InviteGuildId(GuildId),
}

/// 履歴管理クラス
pub struct HistoryLog {
    /// sql接続情報
    conn: Arc<Mutex<Connection>>,
    /// 同じ鯖の宣伝を禁止する設定
    pub ban_period: BanPeriodConfig,
}

impl HistoryLog {
    /// データベースを初期化する
    pub fn new(basedir: &str, ban_period: BanPeriodConfig) -> Result<HistoryLog> {
        // データベースに接続
        let conn = Connection::open(format!("{}/history_log.db", basedir))
            .context("履歴データベースのオープンに失敗")?;

        // テーブルを作成
        conn.execute(
            "CREATE TABLE IF NOT EXISTS history (
                id               INTEGER PRIMARY KEY AUTOINCREMENT,
                invite_code      VARCHAR(20) NOT NULL,
                invite_guild_id  VARCHAR(20) NOT NULL,
                guild_id         VARCHAR(20),
                channel_id       VARCHAR(20) NOT NULL,
                message_id       VARCHAR(20) NOT NULL,
                user_id          VARCHAR(20) NOT NULL,
                timestamp        TIMESTAMP   NOT NULL,
                deleted          INTEGER     NOT NULL DEFAULT 0
            )",
            params!(),
        )
        .context("履歴データベースの作成に失敗")?;

        // 初期化
        Ok(HistoryLog {
            conn: Arc::new(Mutex::new(conn)),
            ban_period,
        })
    }

    // 履歴にレコードを登録する
    pub async fn insert(&self, record: HistoryRecord) -> Result<()> {
        // データベースに書き込み
        self.conn
            .lock()
            .await
            .execute(
                "REPLACE INTO history (
                invite_code,
                invite_guild_id,
                guild_id,
                channel_id,
                message_id,
                user_id,
                timestamp,
                deleted
            )
            VALUES
                (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params!(
                    record.invite_code,
                    record.invite_guild_id.to_string(),
                    match record.guild_id {
                        Some(guild_id) => Some(guild_id.to_string()),
                        None => None,
                    },
                    record.channel_id.to_string(),
                    record.message_id.to_string(),
                    record.user_id.to_string(),
                    record.timestamp,
                    record.deleted,
                ),
            )
            .with_context(|| format!("履歴データベースへの書き込みに失敗: {:?}", record))?;

        Ok(())
    }

    // 履歴からレコードを削除
    pub async fn delete(&self, message_id: &MessageId) -> Result<()> {
        let ban_period_user_start =
            (Utc::now() - Duration::minutes(self.ban_period.min_per_user_start)).timestamp();

        self.conn
            .lock()
            .await
            .execute(
                "DELETE FROM
                    history
                WHERE
                    message_id = ?1
                    AND ?2 < timestamp",
                params!(message_id.to_string(), ban_period_user_start),
            )
            .with_context(|| format!("履歴データベースからの削除に失敗: {:?}", message_id))?;

        self.conn
            .lock()
            .await
            .execute(
                "UPDATE
                    history
                SET
                    deleted = 1
                WHERE
                    message_id = ?1
                    AND timestamp <= ?2",
                params!(message_id.to_string(), ban_period_user_start),
            )
            .with_context(|| {
                format!("履歴データベースで削除フラグの設定に失敗: {:?}", message_id)
            })?;

        Ok(())
    }

    // RowsからHistoryRecordを生成する
    fn rows_to_records(rows: Rows<'_>) -> impl Iterator<Item = HistoryRecord> + '_ {
        rows.mapped(|row| {
            // レコードの要素をSQLから取得
            let invite_code: String = row.get(0)?;
            let invite_guild_id: String = row.get(1)?;
            let guild_id: Option<String> = row.get(2)?;
            let channel_id: String = row.get(3)?;
            let message_id: String = row.get(4)?;
            let user_id: String = row.get(5)?;
            let timestamp: i64 = row.get(6)?;
            let deleted: i64 = row.get(7)?;
            Ok((
                invite_code,
                invite_guild_id,
                guild_id,
                channel_id,
                message_id,
                user_id,
                timestamp,
                deleted,
            ))
        })
        .map(|row| -> Result<HistoryRecord> {
            // 未パースの文字変数を展開
            let (
                invite_code,
                invite_guild_id,
                guild_id,
                channel_id,
                message_id,
                user_id,
                timestamp,
                deleted,
            ) = row?;
            // パースして構造体を作る
            Ok(HistoryRecord {
                invite_code,
                invite_guild_id: GuildId(invite_guild_id.parse()?),
                guild_id: match guild_id {
                    Some(guild_id) => Some(GuildId(guild_id.parse()?)),
                    None => None,
                },
                channel_id: ChannelId(channel_id.parse()?),
                message_id: MessageId(message_id.parse()?),
                user_id: UserId(user_id.parse()?),
                timestamp,
                deleted: deleted != 0,
            })
        })
        .filter_map(|row| row.ok())
    }

    // すでに履歴に登録されていないかチェックする
    pub async fn validate(
        &self,
        event_message_id: &MessageId,
        channel_id: &ChannelId,
        user_id: &UserId,
        key: &HistoryFindKey,
    ) -> Result<Vec<HistoryRecord>> {
        // データベースをロック
        let conn = self.conn.lock().await;
        // 検索するキーを指定
        let (search_key, search_value) = match key {
            HistoryFindKey::InviteCode(invite_code) => ("invite_code", invite_code.to_owned()),
            HistoryFindKey::InviteGuildId(invite_guild_id) => {
                ("invite_guild_id", invite_guild_id.to_string())
            }
        };
        // クエリを作成 (prepareでカラムを指定できなかったため、ここで検索キーを埋め込んで指定する)
        let query = format!(
            "SELECT
                invite_code,
                invite_guild_id,
                guild_id,
                channel_id,
                message_id,
                user_id,
                timestamp,
                deleted
            FROM
                history
            WHERE
                message_id != ?1
                AND channel_id = ?2
                AND {} = ?3
                AND (
                    (
                        user_id = ?4
                        AND ?5 < timestamp
                    )
                    OR (
                        user_id != ?4
                        AND ?6 < timestamp
                    )
                )",
            search_key
        );
        // クエリを構築
        let mut stmt = conn
            .prepare(&query)
            .with_context(|| format!("履歴チェック用のSQL文の構築に失敗: {}", query))?;
        // n日前以降を指定
        let ban_period = (Utc::now() - Duration::days(self.ban_period.day)).timestamp();
        let ban_period_user_end =
            (Utc::now() - Duration::days(self.ban_period.day_per_user)).timestamp();
        // クエリを実行
        let records = Self::rows_to_records(
            stmt.query(params!(
                event_message_id.to_string(),
                channel_id.to_string(),
                search_value,
                user_id.to_string(),
                ban_period_user_end,
                ban_period,
            ))
            .context("履歴データベースの読み込みに失敗")?,
        )
        .collect::<Vec<_>>();
        Ok(records)
    }

    // すでに履歴に登録されていないかチェックする
    pub async fn get_records_by_user(
        &self,
        guild_id: &Option<GuildId>,
        user_id: &UserId,
    ) -> Result<Vec<HistoryRecord>> {
        // データベースをロック
        let conn = self.conn.lock().await;
        // クエリを作成 (prepareでカラムを指定できなかったため、ここで検索キーを埋め込んで指定する)
        let query = "SELECT
                invite_code,
                invite_guild_id,
                guild_id,
                channel_id,
                message_id,
                user_id,
                timestamp,
                deleted
            FROM
                history
            WHERE
                guild_id = ?1
                AND user_id = ?2
                AND deleted = 0";
        // クエリを構築
        let mut stmt = conn
            .prepare(&query)
            .with_context(|| format!("ユーザー履歴チェック用のSQL文の構築に失敗: {}", query))?;
        // クエリを実行
        let records = Self::rows_to_records(
            stmt.query(params!(
                guild_id.map(|guild_id| guild_id.to_string()),
                user_id.to_string(),
            ))
            .context("履歴データベースの読み込みに失敗")?,
        )
        .collect::<Vec<_>>();
        Ok(records)
    }
}
