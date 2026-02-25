fn main() {
    if std::env::var("CARGO_FEATURE_UI").is_ok() {
        let ui_dir = std::path::Path::new("ui");
        let (bun, augmented_path) = find_bun();

        // bun install (only if node_modules is missing)
        if !ui_dir.join("node_modules").exists() {
            let status = std::process::Command::new(&bun)
                .args(["install"])
                .current_dir(ui_dir)
                .env("PATH", &augmented_path)
                .status()
                .expect("failed to run bun install");
            assert!(status.success(), "bun install failed");
        }

        // bun run build
        let status = std::process::Command::new(&bun)
            .args(["run", "build"])
            .current_dir(ui_dir)
            .env("PATH", &augmented_path)
            .status()
            .expect("failed to run bun run build");
        assert!(status.success(), "bun run build failed");

        // Rerun if anything in ui/src/ changes
        println!("cargo:rerun-if-changed=ui/src");
        println!("cargo:rerun-if-changed=ui/vite.config.ts");
        println!("cargo:rerun-if-changed=ui/package.json");
    }
}

/// Returns the path to the bun executable and an augmented PATH string that
/// includes bun's bin directory. The augmented PATH is passed to child
/// processes so that scripts run by bun (e.g. `bunx`, `vite`) are also found
/// even when the parent process was launched without the user's shell profile.
fn find_bun() -> (std::path::PathBuf, String) {
    let home = std::env::var("HOME").unwrap_or_default();
    let bun_bin_dir = format!("{home}/.bun/bin");

    // Candidate bun executables in priority order
    let candidates = [
        format!("{bun_bin_dir}/bun"),
        "/usr/local/bin/bun".to_string(),
        "/opt/homebrew/bin/bun".to_string(),
    ];

    let bun_path = candidates
        .iter()
        .find(|p| std::path::Path::new(p.as_str()).exists())
        .map(|p| std::path::PathBuf::from(p))
        .unwrap_or_else(|| std::path::PathBuf::from("bun"));

    // Prepend bun's bin dir to PATH so that child shells spawned by bun
    // (e.g. to run `bunx` or `vite`) can find those binaries too.
    let existing_path = std::env::var("PATH").unwrap_or_default();
    let augmented_path = if existing_path.is_empty() {
        bun_bin_dir
    } else {
        format!("{bun_bin_dir}:{existing_path}")
    };

    (bun_path, augmented_path)
}
