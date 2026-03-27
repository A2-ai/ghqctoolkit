//! API module for the GHQC Toolkit REST API.
//!
//! This module provides an Axum-based REST API to expose ghqctoolkit
//! functionality for GUI consumption.

mod cache;
mod error;
mod fetch_helpers;
mod routes;
mod server;
mod state;
mod types;

#[cfg(test)]
mod tests;

pub use error::ApiError;
pub use server::{bind_local_server, bind_local_server_with_url, create_router, local_server_url};
pub use state::AppState;
