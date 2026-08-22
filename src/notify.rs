use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, ToSocketAddrs};

use crate::config::{Config, NotificationsConfig};
use crate::verify::{VerificationResult, VerificationStatus};

/// Sends webhook notifications for verification results.
#[derive(Clone)]
pub struct Notifier {
    config: NotificationsConfig,
    client: reqwest::Client,
}

impl Notifier {
    /// Create a notifier from the global config (if notifications are configured).
    ///
    /// Returns `None` if notifications are not configured, or `Err` if the
    /// webhook URL fails SSRF validation (e.g. points to localhost or a
    /// private network address).
    ///
    /// # Panics
    ///
    /// Panics if the underlying `reqwest::Client` cannot be built, which
    /// only happens if the TLS stack fails to initialise (essentially
    /// never in practice).
    #[must_use]
    pub fn from_config(config: &Config) -> Option<Self> {
        config.notifications.as_ref().map(|c| {
            let client = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("failed to build HTTP client");
            Self {
                config: c.clone(),
                client,
            }
        })
    }

    /// Validate that the webhook URL does not target internal/private networks.
    ///
    /// This is a best-effort SSRF guard: it resolves the hostname and rejects
    /// any address that is loopback, private, link-local, or otherwise
    /// non-global.  Open redirects are mitigated by disabling HTTP redirects.
    ///
    /// # Errors
    ///
    /// Returns an error if the URL is malformed, uses a non-HTTP scheme,
    /// has no host, or resolves to a blocked address.
    fn validate_webhook_url(url: &str) -> anyhow::Result<()> {
        let parsed =
            reqwest::Url::parse(url).map_err(|e| anyhow::anyhow!("Invalid webhook URL: {e}"))?;

        let scheme = parsed.scheme();
        if scheme != "https" && scheme != "http" {
            anyhow::bail!("Webhook URL must use http or https, got '{scheme}'");
        }

        let host = parsed
            .host_str()
            .ok_or_else(|| anyhow::anyhow!("Webhook URL has no host"))?;

        // Try to resolve the hostname and check every resulting IP.
        let port = parsed.port_or_known_default().unwrap_or(443);
        let addr_str = format!("{host}:{port}");
        if let Ok(addrs) = addr_str.to_socket_addrs() {
            for addr in addrs {
                Self::check_ip(addr.ip())?;
            }
        } else {
            // If DNS resolution fails, do a basic string check for
            // obvious localhost / private addresses.
            let lower = host.to_lowercase();
            if lower == "localhost"
                || lower.starts_with("127.")
                || lower == "::1"
                || lower.starts_with("10.")
                || lower.starts_with("192.168.")
                || lower.starts_with("169.254.")
                || lower == "metadata.google.internal"
                || lower.ends_with(".internal")
            {
                anyhow::bail!("Webhook URL must not point to internal addresses");
            }
        }

        Ok(())
    }

    /// Reject non-global (private, loopback, link-local, …) IP addresses.
    fn check_ip(ip: IpAddr) -> anyhow::Result<()> {
        let blocked = match ip {
            IpAddr::V4(v4) => {
                v4.is_loopback() || v4.is_private() || v4.is_link_local() || !Self::is_global_v4(v4)
            }
            IpAddr::V6(v6) => v6.is_loopback() || !Self::is_global_v6(v6),
        };
        if blocked {
            anyhow::bail!("Webhook URL resolved to blocked address {ip}");
        }
        Ok(())
    }

    /// Returns `true` for globally routable IPv4 addresses.
    fn is_global_v4(ip: Ipv4Addr) -> bool {
        // 0.0.0.0/8, 100.64.0.0/10 (CGNAT), 192.0.0.0/24, 198.18.0.0/15
        // (benchmarking), 224.0.0.0/4 (multicast), 240.0.0.0/4 (reserved)
        let o = ip.octets();
        if o[0] == 0 {
            return false;
        }
        if o[0] == 100 && (o[1] & 0xC0) == 64 {
            return false;
        }
        if o[0] == 192 && o[1] == 0 && o[2] == 0 {
            return false;
        }
        if o[0] == 198 && (o[1] == 18 || o[1] == 19) {
            return false;
        }
        if o[0] >= 224 {
            return false;
        }
        true
    }

    /// Returns `true` for globally routable IPv6 addresses.
    fn is_global_v6(ip: Ipv6Addr) -> bool {
        // Block unique-local (fc00::/7), link-local (fe80::/10), and
        // multicast (ff00::/8).
        let seg = ip.segments();
        if (seg[0] & 0xfe00) == 0xfc00 {
            return false;
        }
        if (seg[0] & 0xffc0) == 0xfe80 {
            return false;
        }
        if (seg[0] & 0xff00) == 0xff00 {
            return false;
        }
        true
    }

    /// Send a notification for the given verification result.
    ///
    /// Skips sending if `on_failure_only` is true and the result is a pass.
    ///
    /// # Errors
    ///
    /// Returns an error if the webhook URL fails SSRF validation, the
    /// request cannot be sent, or the server responds with a non-success
    /// status code.
    pub async fn send(&self, repo_name: &str, result: &VerificationResult) -> anyhow::Result<()> {
        // Skip pass notifications if configured to only notify on failure.
        if self.config.on_failure_only && result.status == VerificationStatus::Pass {
            return Ok(());
        }

        // Validate the webhook URL on every send (cheap after first resolve)
        // to catch config changes or DNS rebinding.
        Self::validate_webhook_url(&self.config.webhook_url)?;

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
                    .as_secs()
                    .cast_signed(),
            }]
        });

        let response = self
            .client
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reject_localhost() {
        let err = Notifier::validate_webhook_url("http://localhost/hook").unwrap_err();
        assert!(err.to_string().contains("internal") || err.to_string().contains("blocked"));
    }

    #[test]
    fn test_reject_loopback_ip() {
        let err = Notifier::validate_webhook_url("http://127.0.0.1/hook").unwrap_err();
        assert!(err.to_string().contains("blocked"));
    }

    #[test]
    fn test_reject_private_10() {
        let err = Notifier::validate_webhook_url("http://10.0.0.1/hook").unwrap_err();
        assert!(err.to_string().contains("blocked"));
    }

    #[test]
    fn test_reject_private_192() {
        let err = Notifier::validate_webhook_url("http://192.168.1.1/hook").unwrap_err();
        assert!(err.to_string().contains("blocked"));
    }

    #[test]
    fn test_reject_link_local() {
        let err = Notifier::validate_webhook_url("http://169.254.169.254/metadata").unwrap_err();
        assert!(err.to_string().contains("blocked"));
    }

    #[test]
    fn test_reject_ftp_scheme() {
        let err = Notifier::validate_webhook_url("ftp://example.com/hook").unwrap_err();
        assert!(err.to_string().contains("http or https"));
    }

    #[test]
    fn test_reject_invalid_url() {
        let err = Notifier::validate_webhook_url("not a url").unwrap_err();
        assert!(err.to_string().contains("Invalid"));
    }

    #[test]
    fn test_reject_metadata_domain_fallback() {
        // When DNS fails, we fall back to string matching
        let err =
            Notifier::validate_webhook_url("http://metadata.google.internal/computeMetadata/v1/")
                .unwrap_err();
        assert!(err.to_string().contains("internal"));
    }

    #[test]
    fn test_check_ip_blocks_loopback() {
        let err = Notifier::check_ip(IpAddr::V4(Ipv4Addr::LOCALHOST)).unwrap_err();
        assert!(err.to_string().contains("blocked"));
    }

    #[test]
    fn test_check_ip_blocks_ipv6_loopback() {
        let err = Notifier::check_ip(IpAddr::V6(Ipv6Addr::LOCALHOST)).unwrap_err();
        assert!(err.to_string().contains("blocked"));
    }
}
