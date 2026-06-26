use crate::model::{AppConfig, AppError};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Ensure and return the app's data directory, creating `logs/` subdirectory as well.
pub fn config_dir() -> Result<PathBuf, AppError> {
	let base = dirs::data_dir().ok_or_else(|| AppError::Message("no data dir".into()))?;
	let dir = base.join("com.abhi.quay");
	std::fs::create_dir_all(dir.join("logs"))?;
	Ok(dir)
}

/// Load `config.json` from `dir`; return default if missing, backup + return default if corrupt.
pub fn load_config(dir: &Path) -> AppConfig {
	let path = dir.join("config.json");
	let Ok(text) = std::fs::read_to_string(&path) else { return AppConfig::default(); };
	match serde_json::from_str::<AppConfig>(&text) {
		Ok(cfg) => cfg,
		Err(_) => {
			let _ = std::fs::rename(&path, dir.join("config.bad.json"));
			AppConfig::default()
		}
	}
}

/// Atomically persist `cfg` as `config.json` in `dir` using a temp file + rename.
pub fn save_config(dir: &Path, cfg: &AppConfig) -> Result<(), AppError> {
	let tmp = dir.join("config.json.tmp");
	let text = serde_json::to_string_pretty(cfg).map_err(|e| AppError::Message(e.to_string()))?;
	std::fs::write(&tmp, text)?;
	std::fs::rename(&tmp, dir.join("config.json"))?;
	Ok(())
}

// ── Runtime PID map (volatile; kept out of config.json) ───────────────────────

/// Load the persisted `id → pid` map of background processes from `pids.json`.
///
/// Returns an empty map if the file is missing or unreadable/corrupt. These PIDs
/// are runtime hints used to reattach to processes that outlived the app; identity
/// is re-verified (alive + listening on the configured port) before adoption.
pub fn load_pids(dir: &Path) -> HashMap<String, u32> {
	let path = dir.join("pids.json");
	let Ok(text) = std::fs::read_to_string(&path) else { return HashMap::new(); };
	serde_json::from_str(&text).unwrap_or_default()
}

/// Atomically persist the `id → pid` map as `pids.json` via temp file + rename.
pub fn save_pids(dir: &Path, pids: &HashMap<String, u32>) -> Result<(), AppError> {
	let tmp = dir.join("pids.json.tmp");
	let text = serde_json::to_string_pretty(pids).map_err(|e| AppError::Message(e.to_string()))?;
	std::fs::write(&tmp, text)?;
	std::fs::rename(&tmp, dir.join("pids.json"))?;
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::model::AppConfig;

	#[test]
	fn save_then_load_roundtrips() {
		let dir = std::env::temp_dir().join(format!("msm-test-{}", uuid::Uuid::new_v4()));
		std::fs::create_dir_all(&dir).unwrap();
		let mut cfg = AppConfig::default();
		cfg.settings.poll_interval_sec = 9;
		save_config(&dir, &cfg).unwrap();
		let loaded = load_config(&dir);
		assert_eq!(loaded.settings.poll_interval_sec, 9);
		std::fs::remove_dir_all(&dir).ok();
	}

	#[test]
	fn missing_config_yields_default() {
		let dir = std::env::temp_dir().join(format!("msm-test-{}", uuid::Uuid::new_v4()));
		std::fs::create_dir_all(&dir).unwrap();
		let loaded = load_config(&dir);
		assert_eq!(loaded.settings.poll_interval_sec, 3);
		std::fs::remove_dir_all(&dir).ok();
	}

	#[test]
	fn pids_save_then_load_roundtrips() {
		let dir = std::env::temp_dir().join(format!("msm-test-{}", uuid::Uuid::new_v4()));
		std::fs::create_dir_all(&dir).unwrap();
		let mut pids = HashMap::new();
		pids.insert("svc-a".to_string(), 4242u32);
		pids.insert("svc-b".to_string(), 9001u32);
		save_pids(&dir, &pids).unwrap();
		assert_eq!(load_pids(&dir), pids);
		std::fs::remove_dir_all(&dir).ok();
	}

	#[test]
	fn missing_pids_yields_empty_map() {
		let dir = std::env::temp_dir().join(format!("msm-test-{}", uuid::Uuid::new_v4()));
		std::fs::create_dir_all(&dir).unwrap();
		assert!(load_pids(&dir).is_empty());
		std::fs::remove_dir_all(&dir).ok();
	}

	#[test]
	fn corrupt_config_is_backed_up_and_defaults_returned() {
		let dir = std::env::temp_dir().join(format!("msm-test-{}", uuid::Uuid::new_v4()));
		std::fs::create_dir_all(&dir).unwrap();
		std::fs::write(dir.join("config.json"), b"{not valid json").unwrap();
		let loaded = load_config(&dir);
		assert_eq!(loaded.settings.poll_interval_sec, 3);
		assert!(dir.join("config.bad.json").exists());
		std::fs::remove_dir_all(&dir).ok();
	}
}
