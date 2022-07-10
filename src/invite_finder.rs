use anyhow::{anyhow, Context as _, Result};
use chrono::prelude::*;
use futures::future::try_join_all;
use regex::Regex;
use serenity::model::id::GuildId;

/// パース用ギルド情報
#[derive(Debug, Default, serde::Deserialize, PartialEq, Clone)]
pub struct DiscordInviteGuild {
    /// ギルドID
    pub id: GuildId,
}

/// パース用招待コード
#[derive(Debug, Default, serde::Deserialize, PartialEq, Clone)]
pub struct DiscordInvite {
    /// 有効期限
    pub expires_at: Option<String>,
    /// ギルド情報
    pub guild: Option<DiscordInviteGuild>,
}

/// 招待リンクの情報
#[derive(Debug, Default, serde::Deserialize, PartialEq, Clone)]
pub struct DiscordInviteLink<'t> {
    /// 招待リンクのURL
    pub invite_link: &'t str,
    /// 招待コード
    pub invite_code: &'t str,
    /// 招待コードの有効期限
    pub expires_at: Option<DateTime<FixedOffset>>,
    /// 招待コードのギルドID
    pub guild_id: Option<GuildId>,
}

/// 招待リンク検索用クラス
pub struct InviteFinder<'t> {
    /// 招待
    pub invite_codes: Vec<DiscordInviteLink<'t>>,
}

impl<'t> InviteFinder<'t> {
    /// メッセージをパースする
    pub fn new(message: &'t str) -> Result<InviteFinder<'t>> {
        // 正規表現パターンを準備
        let invite_regex = Regex::new(
            r"(?:https?://)?(?:discord\.(?:gg|io|me|li)|(?:discord|discordapp)\.com/invite)/([A-Za-z1-9]+)",
        )
        .context("正規表現のパターンの作成に失敗")?;

        // 招待コードリストを取得
        let invite_codes = invite_regex
            .captures_iter(message)
            .map(|c| {
                Ok(DiscordInviteLink {
                    invite_link: c
                        .get(0)
                        .ok_or_else(|| anyhow!("招待リンクのパース失敗"))?
                        .as_str(),
                    invite_code: c
                        .get(1)
                        .ok_or_else(|| anyhow!("招待コードのパース失敗"))?
                        .as_str(),
                    expires_at: None,
                    guild_id: None,
                })
            })
            .collect::<Result<Vec<DiscordInviteLink>>>()?;

        Ok(InviteFinder { invite_codes })
    }

    /// APIから招待リンクの詳細を取得する
    pub async fn get_invite_list(&self) -> Result<Vec<DiscordInviteLink<'t>>> {
        try_join_all(self.invite_codes.iter().map(|invite_link| async move {
            // APIリクエストを構築
            let invite_url = format!(
                "https://discord.com/api/v10/invites/{}",
                invite_link.invite_code
            );
            // APIリクエストを実行
            let invite_response = reqwest::get(&invite_url)
                .await
                .context("招待リンクの取得に失敗しました")?;
            // 招待リンク情報をパース
            let invite_result = invite_response
                .json::<DiscordInvite>()
                .await
                .context("招待リンク情報のパースに失敗しました")?;
            // 招待リンクの有効期限を抽出
            let expires_at = match invite_result.expires_at {
                Some(expires_at) => Some(
                    // 期限付きの招待リンク
                    DateTime::parse_from_rfc3339(expires_at.as_str())
                        .context("招待リンクの有効期限のパースに失敗しました")?,
                ),
                None => None, // 無期限リンク
            };
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
