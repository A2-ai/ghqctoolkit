use std::path::PathBuf;

use extendr_api::prelude::*;
use ghqctoolkit::{Configuration, DiskCache, GitInfo, api::AppState, determine_config_dir, utils::StdEnvProvider};


#[extendr]
fn run(port: u16, directory: String) -> Result<()> {
    let config_dir = determine_config_dir(None, &StdEnvProvider).map_to_extendr_err("Failed to determine config dir")?;
    let mut configuration = Configuration::from_path(&config_dir);
    configuration.load_checklists();
    let config_git_info = GitInfo::from_path(&config_dir, &StdEnvProvider).ok();

    let git_info = GitInfo::from_path(&PathBuf::from(directory), &StdEnvProvider).map_to_extendr_err("Failed to initiailize git information")?;
    
    let disk_cache = DiskCache::from_git_info(&git_info).ok();

    let app_state = AppState::new(git_info, configuration, config_git_info, disk_cache);
    
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_to_extendr_err("Failed to initialize runtime")?;
        
    runtime.block_on(
        ghqctoolkit::ui::run(port, app_state, false)
    )
        .map_to_extendr_err("Failed to run app")?;
    
    Ok(())
}

pub trait ResultExt<T> {
    fn map_to_extendr_err(self, message: &str) -> Result<T>;
}

impl<T, E: std::fmt::Debug> ResultExt<T> for std::result::Result<T, E> {
    fn map_to_extendr_err(self, message: &str) -> extendr_api::Result<T> {
        self.map_err(|x| extendr_api::Error::Other(format!("{}: {x:?}", message)))
    }
}

// Macro to generate exports.
// This ensures exported functions are registered with R.
// See corresponding C code in `entrypoint.c`.
extendr_module! {
    mod ghqc;
    fn run;
}
