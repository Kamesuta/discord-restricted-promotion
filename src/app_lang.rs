#![macro_use]

use gettext::Catalog;
use gettext_macros::*;
use anyhow::{Context as _, Result};

// 国際化対応
init_i18n!("discord_restricted_promotion", ja);

// .pot から .po を生成し、.mo を生成する
// コンパイルにはPCに gettext.tools をインストールする必要がある
// インストール (Windows): https://mlocati.github.io/articles/gettext-iconv-windows.html
// .po のエディター: https://poedit.net/download
compile_i18n!();

pub fn cat(lang: &str) -> Result<Catalog> {
    // include_i18n! embeds translations in your binary.
    // It gives a Vec<(&'static str, Catalog)> (list of catalogs with their associated language).
    let catalogs = include_i18n!();
    let catalog = catalogs.into_iter()
        .find(|&(langkey, _)| lang == langkey)
        .map(|(_, catalog)| catalog)
        .with_context(|| format!("指定された言語 {} は対応していません", lang))?;
    Ok(catalog)
}
