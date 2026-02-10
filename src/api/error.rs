//! API error types and HTTP status code mapping.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;

/// API error type with automatic HTTP status code mapping.
#[derive(Debug)]
pub enum ApiError {
    /// Resource not found (404)
    NotFound(String),
    /// Validation error (400)
    BadRequest(String),
    /// Conflict error, e.g., blocking QCs not approved (409)
    Conflict(String),
    /// GitHub API error (502)
    GitHubApi(String),
    /// Internal server error (500)
    Internal(String),
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            ApiError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
            ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
            ApiError::Conflict(msg) => (StatusCode::CONFLICT, msg),
            ApiError::GitHubApi(msg) => (StatusCode::BAD_GATEWAY, msg),
            ApiError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg),
        };

        (status, Json(ErrorResponse { error: message })).into_response()
    }
}

impl From<crate::GitHubApiError> for ApiError {
    fn from(err: crate::GitHubApiError) -> Self {
        ApiError::GitHubApi(err.to_string())
    }
}

impl From<crate::GitInfoError> for ApiError {
    fn from(err: crate::GitInfoError) -> Self {
        ApiError::Internal(err.to_string())
    }
}

impl From<crate::QCStatusError> for ApiError {
    fn from(err: crate::QCStatusError) -> Self {
        ApiError::Internal(err.to_string())
    }
}

impl From<crate::IssueError> for ApiError {
    fn from(err: crate::IssueError) -> Self {
        ApiError::Internal(err.to_string())
    }
}

impl From<crate::ApprovalError> for ApiError {
    fn from(err: crate::ApprovalError) -> Self {
        match &err {
            crate::ApprovalError::BlockingQCsNotApproved { .. } => {
                ApiError::Conflict(err.to_string())
            }
            _ => ApiError::Internal(err.to_string()),
        }
    }
}
