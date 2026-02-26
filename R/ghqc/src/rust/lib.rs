use std::{path::PathBuf, sync::{Mutex}};

use extendr_api::prelude::*;
use ghqctoolkit::{Configuration, DiskCache, GitInfo, api::AppState, determine_config_dir, utils::StdEnvProvider};

// Macro to generate exports.
// This ensures exported functions are registered with R.
// See corresponding C code in `entrypoint.c`.
extendr_module! {
    mod ghqc;
    fn run;
    fn init_logger;
}

static LOGGER_INIT: Mutex<bool> = Mutex::new(false);


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

#[extendr]
fn init_logger() -> String {
    let mut initialized = LOGGER_INIT.lock().unwrap();
    if *initialized {
        return format!("[GHQC] Logger already initialized");
    }

    let mut builder = env_logger::Builder::new();
    builder.filter_level(log::LevelFilter::Off);
    let (level, display) = match std::env::var("GHQC_LOG_LEVEL").unwrap_or_default().to_uppercase().as_str() {
        "ERROR" => (log::LevelFilter::Error, "ERROR"),
        "WARN" => (log::LevelFilter::Warn, "WARN"),
        "DEBUG" => (log::LevelFilter::Debug, "DEBUG"),
        "TRACE" => (log::LevelFilter::Trace, "TRACE"),
        "INFO" => (log::LevelFilter::Info, "INFO"),
        _ => (log::LevelFilter::Info, "INFO (default)"),
    };

    builder
        .filter_module("ghqctoolkit", level)
        .filter_module("ghqc", level)
        .filter_module("octocrab", level);
    
    *initialized = true;
    
    match builder.try_init() {
        Ok(_) => {
            format!("[GHQC] Logger initialized with level: {display}")
        }
        Err(_) => {
            format!("[GHQC] Logger already initialized")
        }
    }
}


pub trait ResultExt<T> {
    fn map_to_extendr_err(self, message: &str) -> Result<T>;
}

impl<T, E: std::fmt::Debug> ResultExt<T> for std::result::Result<T, E> {
    fn map_to_extendr_err(self, message: &str) -> extendr_api::Result<T> {
        self.map_err(|x| extendr_api::Error::Other(format!("{}: {x:?}", message)))
    }
}
