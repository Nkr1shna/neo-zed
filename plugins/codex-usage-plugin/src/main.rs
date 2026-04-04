#![cfg_attr(not(feature = "mirror"), allow(dead_code))]

use std::fs;
use std::fs::OpenOptions;
use std::io;
use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use std::{fmt::Write as _, process};

use anyhow::{Context as _, Result};
use base64::Engine as _;
use keyring::Entry;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use url::Url;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;

#[cfg(feature = "mirror")]
use gpui_plugin::{
    ActiveTheme, ClickEvent, Context, IntoElement, Render, StatefulInteractiveElement, Styled,
    Window, h_flex, px, run, v_flex,
};
#[cfg(feature = "mirror")]
use ui_plugin::{Button, Divider, Icon, Label, MenuItem, ProgressBar};

pub const PLUGIN_ID: &str = "codex-usage-plugin";
pub const PANEL_ID: &str = "codex-usage-panel";
pub const PANEL_TITLE: &str = "Codex Usage";
pub const TITLEBAR_WIDGET_ID: &str = "codex-usage-titlebar";
pub const TITLEBAR_WIDGET_TITLE: &str = "Codex Usage";

const AUTH_STATE_FILENAME: &str = "codex-chatgpt-auth.json";
const AUTH_SECRET_FILENAME: &str = ".codex-chatgpt-auth-secrets.json";
const AUTH_KEYRING_SERVICE: &str = "neo-zed.codex-usage-plugin";
const AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
const CHATGPT_URL: &str = "https://chatgpt.com";
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const OAUTH_SCOPE: &str = "openid profile email offline_access";
const ACCOUNT_ID_CLAIM: &str = "https://api.openai.com/auth";
#[cfg(test)]
const REDIRECT_PORT: u16 = 1455;
const REDIRECT_PATH: &str = "/auth/callback";
const DEFAULT_TOKEN_EXPIRY_SECONDS: u64 = 3_600;
const TOKEN_REFRESH_SKEW_MILLIS: u64 = 5 * 60 * 1_000;
const AUTO_REFRESH_INTERVAL_MILLIS: u64 = 60 * 1_000;
const VIEW_POLL_INTERVAL_MILLIS: u64 = 500;
const LOGIN_TIMEOUT_SECONDS: u64 = 10 * 60;
const SUCCESS_HTML: &str =
    "<!doctype html><html><body><p>Authentication successful. Return to Zed.</p></body></html>";

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthSecretBackend {
    Auto,
    FileOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StoredAuthSecrets {
    access_token: Option<String>,
    refresh_token: Option<String>,
}

impl StoredAuthSecrets {
    fn from_state(state: &PersistedAuthState) -> Self {
        Self {
            access_token: state.access_token.clone(),
            refresh_token: state.refresh_token.clone(),
        }
    }

    fn is_empty(&self) -> bool {
        self.access_token.is_none() && self.refresh_token.is_none()
    }
}

#[derive(Debug, Clone)]
struct AuthSecretStore {
    backend: AuthSecretBackend,
    metadata_path: PathBuf,
}

impl AuthSecretStore {
    fn for_metadata_path(metadata_path: &Path) -> Self {
        Self {
            backend: AuthSecretBackend::Auto,
            metadata_path: metadata_path.to_path_buf(),
        }
    }

    #[cfg(test)]
    fn file_only_for_metadata_path(metadata_path: &Path) -> Self {
        Self {
            backend: AuthSecretBackend::FileOnly,
            metadata_path: metadata_path.to_path_buf(),
        }
    }

    fn load(&self) -> Result<Option<StoredAuthSecrets>> {
        let keyring_result = match self.backend {
            AuthSecretBackend::Auto => self.load_from_keyring(),
            AuthSecretBackend::FileOnly => Ok(None),
        };

        match keyring_result {
            Ok(Some(secrets)) => Ok(Some(secrets)),
            Ok(None) => self.load_from_file(),
            Err(keyring_error) => {
                if let Some(secrets) = self.load_from_file()? {
                    Ok(Some(secrets))
                } else {
                    Err(keyring_error)
                }
            }
        }
    }

    fn save(&self, secrets: &StoredAuthSecrets) -> Result<()> {
        if secrets.is_empty() {
            return self.clear();
        }

        let encoded = serde_json::to_string(secrets)
            .with_context(|| "failed to encode stored auth secrets")?;

        match self.backend {
            AuthSecretBackend::Auto => match self.keyring_entry() {
                Ok(entry) => {
                    entry
                        .set_password(&encoded)
                        .with_context(|| "failed to write auth secrets to secure storage")?;
                    self.delete_secret_file_if_exists()?;
                    Ok(())
                }
                Err(keyring_error) => {
                    self.save_to_file(secrets)
                        .with_context(|| format!("{keyring_error:#}"))?;
                    Ok(())
                }
            },
            AuthSecretBackend::FileOnly => self.save_to_file(secrets),
        }
    }

    fn clear(&self) -> Result<()> {
        let mut clear_error = None;

        if let AuthSecretBackend::Auto = self.backend {
            if let Ok(entry) = self.keyring_entry() {
                if let Err(error) = entry.delete_credential() {
                    if !matches!(error, keyring::Error::NoEntry) {
                        clear_error = Some(
                            anyhow::Error::new(error)
                                .context("failed to clear auth secrets from secure storage"),
                        );
                    }
                }
            }
        }

        if let Err(error) = self.delete_secret_file_if_exists() {
            if clear_error.is_none() {
                clear_error = Some(error);
            }
        }

        if let Some(error) = clear_error {
            Err(error)
        } else {
            Ok(())
        }
    }

    fn load_from_keyring(&self) -> Result<Option<StoredAuthSecrets>> {
        let entry = self.keyring_entry()?;
        match entry.get_password() {
            Ok(raw) => {
                let secrets: StoredAuthSecrets = serde_json::from_str(&raw)
                    .with_context(|| "failed to parse auth secrets from secure storage")?;
                if secrets.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(secrets))
                }
            }
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(anyhow::Error::new(error)
                .context("failed to read auth secrets from secure storage")),
        }
    }

    fn load_from_file(&self) -> Result<Option<StoredAuthSecrets>> {
        let path = self.secret_file_path();
        let raw = match fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to read auth secrets `{}`", path.display()));
            }
        };

        let secrets: StoredAuthSecrets = serde_json::from_str(&raw)
            .with_context(|| format!("failed to parse auth secrets `{}`", path.display()))?;
        if secrets.is_empty() {
            Ok(None)
        } else {
            Ok(Some(secrets))
        }
    }

    fn save_to_file(&self, secrets: &StoredAuthSecrets) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(secrets)
            .with_context(|| "failed to encode auth secrets for file storage")?;
        write_bytes_atomically(&self.secret_file_path(), &bytes)
    }

    fn delete_secret_file_if_exists(&self) -> Result<()> {
        let path = self.secret_file_path();
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error)
                .with_context(|| format!("failed to delete auth secrets `{}`", path.display())),
        }
    }

    fn keyring_entry(&self) -> Result<Entry> {
        let account = format!(
            "codex-usage-plugin:{}",
            stable_secret_scope(&self.metadata_path)
        );
        Entry::new(AUTH_KEYRING_SERVICE, &account)
            .map_err(anyhow::Error::new)
            .with_context(|| "failed to initialize secure auth storage")
    }

    fn secret_file_path(&self) -> PathBuf {
        self.metadata_path.with_file_name(AUTH_SECRET_FILENAME)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageLimitCard {
    pub id: String,
    pub title: String,
    pub remaining_percent: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at_label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreditsSummary {
    pub balance_label: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SidecarSnapshot {
    pub auth_status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_type: Option<String>,
    pub status_label: String,
    pub detail: String,
    pub primary_used_percent: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub usage_limits: Vec<UsageLimitCard>,
    pub credits_summary: CreditsSummary,
}

impl SidecarSnapshot {
    pub fn signed_out() -> Self {
        Self {
            auth_status: String::from("signed-out"),
            plan_type: None,
            status_label: String::from("Sign in to ChatGPT"),
            detail: String::from("Sign in to view ChatGPT Codex usage and credits."),
            primary_used_percent: 0,
            usage_limits: Vec::new(),
            credits_summary: CreditsSummary {
                balance_label: String::from("$0.00"),
                detail: String::from("Use credits to send messages beyond your plan limit."),
            },
        }
    }

    fn pending() -> Self {
        Self {
            auth_status: String::from("pending"),
            status_label: String::from("Waiting for ChatGPT sign-in"),
            detail: String::from("Complete the browser sign-in flow to refresh Codex usage."),
            ..Self::signed_out()
        }
    }

    fn error(message: Option<&str>) -> Self {
        Self {
            auth_status: String::from("error"),
            status_label: String::from("ChatGPT sign-in failed"),
            detail: message
                .unwrap_or("ChatGPT authentication failed.")
                .to_string(),
            ..Self::signed_out()
        }
    }

    fn is_authenticated(&self) -> bool {
        self.auth_status == "authenticated"
    }

    fn is_pending(&self) -> bool {
        self.auth_status == "pending"
    }

    fn primary_remaining_percent(&self) -> u32 {
        self.usage_limits
            .first()
            .map(|usage_limit| usage_limit.remaining_percent.min(100))
            .unwrap_or_else(|| 100_u32.saturating_sub(self.primary_used_percent.min(100)))
    }

    fn panel_plan_label(&self) -> Option<String> {
        let plan_type = self.plan_type.as_deref()?;
        let normalized_plan_type = plan_type.replace(['-', '_'], " ");
        let formatted_plan_type = normalized_plan_type
            .split_whitespace()
            .map(capitalize_plan_word)
            .collect::<Vec<_>>()
            .join(" ");

        let label = if formatted_plan_type.to_ascii_lowercase().contains("codex") {
            formatted_plan_type
        } else {
            format!("Codex {formatted_plan_type}")
        };

        Some(format!("Plan: {label}"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedAuthState {
    pub auth_status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    pub expires_at: u64,
    pub last_refresh_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_snapshot: Option<SidecarSnapshot>,
    pub last_fetched_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

impl PersistedAuthState {
    fn signed_out() -> Self {
        Self {
            auth_status: String::from("signed-out"),
            access_token: None,
            refresh_token: None,
            account_id: None,
            expires_at: 0,
            last_refresh_at: 0,
            last_snapshot: None,
            last_fetched_at: 0,
            last_error: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ViewModel {
    serial: u64,
    busy: bool,
    has_session: bool,
    account_label: Option<String>,
    last_error: Option<String>,
    snapshot: SidecarSnapshot,
}

impl ViewModel {
    fn auth_button_label(&self) -> &'static str {
        if self.snapshot.is_pending() || self.busy {
            "Waiting..."
        } else if self.snapshot.is_authenticated() {
            "Sign Out"
        } else {
            "Sign In"
        }
    }

    fn auth_button_disabled(&self) -> bool {
        self.snapshot.is_pending() || self.busy
    }

    fn refresh_button_disabled(&self) -> bool {
        !self.has_session || self.busy || self.snapshot.is_pending()
    }
}

#[derive(Debug, Clone)]
struct SharedState {
    persisted: PersistedAuthState,
    busy: bool,
    serial: u64,
}

#[derive(Clone)]
pub struct CodexUsageStore {
    auth_state_path: PathBuf,
    inner: Arc<Mutex<SharedState>>,
}

impl CodexUsageStore {
    fn new(auth_state_path: PathBuf) -> Result<Self> {
        let persisted = load_persisted_auth_state_from_disk_at(&auth_state_path)?;
        Ok(Self {
            auth_state_path,
            inner: Arc::new(Mutex::new(SharedState {
                persisted,
                busy: false,
                serial: 1,
            })),
        })
    }

    fn current_view_model(&self) -> ViewModel {
        let state = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let snapshot = current_snapshot(&state.persisted);
        let account_label = state
            .persisted
            .account_id
            .as_deref()
            .map(|account_id| format!("Account {}", last_n_characters(account_id, 8)));

        ViewModel {
            serial: state.serial,
            busy: state.busy,
            has_session: state.persisted.refresh_token.is_some(),
            account_label,
            last_error: state.persisted.last_error.clone(),
            snapshot,
        }
    }

    fn maybe_refresh_stale(&self) {
        let should_refresh = {
            let state = self
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if state.busy {
                false
            } else if state.persisted.refresh_token.is_none() {
                false
            } else if state.persisted.last_snapshot.is_none() {
                true
            } else {
                let now = current_time_millis().unwrap_or(0);
                now.saturating_sub(state.persisted.last_fetched_at) >= AUTO_REFRESH_INTERVAL_MILLIS
            }
        };

        if should_refresh {
            self.refresh_usage(false);
        }
    }

    fn begin_login(&self) {
        let prepared_state = {
            let mut state = self
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if state.busy {
                return;
            }

            state.busy = true;
            state.serial = state.serial.saturating_add(1);
            state.persisted.auth_status = String::from("pending");
            state.persisted.last_error = None;
            state.persisted.last_snapshot = Some(SidecarSnapshot::pending());

            if let Err(error) =
                save_persisted_auth_state_to_disk_at(&self.auth_state_path, &state.persisted)
            {
                state.busy = false;
                state.serial = state.serial.saturating_add(1);
                state.persisted.auth_status = String::from("error");
                state.persisted.last_error = Some(error.to_string());
                state.persisted.last_snapshot =
                    Some(SidecarSnapshot::error(Some(&error.to_string())));
                return;
            }

            state.persisted.clone()
        };

        let store = self.clone();
        thread::spawn(move || {
            let result = run_login_flow(prepared_state);
            match result {
                Ok(updated_state) => store.finish_success(updated_state),
                Err(error) => store.finish_failure(OperationKind::Login, error),
            }
        });
    }

    fn refresh_usage(&self, force_refresh: bool) {
        let prepared_state = {
            let mut state = self
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if state.busy || state.persisted.refresh_token.is_none() {
                return;
            }

            state.busy = true;
            state.serial = state.serial.saturating_add(1);
            state.persisted.last_error = None;
            state.persisted.clone()
        };

        let store = self.clone();
        thread::spawn(move || {
            let result = run_refresh_flow(prepared_state, force_refresh);
            match result {
                Ok(updated_state) => store.finish_success(updated_state),
                Err(error) => store.finish_failure(OperationKind::Refresh, error),
            }
        });
    }

    fn sign_out(&self) {
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        state.busy = false;
        state.serial = state.serial.saturating_add(1);
        state.persisted = PersistedAuthState::signed_out();

        if let Err(error) =
            save_persisted_auth_state_to_disk_at(&self.auth_state_path, &state.persisted)
        {
            state.persisted.auth_status = String::from("error");
            state.persisted.last_error = Some(error.to_string());
            state.persisted.last_snapshot = Some(SidecarSnapshot::error(Some(&error.to_string())));
        }
    }

    fn open_chatgpt(&self) {
        if let Err(error) = open::that_detached(CHATGPT_URL) {
            self.record_error(String::from("error"), error.to_string());
        }
    }

    fn finish_success(&self, updated_state: PersistedAuthState) {
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        state.persisted = updated_state;
        state.busy = false;
        state.serial = state.serial.saturating_add(1);

        if let Err(error) =
            save_persisted_auth_state_to_disk_at(&self.auth_state_path, &state.persisted)
        {
            state.persisted.auth_status = String::from("error");
            state.persisted.last_error = Some(error.to_string());
            state.persisted.last_snapshot = Some(SidecarSnapshot::error(Some(&error.to_string())));
        }
    }

    fn finish_failure(&self, operation: OperationKind, error: anyhow::Error) {
        self.record_error(operation.error_status(), error.to_string());
    }

    fn record_error(&self, error_status: String, error_message: String) {
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        state.busy = false;
        state.serial = state.serial.saturating_add(1);
        state.persisted.last_error = Some(error_message.clone());
        if operation_should_override_auth_status(&state.persisted.auth_status, &error_status) {
            state.persisted.auth_status = error_status;
        }
        if state.persisted.last_snapshot.is_none() || state.persisted.auth_status == "error" {
            state.persisted.last_snapshot = Some(SidecarSnapshot::error(Some(&error_message)));
        }

        if let Err(save_error) =
            save_persisted_auth_state_to_disk_at(&self.auth_state_path, &state.persisted)
        {
            state.persisted.last_error = Some(save_error.to_string());
            state.persisted.last_snapshot =
                Some(SidecarSnapshot::error(Some(&save_error.to_string())));
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OperationKind {
    Login,
    Refresh,
}

impl OperationKind {
    fn error_status(self) -> String {
        match self {
            Self::Login => String::from("error"),
            Self::Refresh => String::from("authenticated"),
        }
    }
}

fn operation_should_override_auth_status(current_status: &str, next_status: &str) -> bool {
    current_status == "pending" || current_status.is_empty() || next_status == "error"
}

fn current_snapshot(state: &PersistedAuthState) -> SidecarSnapshot {
    if state.auth_status == "pending" {
        SidecarSnapshot::pending()
    } else if let Some(snapshot) = state.last_snapshot.clone() {
        snapshot
    } else if state.auth_status == "error" {
        SidecarSnapshot::error(state.last_error.as_deref())
    } else {
        SidecarSnapshot::signed_out()
    }
}

fn run_login_flow(mut state: PersistedAuthState) -> Result<PersistedAuthState> {
    let verifier_bytes: [u8; 64] = rand::random();
    let verifier = base64_url_encode(&verifier_bytes);
    let challenge_digest = Sha256::digest(verifier.as_bytes());
    let challenge = base64_url_encode(&challenge_digest);
    let state_bytes: [u8; 16] = rand::random();
    let oauth_state = base64_url_encode(&state_bytes);
    let listener = bind_oauth_listener()?;
    let redirect_uri = oauth_redirect_uri(listener.local_addr()?.port());
    let authorize_url = build_authorize_url(&redirect_uri, &challenge, &oauth_state)?;
    open::that_detached(authorize_url.as_str())
        .with_context(|| "failed to open the ChatGPT sign-in flow")?;

    let authorization_code = wait_for_authorization_code(
        listener,
        &oauth_state,
        Duration::from_secs(LOGIN_TIMEOUT_SECONDS),
    )?;
    apply_tokens(
        &mut state,
        exchange_authorization_code(&authorization_code, &verifier, &redirect_uri, None)?,
    );
    fetch_and_store_usage_snapshot(&mut state, true)?;
    Ok(state)
}

fn run_refresh_flow(
    mut state: PersistedAuthState,
    force_refresh: bool,
) -> Result<PersistedAuthState> {
    fetch_and_store_usage_snapshot(&mut state, force_refresh)?;
    Ok(state)
}

fn fetch_and_store_usage_snapshot(
    state: &mut PersistedAuthState,
    force_refresh: bool,
) -> Result<SidecarSnapshot> {
    let access_token = ensure_access_token(state, force_refresh)?;

    let response = fetch_usage_response(state, &access_token).or_else(|error| {
        if error
            .downcast_ref::<UsageFetchError>()
            .map(UsageFetchError::should_retry_with_fresh_token)
            .unwrap_or(false)
        {
            let fresh_access_token = ensure_access_token(state, true)?;
            fetch_usage_response(state, &fresh_access_token)
        } else {
            Err(error)
        }
    })?;
    let snapshot = normalize_usage_snapshot(&response)?;

    state.auth_status = String::from("authenticated");
    state.last_snapshot = Some(snapshot.clone());
    state.last_fetched_at = current_time_millis()?;
    state.last_error = None;

    Ok(snapshot)
}

fn ensure_access_token(state: &mut PersistedAuthState, force_refresh: bool) -> Result<String> {
    let now = current_time_millis()?;
    let expires_soon =
        state.expires_at == 0 || state.expires_at <= now.saturating_add(TOKEN_REFRESH_SKEW_MILLIS);

    if force_refresh || state.access_token.is_none() || expires_soon {
        refresh_access_token(state)?;
    }

    state
        .access_token
        .clone()
        .ok_or_else(|| anyhow::anyhow!("missing access token after refresh"))
}

fn refresh_access_token(state: &mut PersistedAuthState) -> Result<()> {
    let refresh_token = state
        .refresh_token
        .clone()
        .ok_or_else(|| anyhow::anyhow!("no refresh token is available"))?;
    let response = exchange_refresh_token(&refresh_token)?;
    apply_tokens(state, response);
    Ok(())
}

fn apply_tokens(state: &mut PersistedAuthState, tokens: TokenResponse) {
    state.access_token = Some(tokens.access_token);
    state.refresh_token = Some(tokens.refresh_token);
    state.account_id = tokens.account_id;
    state.expires_at = tokens.expires_at;
    state.last_refresh_at = tokens.last_refresh_at;
    state.auth_status = String::from("authenticated");
}

fn exchange_authorization_code(
    authorization_code: &str,
    verifier: &str,
    redirect_uri: &str,
    fallback_refresh_token: Option<&str>,
) -> Result<TokenResponse> {
    let client = reqwest::blocking::Client::new();
    let response = client
        .post(TOKEN_URL)
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", CLIENT_ID),
            ("code", authorization_code),
            ("code_verifier", verifier),
            ("redirect_uri", redirect_uri),
        ])
        .send()
        .with_context(|| "authorization code exchange request failed")?;

    parse_token_response(response, fallback_refresh_token)
}

fn exchange_refresh_token(refresh_token: &str) -> Result<TokenResponse> {
    let client = reqwest::blocking::Client::new();
    let response = client
        .post(TOKEN_URL)
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", CLIENT_ID),
            ("refresh_token", refresh_token),
        ])
        .send()
        .with_context(|| "refresh token request failed")?;

    parse_token_response(response, Some(refresh_token))
}

fn parse_token_response(
    response: reqwest::blocking::Response,
    fallback_refresh_token: Option<&str>,
) -> Result<TokenResponse> {
    let status = response.status();
    if !status.is_success() {
        let body = response.text().unwrap_or_default();
        anyhow::bail!(
            "token request failed with HTTP {}{}",
            status.as_u16(),
            if body.is_empty() {
                String::new()
            } else {
                format!(": {body}")
            }
        );
    }

    let body = response
        .text()
        .with_context(|| "failed to read the token response body")?;
    let payload: Value =
        serde_json::from_str(&body).with_context(|| "failed to parse token response JSON")?;
    normalize_tokens(payload, fallback_refresh_token)
}

fn normalize_tokens(payload: Value, fallback_refresh_token: Option<&str>) -> Result<TokenResponse> {
    let object = payload
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("token response was not an object"))?;
    let access_token = required_string(object, "access_token")?;
    let refresh_token = optional_string(object, "refresh_token")
        .or_else(|| fallback_refresh_token.map(ToOwned::to_owned))
        .ok_or_else(|| anyhow::anyhow!("token response is missing a refresh token"))?;
    let expires_in = optional_u64(object, "expires_in").unwrap_or(DEFAULT_TOKEN_EXPIRY_SECONDS);
    let account_id = extract_account_id(&access_token).or_else(|| {
        optional_string(object, "id_token")
            .as_deref()
            .and_then(extract_account_id)
    });
    let now = current_time_millis()?;

    Ok(TokenResponse {
        access_token,
        refresh_token,
        account_id,
        expires_at: now.saturating_add(expires_in.saturating_mul(1_000)),
        last_refresh_at: now,
    })
}

fn fetch_usage_response(state: &PersistedAuthState, access_token: &str) -> Result<Value> {
    let client = reqwest::blocking::Client::new();
    let mut request = client
        .get(USAGE_URL)
        .header("Accept", "application/json")
        .bearer_auth(access_token);

    if let Some(account_id) = state.account_id.as_deref() {
        request = request.header("ChatGPT-Account-Id", account_id);
    }

    let response = request.send().with_context(|| "usage request failed")?;
    let status = response.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(anyhow::Error::new(UsageFetchError::from_status(
            status.as_u16(),
        )));
    }
    if !status.is_success() {
        anyhow::bail!("usage request failed with HTTP {}", status.as_u16());
    }

    let body = response
        .text()
        .with_context(|| "failed to read the usage response body")?;
    serde_json::from_str(&body).with_context(|| "failed to parse usage response JSON")
}

struct UsageFetchError {
    status_code: u16,
}

impl std::fmt::Debug for UsageFetchError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("UsageFetchError")
            .field("status_code", &self.status_code)
            .finish()
    }
}

impl std::fmt::Display for UsageFetchError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "usage request failed with retryable HTTP {}",
            self.status_code
        )
    }
}

impl std::error::Error for UsageFetchError {}

impl UsageFetchError {
    fn from_status(status_code: u16) -> Self {
        Self { status_code }
    }

    fn should_retry_with_fresh_token(&self) -> bool {
        matches!(self.status_code, 401 | 403)
    }
}

pub fn normalize_usage_snapshot(value: &Value) -> Result<SidecarSnapshot> {
    let object = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("usage response was not a JSON object"))?;
    let rate_limit = object.get("rate_limit");
    let code_review_rate_limit = object.get("code_review_rate_limit");
    let additional_rate_limits = object
        .get("additional_rate_limits")
        .and_then(Value::as_array);
    let primary_window = rate_limit
        .and_then(Value::as_object)
        .and_then(|rate_limit| rate_limit.get("primary_window"))
        .and_then(Value::as_object);

    let mut usage_limits = normalize_limit_windows(
        rate_limit,
        "codex",
        "5 hour usage limit",
        "Weekly usage limit",
    );
    usage_limits.extend(normalize_limit_windows(
        code_review_rate_limit,
        "code-review",
        "Code review",
        "Code review weekly usage limit",
    ));

    if let Some(additional_rate_limits) = additional_rate_limits {
        for (index, entry) in additional_rate_limits.iter().enumerate() {
            let limit_name = entry
                .as_object()
                .and_then(|entry| entry.get("limit_name"))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| format!("Additional limit {}", index + 1));
            usage_limits.extend(normalize_limit_windows(
                entry.as_object().and_then(|entry| entry.get("rate_limit")),
                &format!("additional-{index}"),
                &format!("{limit_name} 5 hour usage limit"),
                &format!("{limit_name} Weekly usage limit"),
            ));
        }
    }

    let primary_used_percent = primary_window
        .and_then(|window| window.get("used_percent"))
        .and_then(read_percent)
        .unwrap_or(0);
    let credits_summary = normalize_credits_summary(
        object
            .get("credits_summary")
            .or_else(|| object.get("credits")),
    );

    Ok(SidecarSnapshot {
        auth_status: String::from("authenticated"),
        plan_type: optional_string(object, "plan_type"),
        status_label: String::from("ChatGPT connected"),
        detail: format!("{}% used in the 5 hour Codex window", primary_used_percent),
        primary_used_percent,
        usage_limits,
        credits_summary,
    })
}

fn normalize_limit_windows(
    rate_limit: Option<&Value>,
    prefix: &str,
    primary_title: &str,
    secondary_title: &str,
) -> Vec<UsageLimitCard> {
    let Some(rate_limit) = rate_limit.and_then(Value::as_object) else {
        return Vec::new();
    };

    let mut windows = Vec::new();
    if let Some(window) = normalize_usage_window(
        rate_limit.get("primary_window"),
        &format!("{prefix}-primary"),
        primary_title,
    ) {
        windows.push(window);
    }
    if let Some(window) = normalize_usage_window(
        rate_limit.get("secondary_window"),
        &format!("{prefix}-secondary"),
        secondary_title,
    ) {
        windows.push(window);
    }
    windows
}

fn normalize_usage_window(window: Option<&Value>, id: &str, title: &str) -> Option<UsageLimitCard> {
    let window = window?.as_object()?;
    let used_percent = window
        .get("used_percent")
        .and_then(read_percent)
        .unwrap_or(0);
    let remaining_percent = 100_u32.saturating_sub(used_percent);

    Some(UsageLimitCard {
        id: id.to_string(),
        title: title.to_string(),
        remaining_percent,
        resets_at_label: window
            .get("remaining_seconds")
            .and_then(read_integer)
            .map(format_duration_label),
    })
}

fn normalize_credits_summary(value: Option<&Value>) -> CreditsSummary {
    let Some(object) = value.and_then(Value::as_object) else {
        return CreditsSummary {
            balance_label: String::from("$0.00"),
            detail: String::from("Use credits to send messages beyond your plan limit."),
        };
    };

    if object
        .get("unlimited")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return CreditsSummary {
            balance_label: String::from("Unlimited"),
            detail: String::from("Credits are unlimited."),
        };
    }

    let balance = object.get("balance").and_then(read_number).unwrap_or(0.0);

    CreditsSummary {
        balance_label: format!("${balance:.2}"),
        detail: String::from("Use credits to send messages beyond your plan limit."),
    }
}

pub(crate) fn parse_persisted_auth_state(value: Value) -> Result<PersistedAuthState> {
    let object = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("persisted auth state must be a JSON object"))?;
    let refresh_token = optional_string(object, "refresh_token");
    let auth_status = optional_string(object, "auth_status").unwrap_or_else(|| {
        if refresh_token.is_some() {
            String::from("authenticated")
        } else if optional_string(object, "last_error").is_some() {
            String::from("error")
        } else {
            String::from("signed-out")
        }
    });

    Ok(PersistedAuthState {
        auth_status,
        access_token: optional_string(object, "access_token"),
        refresh_token,
        account_id: optional_string(object, "account_id"),
        expires_at: optional_u64(object, "expires_at").unwrap_or(0),
        last_refresh_at: optional_u64(object, "last_refresh_at").unwrap_or(0),
        last_snapshot: object
            .get("last_snapshot")
            .filter(|value| !value.is_null())
            .cloned()
            .map(parse_sidecar_snapshot)
            .transpose()?,
        last_fetched_at: optional_u64(object, "last_fetched_at").unwrap_or(0),
        last_error: optional_string(object, "last_error"),
    })
}

fn parse_sidecar_snapshot(value: Value) -> Result<SidecarSnapshot> {
    let object = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("snapshot must be a JSON object"))?;

    Ok(SidecarSnapshot {
        auth_status: required_string(object, "auth_status")?,
        plan_type: optional_string(object, "plan_type"),
        status_label: optional_string(object, "status_label")
            .unwrap_or_else(|| String::from("ChatGPT connected")),
        detail: optional_string(object, "detail")
            .unwrap_or_else(|| String::from("Codex usage is available.")),
        primary_used_percent: optional_u32(object, "primary_used_percent")
            .unwrap_or(0)
            .min(100),
        usage_limits: parse_usage_limits(object.get("usage_limits")),
        credits_summary: parse_credits_summary(object.get("credits_summary")),
    })
}

fn parse_usage_limits(value: Option<&Value>) -> Vec<UsageLimitCard> {
    let Some(array) = value.and_then(Value::as_array) else {
        return Vec::new();
    };

    array
        .iter()
        .filter_map(Value::as_object)
        .filter_map(|object| {
            Some(UsageLimitCard {
                id: required_string(object, "id").ok()?,
                title: required_string(object, "title").ok()?,
                remaining_percent: optional_u32(object, "remaining_percent")
                    .unwrap_or(0)
                    .min(100),
                resets_at_label: optional_string(object, "resets_at_label"),
            })
        })
        .collect()
}

fn parse_credits_summary(value: Option<&Value>) -> CreditsSummary {
    let Some(object) = value.and_then(Value::as_object) else {
        return CreditsSummary {
            balance_label: String::from("$0.00"),
            detail: String::from("Use credits to send messages beyond your plan limit."),
        };
    };

    CreditsSummary {
        balance_label: optional_string(object, "balance_label")
            .unwrap_or_else(|| String::from("$0.00")),
        detail: optional_string(object, "detail").unwrap_or_else(|| {
            String::from("Use credits to send messages beyond your plan limit.")
        }),
    }
}

fn load_persisted_auth_state_from_disk_at(path: &Path) -> Result<PersistedAuthState> {
    load_persisted_auth_state_from_disk_with_store(path, &AuthSecretStore::for_metadata_path(path))
}

#[cfg(test)]
pub(crate) fn load_persisted_auth_state_from_disk_for_tests(
    path: &Path,
) -> Result<PersistedAuthState> {
    let secret_store = AuthSecretStore::file_only_for_metadata_path(path);
    load_persisted_auth_state_from_disk_with_store(path, &secret_store)
}

fn load_persisted_auth_state_from_disk_with_store(
    path: &Path,
    secret_store: &AuthSecretStore,
) -> Result<PersistedAuthState> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(PersistedAuthState::signed_out());
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to read auth state `{}`", path.display()));
        }
    };
    let value = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse auth state `{}`", path.display()))?;
    let mut state = parse_persisted_auth_state(value)?;
    let legacy_secrets = StoredAuthSecrets::from_state(&state);

    if let Some(secrets) = secret_store.load()? {
        state.access_token = secrets.access_token;
        state.refresh_token = secrets.refresh_token;
    } else if !legacy_secrets.is_empty() {
        secret_store.save(&legacy_secrets)?;
    }

    if !legacy_secrets.is_empty() {
        save_persisted_auth_state_to_disk_with_store(path, &state, secret_store)?;
    }

    Ok(state)
}

fn save_persisted_auth_state_to_disk_at(path: &Path, state: &PersistedAuthState) -> Result<()> {
    save_persisted_auth_state_to_disk_with_store(
        path,
        state,
        &AuthSecretStore::for_metadata_path(path),
    )
}

#[cfg(test)]
pub(crate) fn save_persisted_auth_state_to_disk_for_tests(
    path: &Path,
    state: &PersistedAuthState,
) -> Result<()> {
    let secret_store = AuthSecretStore::file_only_for_metadata_path(path);
    save_persisted_auth_state_to_disk_with_store(path, state, &secret_store)
}

#[cfg(test)]
pub(crate) fn auth_secret_path_for_tests(path: &Path) -> PathBuf {
    AuthSecretStore::file_only_for_metadata_path(path).secret_file_path()
}

fn save_persisted_auth_state_to_disk_with_store(
    path: &Path,
    state: &PersistedAuthState,
    secret_store: &AuthSecretStore,
) -> Result<()> {
    secret_store.save(&StoredAuthSecrets::from_state(state))?;
    let bytes = serde_json::to_vec_pretty(&persisted_auth_state_to_json(state))
        .with_context(|| format!("failed to encode auth state `{}`", path.display()))?;
    write_bytes_atomically(path, &bytes)
        .with_context(|| format!("failed to write auth state `{}`", path.display()))
}

fn persisted_auth_state_to_json(state: &PersistedAuthState) -> Value {
    json!({
        "auth_status": state.auth_status,
        "account_id": state.account_id,
        "expires_at": state.expires_at,
        "last_refresh_at": state.last_refresh_at,
        "last_snapshot": state.last_snapshot,
        "last_fetched_at": state.last_fetched_at,
        "last_error": state.last_error,
    })
}

pub fn compact_titlebar_label(snapshot: &SidecarSnapshot) -> String {
    if snapshot.is_pending() {
        String::from("Waiting")
    } else if snapshot.is_authenticated() {
        format!("{}%", snapshot.primary_remaining_percent())
    } else if snapshot.auth_status == "error" {
        String::from("Retry")
    } else {
        String::from("Sign In")
    }
}

fn last_n_characters(value: &str, count: usize) -> String {
    let length = value.chars().count();
    value
        .chars()
        .skip(length.saturating_sub(count))
        .collect::<String>()
}

fn format_duration_label(seconds: u64) -> String {
    if seconds >= 86_400 {
        format!("resets in {}d", seconds / 86_400)
    } else if seconds >= 3_600 {
        format!("resets in {}h", seconds / 3_600)
    } else if seconds >= 60 {
        format!("resets in {}m", seconds / 60)
    } else {
        format!("resets in {seconds}s")
    }
}

fn read_number(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_u64().map(|value| value as f64))
        .or_else(|| {
            value
                .as_str()
                .and_then(|value| value.trim().parse::<f64>().ok())
        })
}

fn read_integer(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|value| u64::try_from(value).ok()))
        .or_else(|| {
            value
                .as_str()
                .and_then(|value| value.trim().parse::<u64>().ok())
        })
}

fn read_percent(value: &Value) -> Option<u32> {
    read_number(value).map(|value| value.round().clamp(0.0, 100.0) as u32)
}

fn required_string(object: &Map<String, Value>, field_name: &str) -> Result<String> {
    object
        .get(field_name)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow::anyhow!("missing `{field_name}`"))
}

fn optional_string(object: &Map<String, Value>, field_name: &str) -> Option<String> {
    object
        .get(field_name)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn optional_u32(object: &Map<String, Value>, field_name: &str) -> Option<u32> {
    object
        .get(field_name)
        .and_then(read_integer)
        .and_then(|value| u32::try_from(value).ok())
}

fn optional_u64(object: &Map<String, Value>, field_name: &str) -> Option<u64> {
    object.get(field_name).and_then(read_integer)
}

fn stable_secret_scope(path: &Path) -> String {
    let digest = Sha256::digest(path.to_string_lossy().as_bytes());
    let mut scope = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(&mut scope, "{byte:02x}");
    }
    scope
}

fn write_bytes_atomically(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create `{}`", parent.display()))?;
    }

    let temp_path = path.with_extension(format!(
        "tmp-{}-{}",
        process::id(),
        current_time_millis().unwrap_or(0)
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp_path)
        .with_context(|| format!("failed to create temporary file `{}`", temp_path.display()))?;

    set_owner_only_permissions(&temp_path)?;
    file.write_all(bytes)
        .with_context(|| format!("failed to write temporary file `{}`", temp_path.display()))?;
    file.flush()
        .with_context(|| format!("failed to flush temporary file `{}`", temp_path.display()))?;
    drop(file);

    if let Err(rename_error) = fs::rename(&temp_path, path) {
        match fs::remove_file(path) {
            Ok(()) => fs::rename(&temp_path, path).with_context(|| {
                format!(
                    "failed to replace `{}` after rename error: {rename_error}",
                    path.display()
                )
            })?,
            Err(remove_error) if remove_error.kind() == io::ErrorKind::NotFound => {
                fs::rename(&temp_path, path).with_context(|| {
                    format!(
                        "failed to move temporary file into place for `{}` after rename error: {rename_error}",
                        path.display()
                    )
                })?;
            }
            Err(remove_error) => {
                let _ = fs::remove_file(&temp_path);
                return Err(remove_error).with_context(|| {
                    format!(
                        "failed to replace `{}` after rename error: {rename_error}",
                        path.display()
                    )
                });
            }
        }
    }

    set_owner_only_permissions(path)?;
    Ok(())
}

fn set_owner_only_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        let permissions = fs::Permissions::from_mode(0o600);
        fs::set_permissions(path, permissions)
            .with_context(|| format!("failed to restrict permissions for `{}`", path.display()))?;
    }

    #[cfg(not(unix))]
    {
        let _ = path;
    }

    Ok(())
}

fn current_time_millis() -> Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64)
        .map_err(|error| anyhow::anyhow!("system clock is before the Unix epoch: {error}"))
}

fn capitalize_plan_word(word: &str) -> String {
    let mut characters = word.chars();
    let Some(first_character) = characters.next() else {
        return String::new();
    };

    first_character
        .to_uppercase()
        .chain(characters.flat_map(|character| character.to_lowercase()))
        .collect()
}

fn oauth_redirect_uri(port: u16) -> String {
    format!("http://localhost:{port}{REDIRECT_PATH}")
}

fn build_authorize_url(redirect_uri: &str, challenge: &str, oauth_state: &str) -> Result<Url> {
    let mut authorize_url =
        Url::parse(AUTHORIZE_URL).with_context(|| "failed to parse the OpenAI authorize URL")?;
    authorize_url
        .query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", CLIENT_ID)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("scope", OAUTH_SCOPE)
        .append_pair("code_challenge", challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", oauth_state)
        .append_pair("id_token_add_organizations", "true")
        .append_pair("codex_cli_simplified_flow", "true")
        .append_pair("originator", "pi");
    Ok(authorize_url)
}

fn bind_oauth_listener() -> Result<TcpListener> {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .with_context(|| "failed to bind the OAuth callback server on an ephemeral port")?;
    listener
        .set_nonblocking(true)
        .with_context(|| "failed to configure the OAuth callback server")?;
    Ok(listener)
}

fn wait_for_authorization_code(
    listener: TcpListener,
    expected_state: &str,
    timeout: Duration,
) -> Result<String> {
    let started_at = Instant::now();
    while started_at.elapsed() < timeout {
        match listener.accept() {
            Ok((mut stream, _)) => {
                return handle_oauth_connection(&mut stream, expected_state);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(100));
            }
            Err(error) => {
                return Err(error).with_context(|| "failed while waiting for the OAuth callback");
            }
        }
    }

    anyhow::bail!("ChatGPT sign-in timed out before the OAuth callback arrived")
}

fn handle_oauth_connection(
    stream: &mut std::net::TcpStream,
    expected_state: &str,
) -> Result<String> {
    let mut buffer = [0_u8; 4096];
    let bytes_read = stream
        .read(&mut buffer)
        .with_context(|| "failed to read the OAuth callback request")?;
    let request = String::from_utf8_lossy(&buffer[..bytes_read]);
    let request_target = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .ok_or_else(|| anyhow::anyhow!("OAuth callback request was missing a request target"))?;
    let callback_url = Url::parse(&format!("http://localhost{request_target}"))
        .with_context(|| "failed to parse the OAuth callback URL")?;

    if callback_url.path() != REDIRECT_PATH {
        write_http_response(stream, 404, "Not found")?;
        anyhow::bail!("unexpected OAuth callback path `{}`", callback_url.path());
    }

    if callback_url
        .query_pairs()
        .find(|(key, _)| key == "state")
        .map(|(_, value)| value.to_string())
        .as_deref()
        != Some(expected_state)
    {
        write_http_response(stream, 400, "State mismatch")?;
        anyhow::bail!("OAuth callback state mismatch");
    }

    let authorization_code = callback_url
        .query_pairs()
        .find(|(key, _)| key == "code")
        .map(|(_, value)| value.to_string())
        .ok_or_else(|| anyhow::anyhow!("OAuth callback is missing an authorization code"))?;

    write_http_response(stream, 200, SUCCESS_HTML)?;
    Ok(authorization_code)
}

fn write_http_response(
    stream: &mut std::net::TcpStream,
    status_code: u16,
    body: &str,
) -> Result<()> {
    let status_text = match status_code {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        _ => "Error",
    };
    let response = format!(
        "HTTP/1.1 {status_code} {status_text}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .with_context(|| "failed to write the OAuth callback response")
}

fn base64_url_encode(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn extract_account_id(token: &str) -> Option<String> {
    let payload = token.split('.').nth(1)?;
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    let value: Value = serde_json::from_slice(&decoded).ok()?;
    value
        .get(ACCOUNT_ID_CLAIM)
        .and_then(Value::as_object)
        .and_then(|claim| claim.get("chatgpt_account_id"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

pub fn plugin_manifest_directory() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn auth_state_path() -> PathBuf {
    plugin_manifest_directory().join(AUTH_STATE_FILENAME)
}

#[cfg(feature = "mirror")]
#[derive(Clone)]
struct PluginRuntimeContext {
    store: CodexUsageStore,
}

#[cfg(feature = "mirror")]
impl PluginRuntimeContext {
    fn new() -> Result<Self> {
        Ok(Self {
            store: CodexUsageStore::new(auth_state_path())?,
        })
    }
}

#[cfg(feature = "mirror")]
struct CodexUsagePanel {
    store: CodexUsageStore,
    view_model: ViewModel,
}

#[cfg(feature = "mirror")]
impl CodexUsagePanel {
    fn new(store: CodexUsageStore, cx: &mut Context<Self>) -> Self {
        let this = Self {
            view_model: store.current_view_model(),
            store,
        };
        this.store.maybe_refresh_stale();
        this.spawn_sync_loop(cx);
        this
    }

    fn spawn_sync_loop(&self, cx: &mut Context<Self>) {
        let store = self.store.clone();
        cx.spawn(async move |this, cx| {
            loop {
                smol::Timer::after(Duration::from_millis(VIEW_POLL_INTERVAL_MILLIS)).await;
                store.maybe_refresh_stale();
                let view_model = store.current_view_model();
                if this
                    .update(cx, |this, cx| this.sync_view_model(view_model, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    fn sync_view_model(&mut self, view_model: ViewModel, cx: &mut Context<Self>) {
        if self.view_model.serial != view_model.serial {
            self.view_model = view_model;
            cx.notify();
        }
    }

    fn begin_login(&mut self, cx: &mut Context<Self>) {
        self.store.begin_login();
        self.sync_view_model(self.store.current_view_model(), cx);
    }

    fn refresh_usage(&mut self, cx: &mut Context<Self>) {
        self.store.refresh_usage(true);
        self.sync_view_model(self.store.current_view_model(), cx);
    }

    fn sign_out(&mut self, cx: &mut Context<Self>) {
        self.store.sign_out();
        self.sync_view_model(self.store.current_view_model(), cx);
    }

    fn open_chatgpt(&mut self, cx: &mut Context<Self>) {
        self.store.open_chatgpt();
        self.sync_view_model(self.store.current_view_model(), cx);
    }

    fn render_usage_card(
        &self,
        usage_limit: &UsageLimitCard,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let mut card = v_flex()
            .w_full()
            .gap(px(1.0))
            .rounded_md()
            .bg(cx.theme().colors().surface_background)
            .p_2()
            .child(Label::new(usage_limit.title.clone()))
            .child(Label::new(format!(
                "{}% remaining",
                usage_limit.remaining_percent
            )))
            .child(ProgressBar::new(
                format!("usage-{}", usage_limit.id),
                usage_limit.remaining_percent as f32,
                100.0,
                cx,
            ));

        if let Some(resets_at_label) = usage_limit.resets_at_label.as_deref() {
            card = card.child(Label::new(resets_at_label.to_string()));
        }

        card
    }
}

#[cfg(feature = "mirror")]
impl Render for CodexUsagePanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view_model = self.view_model.clone();

        let mut content = v_flex().w_full().gap(px(2.0));

        if !view_model.snapshot.is_authenticated() && !view_model.snapshot.is_pending() {
            content = content.child(
                v_flex()
                    .w_full()
                    .gap(px(2.0))
                    .child(Label::new(view_model.snapshot.status_label.clone()))
                    .child(
                        Button::new("codex-usage-sign-in", "Sign In to ChatGPT").on_click(
                            cx.listener(|this, _: &ClickEvent, _window, cx| {
                                this.begin_login(cx);
                            }),
                        ),
                    ),
            );
        } else if view_model.snapshot.is_pending() {
            content = content.child(Label::new("Waiting for ChatGPT sign-in..."));
        } else {
            if let Some(plan_label) = view_model.snapshot.panel_plan_label() {
                content = content.child(Label::new(plan_label));
            }

            if view_model.snapshot.usage_limits.is_empty() {
                content = content.child(Label::new("No usage data available."));
            } else {
                for usage_limit in &view_model.snapshot.usage_limits {
                    content = content
                        .child(self.render_usage_card(usage_limit, cx))
                        .child(Divider::horizontal());
                }
            }

            content = content.child(Label::new(format!(
                "Credits: {}",
                view_model.snapshot.credits_summary.balance_label
            )));
        }

        if let Some(error_message) = view_model.last_error.as_deref() {
            content = content.child(Label::new(error_message.to_string()));
        }

        // Menu items rendered by the host as a kebab menu in the panel header
        content = content
            .child(
                MenuItem::new(
                    "codex-menu-refresh",
                    if view_model.busy {
                        "Refreshing..."
                    } else {
                        "Refresh Usage"
                    },
                )
                .disabled(view_model.refresh_button_disabled())
                .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                    this.refresh_usage(cx);
                })),
            )
            .child(
                MenuItem::new("codex-menu-auth", view_model.auth_button_label())
                    .disabled(view_model.auth_button_disabled())
                    .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                        if this.view_model.snapshot.is_authenticated() {
                            this.sign_out(cx);
                        } else {
                            this.begin_login(cx);
                        }
                    })),
            )
            .child(
                MenuItem::new("codex-menu-open-chatgpt", "Open ChatGPT").on_click(cx.listener(
                    |this, _: &ClickEvent, _window, cx| {
                        this.open_chatgpt(cx);
                    },
                )),
            );

        v_flex()
            .size_full()
            .w_full()
            .overflow_y_scroll()
            .p_4()
            .child(content)
    }
}

#[cfg(feature = "mirror")]
struct CodexUsageTitlebarWidget {
    store: CodexUsageStore,
    view_model: ViewModel,
}

#[cfg(feature = "mirror")]
impl CodexUsageTitlebarWidget {
    fn new(store: CodexUsageStore, cx: &mut Context<Self>) -> Self {
        let this = Self {
            view_model: store.current_view_model(),
            store,
        };
        this.store.maybe_refresh_stale();
        this.spawn_sync_loop(cx);
        this
    }

    fn spawn_sync_loop(&self, cx: &mut Context<Self>) {
        let store = self.store.clone();
        cx.spawn(async move |this, cx| {
            loop {
                smol::Timer::after(Duration::from_millis(VIEW_POLL_INTERVAL_MILLIS)).await;
                store.maybe_refresh_stale();
                let view_model = store.current_view_model();
                if this
                    .update(cx, |this, cx| this.sync_view_model(view_model, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    fn sync_view_model(&mut self, view_model: ViewModel, cx: &mut Context<Self>) {
        if self.view_model.serial != view_model.serial {
            self.view_model = view_model;
            cx.notify();
        }
    }
}

#[cfg(feature = "mirror")]
impl Render for CodexUsageTitlebarWidget {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let view_model = self.view_model.clone();

        h_flex()
            .gap(px(1.0))
            .child(Icon::new("ai_open_ai"))
            .child(Label::new(compact_titlebar_label(&view_model.snapshot)))
    }
}

#[cfg(feature = "mirror")]
pub fn run_plugin() -> Result<()> {
    std::env::set_current_dir(plugin_manifest_directory())?;
    let runtime_context = PluginRuntimeContext::new()?;
    run(|app| {
        let panel_store = runtime_context.store.clone();
        app.register_panel(PANEL_ID, move |cx: &mut Context<CodexUsagePanel>| {
            CodexUsagePanel::new(panel_store.clone(), cx)
        });

        let titlebar_store = runtime_context.store.clone();
        app.register_titlebar_widget(
            TITLEBAR_WIDGET_ID,
            move |cx: &mut Context<CodexUsageTitlebarWidget>| {
                CodexUsageTitlebarWidget::new(titlebar_store.clone(), cx)
            },
        );
    })
}

#[cfg(feature = "mirror")]
fn main() {
    if let Err(error) = run_plugin() {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}

#[cfg(not(feature = "mirror"))]
#[cfg_attr(test, allow(dead_code))]
fn main() {
    println!("{PLUGIN_ID} is built for the mirror plugin runtime");
}

#[derive(Debug, Clone)]
struct TokenResponse {
    access_token: String,
    refresh_token: String,
    account_id: Option<String>,
    expires_at: u64,
    last_refresh_at: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Shutdown;
    use std::net::TcpStream;
    use std::thread;

    #[test]
    fn oauth_listener_uses_ephemeral_port() {
        let listener = bind_oauth_listener().expect("listener should bind");
        let port = listener
            .local_addr()
            .expect("listener should have an address")
            .port();

        assert_ne!(port, 0);
        assert_ne!(port, REDIRECT_PORT);
        assert_eq!(
            oauth_redirect_uri(port),
            format!("http://localhost:{port}{REDIRECT_PATH}")
        );
    }

    #[test]
    fn oauth_callback_returns_authorization_code_for_bound_port() {
        let listener = bind_oauth_listener().expect("listener should bind");
        let expected_state = "expected-state";
        let authorization_code = "auth-code-123";
        let callback_address = listener
            .local_addr()
            .expect("listener should have an address");

        let sender = thread::spawn(move || -> Result<()> {
            let mut stream = TcpStream::connect(callback_address)
                .with_context(|| "failed to connect to the OAuth listener")?;
            std::io::Write::write_all(
                &mut stream,
                format!(
                    "GET /auth/callback?state={expected_state}&code={authorization_code} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
                )
                .as_bytes(),
            )
                .with_context(|| "failed to write the OAuth callback request")?;
            stream
                .shutdown(Shutdown::Write)
                .with_context(|| "failed to close the OAuth callback write half")?;
            Ok(())
        });

        let received_code =
            wait_for_authorization_code(listener, expected_state, Duration::from_secs(5))
                .expect("callback should complete successfully");

        sender
            .join()
            .expect("callback sender should not panic")
            .expect("callback sender should succeed");

        assert_eq!(received_code, authorization_code);
    }

    #[test]
    fn oauth_callback_rejects_state_mismatch() {
        let listener = bind_oauth_listener().expect("listener should bind");
        let callback_address = listener
            .local_addr()
            .expect("listener should have an address");

        let sender = thread::spawn(move || -> Result<()> {
            let mut stream = TcpStream::connect(callback_address)
                .with_context(|| "failed to connect to the OAuth listener")?;
            std::io::Write::write_all(
                &mut stream,
                b"GET /auth/callback?state=wrong-state&code=auth-code-123 HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
            )
                .with_context(|| "failed to write the OAuth callback request")?;
            stream
                .shutdown(Shutdown::Write)
                .with_context(|| "failed to close the OAuth callback write half")?;
            Ok(())
        });

        let error = wait_for_authorization_code(listener, "expected-state", Duration::from_secs(5))
            .expect_err("callback should fail");

        sender
            .join()
            .expect("callback sender should not panic")
            .expect("callback sender should succeed");

        assert!(error.to_string().contains("state mismatch"));
    }
}
