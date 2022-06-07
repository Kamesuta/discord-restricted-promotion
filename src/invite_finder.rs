use chrono::prelude::*;
use regex::Regex;
use serenity::model::id::GuildId;
use std::error::Error;

/// パース用ギルド情報
#[derive(Debug, Default, serde::Deserialize, PartialEq)]
pub struct DiscordInviteGuild {
    /// ギルドID
    pub id: GuildId,
}

/// パース用招待コード
#[derive(Debug, Default, serde::Deserialize, PartialEq)]
pub struct DiscordInvite {
    /// 有効期限
    pub expires_at: Option<String>,
    /// ギルド情報
    pub guild: Option<DiscordInviteGuild>,
}

/// 招待リンクの情報
pub struct DiscordInviteLink<'t> {
    /// 招待リンクのURL
    pub invite_link: &'t str,
    /// 招待コード
    pub invite_code: &'t str,
    /// 招待コードの有効期限
    pub expires_at: Option<DateTime<FixedOffset>>,
    /// 招待コードが有効期限切れかどうか
    pub guild_id: Option<GuildId>,
}

/// 招待リンク検索用クラス
pub struct InviteFinder<'t> {
    /// 招待
    pub invite_codes: Vec<DiscordInviteLink<'t>>,
}

impl<'t> InviteFinder<'t> {
    /// メッセージをパースする
    pub fn new(message: &'t str) -> InviteFinder<'t> {
        // 正規表現パターンを準備
        let invite_regex = Regex::new(r"(?:https?://)?discord\.gg/(\w+)").unwrap();

        // 招待コードリストを取得
        let invite_codes = invite_regex
            .captures_iter(message)
            .map(|c| DiscordInviteLink {
                invite_link: c.get(0).unwrap().as_str(),
                invite_code: c.get(1).unwrap().as_str(),
                expires_at: None,
                guild_id: None,
            })
            .collect::<Vec<_>>();

        InviteFinder { invite_codes }
    }

    /// APIから招待リンクの詳細を取得する
    pub async fn get_invite_list(&self) -> Result<Vec<DiscordInviteLink<'t>>, Box<dyn Error>> {
        futures::future::try_join_all(self.invite_codes.iter().map(|invite_link| async move {
            // APIリクエストを構築
            let invite_url = format!(
                "https://discord.com/api/v10/invites/{}",
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
            // 招待リンクのギルドIDを抽出
            let guild_id = invite_result.guild.map(|g| g.id);

            // 有効期限をセットした構造体を返す
            Ok(DiscordInviteLink {
                expires_at,
                guild_id,
                ..*invite_link
            })
        }))
        .await
    }
}
