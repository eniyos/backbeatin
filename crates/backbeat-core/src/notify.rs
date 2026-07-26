use crate::config::{Config, NotificationsConfig};
use crate::verify::{VerificationResult, VerificationStatus};

/// Sends webhook notifications for verification results.
#[derive(Clone)]
pub struct Notifier {
    config: NotificationsConfig,
}

impl Notifier {
    /// Create a notifier from the global config (if notifications are configured).
    pub fn from_config(config: &Config) -> Option<Self> {
        config.notifications.as_ref().map(|c| Self { config: c.clone() })
    }

    /// Send a notification for the given verification result.
    ///
    /// Skips sending if `on_failure_only` is true and the result is a pass.
    pub async fn send(
        &self,
        repo_name: &str,
        result: &VerificationResult,
    ) -> anyhow::Result<()> {
        // Skip pass notifications if configured to only notify on failure.
        if self.config.on_failure_only && result.status == VerificationStatus::Pass {
            return Ok(());
        }

        let color = match result.status {
            VerificationStatus::Pass => "good",
            VerificationStatus::Fail => "danger",
        };

        let status_emoji = match result.status {
            VerificationStatus::Pass => "✅",
            VerificationStatus::Fail => "❌",
        };

        let payload = serde_json::json!({
            "text": format!("{} Backbeatin: {}", status_emoji, result.message),
            "attachments": [{
                "color": color,
                "title": format!("Verification: {}", repo_name),
                "fields": [
                    { "title": "Status", "value": format!("{:?}", result.status), "short": true },
                    { "title": "Message", "value": result.message, "short": false },
                ],
                "ts": std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64,
            }]
        });

        let client = reqwest::Client::new();
        let response = client
            .post(&self.config.webhook_url)
            .json(&payload)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Webhook returned {}: {}", status, body.trim());
        }

        Ok(())
    }
}
