//! Configuration endpoints.

use crate::GitProvider;
use crate::api::error::ApiError;
use crate::api::state::AppState;
use crate::api::types::{
    Checklist, ChecklistInfo, ConfigGitRepository, ConfigurationOptions,
    ConfigurationStatusResponse,
};
use axum::{Json, extract::State};

/// GET /api/configuration/checklists
pub async fn list_checklists<G: GitProvider + 'static>(
    State(state): State<AppState<G>>,
) -> Result<Json<Vec<Checklist>>, ApiError> {
    let response: Vec<Checklist> = state
        .configuration
        .checklists
        .values()
        .cloned()
        .map(Into::into)
        .collect();

    Ok(Json(response))
}

/// GET /api/configuration/status
pub async fn get_configuration_status<G: GitProvider + 'static>(
    State(state): State<AppState<G>>,
) -> Result<Json<ConfigurationStatusResponse>, ApiError> {
    let config = &state.configuration;
    let options = &config.options;

    let checklists: Vec<ChecklistInfo> = config
        .checklists
        .values()
        .map(|c| ChecklistInfo {
            name: c.name.clone(),
            item_count: c.content.matches("- [ ]").count() as u32,
        })
        .collect();

    let response = ConfigurationStatusResponse {
        directory: config.path.to_string_lossy().to_string(),
        git_repository: state
            .configuration_git_info()
            .map(ConfigGitRepository::new)
            .transpose()?,
        options: ConfigurationOptions {
            prepended_checklist_note: options.prepended_checklist_note.clone(),
            checklist_display_name: options.checklist_display_name.clone(),
            logo_path: options.logo_path.to_string_lossy().to_string(),
            logo_found: config.path.join(&options.logo_path).exists(),
            checklist_directory: options.checklist_directory.to_string_lossy().to_string(),
            record_path: options.record_path.to_string_lossy().to_string(),
        },
        checklists,
    };

    Ok(Json(response))
}
