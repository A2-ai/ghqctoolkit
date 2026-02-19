//! Shared test utilities for creating mock GitHub objects

use octocrab::models::{Milestone, issues::Issue};
use serde_json::json;

/// Creates a mock Issue object for testing with configurable parameters
pub fn create_test_issue(
    owner: &str,
    repo: &str,
    issue_number: u64,
    title: &str,
    body: &str,
    milestone_number: Option<i64>,
    state: &str,
) -> Issue {
    // Pre-compute all format strings to avoid recursion limit
    let issue_url = format!(
        "https://api.github.com/repos/{}/{}/issues/{}",
        owner, repo, issue_number
    );
    let repo_url = format!("https://api.github.com/repos/{}/{}", owner, repo);
    let labels_url = format!(
        "https://api.github.com/repos/{}/{}/issues/{}/labels{{/name}}",
        owner, repo, issue_number
    );
    let comments_url = format!(
        "https://api.github.com/repos/{}/{}/issues/{}/comments",
        owner, repo, issue_number
    );
    let events_url = format!(
        "https://api.github.com/repos/{}/{}/issues/{}/events",
        owner, repo, issue_number
    );
    let html_url = format!(
        "https://github.com/{}/{}/issues/{}",
        owner, repo, issue_number
    );
    let node_id = format!("MDU6SXNzdWU={}", issue_number);
    let timeline_url = format!(
        "https://api.github.com/repos/{}/{}/issues/{}/timeline",
        owner, repo, issue_number
    );
    let reactions_url = format!(
        "https://api.github.com/repos/{}/{}/issues/{}/reactions",
        owner, repo, issue_number
    );

    // Build milestone using create_test_milestone to avoid duplication
    let milestone_value = milestone_number.map(|num| {
        let milestone = create_test_milestone(
            owner,
            repo,
            num,
            &format!("v{}.0", num),
            Some("Test milestone"),
            "open",
        );
        serde_json::to_value(milestone).expect("Failed to serialize milestone")
    });

    let issue_json = json!({
        "id": issue_number * 1000,
        "node_id": node_id,
        "url": issue_url,
        "repository_url": repo_url,
        "labels_url": labels_url,
        "comments_url": comments_url,
        "events_url": events_url,
        "html_url": html_url,
        "number": issue_number,
        "title": title,
        "user": {
            "login": "octocat",
            "id": 1,
            "node_id": "MDQ6VXNlcjE=",
            "avatar_url": "https://github.com/images/error/octocat_happy.gif",
            "gravatar_id": "",
            "url": "https://api.github.com/users/octocat",
            "html_url": "https://github.com/octocat",
            "followers_url": "https://api.github.com/users/octocat/followers",
            "following_url": "https://api.github.com/users/octocat/following{/other_user}",
            "gists_url": "https://api.github.com/users/octocat/gists{/gist_id}",
            "starred_url": "https://api.github.com/users/octocat/starred{/owner}{/repo}",
            "subscriptions_url": "https://api.github.com/users/octocat/subscriptions",
            "organizations_url": "https://api.github.com/users/octocat/orgs",
            "repos_url": "https://api.github.com/users/octocat/repos",
            "events_url": "https://api.github.com/users/octocat/events{/privacy}",
            "received_events_url": "https://api.github.com/users/octocat/received_events",
            "type": "User",
            "site_admin": false
        },
        "labels": [],
        "state": state,
        "locked": false,
        "assignee": null,
        "assignees": [],
        "milestone": milestone_value,
        "comments": 0,
        "created_at": "2011-04-22T13:33:48Z",
        "updated_at": "2011-04-22T13:33:48Z",
        "closed_at": null,
        "author_association": "COLLABORATOR",
        "active_lock_reason": null,
        "draft": false,
        "pull_request": null,
        "body": body,
        "reactions": {
            "url": reactions_url,
            "total_count": 0,
            "+1": 0,
            "-1": 0,
            "laugh": 0,
            "hooray": 0,
            "confused": 0,
            "heart": 0,
            "rocket": 0,
            "eyes": 0
        },
        "timeline_url": timeline_url,
        "performed_via_github_app": null,
        "state_reason": null
    });

    serde_json::from_value(issue_json).expect("Failed to create test issue")
}

/// Creates a mock Milestone object for testing with configurable parameters
pub fn create_test_milestone(
    owner: &str,
    repo: &str,
    milestone_number: i64,
    title: &str,
    description: Option<&str>,
    state: &str,
) -> Milestone {
    // Pre-compute format strings
    let ms_url = format!(
        "https://api.github.com/repos/{}/{}/milestones/{}",
        owner, repo, milestone_number
    );
    let ms_html_url = format!(
        "https://github.com/{}/{}/milestone/{}",
        owner, repo, milestone_number
    );
    let ms_labels_url = format!(
        "https://api.github.com/repos/{}/{}/milestones/{}/labels",
        owner, repo, milestone_number
    );
    let ms_node_id = format!("MDk6TWlsZXN0b25l{}", milestone_number);
    let cr_avatar_url = format!("https://github.com/images/error/{}_happy.gif", owner);
    let cr_url = format!("https://api.github.com/users/{}", owner);
    let cr_html_url = format!("https://github.com/{}", owner);
    let cr_followers_url = format!("https://api.github.com/users/{}/followers", owner);
    let cr_following_url = format!(
        "https://api.github.com/users/{}/following{{/other_user}}",
        owner
    );
    let cr_gists_url = format!("https://api.github.com/users/{}/gists{{/gist_id}}", owner);
    let cr_starred_url = format!(
        "https://api.github.com/users/{}/starred{{/owner}}{{/repo}}",
        owner
    );
    let cr_subscriptions_url = format!("https://api.github.com/users/{}/subscriptions", owner);
    let cr_organizations_url = format!("https://api.github.com/users/{}/orgs", owner);
    let cr_repos_url = format!("https://api.github.com/users/{}/repos", owner);
    let cr_events_url = format!("https://api.github.com/users/{}/events{{/privacy}}", owner);
    let cr_received_events_url = format!("https://api.github.com/users/{}/received_events", owner);

    let milestone_json = json!({
        "url": ms_url,
        "html_url": ms_html_url,
        "labels_url": ms_labels_url,
        "id": milestone_number * 1000,
        "node_id": ms_node_id,
        "number": milestone_number,
        "title": title,
        "description": description,
        "creator": {
            "login": owner,
            "id": 1,
            "node_id": "MDQ6VXNlcjE=",
            "avatar_url": cr_avatar_url,
            "gravatar_id": "",
            "url": cr_url,
            "html_url": cr_html_url,
            "followers_url": cr_followers_url,
            "following_url": cr_following_url,
            "gists_url": cr_gists_url,
            "starred_url": cr_starred_url,
            "subscriptions_url": cr_subscriptions_url,
            "organizations_url": cr_organizations_url,
            "repos_url": cr_repos_url,
            "events_url": cr_events_url,
            "received_events_url": cr_received_events_url,
            "type": "User",
            "site_admin": false
        },
        "open_issues": 1,
        "closed_issues": 0,
        "state": state,
        "created_at": "2011-04-10T20:09:31Z",
        "updated_at": "2011-04-10T20:09:31Z",
        "due_on": null,
        "closed_at": null
    });

    serde_json::from_value(milestone_json).expect("Failed to create test milestone")
}
