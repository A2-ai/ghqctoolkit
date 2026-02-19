# AGENTS.md

Instructions for AI coding agents working on this repository.

## OpenAPI Specification Maintenance

The API is documented in `openapi/openapi.yml`. This file is manually maintained (not auto-generated).

### After completing any API-related task, you MUST:

1. **Check if `openapi/openapi.yml` needs updates** for:
   - New endpoints added
   - Endpoint paths or methods changed
   - Request body schemas modified
   - Response schemas modified
   - New error responses added
   - Path parameters changed

2. **Update `openapi/openapi.yml`** to reflect the changes:
   - Add/modify path definitions under `paths:`
   - Add/modify schema definitions under `components/schemas:`
   - Ensure request/response examples are accurate
   - Update descriptions to match implementation

3. **Verify consistency** between:
   - Route definitions in `src/api/routes/*.rs`
   - Request types in `src/api/types/requests.rs`
   - Response types in `src/api/types/responses.rs`
   - The `openapi/openapi.yml` specification

### OpenAPI File Location
- Path: `openapi/openapi.yml`
- Format: YAML (OpenAPI 3.0.3)
