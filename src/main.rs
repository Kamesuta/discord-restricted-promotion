mod app_config;
mod event_handler;
mod invite_finder;

use app_config::AppConfig;
use event_handler::Handler;
use std::{env, error::Error};

use serenity::framework::standard::StandardFramework;
use serenity::prelude::*;

/// メイン処理
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // フレームワークを初期化
    let framework = StandardFramework::new().configure(|c| c.prefix("~"));

    // 設定ファイルを読み込む
    let app_config = match AppConfig::load_config() {
        Ok(config) => config,
        Err(why) => {
            println!("設定ファイルの読み込みに失敗: {:?}", why);
            return Err(why);
        }
    };

    // イベント受信リスナーを構築
    let handler = Handler { app_config };

    // 環境変数のトークンを使用してDiscord APIを初期化
    let token = env::var("DISCORD_TOKEN").expect("token");
    let intents = GatewayIntents::non_privileged() | GatewayIntents::MESSAGE_CONTENT;
    let mut client = match Client::builder(token, intents)
        .event_handler(handler)
        .framework(framework)
        .await
    {
        Ok(client) => client,
        Err(why) => {
            println!("Botの初期化に失敗しました: {:?}", why);
            return Err(why.into());
        }
    };

    // イベント受信を開始
    match client.start().await {
        Ok(_) => (),
        Err(why) => {
            println!("Bot動作中にエラーが発生しました: {:?}", why);
            return Err(why.into());
        }
    };

    Ok(())
}
