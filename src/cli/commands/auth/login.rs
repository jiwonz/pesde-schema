use anyhow::Context;
use clap::Args;
use colored::Colorize;
use serde::Deserialize;
use std::thread::spawn;
use url::Url;

use pesde::{
    source::{pesde::PesdePackageSource, traits::PackageSource},
    Project,
};

use crate::cli::auth::{get_token_login, set_token};

#[derive(Debug, Args)]
pub struct LoginCommand {
    /// The token to use for authentication, skipping login
    #[arg(short, long)]
    token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: Url,
    expires_in: u64,
    interval: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", tag = "error")]
enum AccessTokenError {
    AuthorizationPending,
    SlowDown { interval: u64 },
    ExpiredToken,
    AccessDenied,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum AccessTokenResponse {
    Success { access_token: String },

    Error(AccessTokenError),
}

impl LoginCommand {
    pub fn authenticate_device_flow(
        &self,
        index_url: &gix::Url,
        project: &Project,
        reqwest: &reqwest::blocking::Client,
    ) -> anyhow::Result<String> {
        println!("logging in into {index_url}");

        let source = PesdePackageSource::new(index_url.clone());
        source.refresh(project).context("failed to refresh index")?;

        let config = source
            .config(project)
            .context("failed to read index config")?;
        let Some(client_id) = config.github_oauth_client_id else {
            anyhow::bail!("index not configured for Github oauth.");
        };

        let response = reqwest
            .post(Url::parse_with_params(
                "https://github.com/login/device/code",
                &[("client_id", &client_id)],
            )?)
            .send()
            .context("failed to send device code request")?
            .error_for_status()
            .context("failed to get device code response")?
            .json::<DeviceCodeResponse>()
            .context("failed to parse device code response")?;

        println!(
            "copy your one-time code: {}\npress enter to open {} in your browser...",
            response.user_code.bold(),
            response.verification_uri.as_str().blue()
        );

        spawn(move || {
            {
                let mut input = String::new();
                std::io::stdin()
                    .read_line(&mut input)
                    .expect("failed to read input");
            }

            match open::that(response.verification_uri.as_str()) {
                Ok(_) => (),
                Err(e) => {
                    eprintln!("failed to open browser: {e}");
                }
            }
        });

        let mut time_left = response.expires_in;
        let mut interval = std::time::Duration::from_secs(response.interval);

        while time_left > 0 {
            std::thread::sleep(interval);
            time_left = time_left.saturating_sub(interval.as_secs());

            let response = reqwest
                .post(Url::parse_with_params(
                    "https://github.com/login/oauth/access_token",
                    &[
                        ("client_id", &client_id),
                        ("device_code", &response.device_code),
                        (
                            "grant_type",
                            &"urn:ietf:params:oauth:grant-type:device_code".to_string(),
                        ),
                    ],
                )?)
                .send()
                .context("failed to send access token request")?
                .error_for_status()
                .context("failed to get access token response")?
                .json::<AccessTokenResponse>()
                .context("failed to parse access token response")?;

            match response {
                AccessTokenResponse::Success { access_token } => {
                    return Ok(access_token);
                }
                AccessTokenResponse::Error(e) => match e {
                    AccessTokenError::AuthorizationPending => continue,
                    AccessTokenError::SlowDown {
                        interval: new_interval,
                    } => {
                        interval = std::time::Duration::from_secs(new_interval);
                        continue;
                    }
                    AccessTokenError::ExpiredToken => {
                        break;
                    }
                    AccessTokenError::AccessDenied => {
                        anyhow::bail!("access denied, re-run the login command");
                    }
                },
            }
        }

        anyhow::bail!("code expired, please re-run the login command");
    }

    pub fn run(
        self,
        index_url: gix::Url,
        project: Project,
        reqwest: reqwest::blocking::Client,
    ) -> anyhow::Result<()> {
        let token_given = self.token.is_some();
        let token = match self.token {
            Some(token) => token,
            None => self.authenticate_device_flow(&index_url, &project, &reqwest)?,
        };

        let token = if token_given {
            println!("set token for {index_url}");
            token
        } else {
            let token = format!("Bearer {token}");
            println!(
                "logged in as {} for {index_url}",
                get_token_login(&reqwest, &token)?.bold()
            );

            token
        };

        set_token(&index_url, Some(&token))?;

        Ok(())
    }
}
