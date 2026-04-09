#![allow(dead_code)]

#[path = "../src/main.rs"]
mod main_binary;

use serde_json::json;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
use tempfile::tempdir;

#[test]
fn plugin_manifest_declares_panel_and_titlebar_widget() {
    let manifest =
        fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("plugin.toml"))
            .expect("plugin manifest should exist");

    assert!(manifest.contains("id = \"codex-usage-plugin\""));
    assert!(manifest.contains("entry = \"codex-usage-plugin\""));
    assert!(manifest.contains("[[panels]]"));
    assert!(manifest.contains("id = \"codex-usage-panel\""));
    assert!(manifest.contains("title = \"Codex Usage\""));
    assert!(manifest.contains("[[titlebar_widgets]]"));
    assert!(manifest.contains("id = \"codex-usage-titlebar\""));
    assert!(manifest.contains("opens_panel_id = \"codex-usage-panel\""));
    assert!(manifest.contains("[[actions]]"));
    assert!(manifest.contains("id = \"increment-fixture-counter\""));
    assert!(manifest.contains("title = \"Increment Fixture Counter\""));
}

#[test]
fn plugin_manifest_declares_validation_fixture_panel_and_titlebar_widget() {
    let manifest =
        fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("plugin.toml"))
            .expect("plugin manifest should exist");

    assert!(manifest.contains("[[panels]]"));
    assert!(manifest.contains("id = \"plugin-surface-fixture-panel\""));
    assert!(manifest.contains("title = \"Plugin Surface Fixture\""));
    assert!(manifest.contains("[[titlebar_widgets]]"));
    assert!(manifest.contains("id = \"plugin-surface-fixture-titlebar\""));
    assert!(manifest.contains("opens_panel_id = \"plugin-surface-fixture-panel\""));
}

#[test]
fn parse_persisted_auth_state_reads_cached_snapshot() {
    let state = main_binary::parse_persisted_auth_state(json!({
        "auth_status": "pending",
        "refresh_token": "refresh-token",
        "account_id": "acct_123",
        "expires_at": 55,
        "last_refresh_at": 44,
        "last_fetched_at": 66,
        "last_error": null,
        "last_snapshot": {
            "auth_status": "authenticated",
            "plan_type": "pro",
            "status_label": "ChatGPT connected",
            "detail": "6% used in the 5 hour Codex window",
            "primary_used_percent": 6,
            "usage_limits": [],
            "credits_summary": {
                "balance_label": "0",
                "detail": "Use credits to send messages beyond your plan limit."
            }
        }
    }))
    .expect("state should parse");

    assert_eq!(state.auth_status, "pending");
    assert_eq!(state.account_id.as_deref(), Some("acct_123"));
    assert_eq!(state.last_fetched_at, 66);
    assert_eq!(
        state
            .last_snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.plan_type.as_deref()),
        Some("pro")
    );
}

#[test]
fn normalize_usage_snapshot_extracts_primary_limits_and_credits() {
    let snapshot = main_binary::normalize_usage_snapshot(&json!({
        "plan_type": "pro",
        "rate_limit": {
            "primary_window": {
                "used_percent": 6,
                "limit_window_seconds": 18000,
                "remaining_seconds": 3600
            },
            "secondary_window": {
                "used_percent": 71,
                "limit_window_seconds": 604800,
                "remaining_seconds": 86400
            }
        },
        "code_review_rate_limit": {
            "primary_window": {
                "used_percent": 0,
                "limit_window_seconds": 18000,
                "remaining_seconds": 18000
            }
        },
        "credits_summary": {
            "balance": 12.5
        }
    }))
    .expect("usage snapshot should normalize");

    assert_eq!(snapshot.auth_status, "authenticated");
    assert_eq!(snapshot.plan_type.as_deref(), Some("pro"));
    assert_eq!(snapshot.primary_used_percent, 6);
    assert_eq!(snapshot.usage_limits.len(), 3);
    assert_eq!(snapshot.usage_limits[0].remaining_percent, 94);
    assert_eq!(snapshot.usage_limits[1].remaining_percent, 29);
    assert_eq!(snapshot.credits_summary.balance_label, "$12.50");
}

#[test]
fn compact_titlebar_label_tracks_auth_state() {
    let signed_out = main_binary::SidecarSnapshot::signed_out();
    assert_eq!(main_binary::compact_titlebar_label(&signed_out), "Sign In");

    let authenticated = main_binary::SidecarSnapshot {
        auth_status: String::from("authenticated"),
        plan_type: Some(String::from("pro")),
        status_label: String::from("ChatGPT connected"),
        detail: String::from("6% used in the 5 hour Codex window"),
        primary_used_percent: 6,
        usage_limits: vec![main_binary::UsageLimitCard {
            id: String::from("codex-primary"),
            title: String::from("5 hour usage limit"),
            remaining_percent: 94,
            resets_at_label: Some(String::from("1 hour")),
        }],
        credits_summary: main_binary::CreditsSummary {
            balance_label: String::from("$12.50"),
            detail: String::from("Credits remain available."),
        },
    };

    assert_eq!(main_binary::compact_titlebar_label(&authenticated), "94%");
}

#[test]
fn save_persisted_auth_state_separates_tokens_from_metadata_file() {
    let temp_dir = tempdir().expect("temp dir should exist");
    let auth_state_path = temp_dir.path().join("codex-chatgpt-auth.json");
    let state = main_binary::PersistedAuthState {
        auth_status: String::from("authenticated"),
        access_token: Some(String::from("access-token")),
        refresh_token: Some(String::from("refresh-token")),
        account_id: Some(String::from("acct_123")),
        expires_at: 55,
        last_refresh_at: 44,
        last_snapshot: None,
        last_fetched_at: 66,
        last_error: None,
    };

    main_binary::save_persisted_auth_state_to_disk_for_tests(&auth_state_path, &state)
        .expect("auth state should save");

    let metadata: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(&auth_state_path).expect("metadata file should exist"),
    )
    .expect("metadata json should parse");
    assert_eq!(metadata.get("access_token"), None);
    assert_eq!(metadata.get("refresh_token"), None);
    assert_eq!(
        metadata.get("account_id").and_then(|value| value.as_str()),
        Some("acct_123")
    );

    let secret_path = main_binary::auth_secret_path_for_tests(&auth_state_path);
    let secrets: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&secret_path).expect("secret file should exist"))
            .expect("secret file should parse");
    assert_eq!(
        secrets.get("access_token").and_then(|value| value.as_str()),
        Some("access-token")
    );
    assert_eq!(
        secrets
            .get("refresh_token")
            .and_then(|value| value.as_str()),
        Some("refresh-token")
    );

    #[cfg(unix)]
    {
        let metadata_mode = fs::metadata(&auth_state_path)
            .expect("metadata should have permissions")
            .permissions()
            .mode()
            & 0o777;
        let secret_mode = fs::metadata(&secret_path)
            .expect("secret file should have permissions")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(metadata_mode, 0o600);
        assert_eq!(secret_mode, 0o600);
    }
}

#[test]
fn load_persisted_auth_state_migrates_legacy_plaintext_tokens() {
    let temp_dir = tempdir().expect("temp dir should exist");
    let auth_state_path = temp_dir.path().join("codex-chatgpt-auth.json");
    fs::write(
        &auth_state_path,
        serde_json::to_vec_pretty(&json!({
            "auth_status": "authenticated",
            "access_token": "legacy-access-token",
            "refresh_token": "legacy-refresh-token",
            "account_id": "acct_legacy",
            "expires_at": 100,
            "last_refresh_at": 90,
            "last_snapshot": null,
            "last_fetched_at": 80,
            "last_error": null,
        }))
        .expect("legacy json should encode"),
    )
    .expect("legacy json should write");

    let state = main_binary::load_persisted_auth_state_from_disk_for_tests(&auth_state_path)
        .expect("legacy state should load");

    assert_eq!(state.access_token.as_deref(), Some("legacy-access-token"));
    assert_eq!(state.refresh_token.as_deref(), Some("legacy-refresh-token"));

    let metadata: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(&auth_state_path).expect("metadata file should still exist"),
    )
    .expect("rewritten metadata should parse");
    assert_eq!(metadata.get("access_token"), None);
    assert_eq!(metadata.get("refresh_token"), None);

    let secret_path = main_binary::auth_secret_path_for_tests(&auth_state_path);
    let secrets: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&secret_path).expect("secret file should exist"))
            .expect("secret file should parse");
    assert_eq!(
        secrets.get("access_token").and_then(|value| value.as_str()),
        Some("legacy-access-token")
    );
    assert_eq!(
        secrets
            .get("refresh_token")
            .and_then(|value| value.as_str()),
        Some("legacy-refresh-token")
    );
}
