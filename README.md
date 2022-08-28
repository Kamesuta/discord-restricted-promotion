# Discord鯖宣伝規制Bot

## 概要
Discord鯖宣伝で
- 無期限リンクであること
- 説明文が書かれていること
- 直近で宣伝されていないこと

を自動でチェックする

# スクリーンショット
![イメージ](https://user-images.githubusercontent.com/16362824/187067515-6883981a-c5d8-40ea-995b-de5cf084b790.png)

## セットアップ

- 環境変数 `DISCORD_TOKEN` にBotのトークンを登録します
- `config.default.toml` をコピーし `config.toml` を作成します
- `config.toml` の設定を変更します
- `cargo run` で起動します

|設定名|説明|
|----|----|
|discord.channels|規制対象のチャンネルID|
|discord.alert_sec|警告を表示する秒数|
|discord.required_message_length|必要なメッセージの長さ|
|discord.ignore_roles|警告を貫通するロールID|
|ban_period.day|同じ鯖の宣伝を禁止する日数|
|ban_period.day_per_user|同じユーザーが同じ鯖の宣伝を禁止する日数|
|ban_period.min_per_user_start|同じユーザーが同じ鯖の宣伝を再投稿できる分数|
|message.alert_emoji|警告の絵文字|
|message.no_expiration_invite_link_guide|無期限招待リンクの作成方法紹介ページURL|
