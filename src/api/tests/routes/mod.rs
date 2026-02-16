// Phase 1: Converted to YAML (removed)
// mod health_tests;

// Phase 2: Partially converted to YAML
mod issues_tests;      // KEEP - has cache behavior tests
mod milestones_tests;  // KEEP - still has test_create_milestone (POST endpoint)
// mod status_tests;    // REMOVE - fully converted to YAML

// Phase 3+: Keep these until converted
mod comments_tests;
mod configuration_tests;
