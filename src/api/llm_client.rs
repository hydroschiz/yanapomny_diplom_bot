//! HTTP client for LLM API service.

use anyhow::{Context, Result};
use tracing::{debug, warn};

use super::llm_models::{ParseReminderRequest, ReminderResponse};

/// Client for interacting with the LLM API service.
#[derive(Clone)]
pub struct LlmClient {
    client: reqwest::Client,
    base_url: String,
}

impl LlmClient {
    fn timeout_from_env() -> std::time::Duration {
        let secs = std::env::var("LLM_API_TIMEOUT_SECS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(60);

        std::time::Duration::from_secs(secs.max(1))
    }

    /// Create a new LLM client from environment variables.
    /// 
    /// Uses `LLM_API_URL` env var, defaults to `http://localhost:8080`.
    pub fn from_env() -> Result<Self> {
        let base_url = std::env::var("LLM_API_URL")
            .unwrap_or_else(|_| "http://localhost:8080".to_string());
        let timeout = Self::timeout_from_env();
        
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .context("Failed to build HTTP client")?;
        
        Ok(Self { client, base_url })
    }

    /// Create a new LLM client with a specific base URL.
    pub fn new(base_url: impl Into<String>) -> Result<Self> {
        let timeout = Self::timeout_from_env();
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .context("Failed to build HTTP client")?;
        
        Ok(Self {
            client,
            base_url: base_url.into(),
        })
    }

    /// Parse a reminder from natural language text.
    /// 
    /// Sends the text to LLM API and returns a structured reminder.
    /// Includes user's timezone and current datetime for context.
    pub async fn parse_reminder(
        &self, 
        text: &str, 
        user_timezone: &str,
        user_datetime: &str,
    ) -> Result<ReminderResponse> {
        let url = format!("{}/api/v1/parse-reminder", self.base_url);
        
        let request = ParseReminderRequest::with_context(
            text, 
            user_timezone.to_string(), 
            user_datetime.to_string()
        );
        
        debug!(url = %url, text = %text, timezone = %user_timezone, "Sending parse request to LLM API");
        
        let response = self.client
            .post(&url)
            .json(&request)
            .send()
            .await
            .context("Failed to send request to LLM API")?;
        
        let status = response.status();
        
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            warn!(status = %status, error = %error_text, "LLM API returned error");
            anyhow::bail!("LLM API error: {} - {}", status, error_text);
        }
        
        let reminder_response: ReminderResponse = response
            .json()
            .await
            .context("Failed to parse LLM API response")?;
        
        debug!(
            status = %reminder_response.status,
            "Received response from LLM API"
        );
        
        Ok(reminder_response)
    }

    /// Check if the LLM API is available.
    pub async fn health_check(&self) -> Result<bool> {
        let url = format!("{}/api/v1/health", self.base_url);
        
        match self.client.get(&url).send().await {
            Ok(response) => Ok(response.status().is_success()),
            Err(_) => Ok(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_creation() {
        let client = LlmClient::new("http://localhost:8080").unwrap();
        assert_eq!(client.base_url, "http://localhost:8080");
    }
}
