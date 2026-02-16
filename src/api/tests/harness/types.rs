use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Complete test specification loaded from YAML
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TestCase {
    /// Test name (displayed in output)
    pub name: String,
    /// Test description (optional)
    #[serde(default)]
    pub description: String,
    /// References to JSON fixture files
    #[serde(default)]
    pub fixtures: Fixtures,
    /// Git repository state for MockGitInfo
    #[serde(default)]
    pub git_state: GitState,
    /// HTTP request specification
    pub request: HttpRequest,
    /// Expected response
    pub response: ExpectedResponse,
}

/// Fixture references (files to load)
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Fixtures {
    /// Issue fixture files (from fixtures/issues/)
    #[serde(default)]
    pub issues: Vec<String>,
    /// Milestone fixture files (from fixtures/milestones/)
    #[serde(default)]
    pub milestones: Vec<String>,
    /// User fixture files (from fixtures/users/)
    #[serde(default)]
    pub users: Vec<String>,
    /// Blocking relationships between issues
    #[serde(default)]
    pub blocking: Vec<BlockingRelationship>,
}

/// Defines which issues block other issues
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BlockingRelationship {
    /// Issue number that blocks others
    pub issue: u64,
    /// Issue numbers that are blocked
    pub blocks: Vec<u64>,
}

/// Git repository state for MockGitInfo
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GitState {
    /// Repository owner
    #[serde(default = "default_owner")]
    pub owner: String,
    /// Repository name
    #[serde(default = "default_repo")]
    pub repo: String,
    /// Current commit hash
    #[serde(default = "default_commit")]
    pub commit: String,
    /// Current branch name
    #[serde(default = "default_branch")]
    pub branch: String,
    /// Dirty files in working directory
    #[serde(default)]
    pub dirty_files: Vec<String>,
}

impl Default for GitState {
    fn default() -> Self {
        Self {
            owner: default_owner(),
            repo: default_repo(),
            commit: default_commit(),
            branch: default_branch(),
            dirty_files: Vec::new(),
        }
    }
}

fn default_owner() -> String {
    "test-owner".to_string()
}

fn default_repo() -> String {
    "test-repo".to_string()
}

fn default_commit() -> String {
    "abc123".to_string()
}

fn default_branch() -> String {
    "main".to_string()
}

/// HTTP request specification
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HttpRequest {
    /// HTTP method
    pub method: HttpMethod,
    /// Request path (e.g., "/api/issues/1")
    pub path: String,
    /// Query parameters (optional)
    #[serde(default)]
    pub query: HashMap<String, String>,
    /// Request body as JSON value (optional)
    #[serde(default)]
    pub body: Option<serde_json::Value>,
}

/// HTTP method enum
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Delete,
    Patch,
}

impl HttpMethod {
    pub fn as_str(&self) -> &'static str {
        match self {
            HttpMethod::Get => "GET",
            HttpMethod::Post => "POST",
            HttpMethod::Put => "PUT",
            HttpMethod::Delete => "DELETE",
            HttpMethod::Patch => "PATCH",
        }
    }
}

/// Expected HTTP response
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ExpectedResponse {
    /// Expected HTTP status code
    pub status: u16,
    /// Expected response body (optional)
    #[serde(default)]
    pub body: Option<ResponseBody>,
}

/// Response body assertion configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResponseBody {
    /// Match type: exact, partial, or schema
    pub match_type: MatchType,
    /// For exact matching: full expected value (used when matching arrays/scalars)
    #[serde(default)]
    pub value: Option<serde_json::Value>,
    /// For exact/partial matching: expected fields (used when matching objects)
    #[serde(default)]
    pub fields: HashMap<String, serde_json::Value>,
    /// For schema matching: schema definition
    #[serde(default)]
    pub schema: Option<SchemaAssertion>,
}

/// Match type for response body validation
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MatchType {
    /// Full JSON equality
    Exact,
    /// Check only specified fields
    Partial,
    /// Validate structure/schema
    Schema,
}

/// Schema validation rules
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SchemaAssertion {
    /// Expected type
    #[serde(rename = "type")]
    pub schema_type: SchemaType,
    /// For arrays: minimum length
    #[serde(default)]
    pub min_length: Option<usize>,
    /// For arrays: exact length (optional)
    #[serde(default)]
    pub exact_length: Option<usize>,
    /// For arrays/objects: required field names
    #[serde(default)]
    pub item_fields: Vec<String>,
    /// For arrays: expected field values in first item
    #[serde(default)]
    pub first_item: Option<HashMap<String, serde_json::Value>>,
}

/// Schema type enum
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SchemaType {
    Object,
    Array,
    String,
    Number,
    Boolean,
    Null,
}
