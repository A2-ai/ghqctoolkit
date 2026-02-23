//! Configuration endpoints.

use crate::api::error::ApiError;
use crate::api::state::AppState;
use crate::api::types::{
    Checklist, ChecklistInfo, ConfigGitRepository, ConfigurationOptions,
    ConfigurationStatusResponse, SetupConfigurationRequest,
};
use crate::configuration::ConfigurationError;
use crate::{Configuration, GitProvider, setup_configuration};
use axum::{Json, extract::State};

/// GET /api/configuration/checklists
pub async fn list_checklists<G: GitProvider + 'static>(
    State(state): State<AppState<G>>,
) -> Result<Json<Vec<Checklist>>, ApiError> {
    let response: Vec<Checklist> = state
        .configuration
        .read()
        .await
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
    let config = state.configuration.read().await;
    let options = &config.options;

    let checklists: Vec<ChecklistInfo> = config
        .checklists
        .values()
        .map(|c| ChecklistInfo {
            name: c.name.clone(),
            item_count: c.content.matches("- [ ]").count() as u32,
        })
        .collect();

    let git_repository = match state.configuration_git_info().await {
        Some(git_info) => Some(ConfigGitRepository::new(&git_info).await?),
        None => None,
    };

    let response = ConfigurationStatusResponse {
        directory: config.path.to_string_lossy().to_string(),
        git_repository,
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

pub async fn setup_configuration_repo<G: GitProvider + 'static>(
    State(state): State<AppState<G>>,
    Json(body): Json<SetupConfigurationRequest>,
) -> Result<Json<ConfigurationStatusResponse>, ApiError> {
    {
        let guard = state.configuration_git_info().await;
        if let Some(g) = guard {
            return Err(ApiError::Conflict(format!(
                "Configuration repository already set up at {} for {}/{}",
                g.path().display(),
                g.owner(),
                g.repo()
            )));
        }
    }

    let url = gix::url::parse(body.url.as_bytes().into())
        .map_err(|e| ApiError::BadRequest(format!("Invalid git URL: {e}")))?;

    let config_dir = state.configuration.read().await.path.clone();

    setup_configuration(&config_dir, url, state.git_cli())
        .await
        .map_err(|e| match e {
            ConfigurationError::Io(ref io_err)
                if io_err.kind() == std::io::ErrorKind::AlreadyExists =>
            {
                ApiError::Conflict(e.to_string())
            }
            _ => ApiError::Internal(e.to_string()),
        })?;

    let mut new_configuration = Configuration::from_path(&config_dir);
    new_configuration.load_checklists();

    {
        let mut config_lock = state.configuration.write().await;
        *config_lock = new_configuration;
    }

    state.update_config_git_info(&config_dir).await;

    get_configuration_status(State(state)).await
}

#[cfg(test)]
mod tests {
    use crate::Configuration;
    use crate::api::server::create_router;
    use crate::api::state::AppState;
    use crate::api::tests::helpers::MockGitInfo;
    use crate::git::{GitCliError, MockGitCli};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use serde_json::json;
    use tower::ServiceExt;

    async fn post_setup(app: axum::Router, url: &str) -> axum::http::Response<Body> {
        let body = json!({ "url": url }).to_string();
        app.oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/configuration/setup")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap()
    }

    /// Guard fires when configuration_git_info is already Some.
    #[tokio::test]
    async fn setup_already_configured_returns_409() {
        let mock = MockGitInfo::builder().build();
        let config = Configuration::default();
        let state = AppState::new(mock.clone(), config, Some(mock), None);
        let app = create_router(state);

        let response = post_setup(app, "https://github.com/owner/config-repo").await;
        assert_eq!(response.status(), StatusCode::CONFLICT);
    }

    /// Successful clone updates state and returns configuration status.
    #[tokio::test]
    async fn setup_success_returns_200_with_status() {
        let mut mock_cli = MockGitCli::new();
        mock_cli.expect_clone().returning(|_, _| Ok(()));

        let mock = MockGitInfo::builder().build();
        let config = Configuration::default();
        let state = AppState::new(mock, config, None, None).with_git_cli(mock_cli);
        let app = create_router(state);

        let response = post_setup(app, "https://github.com/owner/config-repo").await;
        assert_eq!(response.status(), StatusCode::OK);

        let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert!(body.get("directory").is_some());
        assert!(body.get("options").is_some());
        assert!(body.get("checklists").is_some());
        // creator defaults to |_| None so git_repository is absent
        assert!(body["git_repository"].is_null());
    }

    /// A failed clone (e.g. auth error) maps to 500.
    #[tokio::test]
    async fn setup_clone_failure_returns_500() {
        let mut mock_cli = MockGitCli::new();
        mock_cli
            .expect_clone()
            .returning(|_, _| Err(GitCliError::GitCommandFailed("auth failed".to_string())));

        let mock = MockGitInfo::builder().build();
        let config = Configuration::default();
        let state = AppState::new(mock, config, None, None).with_git_cli(mock_cli);
        let app = create_router(state);

        let response = post_setup(app, "https://github.com/owner/config-repo").await;
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
