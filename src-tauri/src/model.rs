use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// What kind of managed item this is.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ItemKind {
	Project,
	Brew,
	/// `alias = "agent"` keeps configs written before the rename loadable.
	#[serde(alias = "agent")]
	Cli,
	Docker,
	/// A detached daemon controlled by user `startCmd`/`stopCmd` (e.g. a
	/// launchd/CLI-managed service). Quay does not own the process; status is
	/// driven by the configured port, like `brew`/`docker`.
	Command,
}

/// How an item is launched.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum RunMode { Background, Terminal }

/// Live status of an item.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Status { Stopped, Starting, Running, Error }

/// A registered service the app manages.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedItem {
	pub id: String,
	pub name: String,
	pub kind: ItemKind,
	pub dir: Option<String>,
	#[serde(rename = "startCmd")] pub start_cmd: Option<String>,
	#[serde(rename = "stopCmd")] pub stop_cmd: Option<String>,
	pub port: Option<u16>,
	#[serde(rename = "runMode")] pub run_mode: RunMode,
	#[serde(rename = "brewFormula")] pub brew_formula: Option<String>,
	/// Docker image "repo:tag" — drives add-form autofill only (not operational).
	#[serde(rename = "dockerImage", default)] pub docker_image: Option<String>,
	/// Container name — the join key for Docker status, stop, and metrics.
	#[serde(rename = "containerName", default)] pub container_name: Option<String>,
	/// Detected tech stack keyword (e.g. "vite", "django") for the row icon.
	#[serde(default)] pub stack: Option<String>,
	/// Optional group label — items sharing one cluster in the UI and can be
	/// started/stopped together. Trimmed on save; empty means ungrouped.
	#[serde(default)] pub group: Option<String>,
	pub order: u32,
	pub favorite: bool,
	#[serde(default)] pub env: BTreeMap<String, String>,
	#[serde(rename = "healthPath")] pub health_path: Option<String>,
	#[serde(rename = "autoStart")] pub auto_start: bool,
}

/// One agent+cwd pair hidden from the Agents (agent radar) section.
/// Structured, not a `"kind:cwd"` string — macOS paths may contain `:`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IgnoredAgent {
	pub agent: String,
	pub cwd: String,
}

/// App-wide settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
	#[serde(rename = "terminalApp")] pub terminal_app: String,
	#[serde(rename = "pollIntervalSec")] pub poll_interval_sec: u64,
	#[serde(rename = "metricsIntervalSec", default = "default_metrics_interval_sec")]
	pub metrics_interval_sec: u64,
	pub browser: String,
	#[serde(rename = "launchAtLogin")] pub launch_at_login: bool,
	/// Ports hidden from the discovered-listeners ("Detected") section.
	#[serde(rename = "ignoredPorts", default)] pub ignored_ports: Vec<u16>,
	/// Agent sessions hidden from the Agents section. Ignoring hides **all**
	/// sessions of that agent in that cwd.
	#[serde(rename = "ignoredAgents", default)] pub ignored_agents: Vec<IgnoredAgent>,
	/// When true, the Detected section hides listeners with no recognized dev
	/// stack (databases, caches, system services). On by default; the serde
	/// default keeps it on for configs written before this field existed.
	#[serde(rename = "radarDevOnly", default = "default_radar_dev_only")]
	pub radar_dev_only: bool,
	/// When true, the menubar title shows the count of agents waiting on the user
	/// (e.g. `●2`) beside the tray icon. On by default. The tray *icon* still
	/// switches to the waiting glyph regardless — this gates only the title text.
	#[serde(rename = "waitingTitleBadge", default = "default_waiting_title_badge")]
	pub waiting_title_badge: bool,
}

/// Default for [`Settings::radar_dev_only`] — on. A bare `#[serde(default)]`
/// would give `false`, so old configs need this to inherit the new default.
fn default_radar_dev_only() -> bool { true }

/// Default for [`Settings::waiting_title_badge`] — on. Serde fallback so configs
/// written before this field existed keep the badge on.
fn default_waiting_title_badge() -> bool { true }

/// Default metrics sampling interval (seconds). Used both by `Settings::default`
/// and as the serde fallback for configs written before this field existed.
fn default_metrics_interval_sec() -> u64 { 10 }

impl Default for Settings {
	fn default() -> Self {
		Self {
			terminal_app: "Terminal".into(),
			poll_interval_sec: 3,
			metrics_interval_sec: default_metrics_interval_sec(),
			browser: "default".into(),
			launch_at_login: false,
			ignored_ports: Vec::new(),
			ignored_agents: Vec::new(),
			radar_dev_only: true,
			waiting_title_badge: true,
		}
	}
}

/// The persisted configuration file shape.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {
	#[serde(default)] pub settings: Settings,
	#[serde(default)] pub items: Vec<ManagedItem>,
}

/// Available-update payload pushed to the frontend (and cached in `AppState` so a
/// popover that mounts after the check can pull the pending update on demand).
/// `notes` is empty when the release carries no changelog; the frontend renders it
/// as plain text (never markdown/HTML).
#[derive(Debug, Clone, Serialize)]
pub struct UpdateInfo {
	pub version: String,
	#[serde(rename = "currentVersion")] pub current_version: String,
	pub notes: String,
}

/// Status event payload pushed to the frontend.
#[derive(Debug, Clone, Serialize)]
pub struct ItemStatus {
	pub id: String,
	pub status: Status,
	#[serde(rename = "lastError")] pub last_error: Option<String>,
}

/// All recoverable errors surfaced to the frontend.
#[derive(Debug)]
pub enum AppError { Message(String) }

impl std::fmt::Display for AppError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self { AppError::Message(m) => write!(f, "{m}") }
	}
}

impl std::error::Error for AppError {}

impl Serialize for AppError {
	fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
		s.serialize_str(&self.to_string())
	}
}

impl From<std::io::Error> for AppError {
	fn from(e: std::io::Error) -> Self { AppError::Message(e.to_string()) }
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn status_serializes_to_canonical_strings() {
		assert_eq!(serde_json::to_string(&Status::Running).unwrap(), "\"running\"");
		assert_eq!(serde_json::to_string(&ItemKind::Brew).unwrap(), "\"brew\"");
		assert_eq!(serde_json::to_string(&ItemKind::Cli).unwrap(), "\"cli\"");
		assert_eq!(serde_json::to_string(&ItemKind::Docker).unwrap(), "\"docker\"");
		assert_eq!(serde_json::to_string(&ItemKind::Command).unwrap(), "\"command\"");
		assert_eq!(serde_json::to_string(&RunMode::Terminal).unwrap(), "\"terminal\"");
	}

	#[test]
	fn legacy_agent_kind_deserializes_as_cli() {
		// Configs written before the Agent→Cli rename persisted `"kind":"agent"`.
		// The serde alias must load them as `Cli` so the item isn't dropped to
		// config.bad.json. Asserted at the ManagedItem level — the real load path.
		let json = r#"{"id":"x","name":"n","kind":"agent","dir":"/tmp","startCmd":"claude",
			"stopCmd":null,"port":null,"runMode":"terminal","brewFormula":null,"order":0,
			"favorite":false,"healthPath":null,"autoStart":false}"#;
		let item: ManagedItem = serde_json::from_str(json).unwrap();
		assert_eq!(item.kind, ItemKind::Cli);
	}

	#[test]
	fn docker_item_deserializes_without_optional_fields() {
		// A config written before Docker fields existed (and without them) must load.
		let json = r#"{"id":"x","name":"n","kind":"docker","dir":null,"startCmd":"docker run -d img",
			"stopCmd":null,"port":null,"runMode":"background","brewFormula":null,"order":0,
			"favorite":false,"healthPath":null,"autoStart":false}"#;
		let item: ManagedItem = serde_json::from_str(json).unwrap();
		assert_eq!(item.kind, ItemKind::Docker);
		assert_eq!(item.docker_image, None);
		assert_eq!(item.container_name, None);
		assert!(item.env.is_empty());
	}

	#[test]
	fn settings_defaults_match_spec() {
		let s = Settings::default();
		assert_eq!(s.terminal_app, "Terminal");
		assert_eq!(s.poll_interval_sec, 3);
		assert_eq!(s.metrics_interval_sec, 10);
		assert_eq!(s.browser, "default");
		assert!(!s.launch_at_login);
	}

	#[test]
	fn app_error_serializes_as_message_string() {
		let e = AppError::Message("boom".into());
		assert_eq!(serde_json::to_string(&e).unwrap(), "\"boom\"");
	}
}
