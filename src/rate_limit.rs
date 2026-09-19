use crate::data::{
    read_auth, read_pi_auth, AuthFile, Context, CreditAmount, ModelUsageResponse,
    PiOpenAiCodexAuth, RateLimitResetCredits, ResetAt, TokenUsageResponse, UsageResponse,
    UsageWindow,
};
use crate::jwt::decode_token_payload;
use chrono::{DateTime, Days, Local, TimeZone, Utc};
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, USER_AGENT};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

const USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
const MODEL_USAGE_URL: &str =
    "https://chatgpt.com/backend-api/wham/usage/daily-token-usage-breakdown";
const MODEL_USAGE_LOOKBACK_DAYS: u64 = 30;
const RESET_CREDITS_URL: &str = "https://chatgpt.com/backend-api/wham/rate-limit-reset-credits";
const TOKEN_REFRESH_URL: &str = "https://auth.openai.com/oauth/token";

pub fn fetch_rate_limit(ctx: &Context) -> Result<UsageResponse, String> {
    fetch_rate_limit_for_auth_path(&ctx.live_auth).map(|(usage, _)| usage)
}

pub fn fetch_rate_limit_read_only(ctx: &Context) -> Result<UsageResponse, String> {
    fetch_rate_limit_for_auth_path_read_only(&ctx.live_auth)
}

pub fn fetch_model_usage_for_auth(auth: &AuthFile) -> Result<ModelUsageResponse, String> {
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("failed to build HTTP client: {}", e))?;
    let access_token = auth
        .tokens
        .access_token
        .as_deref()
        .ok_or_else(|| "missing OAuth access token in ~/.codex/auth.json".to_string())?;
    fetch_model_usage_with_token(&client, access_token, auth.tokens.account_id.as_deref())
}

pub fn fetch_pi_model_usage_for_auth(
    auth: &PiOpenAiCodexAuth,
) -> Result<ModelUsageResponse, String> {
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("failed to build HTTP client: {}", e))?;
    fetch_model_usage_with_token(&client, &auth.access, auth.account_id.as_deref())
}

pub fn fetch_token_usage_for_auth(auth: &AuthFile) -> Result<TokenUsageResponse, String> {
    let access_token = auth
        .tokens
        .access_token
        .as_deref()
        .ok_or_else(|| "missing OAuth access token in ~/.codex/auth.json".to_string())?;
    let (account_id, plan_type) =
        token_usage_metadata(&auth.tokens.id_token, auth.tokens.account_id.clone());
    fetch_token_usage_with_app_server(access_token, account_id.as_deref(), plan_type.as_deref())
}

pub fn fetch_pi_token_usage_for_auth(
    auth: &PiOpenAiCodexAuth,
) -> Result<TokenUsageResponse, String> {
    let (account_id, plan_type) = token_usage_metadata(&auth.access, auth.account_id.clone());
    fetch_token_usage_with_app_server(&auth.access, account_id.as_deref(), plan_type.as_deref())
}

fn token_usage_metadata(
    token: &str,
    account_id: Option<String>,
) -> (Option<String>, Option<String>) {
    let claims = decode_token_payload(token);
    let account_id = account_id.or_else(|| {
        claims
            .as_ref()
            .and_then(|payload| payload.openai_auth.as_ref())
            .and_then(|auth| auth.chatgpt_account_id.clone())
    });
    let plan_type = claims
        .as_ref()
        .and_then(|payload| payload.openai_auth.as_ref())
        .and_then(|auth| auth.chatgpt_plan_type.clone());
    (account_id, plan_type)
}

fn fetch_token_usage_with_app_server(
    access_token: &str,
    account_id: Option<&str>,
    plan_type: Option<&str>,
) -> Result<TokenUsageResponse, String> {
    let account_id = account_id
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "missing ChatGPT account id for account usage API".to_string())?;

    let mut child = Command::new("codex")
        .args(["app-server", "--stdio"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("failed to start codex app-server: {}", error))?;

    let Some(mut stdin) = child.stdin.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err("codex app-server did not provide stdin".to_string());
    };
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err("codex app-server did not provide stdout".to_string());
    };

    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let line = line.map_err(|error| error.to_string());
            if sender.send(line).is_err() {
                break;
            }
        }
    });

    let result = (|| {
        let initialize_params = serde_json::json!({
            "clientInfo": {
                "name": "codex-switch",
                "title": "Codex Switch",
                "version": env!("CARGO_PKG_VERSION")
            },
            "capabilities": {"experimentalApi": true}
        });
        rpc_call(
            &mut stdin,
            &receiver,
            1,
            "initialize",
            Some(initialize_params),
        )?;
        write_rpc(&mut stdin, serde_json::json!({"method": "initialized"}))?;

        let mut login_params = serde_json::json!({
            "type": "chatgptAuthTokens",
            "accessToken": access_token,
            "chatgptAccountId": account_id
        });
        if let Some(plan_type) = plan_type.filter(|value| !value.is_empty()) {
            login_params["chatgptPlanType"] = serde_json::json!(plan_type);
        }
        rpc_call(
            &mut stdin,
            &receiver,
            2,
            "account/login/start",
            Some(login_params),
        )?;

        let result = rpc_call(&mut stdin, &receiver, 3, "account/usage/read", None)?;
        serde_json::from_value(result)
            .map_err(|error| format!("invalid account usage response: {}", error))
    })();

    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();
    result
}

fn write_rpc(stdin: &mut ChildStdin, message: serde_json::Value) -> Result<(), String> {
    serde_json::to_writer(&mut *stdin, &message)
        .map_err(|error| format!("failed to write app-server request: {}", error))?;
    stdin
        .write_all(b"\n")
        .map_err(|error| format!("failed to write app-server request: {}", error))?;
    stdin
        .flush()
        .map_err(|error| format!("failed to flush app-server request: {}", error))
}

fn rpc_call(
    stdin: &mut ChildStdin,
    receiver: &Receiver<Result<String, String>>,
    id: u64,
    method: &str,
    params: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    let mut request = serde_json::json!({"method": method, "id": id});
    if let Some(params) = params {
        request["params"] = params;
    }
    write_rpc(stdin, request)?;
    read_rpc_response(stdin, receiver, id, method)
}

fn read_rpc_response(
    stdin: &mut ChildStdin,
    receiver: &Receiver<Result<String, String>>,
    id: u64,
    method: &str,
) -> Result<serde_json::Value, String> {
    const RESPONSE_TIMEOUT: Duration = Duration::from_secs(20);
    let deadline = Instant::now() + RESPONSE_TIMEOUT;

    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(format!("timed out waiting for codex app-server {}", method));
        }
        let line = receiver
            .recv_timeout(remaining)
            .map_err(|error| match error {
                mpsc::RecvTimeoutError::Timeout => {
                    format!("timed out waiting for codex app-server {}", method)
                }
                mpsc::RecvTimeoutError::Disconnected => {
                    format!("codex app-server exited while handling {}", method)
                }
            })??;
        let value: serde_json::Value = serde_json::from_str(&line)
            .map_err(|error| format!("invalid codex app-server response: {}", error))?;

        if value.get("id").and_then(serde_json::Value::as_u64) == Some(id) {
            if let Some(error) = value.get("error") {
                let message = error
                    .get("message")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("unknown app-server error");
                return Err(format!("codex app-server {} failed: {}", method, message));
            }
            return Ok(value
                .get("result")
                .cloned()
                .unwrap_or(serde_json::Value::Null));
        }

        if value
            .get("method")
            .and_then(serde_json::Value::as_str)
            .is_some()
            && value.get("id").is_some()
        {
            write_rpc(
                stdin,
                serde_json::json!({
                    "id": value["id"].clone(),
                    "error": {
                        "code": -32601,
                        "message": "unsupported app-server request"
                    }
                }),
            )?;
        }
    }
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
    let mut usage = response
        .error_for_status()
        .map_err(|e| format!("usage request failed: {}", e))?
        .json::<UsageResponse>()
        .map_err(|e| format!("invalid usage response: {}", e))?;
    hydrate_reset_credit_details(
        &client,
        auth.tokens.access_token.as_deref().unwrap_or_default(),
        auth.tokens.account_id.as_deref(),
        &mut usage,
    );
    Ok(usage)
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

    let mut usage = response
        .error_for_status()
        .map_err(|e| format!("usage request failed: {}", e))?
        .json::<UsageResponse>()
        .map_err(|e| format!("invalid usage response: {}", e))?;
    hydrate_reset_credit_details(
        &client,
        auth.tokens.access_token.as_deref().unwrap_or_default(),
        auth.tokens.account_id.as_deref(),
        &mut usage,
    );

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
    let mut usage = response
        .error_for_status()
        .map_err(|e| format!("usage request failed: {}", e))?
        .json::<UsageResponse>()
        .map_err(|e| format!("invalid usage response: {}", e))?;
    hydrate_reset_credit_details(
        &client,
        &auth.access,
        auth.account_id.as_deref(),
        &mut usage,
    );
    Ok(usage)
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

    let mut usage = response
        .error_for_status()
        .map_err(|e| format!("usage request failed: {}", e))?
        .json::<UsageResponse>()
        .map_err(|e| format!("invalid usage response: {}", e))?;
    hydrate_reset_credit_details(
        &client,
        &auth.access,
        auth.account_id.as_deref(),
        &mut usage,
    );

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
    send_backend_get_with_token(client, USAGE_URL, access_token, account_id)
        .map_err(|e| format!("usage request failed: {}", e))
}

fn fetch_model_usage_with_token(
    client: &Client,
    access_token: &str,
    account_id: Option<&str>,
) -> Result<ModelUsageResponse, String> {
    let response = send_model_usage_request_with_token(client, access_token, account_id)?;
    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err("401 unauthorized from model usage API".to_string());
    }
    if response.status() == reqwest::StatusCode::FORBIDDEN {
        return Err("403 forbidden from model usage API".to_string());
    }

    response
        .error_for_status()
        .map_err(|e| format!("model usage request failed: {}", e))?
        .json::<ModelUsageResponse>()
        .map_err(|e| format!("invalid model usage response: {}", e))
}

fn send_model_usage_request_with_token(
    client: &Client,
    access_token: &str,
    account_id: Option<&str>,
) -> Result<reqwest::blocking::Response, String> {
    let url = model_usage_url(Utc::now().date_naive());
    send_backend_get_with_token(client, &url, access_token, account_id)
        .map_err(|e| format!("model usage request failed: {}", e))
}

fn model_usage_url(today: chrono::NaiveDate) -> String {
    let start = today - Days::new(MODEL_USAGE_LOOKBACK_DAYS - 1);
    let end = today;
    format!(
        "{}?start_date={}&end_date={}&group_by=day",
        MODEL_USAGE_URL, start, end
    )
}

fn send_reset_credits_request_with_token(
    client: &Client,
    access_token: &str,
    account_id: Option<&str>,
) -> Result<reqwest::blocking::Response, String> {
    send_backend_get_with_token(client, RESET_CREDITS_URL, access_token, account_id)
        .map_err(|e| format!("reset credits request failed: {}", e))
}

fn send_backend_get_with_token(
    client: &Client,
    url: &str,
    access_token: &str,
    account_id: Option<&str>,
) -> Result<reqwest::blocking::Response, String> {
    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    headers.insert("originator", HeaderValue::from_static("Codex Desktop"));
    headers.insert(USER_AGENT, HeaderValue::from_static("codex-switch"));
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {}", access_token))
            .map_err(|e| format!("invalid access token header: {}", e))?,
    );

    let mut request = client.get(url).headers(headers);
    if let Some(account_id) = account_id {
        request = request.header("ChatGPT-Account-Id", account_id);
    }

    request.send().map_err(|e| e.to_string())
}

fn hydrate_reset_credit_details(
    client: &Client,
    access_token: &str,
    account_id: Option<&str>,
    usage: &mut UsageResponse,
) {
    let available_count = usage
        .rate_limit_reset_credits
        .as_ref()
        .and_then(|credits| credits.available_count)
        .unwrap_or(0);
    if available_count == 0 || access_token.is_empty() {
        return;
    }

    let Ok(response) = send_reset_credits_request_with_token(client, access_token, account_id)
    else {
        return;
    };
    let Ok(response) = response.error_for_status() else {
        return;
    };
    let Ok(details) = response.json::<RateLimitResetCredits>() else {
        return;
    };
    merge_reset_credit_details(usage, details);
}

fn merge_reset_credit_details(usage: &mut UsageResponse, details: RateLimitResetCredits) {
    let summary = usage
        .rate_limit_reset_credits
        .get_or_insert_with(Default::default);
    summary.available_count = details.available_count.or(summary.available_count);
    summary.applicable_available_count = details
        .applicable_available_count
        .or(summary.applicable_available_count);
    summary.total_earned_count = details.total_earned_count.or(summary.total_earned_count);
    summary.credits = details.credits;
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
        codex_client_id, merge_reset_credit_details, model_usage_url, needs_pi_refresh,
        needs_refresh, parse_reset_at, pi_client_id, write_pi_auth_at_path_if_unchanged,
    };
    use crate::data::{
        AuthFile, Context, PiOpenAiCodexAuth, RateLimitResetCredits, ResetAt, Tokens, UsageResponse,
    };
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    use chrono::{Duration, NaiveDate, Utc};
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
    fn requests_a_thirty_day_model_usage_window_through_today() {
        let url = model_usage_url(NaiveDate::from_ymd_opt(2026, 9, 18).unwrap());
        assert_eq!(
            url,
            "https://chatgpt.com/backend-api/wham/usage/daily-token-usage-breakdown?start_date=2026-08-20&end_date=2026-09-18&group_by=day"
        );
    }

    #[test]
    fn merges_reset_details_without_losing_applicable_count() {
        let mut usage: UsageResponse = serde_json::from_str(
            r#"{"rate_limit_reset_credits":{"available_count":1,"applicable_available_count":0}}"#,
        )
        .unwrap();
        let details: RateLimitResetCredits = serde_json::from_str(
            r#"{"available_count":1,"total_earned_count":0,"credits":[{"status":"available","expires_at":"2026-08-12T18:09:35Z","title":"Full reset"}]}"#,
        )
        .unwrap();

        merge_reset_credit_details(&mut usage, details);

        let merged = usage.rate_limit_reset_credits.unwrap();
        assert_eq!(merged.available_count, Some(1));
        assert_eq!(merged.applicable_available_count, Some(0));
        assert_eq!(merged.total_earned_count, Some(0));
        assert_eq!(merged.credits.len(), 1);
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
