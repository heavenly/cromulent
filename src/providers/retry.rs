// Shared retry-with-backoff logic for LLM provider HTTP requests.

use std::error::Error;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// Maximum number of request attempts per provider call.
pub(crate) const MAX_REQUEST_ATTEMPTS: usize = 5;

/// Send an HTTP request with retries and exponential backoff.
///
/// Retries on rate limits (HTTP 429), server errors (5xx), and connection
/// errors.  Returns the successful [`reqwest::Response`] or an error message.
pub(crate) async fn send_with_retries(
    client: &reqwest::Client,
    url: &str,
    provider_name: &str,
    api_key: &str,
    body: &serde_json::Value,
    cancel: &CancellationToken,
) -> Result<reqwest::Response, String> {
    let mut last_error = String::new();

    for attempt in 1..=MAX_REQUEST_ATTEMPTS {
        if cancel.is_cancelled() {
            return Err(format!("{provider_name} request cancelled"));
        }

        let result = client
            .post(url)
            .bearer_auth(api_key)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header(reqwest::header::ACCEPT, "text/event-stream")
            .json(body)
            .send()
            .await;

        match result {
            Ok(response) if response.status().is_success() => return Ok(response),
            Ok(response) => {
                let status = response.status();
                let retryable = status.as_u16() == 429 || status.is_server_error();
                let body_text = response.text().await.unwrap_or_default();
                last_error = format!("{provider_name} HTTP {status}: {body_text}");
                if !retryable || attempt == MAX_REQUEST_ATTEMPTS {
                    return Err(with_attempts(last_error, attempt));
                }
            }
            Err(err) => {
                last_error = format!(
                    "{provider_name} request failed: {}",
                    format_reqwest_error(&err)
                );
                if attempt == MAX_REQUEST_ATTEMPTS {
                    return Err(with_attempts(last_error, attempt));
                }
            }
        }

        sleep(retry_delay(attempt)).await;
    }

    Err(with_attempts(last_error, MAX_REQUEST_ATTEMPTS))
}

fn retry_delay(attempt: usize) -> Duration {
    Duration::from_millis(250 * attempt as u64)
}

fn with_attempts(message: String, attempts: usize) -> String {
    format!("{message} (after {attempts}/{MAX_REQUEST_ATTEMPTS} attempts)")
}

fn format_reqwest_error(err: &reqwest::Error) -> String {
    let mut parts = vec![err.to_string()];
    let mut source = err.source();
    while let Some(src) = source {
        parts.push(src.to_string());
        source = src.source();
    }
    parts.join("; source: ")
}

async fn sleep(duration: Duration) {
    tokio::time::sleep(duration).await;
}
