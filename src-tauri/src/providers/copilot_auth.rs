//! GitHub device flow for Copilot, used only when no token can be found on
//! the machine. The token is kept in Windows Credential Manager, never in a file.

use serde::Serialize;
use serde_json::Value;

use crate::engine::http;
use crate::engine::provider::ProviderError;

/// Public client id of GitHub's Copilot editor integration. Device-flow client
/// ids are not secrets; `copilot_internal/user` only accepts tokens issued to
/// a Copilot client.
pub const CLIENT_ID: &str = "Iv1.b507a08c87ecfe98";
const DEVICE_CODE_URL: &str = "https://github.com/login/device/code";
const TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
/// The only page the app ever opens in the browser for this flow.
pub const VERIFICATION_URL: &str = "https://github.com/login/device";
const SCOPE: &str = "read:user";

const KEYRING_SERVICE: &str = "ai-usage-monitor";
const KEYRING_USER: &str = "github-copilot";

/// Deliberately not `Debug`: `device_code` is a secret until the flow ends.
#[derive(Clone, Serialize)]
pub struct DeviceCode {
    /// Shown to the user, who types it on github.com.
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
    #[serde(skip)]
    pub device_code: String,
}

pub fn parse_device_code(body: &str) -> Option<DeviceCode> {
    let value: Value = serde_json::from_str(body).ok()?;
    let text = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
            .map(str::to_string)
    };
    Some(DeviceCode {
        user_code: text("user_code")?,
        device_code: text("device_code")?,
        // Never trust the response for where to send the user.
        verification_uri: VERIFICATION_URL.to_string(),
        expires_in: value
            .get("expires_in")
            .and_then(Value::as_u64)
            .unwrap_or(900),
        interval: value
            .get("interval")
            .and_then(Value::as_u64)
            .unwrap_or(5)
            .clamp(1, 60),
    })
}

/// Deliberately not `Debug`: `Token` holds the secret.
#[derive(PartialEq)]
pub enum PollOutcome {
    Pending,
    SlowDown,
    Token(String),
    /// A final, user-presentable reason.
    Failed(String),
}

pub fn parse_token_poll(body: &str) -> PollOutcome {
    let Ok(value) = serde_json::from_str::<Value>(body) else {
        return PollOutcome::Failed("GitHub sent an unreadable answer.".to_string());
    };
    if let Some(token) = value
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty())
    {
        return PollOutcome::Token(token.to_string());
    }
    match value.get("error").and_then(Value::as_str) {
        Some("authorization_pending") => PollOutcome::Pending,
        Some("slow_down") => PollOutcome::SlowDown,
        Some("expired_token") => PollOutcome::Failed("The code expired. Try again.".to_string()),
        Some("access_denied") => PollOutcome::Failed("Access was denied on GitHub.".to_string()),
        Some(other) => PollOutcome::Failed(format!("GitHub refused the request ({other}).")),
        None => PollOutcome::Failed("GitHub sent an unexpected answer.".to_string()),
    }
}

fn form_request(client: &reqwest::Client, url: &str, body: String) -> reqwest::RequestBuilder {
    client
        .post(url)
        .header("Accept", "application/json")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(body)
}

pub async fn request_device_code(client: &reqwest::Client) -> Result<DeviceCode, ProviderError> {
    let body = format!("client_id={CLIENT_ID}&scope={}", SCOPE.replace(':', "%3A"));
    let response = http::send(form_request(client, DEVICE_CODE_URL, body)).await?;
    if !response.is_success() {
        return Err(ProviderError::Other(format!(
            "GitHub answered HTTP {}",
            response.status
        )));
    }
    parse_device_code(&response.body)
        .ok_or_else(|| ProviderError::Other("GitHub sent an unexpected answer".to_string()))
}

pub async fn poll_token(client: &reqwest::Client, device_code: &str) -> PollOutcome {
    // Device codes are alphanumeric; anything else is not sent.
    if !device_code.chars().all(|c| c.is_ascii_alphanumeric()) {
        return PollOutcome::Failed("GitHub sent an unexpected device code.".to_string());
    }
    let body = format!(
        "client_id={CLIENT_ID}&device_code={device_code}\
         &grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Adevice_code"
    );
    match http::send(form_request(client, TOKEN_URL, body)).await {
        Ok(response) if response.is_success() => parse_token_poll(&response.body),
        // Network hiccups are not final; keep polling until the code expires.
        _ => PollOutcome::Pending,
    }
}

fn entry() -> Option<keyring::Entry> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).ok()
}

pub fn load_token() -> Option<String> {
    entry()?
        .get_password()
        .ok()
        .filter(|token| !token.is_empty())
}

pub fn store_token(token: &str) -> Result<(), String> {
    entry()
        .ok_or_else(|| "Windows Credential Manager is unavailable".to_string())?
        .set_password(token)
        .map_err(|_| "could not save the token to Windows Credential Manager".to_string())
}

pub fn delete_token() {
    if let Some(entry) = entry() {
        let _ = entry.delete_credential();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_code_response() {
        let code = parse_device_code(
            r#"{"device_code":"abc123","user_code":"WDJB-MJHT",
                "verification_uri":"https://evil.example/phish","expires_in":899,"interval":5}"#,
        )
        .unwrap();
        assert_eq!(code.user_code, "WDJB-MJHT");
        assert_eq!(code.device_code, "abc123");
        assert_eq!(code.verification_uri, VERIFICATION_URL);
        assert_eq!(code.expires_in, 899);
        assert_eq!(code.interval, 5);

        // The device code must never reach the UI.
        let json = serde_json::to_string(&code).unwrap();
        assert!(!json.contains("abc123"));
        assert!(json.contains("WDJB-MJHT"));

        assert!(parse_device_code(r#"{"user_code":"X"}"#).is_none());
        assert!(parse_device_code("nope").is_none());
        let defaults = parse_device_code(r#"{"device_code":"d","user_code":"u","interval":0}"#);
        assert_eq!(defaults.as_ref().map(|c| c.interval), Some(1));
        assert_eq!(defaults.map(|c| c.expires_in), Some(900));
    }

    #[test]
    fn token_poll_outcomes() {
        assert!(parse_token_poll(r#"{"error":"authorization_pending"}"#) == PollOutcome::Pending);
        assert!(
            parse_token_poll(r#"{"error":"slow_down","interval":10}"#) == PollOutcome::SlowDown
        );
        assert!(
            parse_token_poll(r#"{"access_token":"REDACTED","token_type":"bearer"}"#)
                == PollOutcome::Token("REDACTED".into())
        );
        for body in [
            r#"{"error":"expired_token"}"#,
            r#"{"error":"access_denied"}"#,
            r#"{"error":"incorrect_client_credentials"}"#,
            r#"{}"#,
            "<html>",
        ] {
            assert!(
                matches!(parse_token_poll(body), PollOutcome::Failed(_)),
                "{body}"
            );
        }
    }
}
