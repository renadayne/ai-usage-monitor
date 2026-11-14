//! Thin HTTP layer shared by the providers. Errors never carry URLs, headers or bodies.

use std::time::Duration;

use super::provider::ProviderError;

pub const USER_AGENT: &str = concat!("ai-usage-monitor/", env!("CARGO_PKG_VERSION"));

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub retry_after: Option<Duration>,
    pub body: String,
}

impl HttpResponse {
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// Client for public provider hosts (certificate validation always on).
pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(25))
        .build()
        .unwrap_or_default()
}

/// Seconds form of `Retry-After` (the HTTP-date form is ignored).
pub fn parse_retry_after(value: &str) -> Option<Duration> {
    value.trim().parse::<u64>().ok().map(Duration::from_secs)
}

fn network_error(error: reqwest::Error) -> ProviderError {
    let reason = if error.is_timeout() {
        "request timed out"
    } else if error.is_connect() {
        "could not connect"
    } else {
        "network error"
    };
    ProviderError::Other(reason.to_string())
}

pub async fn send(request: reqwest::RequestBuilder) -> Result<HttpResponse, ProviderError> {
    let response = request.send().await.map_err(network_error)?;
    let status = response.status().as_u16();
    let retry_after = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(parse_retry_after);
    let body = response.text().await.map_err(network_error)?;
    Ok(HttpResponse {
        status,
        retry_after,
        body,
    })
}

/// Maps the usual failure statuses; `Ok(())` means the body can be parsed.
pub fn check_status(response: &HttpResponse, auth_hint: &str) -> Result<(), ProviderError> {
    match response.status {
        200..=299 => Ok(()),
        401 | 403 => Err(ProviderError::AuthRequired(auth_hint.to_string())),
        429 => Err(ProviderError::RateLimited {
            retry_after: response.retry_after,
        }),
        status => Err(ProviderError::Other(format!("unexpected HTTP {status}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(status: u16, retry_after: Option<u64>) -> HttpResponse {
        HttpResponse {
            status,
            retry_after: retry_after.map(Duration::from_secs),
            body: String::new(),
        }
    }

    #[test]
    fn retry_after_seconds() {
        assert_eq!(parse_retry_after(" 120 "), Some(Duration::from_secs(120)));
        assert_eq!(parse_retry_after("Wed, 21 Oct 2026 07:28:00 GMT"), None);
    }

    #[test]
    fn status_mapping() {
        assert_eq!(check_status(&response(200, None), "hint"), Ok(()));
        assert_eq!(
            check_status(&response(401, None), "hint"),
            Err(ProviderError::AuthRequired("hint".into()))
        );
        assert_eq!(
            check_status(&response(429, Some(90)), "hint"),
            Err(ProviderError::RateLimited {
                retry_after: Some(Duration::from_secs(90))
            })
        );
        assert_eq!(
            check_status(&response(503, None), "hint"),
            Err(ProviderError::Other("unexpected HTTP 503".into()))
        );
    }
}
