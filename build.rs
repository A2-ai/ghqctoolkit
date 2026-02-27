fn main() {
    if std::env::var("CARGO_FEATURE_UI").is_ok() {
        let ui_dir = std::path::Path::new("ui");

        // bun install (only if node_modules is missing)
        if !ui_dir.join("node_modules").exists() {
            let status = std::process::Command::new("bun")
                .args(["install"])
                .current_dir(ui_dir)
                .status()
                .expect("failed to run bun install");
            assert!(status.success(), "bun install failed");
        }

        // bun run build
        let status = std::process::Command::new("bun")
            .args(["run", "build"])
            .current_dir(ui_dir)
            .status()
            .expect("failed to run bun run build");
        assert!(status.success(), "bun run build failed");

        // Rerun if anything in ui/src/ changes
        println!("cargo:rerun-if-changed=ui/src");
        println!("cargo:rerun-if-changed=ui/vite.config.ts");
        println!("cargo:rerun-if-changed=ui/package.json");
    }
}
