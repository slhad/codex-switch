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
    #[serde(default)]
    pub rate_limit: Option<UsageRateLimit>,
    #[serde(default)]
    pub spend_control: Option<SpendControl>,
    #[serde(default)]
    pub credits: Option<UsageCredits>,
    #[serde(default)]
    pub rate_limit_reset_credits: Option<RateLimitResetCredits>,
}

impl UsageResponse {
    pub fn five_hour_window(&self) -> Option<&UsageWindow> {
        self.rate_limit.as_ref()?.five_hour_window()
    }

    pub fn weekly_window(&self) -> Option<&UsageWindow> {
        self.rate_limit.as_ref()?.weekly_window()
    }

    pub fn monthly_limit(&self) -> Option<&MonthlyCreditLimit> {
        self.spend_control.as_ref()?.individual_limit.as_ref()
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct UsageRateLimit {
    #[serde(default)]
    pub primary_window: Option<UsageWindow>,
    #[serde(default)]
    pub secondary_window: Option<UsageWindow>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct UsageWindow {
    #[serde(default)]
    pub used_percent: Option<f64>,
    #[serde(default)]
    pub reset_at: Option<ResetAt>,
    #[serde(default)]
    pub limit_window_seconds: Option<u64>,
}

impl UsageRateLimit {
    pub fn five_hour_window(&self) -> Option<&UsageWindow> {
        self.windows()
            .find(|window| {
                window
                    .limit_window_seconds
                    .is_some_and(|seconds| seconds <= 21_600)
            })
            .or_else(|| {
                self.secondary_window
                    .as_ref()
                    .and(self.primary_window.as_ref())
            })
    }

    pub fn weekly_window(&self) -> Option<&UsageWindow> {
        self.windows()
            .find(|window| {
                window
                    .limit_window_seconds
                    .is_some_and(|seconds| seconds >= 518_400)
            })
            .or(self.secondary_window.as_ref())
            .or(self.primary_window.as_ref())
    }

    fn windows(&self) -> impl Iterator<Item = &UsageWindow> {
        self.primary_window
            .iter()
            .chain(self.secondary_window.iter())
    }
}

#[cfg(test)]
mod usage_tests {
    use super::{CreditAmount, RateLimitResetCredits, UsageResponse};

    #[test]
    fn identifies_weekly_only_primary_window() {
        let usage: UsageResponse = serde_json::from_str(
            r#"{"rate_limit":{"primary_window":{"used_percent":8,"limit_window_seconds":604800,"reset_at":1784487471},"secondary_window":null}}"#,
        )
        .unwrap();

        assert!(usage.five_hour_window().is_none());
        assert_eq!(usage.weekly_window().unwrap().used_percent, Some(8.0));
    }

    #[test]
    fn supports_legacy_windows_without_durations() {
        let usage: UsageResponse = serde_json::from_str(
            r#"{"rate_limit":{"primary_window":{"used_percent":12,"reset_at":1784487471},"secondary_window":{"used_percent":34,"reset_at":1784487472}}}"#,
        )
        .unwrap();

        assert_eq!(usage.five_hour_window().unwrap().used_percent, Some(12.0));
        assert_eq!(usage.weekly_window().unwrap().used_percent, Some(34.0));
    }

    #[test]
    fn supports_business_monthly_spend_controls_without_rate_limits() {
        let usage: UsageResponse = serde_json::from_str(
            r#"{
                "plan_type":"business",
                "rate_limit":null,
                "credits":{"balance":null,"has_credits":true,"unlimited":false,"overage_limit_reached":false},
                "spend_control":{"reached":false,"individual_limit":{
                    "limit":"12500","used":"94.48079252243042","remaining":"12405.51920747757",
                    "used_percent":1,"remaining_percent":99,"reset_after_seconds":1308136,
                    "reset_at":1785542400,"source":"group_based_spend_controls"
                }}
            }"#,
        )
        .unwrap();

        assert!(usage.five_hour_window().is_none());
        assert!(usage.weekly_window().is_none());
        let monthly = usage.monthly_limit().unwrap();
        assert_eq!(monthly.limit.as_ref().unwrap().as_f64(), Some(12_500.0));
        assert_eq!(monthly.used_percent, Some(1.0));
        assert_eq!(monthly.remaining_percent, Some(99.0));
        assert_eq!(usage.spend_control.as_ref().unwrap().reached, Some(false));
        assert_eq!(usage.credits.as_ref().unwrap().has_credits, Some(true));
        assert_eq!(CreditAmount::Number(12.5).as_f64(), Some(12.5));
    }

    #[test]
    fn supports_reset_credit_summary_and_expiration_details() {
        let usage: UsageResponse = serde_json::from_str(
            r#"{"rate_limit_reset_credits":{"available_count":1,"applicable_available_count":0}}"#,
        )
        .unwrap();
        let summary = usage.rate_limit_reset_credits.unwrap();
        assert_eq!(summary.available_count, Some(1));
        assert_eq!(summary.applicable_available_count, Some(0));

        let details: RateLimitResetCredits = serde_json::from_str(
            r#"{"credits":[{"status":"available","granted_at":"2026-07-13T18:09:35Z","expires_at":"2026-08-12T18:09:35Z","title":"Full reset","description":"One free reset"}],"available_count":1,"total_earned_count":0}"#,
        )
        .unwrap();
        assert_eq!(details.available_count, Some(1));
        assert_eq!(details.credits.len(), 1);
        assert_eq!(details.credits[0].status.as_deref(), Some("available"));
        assert_eq!(
            details.credits[0].expires_at.as_deref(),
            Some("2026-08-12T18:09:35Z")
        );
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SpendControl {
    #[serde(default)]
    pub individual_limit: Option<MonthlyCreditLimit>,
    #[serde(default)]
    pub reached: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct MonthlyCreditLimit {
    #[serde(default)]
    pub limit: Option<CreditAmount>,
    #[serde(default)]
    pub used: Option<CreditAmount>,
    #[serde(default)]
    pub remaining: Option<CreditAmount>,
    #[serde(default)]
    pub used_percent: Option<f64>,
    #[serde(default)]
    pub remaining_percent: Option<f64>,
    #[serde(default)]
    pub reset_after_seconds: Option<u64>,
    #[serde(default)]
    pub reset_at: Option<ResetAt>,
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum CreditAmount {
    Number(f64),
    String(String),
}

impl CreditAmount {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Number(value) => Some(*value),
            Self::String(value) => value.parse().ok(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct UsageCredits {
    #[serde(default)]
    pub balance: Option<CreditAmount>,
    #[serde(default)]
    pub has_credits: Option<bool>,
    #[serde(default)]
    pub unlimited: Option<bool>,
    #[serde(default)]
    pub overage_limit_reached: Option<bool>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct RateLimitResetCredits {
    #[serde(default)]
    pub available_count: Option<u64>,
    #[serde(default)]
    pub applicable_available_count: Option<u64>,
    #[serde(default)]
    pub total_earned_count: Option<u64>,
    #[serde(default)]
    pub credits: Vec<RateLimitResetCredit>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct RateLimitResetCredit {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub granted_at: Option<String>,
    #[serde(default)]
    pub expires_at: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
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

    /// Returns the unified profile automatic-switch and transfer options.
    pub fn profile_options_path(&self) -> PathBuf {
        self.state_dir.join("profile-options.json")
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
    use super::{die, read_pi_auth, Context, JwtAudience};
    use std::process::Command;

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

    #[test]
    fn context_uses_home_layout() {
        let ctx = Context::new();
        assert!(ctx.live_auth.ends_with(".codex/auth.json"));
        assert!(ctx.pi_auth.ends_with(".pi/agent/auth.json"));
        assert!(ctx.tracker_file.ends_with("codex-switch/accounts.json"));
        assert!(ctx
            .profile_options_path()
            .ends_with("codex-switch/profile-options.json"));
        assert!(ctx.profile_path("work").ends_with("codex/auth.json.work"));
        assert!(ctx.pi_profile_path("work").ends_with("pi/auth.json.work"));
    }

    #[test]
    fn die_helper() {
        if std::env::var_os("CODEX_SWITCH_TEST_DIE").is_some() {
            die("expected failure");
        }
    }

    #[test]
    fn die_prints_error_and_exits_unsuccessfully() {
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "data::tests::die_helper", "--nocapture"])
            .env("CODEX_SWITCH_TEST_DIE", "1")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&output.stderr).contains("codex-switch: expected failure"));
    }

    #[test]
    fn read_pi_auth_handles_missing_invalid_and_valid_files() {
        let base = std::env::temp_dir().join(format!("codex-switch-data-{}", std::process::id()));
        let path = base.join("auth.json");
        let _ = std::fs::remove_dir_all(&base);
        assert!(read_pi_auth(&path).is_none());

        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(&path, "not json").unwrap();
        assert!(read_pi_auth(&path).is_none());
        std::fs::write(&path, r#"{"openai-codex":null}"#).unwrap();
        assert!(read_pi_auth(&path).is_some());
        std::fs::remove_dir_all(base).unwrap();
    }
}

#[derive(Debug, Default, Deserialize, Serialize, Clone)]
pub struct AccountTracker {
    #[serde(default)]
    #[serde(alias = "accounts")]
    pub sessions: Vec<TrackedSession>,
    #[serde(default)]
    pub last_snapshot: Option<TrackedAuthSnapshot>,
    #[serde(default)]
    pub last_quota_hit: Option<TrackedQuotaHit>,
}

#[derive(Debug, Default, Deserialize, Serialize, Clone)]
pub struct TrackedQuotaHit {
    #[serde(default)]
    pub observed_at: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub window: Option<String>,
    #[serde(default)]
    pub previous_used_percent: Option<f64>,
    #[serde(default)]
    pub used_percent: Option<f64>,
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
    #[serde(default)]
    pub monthly_usage: Option<TrackedMonthlyUsage>,
}

#[derive(Debug, Default, Deserialize, Serialize, Clone)]
pub struct TrackedMonthlyUsage {
    #[serde(default)]
    pub observed_at: Option<String>,
    #[serde(default)]
    pub limit: Option<f64>,
    #[serde(default)]
    pub used: Option<f64>,
    #[serde(default)]
    pub remaining: Option<f64>,
    #[serde(default)]
    pub used_percent: Option<f64>,
    #[serde(default)]
    pub remaining_percent: Option<f64>,
    #[serde(default)]
    pub resets_at: Option<u64>,
    #[serde(default)]
    pub reached: Option<bool>,
    #[serde(default)]
    pub plan_type: Option<String>,
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
