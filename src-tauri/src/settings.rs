//! User settings, stored as JSON in `<app config dir>/settings.json`.
//!
//! Every field has a default, and unknown or missing keys are tolerated, so an
//! old settings file keeps working after an upgrade.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::i18n::Lang;
use crate::protocol::AgentConfig;

/// File name inside the app config directory.
pub const SETTINGS_FILE: &str = "settings.json";

/// Values of [`Settings::language`]: follow the system, or force one.
pub const LANGUAGES: [&str; 3] = ["system", "en", "fr"];

/// Default CodeCarbon API endpoint.
pub const DEFAULT_API_ENDPOINT: &str = "https://api.codecarbon.io";

/// Upload to the CodeCarbon dashboard (API). Filled in by the dashboard
/// sign-in (Settings view), or by hand under "Advanced".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ApiSettings {
    pub enabled: bool,
    pub endpoint: String,
    pub api_key: String,
    pub experiment_id: String,
    /// Display only: where the uploads go ("org / project / experiment").
    pub organization_name: String,
    pub project_name: String,
    pub experiment_name: String,
}

impl Default for ApiSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: DEFAULT_API_ENDPOINT.to_string(),
            api_key: String::new(),
            experiment_id: String::new(),
            organization_name: String::new(),
            project_name: String::new(),
            experiment_name: String::new(),
        }
    }
}

impl ApiSettings {
    /// After signing out: no key, no experiment, upload off (the endpoint
    /// is kept).
    pub fn clear_account(&mut self) {
        *self = ApiSettings {
            endpoint: std::mem::take(&mut self.endpoint),
            ..Default::default()
        };
    }
}

/// Everything the user can change in the Settings view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Seconds between two measurements (1–60).
    pub interval_secs: f64,
    /// 3-letter ISO country code; `None` = detect from IP.
    pub country_iso_code: Option<String>,
    /// State / province (US, Canada).
    pub region: Option<String>,
    /// Let CodeCarbon append each run to `emissions.csv`.
    pub save_csv: bool,
    pub start_tracking_on_launch: bool,
    pub launch_at_login: bool,
    pub api: ApiSettings,
    /// Power usage effectiveness (datacenter overhead), e.g. 1.2.
    pub pue: Option<f64>,
    /// Override of the grid carbon intensity, in g CO₂e / kWh.
    pub force_carbon_intensity_g_per_kwh: Option<f64>,
    /// Language of the panel and the tray: `"system"`, `"en"` or `"fr"`.
    pub language: String,
    /// A native notification with the session's figures when a measurement
    /// stops (not when the app quits).
    pub notify_on_stop: bool,
    /// The user closed the Dashboard's first-run welcome card ("Got it").
    pub welcome_dismissed: bool,
    /// Look for a new release in the background (shortly after launch,
    /// then once a day). Only in builds with an updater key; the About
    /// view's "Check for updates" works either way.
    pub check_updates: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            interval_secs: 3.0,
            country_iso_code: None,
            region: None,
            save_csv: true,
            start_tracking_on_launch: false,
            launch_at_login: false,
            api: ApiSettings::default(),
            pue: None,
            force_carbon_intensity_g_per_kwh: None,
            language: "system".to_string(),
            notify_on_stop: true,
            welcome_dismissed: false,
            check_updates: true,
        }
    }
}

/// Turn `Some("  ")` into `None` and trim the rest.
fn non_empty(s: &Option<String>) -> Option<String> {
    s.as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

impl Settings {
    /// Read the settings file; defaults if it is missing. Never fatal.
    pub fn load(path: &Path) -> Self {
        Self::load_checked(path).settings
    }

    /// Read the settings file and say when it could not be used.
    ///
    /// A missing file is a first start: defaults, no problem. A file that
    /// cannot be read or parsed is kept aside (`settings.invalid.json`) and
    /// the last good copy ([`backup_path`]) is used; without one, defaults.
    /// Either way `problem` tells the user, who would otherwise find their
    /// settings changed with no explanation.
    pub fn load_checked(path: &Path) -> Loaded {
        let why = match fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str(&text) {
                Ok(settings) => {
                    return Loaded {
                        settings,
                        problem: None,
                    }
                }
                Err(e) => e.to_string(),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Loaded {
                    settings: Self::default(),
                    problem: None,
                }
            }
            Err(e) => e.to_string(),
        };
        log::warn!("cannot use the settings file {}: {why}", path.display());
        if let Err(e) = fs::copy(path, invalid_path(path)) {
            log::warn!("cannot keep a copy of the unusable settings file: {e}");
        }
        let backup = fs::read_to_string(backup_path(path))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok());
        match backup {
            Some(settings) => Loaded {
                settings,
                problem: Some(AppError::SettingsRestored),
            },
            None => Loaded {
                settings: Self::default(),
                problem: Some(AppError::SettingsReset),
            },
        }
    }

    /// Write the settings file (creating its directory).
    ///
    /// The new content goes to a temporary file that then replaces the
    /// settings file in one step, so a crash or a full disk leaves the old
    /// file intact, never a truncated one. The file being replaced, when it
    /// is valid, is kept as the last good copy. Both hold the API key: on
    /// Unix they are readable by their owner only (on Windows the app's
    /// configuration folder is already private to the user).
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        use std::io::Write;

        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string_pretty(self)?;

        let is_valid = fs::read_to_string(path)
            .ok()
            .is_some_and(|old| serde_json::from_str::<Settings>(&old).is_ok());
        if is_valid {
            let backup = backup_path(path);
            fs::copy(path, &backup)?;
            make_private(&backup)?;
        }

        let tmp = path.with_extension("json.tmp");
        let written = (|| -> std::io::Result<()> {
            let mut options = fs::OpenOptions::new();
            options.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&tmp)?;
            // A temporary file left by an earlier, less careful version.
            make_private(&tmp)?;
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
            fs::rename(&tmp, path)
        })();
        if let Err(e) = written {
            let _ = fs::remove_file(&tmp);
            return Err(e.into());
        }
        Ok(())
    }

    /// Check the values a user can type.
    pub fn validate(&self) -> Result<(), AppError> {
        if !(1.0..=60.0).contains(&self.interval_secs) {
            return Err(AppError::InvalidInterval);
        }
        if let Some(code) = non_empty(&self.country_iso_code) {
            if code.len() != 3 || !code.chars().all(|c| c.is_ascii_alphabetic()) {
                return Err(AppError::InvalidCountry);
            }
        }
        if let Some(pue) = self.pue {
            if !(1.0..=5.0).contains(&pue) {
                return Err(AppError::InvalidPue);
            }
        }
        if let Some(ci) = self.force_carbon_intensity_g_per_kwh {
            if !(0.0..=2000.0).contains(&ci) {
                return Err(AppError::InvalidCarbonIntensity);
            }
        }
        if self.api.enabled {
            if !self.api.endpoint.starts_with("https://") {
                return Err(AppError::InvalidApiEndpoint);
            }
            if self.api.api_key.trim().is_empty() || self.api.experiment_id.trim().is_empty() {
                return Err(AppError::ApiCredentialsMissing);
            }
        }
        Ok(())
    }

    /// The language to show (the system's for `"system"`).
    pub fn lang(&self) -> Lang {
        Lang::from_setting(&self.language)
    }

    /// Normalise user input (trim, upper-case country, empty → `None`).
    pub fn normalized(mut self) -> Self {
        self.country_iso_code = non_empty(&self.country_iso_code).map(|c| c.to_uppercase());
        self.region = non_empty(&self.region);
        self.api.endpoint = self.api.endpoint.trim().to_string();
        self.api.api_key = self.api.api_key.trim().to_string();
        self.api.experiment_id = self.api.experiment_id.trim().to_string();
        self.api.organization_name = self.api.organization_name.trim().to_string();
        self.api.project_name = self.api.project_name.trim().to_string();
        self.api.experiment_name = self.api.experiment_name.trim().to_string();
        // An unknown language (hand-edited file, newer version) → system.
        let language = self.language.trim().to_ascii_lowercase();
        self.language = if LANGUAGES.contains(&language.as_str()) {
            language
        } else {
            "system".to_string()
        };
        self
    }

    /// Build the protocol `config` object sent with `start`.
    ///
    /// `output_dir` is the app data directory, where CodeCarbon writes
    /// `emissions.csv`; the caller makes sure it exists.
    pub fn to_agent_config(&self, output_dir: &Path) -> AgentConfig {
        let api_on = self.api.enabled;
        let if_api = |s: &str| (api_on && !s.trim().is_empty()).then(|| s.trim().to_string());
        AgentConfig {
            interval_secs: self.interval_secs.clamp(1.0, 60.0),
            country_iso_code: non_empty(&self.country_iso_code).map(|c| c.to_uppercase()),
            region: non_empty(&self.region),
            tracking_mode: "machine".to_string(),
            project_name: "codecarbon-gui".to_string(),
            save_to_file: self.save_csv,
            output_dir: output_dir.to_string_lossy().into_owned(),
            save_to_api: api_on,
            api_endpoint: if_api(&self.api.endpoint),
            api_key: if_api(&self.api.api_key),
            experiment_id: if_api(&self.api.experiment_id),
            force_carbon_intensity_g_co2e_kwh: self.force_carbon_intensity_g_per_kwh,
            pue: self.pue,
            log_level: "warning".to_string(),
        }
    }
}

/// Where the settings file lives.
/// What [`Settings::load_checked`] found.
#[derive(Debug, Clone, PartialEq)]
pub struct Loaded {
    pub settings: Settings,
    /// Why these are not the settings the user saved, if they are not.
    pub problem: Option<AppError>,
}

/// The last settings file that was valid, kept by [`Settings::save`].
pub fn backup_path(path: &Path) -> PathBuf {
    path.with_extension("json.bak")
}

/// Where a settings file that could not be read is kept.
pub fn invalid_path(path: &Path) -> PathBuf {
    path.with_extension("invalid.json")
}

/// Readable and writable by the owner only (Unix).
fn make_private(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

pub fn settings_path(config_dir: &Path) -> PathBuf {
    config_dir.join(SETTINGS_FILE)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "codecarbon-settings-test-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn defaults() {
        let s = Settings::default();
        assert_eq!(s.interval_secs, 3.0);
        assert!(s.save_csv);
        assert!(!s.start_tracking_on_launch);
        assert!(!s.launch_at_login);
        assert!(!s.api.enabled);
        assert_eq!(s.api.endpoint, "https://api.codecarbon.io");
        assert_eq!(s.language, "system");
        assert!(s.notify_on_stop);
        assert!(!s.welcome_dismissed);
        assert!(s.check_updates);
        assert!(s.validate().is_ok());
    }

    #[test]
    fn check_updates_flag() {
        let old: Settings = serde_json::from_str(r#"{"interval_secs": 5}"#).unwrap();
        assert!(old.check_updates, "older files check for updates");
        let s: Settings = serde_json::from_str(r#"{"check_updates": false}"#).unwrap();
        assert!(!s.check_updates);
        let v = serde_json::to_value(s.normalized()).unwrap();
        assert_eq!(v["check_updates"], false);
    }

    #[test]
    fn notification_and_welcome_flags() {
        let old: Settings = serde_json::from_str(r#"{"interval_secs": 5}"#).unwrap();
        assert!(old.notify_on_stop, "older files notify by default");
        assert!(!old.welcome_dismissed, "older files have not dismissed it");
        let s: Settings =
            serde_json::from_str(r#"{"notify_on_stop": false, "welcome_dismissed": true}"#)
                .unwrap();
        assert!(!s.notify_on_stop);
        assert!(s.welcome_dismissed);
        let v = serde_json::to_value(s.normalized()).unwrap();
        assert_eq!(v["notify_on_stop"], false);
        assert_eq!(v["welcome_dismissed"], true);
    }

    #[test]
    fn language() {
        let old: Settings = serde_json::from_str(r#"{"interval_secs": 5}"#).unwrap();
        assert_eq!(old.language, "system", "older files have no language");
        for (given, kept) in [
            ("fr", "fr"),
            (" EN ", "en"),
            ("system", "system"),
            ("de", "system"),
            ("", "system"),
        ] {
            let s = Settings {
                language: given.into(),
                ..Default::default()
            }
            .normalized();
            assert_eq!(s.language, kept, "{given:?}");
        }
        let fr = Settings {
            language: "fr".into(),
            ..Default::default()
        };
        assert_eq!(fr.lang(), Lang::Fr);
        assert_eq!(Settings::default().lang(), Lang::system());
        let v = serde_json::to_value(&fr).unwrap();
        assert_eq!(v["language"], "fr");
    }

    #[test]
    fn partial_file_uses_defaults_for_missing_keys() {
        let s: Settings = serde_json::from_str(
            r#"{"interval_secs": 10, "unknown": 1, "api": {"enabled": true}}"#,
        )
        .unwrap();
        assert_eq!(s.interval_secs, 10.0);
        assert!(s.save_csv);
        assert!(s.api.enabled);
        assert_eq!(s.api.endpoint, "https://api.codecarbon.io");
        assert_eq!(s.api.project_name, "", "files from before v2 have no names");
    }

    #[test]
    fn api_names_round_trip_and_clear() {
        let dir = temp_dir("api-names");
        let path = settings_path(&dir);
        let mut s = Settings {
            api: ApiSettings {
                enabled: true,
                endpoint: "https://staging.example.org".into(),
                api_key: "k".into(),
                experiment_id: "e1".into(),
                organization_name: " Org ".into(),
                project_name: "Proj".into(),
                experiment_name: "Exp".into(),
            },
            ..Default::default()
        }
        .normalized();
        assert_eq!(s.api.organization_name, "Org");
        assert!(s.validate().is_ok());
        s.save(&path).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"experiment_name\": \"Exp\""));
        assert_eq!(Settings::load(&path), s);

        s.api.clear_account();
        assert_eq!(
            s.api,
            ApiSettings {
                endpoint: "https://staging.example.org".into(),
                ..Default::default()
            }
        );
        assert!(s.validate().is_ok());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn names_are_not_sent_to_the_agent() {
        let s = Settings {
            api: ApiSettings {
                enabled: true,
                api_key: "k".into(),
                experiment_id: "e1".into(),
                project_name: "Proj".into(),
                ..Default::default()
            },
            ..Default::default()
        };
        let c = s.to_agent_config(Path::new("/data"));
        assert!(c.save_to_api);
        assert_eq!(c.api_key.as_deref(), Some("k"));
        assert_eq!(c.experiment_id.as_deref(), Some("e1"));
        assert_eq!(c.api_endpoint.as_deref(), Some("https://api.codecarbon.io"));
        let v = serde_json::to_value(&c).unwrap();
        assert!(!v.to_string().contains("Proj"));
    }

    #[test]
    fn round_trip_on_disk() {
        let dir = temp_dir("roundtrip");
        let path = settings_path(&dir.join("nested"));
        assert_eq!(
            Settings::load(&path),
            Settings::default(),
            "missing file → defaults"
        );
        let s = Settings {
            interval_secs: 5.0,
            country_iso_code: Some("FRA".into()),
            launch_at_login: true,
            pue: Some(1.2),
            ..Default::default()
        };
        s.save(&path).unwrap();
        assert_eq!(Settings::load(&path), s);
        fs::write(&path, "not json").unwrap();
        assert_eq!(
            Settings::load(&path),
            Settings::default(),
            "corrupt file → defaults"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn validation() {
        let bad_interval = Settings {
            interval_secs: 0.5,
            ..Default::default()
        };
        assert_eq!(bad_interval.validate(), Err(AppError::InvalidInterval));
        let bad_country = Settings {
            country_iso_code: Some("FR".into()),
            ..Default::default()
        };
        assert_eq!(bad_country.validate(), Err(AppError::InvalidCountry));
        let empty_country = Settings {
            country_iso_code: Some("  ".into()),
            ..Default::default()
        };
        assert!(empty_country.validate().is_ok());
        let api_without_key = Settings {
            api: ApiSettings {
                enabled: true,
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(
            api_without_key.validate(),
            Err(AppError::ApiCredentialsMissing)
        );
    }

    #[test]
    fn agent_config() {
        let s = Settings {
            country_iso_code: Some(" fra ".into()),
            region: Some("".into()),
            save_csv: false,
            api: ApiSettings {
                api_key: "secret".into(),
                ..Default::default()
            },
            ..Default::default()
        }
        .normalized();
        assert_eq!(s.country_iso_code.as_deref(), Some("FRA"));
        let c = s.to_agent_config(Path::new("/data"));
        assert_eq!(c.interval_secs, 3.0);
        assert_eq!(c.country_iso_code.as_deref(), Some("FRA"));
        assert_eq!(c.region, None);
        assert!(!c.save_to_file);
        assert_eq!(c.output_dir, "/data");
        assert_eq!(c.tracking_mode, "machine");
        assert!(!c.save_to_api);
        assert_eq!(
            c.api_key, None,
            "API fields are not sent while upload is off"
        );

        let v = serde_json::to_value(&c).unwrap();
        assert!(v.get("country_iso_code").is_some());
        assert!(v.get("pue").is_none());
    }
    #[test]
    fn a_broken_file_is_replaced_by_the_last_good_one_and_said_so() {
        let dir = temp_dir("recover");
        let path = settings_path(&dir);
        let first = Settings {
            interval_secs: 5.0,
            ..Default::default()
        };
        first.save(&path).unwrap();
        let second = Settings {
            interval_secs: 7.0,
            ..Default::default()
        };
        // The second save keeps the first as the last good copy.
        second.save(&path).unwrap();
        assert_eq!(Settings::load_checked(&path).problem, None);

        fs::write(&path, "{ \"interval_secs\": 7.0, ").unwrap();
        let loaded = Settings::load_checked(&path);
        assert_eq!(loaded.settings, first);
        assert_eq!(loaded.problem, Some(AppError::SettingsRestored));
        // The broken file is kept, for whoever wants to look at it.
        assert!(invalid_path(&path).is_file());
    }

    #[test]
    fn a_broken_file_with_no_good_copy_resets_and_says_so() {
        let dir = temp_dir("reset");
        let path = settings_path(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(&path, "not json").unwrap();
        let loaded = Settings::load_checked(&path);
        assert_eq!(loaded.settings, Settings::default());
        assert_eq!(loaded.problem, Some(AppError::SettingsReset));
    }

    #[test]
    fn saving_never_leaves_a_half_written_file_behind() {
        let dir = temp_dir("atomic");
        let path = settings_path(&dir);
        Settings::default().save(&path).unwrap();
        let names: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(!names.iter().any(|n| n.ends_with(".tmp")), "{names:?}");
    }

    #[cfg(unix)]
    #[test]
    fn the_file_and_its_copy_are_readable_by_the_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("private");
        let path = settings_path(&dir);
        fs::create_dir_all(&dir).unwrap();
        // A file left world-readable by an earlier version.
        fs::write(&path, serde_json::to_string(&Settings::default()).unwrap()).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

        let with_key = Settings {
            api: ApiSettings {
                api_key: "secret".into(),
                ..Default::default()
            },
            ..Default::default()
        };
        with_key.save(&path).unwrap();
        with_key.save(&path).unwrap();
        for file in [path.clone(), backup_path(&path)] {
            let mode = fs::metadata(&file).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "{}", file.display());
        }
    }
}
