use crate::cli::{auth::Tokens, home_dir};
use anyhow::Context;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliConfig {
    #[serde(
        serialize_with = "crate::util::serialize_gix_url",
        deserialize_with = "crate::util::deserialize_gix_url"
    )]
    pub default_index: gix::Url,
    #[serde(
        serialize_with = "crate::util::serialize_gix_url",
        deserialize_with = "crate::util::deserialize_gix_url"
    )]
    pub scripts_repo: gix::Url,

    pub tokens: Tokens,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_checked_updates: Option<(chrono::DateTime<chrono::Utc>, semver::Version)>,
}

impl Default for CliConfig {
    fn default() -> Self {
        Self {
            default_index: "https://github.com/daimond113/pesde-index"
                .try_into()
                .unwrap(),
            scripts_repo: "https://github.com/daimond113/pesde-scripts"
                .try_into()
                .unwrap(),

            tokens: Tokens(Default::default()),

            last_checked_updates: None,
        }
    }
}

pub fn read_config() -> anyhow::Result<CliConfig> {
    let config_string = match std::fs::read_to_string(home_dir()?.join("config.toml")) {
        Ok(config_string) => config_string,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(CliConfig::default());
        }
        Err(e) => return Err(e).context("failed to read config file"),
    };

    let config = toml::from_str(&config_string).context("failed to parse config file")?;

    Ok(config)
}

pub fn write_config(config: &CliConfig) -> anyhow::Result<()> {
    let config_string = toml::to_string(config).context("failed to serialize config")?;
    std::fs::write(home_dir()?.join("config.toml"), config_string)
        .context("failed to write config file")?;

    Ok(())
}
