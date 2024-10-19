use crate::cli::config::{read_config, write_config};
use anyhow::Context;
use gix::bstr::BStr;
use keyring::Entry;
use reqwest::header::AUTHORIZATION;
use serde::{ser::SerializeMap, Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct Tokens(pub BTreeMap<gix::Url, String>);

impl Serialize for Tokens {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::ser::Serializer,
    {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (k, v) in &self.0 {
            map.serialize_entry(&k.to_bstring().to_string(), v)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for Tokens {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::de::Deserializer<'de>,
    {
        Ok(Tokens(
            BTreeMap::<String, String>::deserialize(deserializer)?
                .into_iter()
                .map(|(k, v)| gix::Url::from_bytes(BStr::new(&k)).map(|k| (k, v)))
                .collect::<Result<_, _>>()
                .map_err(serde::de::Error::custom)?,
        ))
    }
}

pub fn get_tokens() -> anyhow::Result<Tokens> {
    let config = read_config()?;
    if !config.tokens.0.is_empty() {
        return Ok(config.tokens);
    }

    match Entry::new("tokens", env!("CARGO_PKG_NAME")) {
        Ok(entry) => match entry.get_password() {
            Ok(token) => return serde_json::from_str(&token).context("failed to parse tokens"),
            Err(keyring::Error::PlatformFailure(_) | keyring::Error::NoEntry) => {}
            Err(e) => return Err(e.into()),
        },
        Err(keyring::Error::PlatformFailure(_)) => {}
        Err(e) => return Err(e.into()),
    }

    Ok(Tokens(BTreeMap::new()))
}

pub fn set_tokens(tokens: Tokens) -> anyhow::Result<()> {
    let entry = Entry::new("tokens", env!("CARGO_PKG_NAME"))?;
    let json = serde_json::to_string(&tokens).context("failed to serialize tokens")?;

    match entry.set_password(&json) {
        Ok(()) => return Ok(()),
        Err(keyring::Error::PlatformFailure(_) | keyring::Error::NoEntry) => {}
        Err(e) => return Err(e.into()),
    }

    let mut config = read_config()?;
    config.tokens = tokens;
    write_config(&config).map_err(Into::into)
}

pub fn set_token(repo: &gix::Url, token: Option<&str>) -> anyhow::Result<()> {
    let mut tokens = get_tokens()?;
    if let Some(token) = token {
        tokens.0.insert(repo.clone(), token.to_string());
    } else {
        tokens.0.remove(repo);
    }
    set_tokens(tokens)
}

#[derive(Debug, Deserialize)]
struct UserResponse {
    login: String,
}

pub fn get_token_login(
    reqwest: &reqwest::blocking::Client,
    access_token: &str,
) -> anyhow::Result<String> {
    let response = reqwest
        .get("https://api.github.com/user")
        .header(AUTHORIZATION, access_token)
        .send()
        .context("failed to send user request")?
        .error_for_status()
        .context("failed to get user")?
        .json::<UserResponse>()
        .context("failed to parse user response")?;

    Ok(response.login)
}
