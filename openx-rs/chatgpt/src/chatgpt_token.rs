use openx_core::OpenXAuth;
use openx_protocol::mcp_protocol::AuthMode;
use std::path::Path;
use std::sync::LazyLock;
use std::sync::RwLock;

use openx_core::token_data::TokenData;

static CHATGPT_TOKEN: LazyLock<RwLock<Option<TokenData>>> = LazyLock::new(|| RwLock::new(None));

pub fn get_chatgpt_token_data() -> Option<TokenData> {
    CHATGPT_TOKEN.read().ok()?.clone()
}

pub fn set_chatgpt_token_data(value: TokenData) {
    if let Ok(mut guard) = CHATGPT_TOKEN.write() {
        *guard = Some(value);
    }
}

/// Initialize the ChatGPT token from auth.json file
pub async fn init_chatgpt_token_from_auth(
    openx_home: &Path,
    originator: &str,
) -> std::io::Result<()> {
    let auth = OpenXAuth::from_openx_home(openx_home, AuthMode::ChatGPT, originator)?;
    if let Some(auth) = auth {
        let token_data = auth.get_token_data().await?;
        set_chatgpt_token_data(token_data);
    }
    Ok(())
}
