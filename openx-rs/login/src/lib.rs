mod pkce;
mod server;

pub use server::LoginServer;
pub use server::ServerOptions;
pub use server::ShutdownHandle;
pub use server::run_login_server;

// Re-export commonly used auth types and helpers from openx-core for compatibility
pub use openx_core::AuthManager;
pub use openx_core::OpenXAuth;
pub use openx_core::auth::AuthDotJson;
pub use openx_core::auth::CLIENT_ID;
pub use openx_core::auth::OPENAI_API_KEY_ENV_VAR;
pub use openx_core::auth::get_auth_file;
pub use openx_core::auth::login_with_api_key;
pub use openx_core::auth::logout;
pub use openx_core::auth::try_read_auth_json;
pub use openx_core::auth::write_auth_json;
pub use openx_core::token_data::TokenData;
pub use openx_protocol::mcp_protocol::AuthMode;
