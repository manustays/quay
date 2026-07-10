fn main() {
	// The quay-hook helper is bundled as a resource (see tauri.conf.json) and
	// tauri-build fails compilation if the file is absent. `npm run hook:build`
	// (run by beforeDev/BuildCommand) fills it with the real binary for bundles;
	// here we ensure at least an empty placeholder exists so a bare `cargo build`
	// / `cargo test` still compiles on a fresh checkout (binaries/ is gitignored).
	let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
	let hook = std::path::Path::new(&manifest).join("binaries/quay-hook");
	if !hook.exists() {
		let _ = std::fs::create_dir_all(hook.parent().unwrap());
		let _ = std::fs::write(&hook, b"");
	}
	tauri_build::build()
}
