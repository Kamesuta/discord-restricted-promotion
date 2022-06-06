use std::env;
use regex::{Regex, Captures};

use serenity::async_trait;
use serenity::prelude::*;
use serenity::model::channel::Message;
use serenity::framework::standard::{StandardFramework};

use config::Config;

#[derive(Debug, Default, serde::Deserialize, PartialEq)]
struct DiscordConfig {
    channel: String,
}

#[derive(Debug, Default, serde::Deserialize, PartialEq)]
struct AppConfig {
    discord: DiscordConfig,
}

#[derive(Debug, Default, serde::Deserialize, PartialEq)]
struct DiscordInvite {
    expires_at: Option<String>,
}

// TODO クラス化
struct InviteFinder<'t> {
    invite_regex: Regex,
    message: String,
    captures: Captures<'t>,
}

struct Handler {
    app_config: AppConfig,
    invite_regex: Regex,
}

impl Handler {
    async fn get_expires_at(&self, msg: &str) -> Result<String, Box<dyn std::error::Error>> {
        let caps = self.invite_regex.captures(msg).ok_or("No invite found")?;
        let cap = caps.get(1).unwrap();
        let resp = reqwest::get(format!("https://discordapp.com/api/invite/{}", cap.as_str()))
            .await?
            .json::<serde_json::Value>()
            .await?;
        let expires_at = resp["expires_at"].as_str().ok_or("No expires_at")?;
        Ok(String::from(expires_at))
    }
}

#[async_trait]
impl EventHandler for Handler {
    async fn message(&self, ctx: Context, msg: Message) {
        if self.app_config.discord.channel != msg.channel_id.to_string() {
            return;
        }
        let result = self.get_expires_at(&msg.content.as_str()).await.unwrap();
        let _ = msg.channel_id.say(&ctx.http, format!("{} is not an invite", result.as_str())).await.unwrap();
    }
}

#[tokio::main]
async fn main() {
    let framework = StandardFramework::new()
        .configure(|c| c.prefix("~"));

    let config = Config::builder()
        // Add in `./Settings.toml`
        .add_source(config::File::with_name("config.toml"))
        // Add in settings from the environment (with a prefix of APP)
        // Eg.. `APP_DEBUG=1 ./target/app` would set the `debug` key
        .add_source(config::Environment::with_prefix("APP"))
        .build()
        .unwrap();
    let app_config = config.try_deserialize::<AppConfig>().unwrap();
    let invite_regex = Regex::new(r"(?:https?://)?discord\.gg/(\w+)").unwrap();
    let handler = Handler { app_config, invite_regex };

    // Login with a bot token from the environment
    let token = env::var("DISCORD_TOKEN").expect("token");
    let intents = GatewayIntents::non_privileged() | GatewayIntents::MESSAGE_CONTENT;
    let mut client = Client::builder(token, intents)
        .event_handler(handler)
        .framework(framework)
        .await
        .expect("Error creating client");

    // start listening for events by starting a single shard
    if let Err(why) = client.start().await {
        println!("An error occurred while running the client: {:?}", why);
    }
}
