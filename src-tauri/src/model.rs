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
	pub order: u32,
	pub favorite: bool,
	#[serde(default)] pub env: BTreeMap<String, String>,
	#[serde(rename = "healthPath")] pub health_path: Option<String>,
	#[serde(rename = "autoStart")] pub auto_start: bool,
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
}

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
		}
	}
}

/// The persisted configuration file shape.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {
	#[serde(default)] pub settings: Settings,
	#[serde(default)] pub items: Vec<ManagedItem>,
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
