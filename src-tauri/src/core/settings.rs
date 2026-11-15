//! User settings, stored as JSON in the app's own data directory.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::model::ProviderId;

pub const SETTINGS_FILE: &str = "settings.json";

/// Fastest allowed quota polling per provider, to stay clear of rate limits.
pub fn min_poll_secs(provider: ProviderId) -> u64 {
    match provider {
        ProviderId::Claude => 300,
        ProviderId::Codex => 60,
        ProviderId::Antigravity => 30,
        ProviderId::Copilot => 300,
    }
}

/// Polling slower than this makes the numbers useless.
pub const MAX_POLL_SECS: u64 = 6 * 3600;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderPrefs {
    pub enabled: bool,
    /// `None` uses the provider's own default.
    pub poll_interval_secs: Option<u64>,
}

impl Default for ProviderPrefs {
    fn default() -> Self {
        Self {
            enabled: true,
            poll_interval_secs: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Keyed by provider id ("claude", "codex", ...).
    pub providers: BTreeMap<String, ProviderPrefs>,
    pub notifications_enabled: bool,
    /// Percent used at which the first notification fires.
    pub warn_threshold: f32,
    /// Percent used at which the second notification fires.
    pub critical_threshold: f32,
    pub autostart: bool,
    /// Manual override for the Claude Code config directory.
    pub claude_dir: Option<String>,
    /// Manual override for the Codex home directory.
    pub codex_dir: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            providers: ProviderId::ALL
                .into_iter()
                .map(|id| (id.as_str().to_string(), ProviderPrefs::default()))
                .collect(),
            notifications_enabled: true,
            warn_threshold: 80.0,
            critical_threshold: 95.0,
            autostart: false,
            claude_dir: None,
            codex_dir: None,
        }
    }
}

fn clean_dir(value: Option<String>) -> Option<String> {
    value
        .map(|text| text.trim().trim_matches('"').to_string())
        .filter(|text| !text.is_empty())
}

impl Settings {
    /// Brings every value into its valid range; unknown providers are dropped.
    pub fn sanitized(mut self) -> Self {
        let mut providers = BTreeMap::new();
        for id in ProviderId::ALL {
            let mut prefs = self.providers.remove(id.as_str()).unwrap_or_default();
            prefs.poll_interval_secs = prefs
                .poll_interval_secs
                .map(|secs| secs.clamp(min_poll_secs(id), MAX_POLL_SECS));
            providers.insert(id.as_str().to_string(), prefs);
        }
        self.providers = providers;

        let finite = |value: f32, fallback: f32| if value.is_finite() { value } else { fallback };
        self.warn_threshold = finite(self.warn_threshold, 80.0).clamp(1.0, 99.0);
        self.critical_threshold =
            finite(self.critical_threshold, 95.0).clamp(self.warn_threshold, 100.0);
        self.claude_dir = clean_dir(self.claude_dir);
        self.codex_dir = clean_dir(self.codex_dir);
        self
    }

    pub fn prefs(&self, provider: ProviderId) -> ProviderPrefs {
        self.providers
            .get(provider.as_str())
            .cloned()
            .unwrap_or_default()
    }

    pub fn poll_interval(&self, provider: ProviderId) -> Option<Duration> {
        self.prefs(provider)
            .poll_interval_secs
            .map(Duration::from_secs)
    }

    /// Missing or unreadable files give the defaults.
    pub fn load(dir: &Path) -> Self {
        std::fs::read_to_string(dir.join(SETTINGS_FILE))
            .ok()
            .and_then(|json| serde_json::from_str::<Settings>(&json).ok())
            .unwrap_or_default()
            .sanitized()
    }

    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(dir.join(SETTINGS_FILE), json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_enable_everything() {
        let settings = Settings::default();
        assert_eq!(settings.providers.len(), 4);
        for id in ProviderId::ALL {
            assert!(settings.prefs(id).enabled);
            assert_eq!(settings.poll_interval(id), None);
        }
        assert_eq!(settings.clone().sanitized(), settings);
    }

    #[test]
    fn intervals_are_floored_and_capped() {
        let mut settings = Settings::default();
        settings.providers.insert(
            "claude".into(),
            ProviderPrefs {
                enabled: false,
                poll_interval_secs: Some(5),
            },
        );
        settings.providers.insert(
            "antigravity".into(),
            ProviderPrefs {
                enabled: true,
                poll_interval_secs: Some(10_000_000),
            },
        );
        settings
            .providers
            .insert("made-up".into(), ProviderPrefs::default());

        let settings = settings.sanitized();
        assert_eq!(
            settings.poll_interval(ProviderId::Claude),
            Some(Duration::from_secs(300))
        );
        assert!(!settings.prefs(ProviderId::Claude).enabled);
        assert_eq!(
            settings.poll_interval(ProviderId::Antigravity),
            Some(Duration::from_secs(MAX_POLL_SECS))
        );
        assert!(!settings.providers.contains_key("made-up"));
        assert_eq!(settings.providers.len(), 4);
    }

    #[test]
    fn thresholds_and_paths_are_normalised() {
        let settings = Settings {
            warn_threshold: 120.0,
            critical_threshold: 50.0,
            claude_dir: Some("  \"C:\\Users\\me\\.claude\"  ".into()),
            codex_dir: Some("   ".into()),
            ..Settings::default()
        }
        .sanitized();
        assert_eq!(settings.warn_threshold, 99.0);
        assert_eq!(settings.critical_threshold, 99.0);
        assert_eq!(
            settings.claude_dir.as_deref(),
            Some("C:\\Users\\me\\.claude")
        );
        assert_eq!(settings.codex_dir, None);

        let nan = Settings {
            warn_threshold: f32::NAN,
            ..Settings::default()
        }
        .sanitized();
        assert_eq!(nan.warn_threshold, 80.0);
    }

    #[test]
    fn round_trips_through_disk_and_tolerates_garbage() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Settings::load(dir.path()), Settings::default());

        let custom = Settings {
            autostart: true,
            warn_threshold: 70.0,
            ..Settings::default()
        };
        custom.save(dir.path()).unwrap();
        assert_eq!(Settings::load(dir.path()), custom);

        // Partial files keep defaults for everything missing.
        std::fs::write(dir.path().join(SETTINGS_FILE), r#"{"autostart":true}"#).unwrap();
        let partial = Settings::load(dir.path());
        assert!(partial.autostart);
        assert_eq!(partial.warn_threshold, 80.0);

        std::fs::write(dir.path().join(SETTINGS_FILE), "not json").unwrap();
        assert_eq!(Settings::load(dir.path()), Settings::default());
    }
}
