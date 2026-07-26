use crate::data::{
    read_auth, read_pi_auth, AuthFile, Context, CreditAmount, PiOpenAiCodexAuth, ResetAt,
    UsageResponse, UsageWindow,
};
use crate::jwt::decode_token_payload;
use chrono::{DateTime, Days, Local, TimeZone, Utc};
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION};
use std::path::Path;

const USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
const TOKEN_REFRESH_URL: &str = "https://auth.openai.com/oauth/token";

pub fn fetch_rate_limit(ctx: &Context) -> Result<UsageResponse, String> {
    fetch_rate_limit_for_auth_path(&ctx.live_auth).map(|(usage, _)| usage)
}

pub fn fetch_rate_limit_read_only(ctx: &Context) -> Result<UsageResponse, String> {
    fetch_rate_limit_for_auth_path_read_only(&ctx.live_auth)
}

pub fn fetch_rate_limit_for_auth_path_read_only(path: &Path) -> Result<UsageResponse, String> {
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("failed to build HTTP client: {}", e))?;
    let auth = read_auth(path);
    let response = send_usage_request(&client, &auth)?;
    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err("401 unauthorized from usage API; dry-run does not refresh tokens".to_string());
    }
    if response.status() == reqwest::StatusCode::FORBIDDEN {
        return Err("403 forbidden from usage API".to_string());
    }
    response
        .error_for_status()
        .map_err(|e| format!("usage request failed: {}", e))?
        .json::<UsageResponse>()
        .map_err(|e| format!("invalid usage response: {}", e))
}

pub fn fetch_rate_limit_for_auth_path(path: &Path) -> Result<(UsageResponse, AuthFile), String> {
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("failed to build HTTP client: {}", e))?;

    let mut auth = read_auth(path);
    if needs_refresh(auth.last_refresh.as_deref()) {
        auth = refresh_auth_at_path(&client, path, auth)?;
    }

    let mut response = send_usage_request(&client, &auth)?;
    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        auth = refresh_auth_at_path(&client, path, auth)?;
        response = send_usage_request(&client, &auth)?;
    }

    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err("401 unauthorized from usage API; run `codex --login` again".to_string());
    }
    if response.status() == reqwest::StatusCode::FORBIDDEN {
        return Err("403 forbidden from usage API".to_string());
    }

    let usage = response
        .error_for_status()
        .map_err(|e| format!("usage request failed: {}", e))?
        .json::<UsageResponse>()
        .map_err(|e| format!("invalid usage response: {}", e))?;

    Ok((usage, auth))
}

pub fn fetch_pi_rate_limit(
    ctx: &Context,
    auth: PiOpenAiCodexAuth,
) -> Result<(UsageResponse, PiOpenAiCodexAuth), String> {
    fetch_pi_rate_limit_for_path(&ctx.pi_auth, auth)
}

pub fn fetch_pi_rate_limit_read_only(auth: &PiOpenAiCodexAuth) -> Result<UsageResponse, String> {
    fetch_pi_rate_limit_for_auth_read_only(auth)
}

pub fn fetch_pi_rate_limit_for_auth_read_only(
    auth: &PiOpenAiCodexAuth,
) -> Result<UsageResponse, String> {
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("failed to build HTTP client: {}", e))?;
    let response =
        send_usage_request_with_token(&client, &auth.access, auth.account_id.as_deref())?;
    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err(
            "401 unauthorized from PI usage API; dry-run does not refresh tokens".to_string(),
        );
    }
    if response.status() == reqwest::StatusCode::FORBIDDEN {
        return Err("403 forbidden from PI usage API".to_string());
    }
    response
        .error_for_status()
        .map_err(|e| format!("usage request failed: {}", e))?
        .json::<UsageResponse>()
        .map_err(|e| format!("invalid usage response: {}", e))
}

pub fn fetch_pi_rate_limit_for_path(
    path: &Path,
    mut auth: PiOpenAiCodexAuth,
) -> Result<(UsageResponse, PiOpenAiCodexAuth), String> {
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("failed to build HTTP client: {}", e))?;

    if needs_pi_refresh(auth.expires) {
        auth = refresh_pi_auth_at_path(&client, path, auth)?;
    }

    let mut response =
        send_usage_request_with_token(&client, &auth.access, auth.account_id.as_deref())?;
    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        auth = refresh_pi_auth_at_path(&client, path, auth)?;
        response =
            send_usage_request_with_token(&client, &auth.access, auth.account_id.as_deref())?;
    }

    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err("401 unauthorized from PI usage API; refresh or re-login required".to_string());
    }
    if response.status() == reqwest::StatusCode::FORBIDDEN {
        return Err("403 forbidden from PI usage API".to_string());
    }

    let usage = response
        .error_for_status()
        .map_err(|e| format!("usage request failed: {}", e))?
        .json::<UsageResponse>()
        .map_err(|e| format!("invalid usage response: {}", e))?;

    Ok((usage, auth))
}

fn send_usage_request(
    client: &Client,
    auth: &AuthFile,
) -> Result<reqwest::blocking::Response, String> {
    let access_token = auth
        .tokens
        .access_token
        .as_deref()
        .ok_or_else(|| "missing OAuth access token in ~/.codex/auth.json".to_string())?;
    send_usage_request_with_token(client, access_token, auth.tokens.account_id.as_deref())
}

fn codex_client_id(auth: &AuthFile) -> Option<String> {
    auth.tokens
        .access_token
        .as_deref()
        .and_then(token_client_id)
        .or_else(|| token_client_id(&auth.tokens.id_token))
}

fn pi_client_id(auth: &PiOpenAiCodexAuth) -> Option<String> {
    token_client_id(&auth.access)
}

fn token_client_id(token: &str) -> Option<String> {
    let payload = decode_token_payload(token)?;
    payload.client_id.or_else(|| {
        payload
            .aud
            .as_ref()
            .and_then(|aud| aud.first_app_client_id())
            .map(str::to_string)
    })
}

fn send_usage_request_with_token(
    client: &Client,
    access_token: &str,
    account_id: Option<&str>,
) -> Result<reqwest::blocking::Response, String> {
    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {}", access_token))
            .map_err(|e| format!("invalid access token header: {}", e))?,
    );

    let mut request = client.get(USAGE_URL).headers(headers);
    if let Some(account_id) = account_id {
        request = request.header("ChatGPT-Account-Id", account_id);
    }

    request
        .send()
        .map_err(|e| format!("usage request failed: {}", e))
}

fn refresh_auth_at_path(
    client: &Client,
    path: &Path,
    mut auth: AuthFile,
) -> Result<AuthFile, String> {
    let refresh_token = auth
        .tokens
        .refresh_token
        .as_deref()
        .ok_or_else(|| "missing OAuth refresh token in ~/.codex/auth.json".to_string())?;
    let client_id = codex_client_id(&auth)
        .ok_or_else(|| "missing OAuth client_id in Codex auth tokens".to_string())?;

    let response = client
        .post(TOKEN_REFRESH_URL)
        .json(&serde_json::json!({
            "client_id": client_id,
            "grant_type": "refresh_token",
            "refresh_token": refresh_token,
        }))
        .send()
        .map_err(|e| format!("token refresh failed: {}", e))?;

    let response = response
        .error_for_status()
        .map_err(|e| format!("token refresh failed: {}", e))?;

    let refreshed: RefreshResponse = response
        .json()
        .map_err(|e| format!("invalid token refresh response: {}", e))?;

    auth.tokens.access_token = Some(refreshed.access_token);
    if let Some(refresh_token) = refreshed.refresh_token {
        auth.tokens.refresh_token = Some(refresh_token);
    }
    if let Some(id_token) = refreshed.id_token {
        auth.tokens.id_token = id_token;
    }
    auth.last_refresh = Some(Utc::now().to_rfc3339());

    let content = serde_json::to_string_pretty(&auth)
        .map_err(|e| format!("failed to serialize refreshed auth: {}", e))?;
    std::fs::write(path, format!("{}\n", content))
        .map_err(|e| format!("failed to write refreshed auth: {}", e))?;

    Ok(auth)
}

fn refresh_pi_auth_at_path(
    client: &Client,
    path: &Path,
    mut auth: PiOpenAiCodexAuth,
) -> Result<PiOpenAiCodexAuth, String> {
    let original_auth = auth.clone();
    let refresh_token = auth
        .refresh
        .as_deref()
        .ok_or_else(|| "missing OAuth refresh token in ~/.pi/agent/auth.json".to_string())?;
    let client_id = pi_client_id(&auth)
        .ok_or_else(|| "missing OAuth client_id in PI auth token".to_string())?;

    let response = client
        .post(TOKEN_REFRESH_URL)
        .json(&serde_json::json!({
            "client_id": client_id,
            "grant_type": "refresh_token",
            "refresh_token": refresh_token,
        }))
        .send()
        .map_err(|e| format!("PI token refresh failed: {}", e))?;

    let response = response
        .error_for_status()
        .map_err(|e| format!("PI token refresh failed: {}", e))?;

    let refreshed: RefreshResponse = response
        .json()
        .map_err(|e| format!("invalid PI token refresh response: {}", e))?;

    auth.access = refreshed.access_token;
    if let Some(refresh_token) = refreshed.refresh_token {
        auth.refresh = Some(refresh_token);
    }
    auth.expires = decode_token_payload(&auth.access)
        .and_then(|payload| payload.exp)
        .map(|exp| exp * 1000);
    auth = write_pi_auth_at_path_if_unchanged(path, &original_auth, auth)?;

    Ok(auth)
}

fn write_pi_auth_at_path_if_unchanged(
    path: &Path,
    expected: &PiOpenAiCodexAuth,
    updated_auth: PiOpenAiCodexAuth,
) -> Result<PiOpenAiCodexAuth, String> {
    let current_file =
        read_pi_auth(path).ok_or_else(|| "failed to read PI auth file".to_string())?;
    let current_auth = current_file
        .openai_codex
        .ok_or_else(|| "missing openai-codex entry in PI auth file".to_string())?;

    if current_auth != *expected {
        return Ok(current_auth);
    }

    let content =
        std::fs::read_to_string(path).map_err(|e| format!("failed to read PI auth file: {}", e))?;
    let mut value = serde_json::from_str::<serde_json::Value>(&content)
        .map_err(|e| format!("invalid JSON in PI auth file: {}", e))?;
    let Some(root) = value.as_object_mut() else {
        return Err("PI auth file root is not a JSON object".to_string());
    };

    root.insert(
        "openai-codex".to_string(),
        serde_json::to_value(&updated_auth)
            .map_err(|e| format!("failed to serialize PI auth: {}", e))?,
    );

    let updated = serde_json::to_string_pretty(&value)
        .map_err(|e| format!("failed to serialize updated PI auth file: {}", e))?;
    std::fs::write(path, format!("{}\n", updated))
        .map_err(|e| format!("failed to write PI auth file: {}", e))?;

    Ok(updated_auth)
}

fn needs_refresh(last_refresh: Option<&str>) -> bool {
    let Some(last_refresh) = last_refresh else {
        return true;
    };

    let Ok(parsed) = DateTime::parse_from_rfc3339(last_refresh) else {
        return true;
    };

    let now = Utc::now();
    let refresh_after = parsed.with_timezone(&Utc) + Days::new(8);
    now >= refresh_after
}

fn needs_pi_refresh(expires_ms: Option<u64>) -> bool {
    let Some(expires_ms) = expires_ms else {
        return false;
    };

    let now_ms = Utc::now().timestamp_millis() as u64;
    expires_ms <= now_ms + 5 * 60 * 1000
}

#[derive(Debug, serde::Deserialize)]
struct RefreshResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
}

/// Format a duration from an epoch target.
pub fn format_duration_until(target_epoch: u64) -> String {
    let now = Utc::now().timestamp() as u64;
    if target_epoch <= now {
        return "expired".to_string();
    }

    let diff = target_epoch - now;
    let days = diff / 86400;
    let hours = (diff % 86400) / 3600;
    let minutes = (diff % 3600) / 60;

    match (days > 0, hours > 0) {
        (true, _) => format!("{}d {}h {}m", days, hours, minutes),
        (false, true) => format!("{}h {}m", hours, minutes),
        (false, false) => format!("{}m", minutes),
    }
}

pub fn summarize_window(window: &UsageWindow) -> Option<(String, String, String)> {
    let (reset_text, reset_at_formatted) = summarize_reset(window.reset_at.as_ref())?;
    let used_str = window
        .used_percent
        .map(|value| format!("{}", value))
        .unwrap_or_else(|| "?".to_string());

    Some((used_str, reset_text, reset_at_formatted))
}

pub fn summarize_reset(reset_at: Option<&ResetAt>) -> Option<(String, String)> {
    let resets_at = parse_reset_at(reset_at)?;
    let reset_text = format_duration_until(resets_at);
    let reset_at_formatted = Local
        .timestamp_opt(resets_at as i64, 0)
        .single()?
        .format("%Y-%m-%d %H:%M:%S %Z")
        .to_string();
    Some((reset_text, reset_at_formatted))
}

pub fn format_credit_amount(amount: Option<&CreditAmount>) -> String {
    let Some(value) = amount.and_then(CreditAmount::as_f64) else {
        return "?".to_string();
    };
    let formatted = format!("{:.2}", value);
    formatted
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}

pub fn parse_reset_at(value: Option<&ResetAt>) -> Option<u64> {
    let value = value?;
    match value {
        ResetAt::Epoch(ts) => Some(*ts),
        ResetAt::Rfc3339(value) => {
            let parsed = DateTime::parse_from_rfc3339(value).ok()?;
            let ts = parsed.timestamp();
            (ts >= 0).then_some(ts as u64)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        codex_client_id, needs_pi_refresh, needs_refresh, parse_reset_at, pi_client_id,
        write_pi_auth_at_path_if_unchanged,
    };
    use crate::data::{AuthFile, Context, PiOpenAiCodexAuth, ResetAt, Tokens};
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    use chrono::{Duration, Utc};
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn jwt(payload: &str) -> String {
        format!("e30.{}.sig", URL_SAFE_NO_PAD.encode(payload.as_bytes()))
    }

    fn test_context(name: &str) -> (Context, PathBuf) {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::temp_dir().join(format!(
            "codex-switch-{}-{}-{}",
            name,
            std::process::id(),
            unique
        ));
        let pi_dir = base.join(".pi").join("agent");
        std::fs::create_dir_all(&pi_dir).unwrap();
        let ctx = Context {
            live_auth: base.join(".codex").join("auth.json"),
            pi_auth: pi_dir.join("auth.json"),
            state_dir: base.join(".local").join("state").join("codex-switch"),
            tracker_file: base
                .join(".local")
                .join("state")
                .join("codex-switch")
                .join("accounts.json"),
        };
        (ctx, base)
    }

    #[test]
    fn parses_reset_timestamp() {
        let ts = parse_reset_at(Some(&ResetAt::Rfc3339("2026-06-15T10:00:00Z".to_string())));
        assert_eq!(ts, Some(1_781_517_600));
    }

    #[test]
    fn parses_epoch_reset_timestamp() {
        let ts = parse_reset_at(Some(&ResetAt::Epoch(1_781_569_991)));
        assert_eq!(ts, Some(1_781_569_991));
    }

    #[test]
    fn refreshes_stale_tokens_after_eight_days() {
        let fresh = (Utc::now() - Duration::days(7)).to_rfc3339();
        let stale = (Utc::now() - Duration::days(9)).to_rfc3339();

        assert!(!needs_refresh(Some(&fresh)));
        assert!(needs_refresh(Some(&stale)));
        assert!(needs_refresh(None));
        assert!(needs_refresh(Some("not-a-timestamp")));
    }

    #[test]
    fn refreshes_pi_tokens_close_to_expiry() {
        let fresh = (Utc::now() + Duration::minutes(10)).timestamp_millis() as u64;
        let stale = (Utc::now() + Duration::minutes(4)).timestamp_millis() as u64;

        assert!(!needs_pi_refresh(Some(fresh)));
        assert!(needs_pi_refresh(Some(stale)));
        assert!(!needs_pi_refresh(None));
    }

    #[test]
    fn derives_codex_client_id_from_oauth_tokens() {
        let auth = AuthFile {
            tokens: Tokens {
                id_token: jwt(r#"{"aud":["app_from_id"]}"#),
                access_token: Some(jwt(r#"{"client_id":"app_from_access"}"#)),
                refresh_token: Some("refresh".to_string()),
                account_id: None,
            },
            auth_mode: None,
            last_refresh: None,
        };

        assert_eq!(codex_client_id(&auth).as_deref(), Some("app_from_access"));
    }

    #[test]
    fn derives_pi_client_id_from_access_token() {
        let auth = PiOpenAiCodexAuth {
            auth_type: Some("oauth".to_string()),
            access: jwt(r#"{"client_id":"app_from_pi"}"#),
            refresh: Some("refresh".to_string()),
            account_id: None,
            expires: None,
        };

        assert_eq!(pi_client_id(&auth).as_deref(), Some("app_from_pi"));
    }

    #[test]
    fn write_pi_auth_preserves_other_entries() {
        let (ctx, base) = test_context("write-pi-auth");
        std::fs::write(
            &ctx.pi_auth,
            r#"{
  "github-copilot": {"type":"oauth","access":"copilot"},
  "openai-codex": {"type":"oauth","access":"old","refresh":"old-refresh","expires":1}
}
"#,
        )
        .unwrap();

        let previous = PiOpenAiCodexAuth {
            auth_type: Some("oauth".to_string()),
            access: "old".to_string(),
            refresh: Some("old-refresh".to_string()),
            account_id: None,
            expires: Some(1),
        };

        write_pi_auth_at_path_if_unchanged(
            &ctx.pi_auth,
            &previous,
            PiOpenAiCodexAuth {
                auth_type: Some("oauth".to_string()),
                access: "new-access".to_string(),
                refresh: Some("new-refresh".to_string()),
                account_id: Some("acct-1".to_string()),
                expires: Some(1234),
            },
        )
        .unwrap();

        let updated = std::fs::read_to_string(&ctx.pi_auth).unwrap();
        assert!(updated.contains("\"github-copilot\""));
        assert!(updated.contains("\"new-access\""));
        assert!(updated.contains("\"new-refresh\""));
        assert!(updated.contains("\"accountId\": \"acct-1\""));

        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn write_pi_auth_uses_newer_disk_state_when_entry_changed() {
        let (ctx, base) = test_context("write-pi-auth-race");
        std::fs::write(
            &ctx.pi_auth,
            r#"{
  "openai-codex": {"type":"oauth","access":"disk-new","refresh":"disk-refresh","accountId":"acct-2","expires":99}
}
"#,
        )
        .unwrap();

        let result = write_pi_auth_at_path_if_unchanged(
            &ctx.pi_auth,
            &PiOpenAiCodexAuth {
                auth_type: Some("oauth".to_string()),
                access: "stale-old".to_string(),
                refresh: Some("stale-refresh".to_string()),
                account_id: Some("acct-1".to_string()),
                expires: Some(1),
            },
            PiOpenAiCodexAuth {
                auth_type: Some("oauth".to_string()),
                access: "our-new".to_string(),
                refresh: Some("our-refresh".to_string()),
                account_id: Some("acct-1".to_string()),
                expires: Some(1234),
            },
        )
        .unwrap();

        assert_eq!(result.access, "disk-new");
        let updated = std::fs::read_to_string(&ctx.pi_auth).unwrap();
        assert!(updated.contains("\"disk-new\""));
        assert!(!updated.contains("\"our-new\""));

        std::fs::remove_dir_all(base).unwrap();
    }
}
