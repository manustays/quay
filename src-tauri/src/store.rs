use crate::model::{AppConfig, AppError};
use std::path::{Path, PathBuf};

/// Ensure and return the app's data directory, creating `logs/` subdirectory as well.
pub fn config_dir() -> Result<PathBuf, AppError> {
	let base = dirs::data_dir().ok_or_else(|| AppError::Message("no data dir".into()))?;
	let dir = base.join("com.abhi.menubar-service-manager");
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
