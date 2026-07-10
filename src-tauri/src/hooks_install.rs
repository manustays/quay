//! Install the `quay-hook` helper and each agent's hook config so the radar
//! gets authoritative working/waiting/idle states instead of the mtime/CPU
//! guess. macOS-only (paths and the app data dir are Apple-specific).
//!
//! The helper is bundled with the app as a resource and copied to a stable,
//! app-managed path — `<data_dir>/bin/quay-hook` — that survives the .app
//! moving or updating. Every agent's config references that one path.

use crate::model::AppError;
use std::path::{Path, PathBuf};

/// Locate the `quay-hook` helper shipped with the app, to copy to the stable
/// path. In a bundle it is a resource at `<resource_dir>/binaries/quay-hook`;
/// under `tauri dev` the app runs from `target/debug/quay`, so fall back to the
/// release binary that `npm run hook:build` produced at `target/release/`.
pub fn bundled_hook(app: &tauri::AppHandle) -> Option<PathBuf> {
	use tauri::Manager;
	if let Ok(res) = app.path().resource_dir() {
		let p = res.join("binaries/quay-hook");
		if p.exists() {
			return Some(p);
		}
	}
	let exe = std::env::current_exe().ok()?;
	let dev = exe.parent()?.parent()?.join("release/quay-hook");
	dev.exists().then_some(dev)
}

/// Copy the helper to `<data_dir>/bin/quay-hook` (mode 0755) when it is missing
/// or its bytes differ from `src`. Returns the stable path. App-managed: a
/// user-modified copy is overwritten to match the bundled helper.
pub fn install_helper(src: &Path, data_dir: &Path) -> Result<PathBuf, AppError> {
	let bin_dir = data_dir.join("bin");
	std::fs::create_dir_all(&bin_dir)?;
	let dst = bin_dir.join("quay-hook");
	let differs = std::fs::read(&dst).ok() != std::fs::read(src).ok();
	if differs {
		std::fs::copy(src, &dst)?;
		#[cfg(unix)]
		{
			use std::os::unix::fs::PermissionsExt;
			std::fs::set_permissions(&dst, std::fs::Permissions::from_mode(0o755))?;
		}
	}
	Ok(dst)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn tmp() -> PathBuf {
		let d = std::env::temp_dir().join(format!("quay-hooks-{}", uuid::Uuid::new_v4()));
		std::fs::create_dir_all(&d).unwrap();
		d
	}

	#[test]
	fn install_helper_copies_sets_exec_and_recopies_on_diff() {
		let d = tmp();
		let src = d.join("src-hook");
		std::fs::write(&src, b"v1").unwrap();
		let data = d.join("data");

		let dst = install_helper(&src, &data).unwrap();
		assert_eq!(std::fs::read(&dst).unwrap(), b"v1");
		#[cfg(unix)]
		{
			use std::os::unix::fs::PermissionsExt;
			assert_eq!(std::fs::metadata(&dst).unwrap().permissions().mode() & 0o777, 0o755);
		}

		// Same bytes → no error, still present.
		install_helper(&src, &data).unwrap();
		assert_eq!(std::fs::read(&dst).unwrap(), b"v1");

		// Changed source → re-copied.
		std::fs::write(&src, b"v2-longer").unwrap();
		install_helper(&src, &data).unwrap();
		assert_eq!(std::fs::read(&dst).unwrap(), b"v2-longer");

		std::fs::remove_dir_all(&d).ok();
	}
}
