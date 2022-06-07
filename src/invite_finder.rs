use chrono::prelude::*;
use regex::Regex;
use std::error::Error;

#[derive(Debug, Default, serde::Deserialize, PartialEq)]
pub struct DiscordInvite {
    pub expires_at: Option<String>,
}

pub struct DiscordInviteLink<'t> {
    pub invite_link: &'t str,
    pub invite_code: &'t str,
    pub expires_at: Option<DateTime<FixedOffset>>,
}

pub struct InviteFinder<'t> {
    pub invite_codes: Vec<DiscordInviteLink<'t>>,
}

impl<'t> InviteFinder<'t> {
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
            })
            .collect::<Vec<_>>();

        InviteFinder { invite_codes }
    }

    pub async fn get_invite_list(&self) -> Result<Vec<DiscordInviteLink<'t>>, Box<dyn Error>> {
        futures::future::try_join_all(self.invite_codes.iter().map(|invite_link| async move {
            // APIリクエストを構築
            let invite_url = format!(
                "https://discordapp.com/api/v9/invites/{}",
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

            // 有効期限をセットした構造体を返す
            Ok(DiscordInviteLink {
                expires_at,
                ..*invite_link
            })
        }))
        .await
    }
}
