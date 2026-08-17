//! API error types and HTTP status code mapping.

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;

/// API error type with automatic HTTP status code mapping.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    /// Resource not found (404)
    #[error("Resource not found: {0}")]
    NotFound(String),
    /// Validation error (400)
    #[error("Validation Error: {0}")]
    BadRequest(String),
    /// Operation not permitted by deployment policy (403)
    #[error("Forbidden: {0}")]
    Forbidden(String),
    /// Conflict error, e.g., blocking QCs not approved (409)
    #[error("Request caused conflict: {0}")]
    Conflict(String),
    /// Conflict with structured data (409) - avoids double JSON encoding
    #[error("Request caused conflict")]
    ConflictDetails(serde_json::Value),
    /// GitHub API error (502)
    #[error("GitHub API Error: {0}")]
    GitHubApi(String),
    /// Not implemented (501)
    #[error("Not implemented: {0}")]
    NotImplemented(String),
    /// Internal server error (500)
    #[error("{0}")]
    Internal(String),
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        match self {
            // ConflictDetails returns structured JSON directly without wrapping
            ApiError::ConflictDetails(value) => (StatusCode::CONFLICT, Json(value)).into_response(),
            // All other errors wrap message in ErrorResponse
            _ => {
                let (status, message) = match self {
                    ApiError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
                    ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
                    ApiError::Forbidden(msg) => (StatusCode::FORBIDDEN, msg),
                    ApiError::Conflict(msg) => (StatusCode::CONFLICT, msg),
                    ApiError::GitHubApi(msg) => (StatusCode::BAD_GATEWAY, msg),
                    ApiError::NotImplemented(msg) => (StatusCode::NOT_IMPLEMENTED, msg),
                    ApiError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg),
                    ApiError::ConflictDetails(_) => unreachable!(),
                };

                (status, Json(ErrorResponse { error: message })).into_response()
            }
        }
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

impl From<crate::StartRoundError> for ApiError {
    fn from(err: crate::StartRoundError) -> Self {
        match &err {
            // A still-open round is a precondition violation, not a bad request.
            crate::StartRoundError::RoundStillOpen { .. } => ApiError::Conflict(err.to_string()),
            crate::StartRoundError::GitHubApiError(_) => ApiError::GitHubApi(err.to_string()),
            // Unprocessable domain state. This crate has no 422 variant: every
            // other domain error (IssueError, QCStatusError, ...) maps to
            // Internal, so these follow that convention rather than adding one.
            crate::StartRoundError::NoRounds
            | crate::StartRoundError::AnchorUnresolved { .. }
            | crate::StartRoundError::BranchUnresolved(_) => ApiError::Internal(err.to_string()),
        }
    }
}

impl From<crate::RepairRoundError> for ApiError {
    fn from(err: crate::RepairRoundError) -> Self {
        match &err {
            // Nothing to repair is a precondition violation, exactly as a still-open
            // round is for starting one.
            crate::RepairRoundError::NoOpenRound { .. }
            | crate::RepairRoundError::InitialRound { .. } => ApiError::Conflict(err.to_string()),
            // Underivable domain state, following `StartRoundError::NoRounds`.
            crate::RepairRoundError::NoRounds => ApiError::Internal(err.to_string()),
        }
    }
}

impl From<crate::GitRepositoryError> for ApiError {
    fn from(err: crate::GitRepositoryError) -> Self {
        ApiError::Internal(err.to_string())
    }
}

impl From<crate::GitStatusError> for ApiError {
    fn from(err: crate::GitStatusError) -> Self {
        ApiError::Internal(err.to_string())
    }
}

impl From<crate::GitFileOpsError> for ApiError {
    fn from(err: crate::GitFileOpsError) -> Self {
        ApiError::Internal(err.to_string())
    }
}
