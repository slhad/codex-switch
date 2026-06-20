use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize, Serialize)]
pub struct AuthFile {
    pub tokens: Tokens,
    #[serde(default)]
    pub auth_mode: Option<String>,
    #[serde(default)]
    pub last_refresh: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Tokens {
    pub id_token: String,
    #[serde(default)]
    pub access_token: Option<String>,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub account_id: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct UsageResponse {
    #[serde(default)]
    pub plan_type: Option<String>,
    pub rate_limit: UsageRateLimit,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct UsageRateLimit {
    pub primary_window: UsageWindow,
    pub secondary_window: UsageWindow,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct UsageWindow {
    #[serde(default)]
    pub used_percent: Option<f64>,
    #[serde(default)]
    pub reset_at: Option<ResetAt>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ResetAt {
    Epoch(u64),
    Rfc3339(String),
}

#[derive(Debug, Deserialize)]
pub struct JwtPayload {
    #[serde(default)]
    pub aud: Option<JwtAudience>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default, rename = "https://api.openai.com/profile")]
    pub openai_profile: Option<OpenAiProfile>,
    #[serde(default, rename = "https://api.openai.com/auth")]
    pub openai_auth: Option<OpenAiAuth>,
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub sub: Option<String>,
    #[serde(default)]
    pub iat: Option<u64>,
    #[serde(default)]
    pub exp: Option<u64>,
    #[serde(default)]
    pub scp: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum JwtAudience {
    One(String),
    Many(Vec<String>),
}

impl JwtAudience {
    pub fn first_app_client_id(&self) -> Option<&str> {
        match self {
            JwtAudience::One(value) => value.starts_with("app_").then_some(value.as_str()),
            JwtAudience::Many(values) => values
                .iter()
                .find(|value| value.starts_with("app_"))
                .map(String::as_str),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct OpenAiProfile {
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub email_verified: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct OpenAiAuth {
    #[serde(default)]
    pub chatgpt_plan_type: Option<String>,
    #[serde(default)]
    pub chatgpt_account_id: Option<String>,
    #[serde(default)]
    pub chatgpt_account_user_id: Option<String>,
    #[serde(default)]
    pub chatgpt_user_id: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct PiAuthFile {
    #[serde(rename = "openai-codex")]
    pub openai_codex: Option<PiOpenAiCodexAuth>,
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq)]
pub struct PiOpenAiCodexAuth {
    #[serde(rename = "type", default)]
    pub auth_type: Option<String>,
    pub access: String,
    #[serde(default)]
    pub refresh: Option<String>,
    #[serde(rename = "accountId")]
    pub account_id: Option<String>,
    pub expires: Option<u64>,
}

#[derive(Debug)]
pub struct Context {
    pub live_auth: PathBuf,
    pub pi_auth: PathBuf,
    pub state_dir: PathBuf,
    pub tracker_file: PathBuf,
}

impl Context {
    pub fn new() -> Self {
        let home = std::env::var("HOME").expect("HOME not set");
        let codex_dir = PathBuf::from(&home).join(".codex");
        let live_auth = codex_dir.join("auth.json");
        let pi_auth = PathBuf::from(&home)
            .join(".pi")
            .join("agent")
            .join("auth.json");
        let state_dir = PathBuf::from(&home)
            .join(".local")
            .join("state")
            .join("codex-switch");
        let tracker_file = state_dir.join("accounts.json");
        Self {
            live_auth,
            pi_auth,
            state_dir,
            tracker_file,
        }
    }

    /// Returns the path for a profile file, e.g. ~/.codex/auth.json.work
    pub fn codex_profiles_dir(&self) -> PathBuf {
        self.state_dir.join("profiles").join("codex")
    }

    /// Returns the path for a PI profile file store, e.g. ~/.local/state/codex-switch/profiles/pi
    pub fn pi_profiles_dir(&self) -> PathBuf {
        self.state_dir.join("profiles").join("pi")
    }

    /// Returns the path for a profile file, e.g. ~/.local/state/codex-switch/profiles/codex/auth.json.work
    pub fn profile_path(&self, name: &str) -> PathBuf {
        self.codex_profiles_dir()
            .join(format!("auth.json.{}", name))
    }

    /// Returns the path for a PI profile file, e.g. ~/.local/state/codex-switch/profiles/pi/auth.json.work
    pub fn pi_profile_path(&self, name: &str) -> PathBuf {
        self.pi_profiles_dir().join(format!("auth.json.{}", name))
    }
}

pub fn die(msg: &str) -> ! {
    eprintln!("codex-switch: {}", msg);
    std::process::exit(1);
}

pub fn read_auth(path: &Path) -> AuthFile {
    let content = std::fs::read_to_string(path)
        .unwrap_or_else(|e| die(&format!("missing file: {} — {}", path.display(), e)));
    serde_json::from_str(&content)
        .unwrap_or_else(|e| die(&format!("invalid JSON: {} — {}", path.display(), e)))
}

pub fn read_pi_auth(path: &Path) -> Option<PiAuthFile> {
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

#[cfg(test)]
mod tests {
    use super::JwtAudience;

    #[test]
    fn jwt_audience_finds_app_client_id() {
        assert_eq!(
            JwtAudience::One("app_one".to_string()).first_app_client_id(),
            Some("app_one")
        );
        assert_eq!(
            JwtAudience::Many(vec!["api".to_string(), "app_many".to_string()])
                .first_app_client_id(),
            Some("app_many")
        );
        assert_eq!(
            JwtAudience::One("https://api.openai.com/v1".to_string()).first_app_client_id(),
            None
        );
    }
}

#[derive(Debug, Default, Deserialize, Serialize, Clone)]
pub struct AccountTracker {
    #[serde(default)]
    #[serde(alias = "accounts")]
    pub sessions: Vec<TrackedSession>,
    #[serde(default)]
    pub last_snapshot: Option<TrackedAuthSnapshot>,
}

#[derive(Debug, Default, Deserialize, Serialize, Clone)]
pub struct TrackedSession {
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub auth_path: Option<String>,
    #[serde(default)]
    pub account_id: String,
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub auth_mode: Option<String>,
    #[serde(default)]
    pub last_refresh: Option<String>,
    #[serde(default)]
    pub access_expires_at: Option<u64>,
    #[serde(default)]
    pub has_refresh: bool,
    #[serde(default)]
    pub refresh_fingerprint: Option<String>,
    #[serde(default)]
    pub last_seen_at: Option<String>,
    #[serde(default)]
    pub rate_limit: Option<TrackedRateLimit>,
}

#[derive(Debug, Default, Deserialize, Serialize, Clone)]
pub struct TrackedRateLimit {
    #[serde(default)]
    pub observed_at: Option<String>,
    #[serde(default)]
    pub used_percent: Option<f64>,
    pub resets_at: u64,
    #[serde(default)]
    pub secondary_used_percent: Option<f64>,
    #[serde(default)]
    pub secondary_resets_at: Option<u64>,
    #[serde(default)]
    pub plan_type: Option<String>,
}

#[derive(Debug, Default, Deserialize, Serialize, Clone)]
pub struct TrackedAuthSnapshot {
    pub auth_json: String,
    #[serde(default)]
    pub observed_at: Option<String>,
    #[serde(default)]
    pub profile: Option<String>,
}
