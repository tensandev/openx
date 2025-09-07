use openx_common::CliConfigOverrides;
use openx_core::OpenXAuth;
use openx_core::auth::CLIENT_ID;
use openx_core::auth::OPENAI_API_KEY_ENV_VAR;
use openx_core::auth::login_with_api_key;
use openx_core::auth::logout;
use openx_core::config::Config;
use openx_core::config::ConfigOverrides;
use openx_login::ServerOptions;
use openx_login::run_login_server;
use openx_protocol::mcp_protocol::AuthMode;
use std::env;
use std::path::PathBuf;

pub async fn login_with_chatgpt(openx_home: PathBuf, originator: String) -> std::io::Result<()> {
    let opts = ServerOptions::new(openx_home, CLIENT_ID.to_string(), originator);
    let server = run_login_server(opts)?;

    eprintln!(
        "Starting local login server on http://localhost:{}.\nIf your browser did not open, navigate to this URL to authenticate:\n\n{}",
        server.actual_port, server.auth_url,
    );

    server.block_until_done().await
}

pub async fn run_login_with_chatgpt(cli_config_overrides: CliConfigOverrides) -> ! {
    let config = load_config_or_exit(cli_config_overrides);

    match login_with_chatgpt(
        config.openx_home,
        config.responses_originator_header.clone(),
    )
    .await
    {
        Ok(_) => {
            eprintln!("Successfully logged in");
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("Error logging in: {e}");
            std::process::exit(1);
        }
    }
}

pub async fn run_login_with_api_key(
    cli_config_overrides: CliConfigOverrides,
    api_key: String,
) -> ! {
    let config = load_config_or_exit(cli_config_overrides);

    match login_with_api_key(&config.openx_home, &api_key) {
        Ok(_) => {
            eprintln!("Successfully logged in");
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("Error logging in: {e}");
            std::process::exit(1);
        }
    }
}

pub async fn run_login_status(cli_config_overrides: CliConfigOverrides) -> ! {
    let config = load_config_or_exit(cli_config_overrides);

    match OpenXAuth::from_openx_home(
        &config.openx_home,
        config.preferred_auth_method,
        &config.responses_originator_header,
    ) {
        Ok(Some(auth)) => match auth.mode {
            AuthMode::ApiKey => match auth.get_token().await {
                Ok(api_key) => {
                    eprintln!("Logged in using an API key - {}", safe_format_key(&api_key));

                    if let Ok(env_api_key) = env::var(OPENAI_API_KEY_ENV_VAR)
                        && env_api_key == api_key
                    {
                        eprintln!(
                            "   API loaded from OPENAI_API_KEY environment variable or .env file"
                        );
                    }
                    std::process::exit(0);
                }
                Err(e) => {
                    eprintln!("Unexpected error retrieving API key: {e}");
                    std::process::exit(1);
                }
            },
            AuthMode::ChatGPT => {
                eprintln!("Logged in using ChatGPT");
                std::process::exit(0);
            }
        },
        Ok(None) => {
            eprintln!("Not logged in");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("Error checking login status: {e}");
            std::process::exit(1);
        }
    }
}

pub async fn run_logout(cli_config_overrides: CliConfigOverrides) -> ! {
    let config = load_config_or_exit(cli_config_overrides);

    match logout(&config.openx_home) {
        Ok(true) => {
            eprintln!("Successfully logged out");
            std::process::exit(0);
        }
        Ok(false) => {
            eprintln!("Not logged in");
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("Error logging out: {e}");
            std::process::exit(1);
        }
    }
}

fn load_config_or_exit(cli_config_overrides: CliConfigOverrides) -> Config {
    let cli_overrides = match cli_config_overrides.parse_overrides() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Error parsing -c overrides: {e}");
            std::process::exit(1);
        }
    };

    let config_overrides = ConfigOverrides::default();
    match Config::load_with_cli_overrides(cli_overrides, config_overrides) {
        Ok(config) => config,
        Err(e) => {
            eprintln!("Error loading configuration: {e}");
            std::process::exit(1);
        }
    }
}

fn safe_format_key(key: &str) -> String {
    if key.len() <= 13 {
        return "***".to_string();
    }
    let prefix = &key[..8];
    let suffix = &key[key.len() - 5..];
    format!("{prefix}***{suffix}")
}

#[cfg(test)]
mod tests {
    use super::safe_format_key;

    #[test]
    fn formats_long_key() {
        let key = "sk-proj-1234567890ABCDE";
        assert_eq!(safe_format_key(key), "sk-proj-***ABCDE");
    }

    #[test]
    fn short_key_returns_stars() {
        let key = "sk-proj-12345";
        assert_eq!(safe_format_key(key), "***");
    }
}
