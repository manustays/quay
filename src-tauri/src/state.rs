use crate::model::{AppConfig, Status};
use crate::supervisor::Running;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

/// All shared, mutable app state behind locks.
pub struct AppState {
	pub dir: PathBuf,
	pub config: Mutex<AppConfig>,
	pub running: Mutex<HashMap<String, Running>>,
	pub statuses: Mutex<HashMap<String, Status>>,
	pub errors: Mutex<HashMap<String, String>>,
}
