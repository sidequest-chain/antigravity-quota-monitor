#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine as _,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, State, Wry,
};
use tokio::sync::Mutex;
use windows_sys::Win32::Security::Credentials::{
    CredFree, CredReadW, CREDENTIALW, CRED_TYPE_GENERIC,
};

const USER_AGENT: &str = "antigravity/cli (os_type=windows; arch=amd64; auth_method=consumer)";
const API_BASE: &str = "https://daily-cloudcode-pa.googleapis.com";
const AGY_CRED_TARGET: &str = "gemini:antigravity";

struct TrayItems {
    toggle: MenuItem<Wry>,
    top: MenuItem<Wry>,
    quit: MenuItem<Wry>,
}

struct AppState {
    cached_token: Mutex<Option<(String, String, Instant)>>, // (access_token, email, expires_at)
    cached_project_id: Mutex<Option<String>>,
    cached_oauth_client: Mutex<Option<(String, String)>>, // Dynamically read from local CLI/IDE binary
    tray_items: Mutex<Option<TrayItems>>,
    http: reqwest::Client,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct StoredCred {
    token: Option<StoredOAuthToken>,
    auth_method: Option<String>,
    id_token: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct StoredOAuthToken {
    access_token: Option<String>,
    token_type: Option<String>,
    refresh_token: Option<String>,
    expiry: Option<String>,
}

#[derive(Deserialize, Debug)]
struct TokenExchangeResponse {
    access_token: String,
    id_token: Option<String>,
    expires_in: Option<u64>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct QuotaBucketView {
    pub id: String,
    pub group_key: String,
    pub group_name: String,
    pub group_description: String,
    pub display_name: String,
    pub window: String,
    pub remaining_fraction: f64,
    pub remaining_percent: f64,
    pub used_percent: f64,
    pub reset_time: Option<String>,
    pub reset_seconds: Option<i64>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ModelQuotaView {
    pub id: String,
    pub display_name: String,
    pub family_key: String,
    pub remaining_fraction: f64,
    pub remaining_percent: f64,
    pub used_percent: f64,
    pub reset_time: Option<String>,
    pub reset_seconds: Option<i64>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct UsageSnapshot {
    pub installed: bool,
    pub authenticated: bool,
    pub auth_source: String,
    pub email: String,
    pub tier_name: Option<String>,
    pub paid_tier_name: Option<String>,
    pub project_id: String,
    pub active_sessions: usize,
    pub buckets: Vec<QuotaBucketView>,
    pub models: Vec<ModelQuotaView>,
    pub updated_at: String,
}

fn to_wide(s: &str) -> Vec<u16> {
    let mut v: Vec<u16> = s.encode_utf16().collect();
    v.push(0);
    v
}

fn read_windows_credential(target: &str) -> Result<String, String> {
    let wide_target = to_wide(target);
    unsafe {
        let mut cred_ptr: *mut CREDENTIALW = std::ptr::null_mut();
        let ok = CredReadW(
            wide_target.as_ptr(),
            CRED_TYPE_GENERIC,
            0,
            &mut cred_ptr,
        );
        if ok == 0 || cred_ptr.is_null() {
            return Err("ERR_CRED_NOT_FOUND".to_string());
        }

        let cred = &*cred_ptr;
        let slice = std::slice::from_raw_parts(
            cred.CredentialBlob,
            cred.CredentialBlobSize as usize,
        );
        let text = String::from_utf8_lossy(slice).to_string();
        CredFree(cred_ptr as *const _);
        Ok(text)
    }
}

/// Locates installed Antigravity CLI (`agy.exe`) and IDE binaries on the user's machine.
fn find_antigravity_binaries() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
        let base = PathBuf::from(&local_app_data);
        let cli_bin = base.join("agy").join("bin").join("agy.exe");
        if cli_bin.is_file() {
            paths.push(cli_bin);
        }
        let ide_ls_1 = base
            .join("Programs")
            .join("Antigravity-IDE")
            .join("resources")
            .join("app")
            .join("extensions")
            .join("antigravity")
            .join("bin")
            .join("language_server_windows_x64.exe");
        if ide_ls_1.is_file() {
            paths.push(ide_ls_1);
        }
        let ide_ls_2 = base
            .join("Programs")
            .join("antigravity")
            .join("resources")
            .join("app")
            .join("extensions")
            .join("antigravity")
            .join("bin")
            .join("language_server_windows_x64.exe");
        if ide_ls_2.is_file() {
            paths.push(ide_ls_2);
        }
    }
    paths
}

fn is_antigravity_installed() -> bool {
    if !find_antigravity_binaries().is_empty() {
        return true;
    }
    if let Some(dir) = agy_dir() {
        if dir.is_dir() {
            return true;
        }
    }
    if let Ok(appdata) = std::env::var("APPDATA") {
        let ide_dir = PathBuf::from(appdata).join("Antigravity IDE");
        if ide_dir.is_dir() {
            return true;
        }
    }
    false
}

/// Dynamically extracts OAuth Client IDs and Secrets from the locally installed Antigravity CLI/IDE binary
/// so no Google credentials or prefixes are ever hardcoded in this application's source code or binary.
fn extract_oauth_clients_from_binary(bytes: &[u8]) -> Vec<(String, String)> {
    let suffix = b".apps.googleusercontent.com";
    let mut client_ids = Vec::new();

    let mut pos = 0;
    while pos + suffix.len() <= bytes.len() {
        if let Some(rel) = bytes[pos..].windows(suffix.len()).position(|w| w == suffix) {
            let match_idx = pos + rel;
            let mut start = match_idx;
            while start > 0 {
                let b = bytes[start - 1];
                if b.is_ascii_alphanumeric() || b == b'-' {
                    start -= 1;
                } else {
                    break;
                }
            }
            let candidate = &bytes[start..match_idx + suffix.len()];
            if let Ok(s) = std::str::from_utf8(candidate) {
                if let Some((num_part, rest)) = s.split_once('-') {
                    if num_part.len() >= 10
                        && num_part.chars().all(|c| c.is_ascii_digit())
                        && rest.len() > suffix.len() + 20
                    {
                        let cid = s.to_string();
                        if !client_ids.contains(&cid) {
                            client_ids.push(cid);
                        }
                    }
                }
            }
            pos = match_idx + suffix.len();
        } else {
            break;
        }
    }

    let secret_prefix = b"GOCSPX-";
    let mut secrets = Vec::new();
    let mut spos = 0;
    while spos + secret_prefix.len() + 28 <= bytes.len() {
        if let Some(rel) = bytes[spos..]
            .windows(secret_prefix.len())
            .position(|w| w == secret_prefix)
        {
            let match_idx = spos + rel;
            let end = (match_idx + 35).min(bytes.len());
            let slice = &bytes[match_idx..end];
            if slice.len() == 35
                && slice[7..]
                    .iter()
                    .all(|&b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            {
                if let Ok(s) = std::str::from_utf8(slice) {
                    let sec = s.to_string();
                    if !secrets.contains(&sec) {
                        secrets.push(sec);
                    }
                }
            }
            spos = match_idx + secret_prefix.len();
        } else {
            break;
        }
    }

    let mut pairs = Vec::new();
    for cid in &client_ids {
        for sec in &secrets {
            pairs.push((cid.clone(), sec.clone()));
        }
    }
    pairs
}

async fn refresh_with_local_oauth_client(
    state: &AppState,
    refresh_token: &str,
) -> Result<TokenExchangeResponse, String> {
    let cached_pair = {
        let guard = state.cached_oauth_client.lock().await;
        guard.clone()
    };

    if let Some((ref cid, ref sec)) = cached_pair {
        if let Ok(resp) = state
            .http
            .post("https://oauth2.googleapis.com/token")
            .form(&[
                ("client_id", cid.as_str()),
                ("client_secret", sec.as_str()),
                ("refresh_token", refresh_token),
                ("grant_type", "refresh_token"),
            ])
            .send()
            .await
        {
            if resp.status().is_success() {
                if let Ok(parsed) = resp.json::<TokenExchangeResponse>().await {
                    return Ok(parsed);
                }
            }
        }
    }

    for bin_path in find_antigravity_binaries() {
        let Ok(bytes) = fs::read(&bin_path) else {
            continue;
        };
        for (cid, sec) in extract_oauth_clients_from_binary(&bytes) {
            let resp = state
                .http
                .post("https://oauth2.googleapis.com/token")
                .form(&[
                    ("client_id", cid.as_str()),
                    ("client_secret", sec.as_str()),
                    ("refresh_token", refresh_token),
                    ("grant_type", "refresh_token"),
                ])
                .send()
                .await
                .map_err(|_| "ERR_TOKEN_REFRESH".to_string())?;

            if resp.status().is_success() {
                let parsed: TokenExchangeResponse = resp
                    .json()
                    .await
                    .map_err(|_| "ERR_INVALID_OAUTH".to_string())?;
                let mut guard = state.cached_oauth_client.lock().await;
                *guard = Some((cid, sec));
                return Ok(parsed);
            }
        }
    }

    Err("NOT_AUTHENTICATED".to_string())
}

/// Attempts to read an OAuth token from Antigravity IDE's `state.vscdb` (or `.backup`).
fn load_ide_vscdb_credential() -> Option<StoredCred> {
    let appdata = std::env::var("APPDATA").ok()?;
    let storage_dir = PathBuf::from(appdata)
        .join("Antigravity IDE")
        .join("User")
        .join("globalStorage");

    for filename in ["state.vscdb", "state.vscdb.backup"] {
        let db_path = storage_dir.join(filename);
        let Ok(bytes) = fs::read(&db_path) else {
            continue;
        };
        let key_marker = b"antigravityUnifiedStateSync.oauthToken";
        let mut pos = 0;
        while pos + key_marker.len() < bytes.len() {
            let Some(rel) = bytes[pos..]
                .windows(key_marker.len())
                .position(|w| w == key_marker)
            else {
                break;
            };
            let start = pos + rel + key_marker.len();
            pos = start;

            let b64_slice: Vec<u8> = bytes[start..]
                .iter()
                .copied()
                .take_while(|&b| {
                    b.is_ascii_alphanumeric() || b == b'+' || b == b'/' || b == b'='
                })
                .collect();

            if b64_slice.len() < 64 {
                continue;
            }
            let Ok(outer_decoded) = STANDARD.decode(&b64_slice) else {
                continue;
            };

            let sentinel = b"oauthTokenInfoSentinelKey";
            if let Some(s_rel) = outer_decoded
                .windows(sentinel.len())
                .position(|w| w == sentinel)
            {
                let after_sentinel = &outer_decoded[s_rel + sentinel.len()..];
                if let Some(b64_start) = after_sentinel
                    .iter()
                    .position(|&b| b == b'C' || b == b'E')
                {
                    let inner_b64: Vec<u8> = after_sentinel[b64_start..]
                        .iter()
                        .copied()
                        .take_while(|&b| {
                            b.is_ascii_alphanumeric() || b == b'+' || b == b'/' || b == b'='
                        })
                        .collect();
                    if let Ok(inner_bytes) = STANDARD.decode(&inner_b64) {
                        let text = String::from_utf8_lossy(&inner_bytes);
                        let access = extract_token_prefix(&text, "ya29.");
                        let refresh = extract_token_prefix(&text, "1//");
                        if refresh.is_some() || access.is_some() {
                            return Some(StoredCred {
                                token: Some(StoredOAuthToken {
                                    access_token: access,
                                    token_type: Some("Bearer".to_string()),
                                    refresh_token: refresh,
                                    expiry: None,
                                }),
                                auth_method: Some("consumer".to_string()),
                                id_token: None,
                            });
                        }
                    }
                }
            }
        }
    }
    None
}

fn extract_token_prefix(haystack: &str, prefix: &str) -> Option<String> {
    let idx = haystack.find(prefix)?;
    let token: String = haystack[idx..]
        .chars()
        .take_while(|&c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' || c == '/')
        .collect();
    if token.len() > prefix.len() + 10 {
        Some(token)
    } else {
        None
    }
}

/// Loads existing session strictly from official Antigravity CLI, IDE, or ACP stores.
fn load_stored_credential() -> Result<(StoredCred, String), String> {
    if let Ok(raw) = read_windows_credential(AGY_CRED_TARGET) {
        if let Ok(cred) = serde_json::from_str::<StoredCred>(&raw) {
            if cred
                .token
                .as_ref()
                .and_then(|t| t.refresh_token.as_ref().or(t.access_token.as_ref()))
                .is_some()
            {
                return Ok((cred, "Antigravity CLI".to_string()));
            }
        }
    }

    if let Some(cred) = load_ide_vscdb_credential() {
        return Ok((cred, "Antigravity IDE".to_string()));
    }

    if let Some(home) = dirs::home_dir() {
        let acp_path = home
            .join(".gemini")
            .join("antigravity-acp")
            .join("acp_token.json");
        if let Ok(raw) = fs::read_to_string(acp_path) {
            if let Ok(val) = serde_json::from_str::<Value>(&raw) {
                if let Some(rt) = val.get("refresh_token").and_then(|v| v.as_str()) {
                    return Ok((
                        StoredCred {
                            token: Some(StoredOAuthToken {
                                access_token: None,
                                token_type: Some("Bearer".to_string()),
                                refresh_token: Some(rt.to_string()),
                                expiry: None,
                            }),
                            auth_method: Some("consumer".to_string()),
                            id_token: None,
                        },
                        "Antigravity ACP".to_string(),
                    ));
                }
            }
        }
    }

    Err("NOT_AUTHENTICATED".to_string())
}

fn decode_jwt_email(id_token: &str) -> Option<String> {
    let parts: Vec<&str> = id_token.split('.').collect();
    if parts.len() < 2 {
        return None;
    }
    let bytes = URL_SAFE_NO_PAD.decode(parts[1]).ok()?;
    let val: Value = serde_json::from_slice(&bytes).ok()?;
    val.get("email")?.as_str().map(|s| s.to_string())
}

async fn fetch_userinfo_email(http: &reqwest::Client, access_token: &str) -> Option<String> {
    let resp = http
        .get("https://www.googleapis.com/oauth2/v2/userinfo")
        .header("Authorization", format!("Bearer {}", access_token))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let val: Value = resp.json().await.ok()?;
    val.get("email")?.as_str().map(|s| s.to_string())
}

fn agy_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".gemini").join("antigravity-cli"))
}

fn read_local_project_id() -> Option<String> {
    if let Some(dir) = agy_dir() {
        let p = dir.join("cache").join("default_project_id.txt");
        if let Ok(s) = fs::read_to_string(p) {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn count_active_sessions() -> usize {
    if let Some(dir) = agy_dir() {
        let presence = dir.join("presence");
        if let Ok(entries) = fs::read_dir(presence) {
            return entries
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.path()
                        .extension()
                        .and_then(|x| x.to_str())
                        .map(|x| x == "lock")
                        .unwrap_or(false)
                })
                .count();
        }
    }
    0
}

fn compute_reset_seconds(reset_iso: Option<&str>) -> Option<i64> {
    let iso = reset_iso?;
    let dt = DateTime::parse_from_rfc3339(iso).ok()?;
    let diff = dt.with_timezone(&Utc) - Utc::now();
    Some(diff.num_seconds().max(0))
}

async fn get_valid_access_token(
    state: &AppState,
    force_refresh: bool,
) -> Result<(String, String, String), String> {
    let (cred, auth_source) = load_stored_credential()?;

    let mut email = cred
        .id_token
        .as_deref()
        .and_then(decode_jwt_email)
        .unwrap_or_default();

    if !force_refresh {
        let guard = state.cached_token.lock().await;
        if let Some((ref tok, ref cached_email, expires_at)) = *guard {
            if Instant::now() < expires_at {
                let final_email = if !email.is_empty() {
                    email
                } else {
                    cached_email.clone()
                };
                return Ok((tok.clone(), final_email, auth_source));
            }
        }
    }

    let token_obj = cred
        .token
        .clone()
        .ok_or_else(|| "NOT_AUTHENTICATED".to_string())?;

    if !force_refresh {
        if let (Some(ref access), Some(ref exp_str)) = (&token_obj.access_token, &token_obj.expiry)
        {
            if let Ok(exp_dt) = DateTime::parse_from_rfc3339(exp_str) {
                let remaining = exp_dt.with_timezone(&Utc) - Utc::now();
                if remaining.num_seconds() > 120 {
                    if email.is_empty() {
                        email = fetch_userinfo_email(&state.http, access)
                            .await
                            .unwrap_or_else(|| auth_source.clone());
                    }
                    let dur = Duration::from_secs((remaining.num_seconds() - 60) as u64);
                    let mut guard = state.cached_token.lock().await;
                    *guard = Some((access.clone(), email.clone(), Instant::now() + dur));
                    return Ok((access.clone(), email, auth_source));
                }
            }
        }
    }

    let refresh_token = token_obj
        .refresh_token
        .ok_or_else(|| "NOT_AUTHENTICATED".to_string())?;

    let refreshed = refresh_with_local_oauth_client(state, &refresh_token).await?;

    if let Some(ref id_tok) = refreshed.id_token {
        if let Some(em) = decode_jwt_email(id_tok) {
            email = em;
        }
    }
    if email.is_empty() {
        email = fetch_userinfo_email(&state.http, &refreshed.access_token)
            .await
            .unwrap_or_else(|| auth_source.clone());
    }

    let ttl = refreshed.expires_in.unwrap_or(3500).saturating_sub(120);
    let mut guard = state.cached_token.lock().await;
    *guard = Some((
        refreshed.access_token.clone(),
        email.clone(),
        Instant::now() + Duration::from_secs(ttl),
    ));

    Ok((refreshed.access_token, email, auth_source))
}

async fn fetch_usage_internal(state: &AppState) -> Result<UsageSnapshot, String> {
    let installed = is_antigravity_installed();
    let token_res = get_valid_access_token(state, false).await;
    let (mut token, email, auth_source) = match token_res {
        Ok(triple) => triple,
        Err(e) if e == "NOT_AUTHENTICATED" => {
            return Ok(UsageSnapshot {
                installed,
                authenticated: false,
                auth_source: String::new(),
                email: String::new(),
                tier_name: None,
                paid_tier_name: None,
                project_id: String::new(),
                active_sessions: count_active_sessions(),
                buckets: Vec::new(),
                models: Vec::new(),
                updated_at: chrono::Local::now().format("%H:%M:%S").to_string(),
            });
        }
        Err(e) => return Err(e),
    };

    let assist_url = format!("{}/v1internal:loadCodeAssist", API_BASE);
    let quota_url = format!("{}/v1internal:retrieveUserQuotaSummary", API_BASE);
    let models_url = format!("{}/v1internal:fetchAvailableModels", API_BASE);

    let mut project_id = {
        let guard = state.cached_project_id.lock().await;
        guard.clone()
    }
    .or_else(read_local_project_id)
    .unwrap_or_default();

    let assist_body = if project_id.is_empty() {
        json!({ "metadata": { "ideType": "ANTIGRAVITY" } })
    } else {
        json!({
            "cloudaicompanionProject": project_id,
            "metadata": { "ideType": "ANTIGRAVITY" }
        })
    };

    let mut assist_resp = state
        .http
        .post(&assist_url)
        .header("Authorization", format!("Bearer {}", token))
        .header("User-Agent", USER_AGENT)
        .json(&assist_body)
        .send()
        .await
        .map_err(|_| "ERR_NETWORK".to_string())?;

    if assist_resp.status().as_u16() == 401 {
        let (new_tok, _, _) = get_valid_access_token(state, true).await?;
        token = new_tok;
        assist_resp = state
            .http
            .post(&assist_url)
            .header("Authorization", format!("Bearer {}", token))
            .header("User-Agent", USER_AGENT)
            .json(&assist_body)
            .send()
            .await
            .map_err(|_| "ERR_NETWORK".to_string())?;
    }

    let assist_val: Value = assist_resp.json().await.unwrap_or(json!({}));

    if project_id.is_empty() {
        project_id = assist_val
            .get("cloudaicompanionProject")
            .and_then(|v| v.as_str())
            .unwrap_or("aicode-consumers")
            .to_string();
    }
    {
        let mut guard = state.cached_project_id.lock().await;
        *guard = Some(project_id.clone());
    }

    let (quota_res, models_res) = tokio::join!(
        state
            .http
            .post(&quota_url)
            .header("Authorization", format!("Bearer {}", token))
            .header("User-Agent", USER_AGENT)
            .json(&json!({ "project": project_id }))
            .send(),
        state
            .http
            .post(&models_url)
            .header("Authorization", format!("Bearer {}", token))
            .header("User-Agent", USER_AGENT)
            .json(&json!({ "project": project_id }))
            .send()
    );

    let quota_val: Value = match quota_res {
        Ok(r) => r.json().await.unwrap_or(json!({})),
        Err(_) => json!({}),
    };

    let models_val: Value = match models_res {
        Ok(r) => r.json().await.unwrap_or(json!({})),
        Err(_) => json!({}),
    };

    let tier_name = assist_val
        .pointer("/currentTier/name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let paid_tier_name = assist_val
        .pointer("/paidTier/name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // 1. Parse Quota Summary Groups & Buckets
    let mut buckets = Vec::new();
    if let Some(groups) = quota_val.get("groups").and_then(|g| g.as_array()) {
        for grp in groups {
            let raw_grp_name = grp
                .get("displayName")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let group_key = match raw_grp_name {
                "Gemini Models" => "gemini".to_string(),
                "Claude and GPT models" => "claude_gpt".to_string(),
                _ => "other".to_string(),
            };
            let grp_desc = grp
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            if let Some(b_arr) = grp.get("buckets").and_then(|b| b.as_array()) {
                let mut sorted_b: Vec<&Value> = b_arr.iter().collect();
                sorted_b.sort_by_key(|b| {
                    match b.get("window").and_then(|w| w.as_str()).unwrap_or("") {
                        "5h" => 0,
                        "weekly" => 1,
                        _ => 2,
                    }
                });

                for b in sorted_b {
                    let id = b
                        .get("bucketId")
                        .and_then(|v| v.as_str())
                        .unwrap_or("bucket")
                        .to_string();
                    let window = b
                        .get("window")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let disp = b
                        .get("displayName")
                        .and_then(|v| v.as_str())
                        .unwrap_or(&id)
                        .to_string();
                    let rem = b
                        .get("remainingFraction")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(1.0)
                        .clamp(0.0, 1.0);
                    let reset_time = b
                        .get("resetTime")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    let reset_seconds = compute_reset_seconds(reset_time.as_deref());
                    buckets.push(QuotaBucketView {
                        id,
                        group_key: group_key.clone(),
                        group_name: raw_grp_name.to_string(),
                        group_description: grp_desc.clone(),
                        display_name: disp,
                        window,
                        remaining_fraction: rem,
                        remaining_percent: (rem * 1000.0).round() / 10.0,
                        used_percent: ((1.0 - rem) * 1000.0).round() / 10.0,
                        reset_time,
                        reset_seconds,
                    });
                }
            }
        }
    }

    // 2. Parse active recommended models from agentModelSorts
    let mut allowed_ids: Vec<String> = Vec::new();
    if let Some(groups) = models_val
        .pointer("/agentModelSorts/groups")
        .and_then(|g| g.as_array())
    {
        for g in groups {
            if let Some(ids) = g.get("modelIds").and_then(|m| m.as_array()) {
                for id_val in ids {
                    if let Some(id_str) = id_val.as_str() {
                        if !id_str.starts_with("gpt-oss") {
                            allowed_ids.push(id_str.to_string());
                        }
                    }
                }
            }
        }
    }

    let allowed_set: HashSet<String> = allowed_ids.iter().cloned().collect();
    let mut models = Vec::new();

    if let Some(m_obj) = models_val.get("models").and_then(|m| m.as_object()) {
        let keys_to_iter: Vec<String> = if !allowed_ids.is_empty() {
            allowed_ids
        } else {
            m_obj.keys().cloned().collect()
        };

        for model_id in keys_to_iter {
            let Some(m_val) = m_obj.get(&model_id) else {
                continue;
            };
            if !allowed_set.is_empty() && !allowed_set.contains(&model_id) {
                continue;
            }
            let Some(display_name) = m_val
                .get("displayName")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
            else {
                continue;
            };

            let Some(q_info) = m_val.get("quotaInfo") else {
                continue;
            };
            let rem = q_info
                .get("remainingFraction")
                .and_then(|v| v.as_f64())
                .unwrap_or(1.0)
                .clamp(0.0, 1.0);
            let reset_time = q_info
                .get("resetTime")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let reset_seconds = compute_reset_seconds(reset_time.as_deref());

            let family_key = if model_id.starts_with("gemini-4-argon") {
                "gemini_4_argon".to_string()
            } else if model_id.starts_with("gemini-3.8-flash") {
                "gemini_3_8_flash".to_string()
            } else if model_id.starts_with("claude-opus") {
                "claude_opus".to_string()
            } else if model_id.starts_with("claude-sonnet") {
                "claude_sonnet".to_string()
            } else {
                "other".to_string()
            };

            models.push(ModelQuotaView {
                id: model_id,
                display_name,
                family_key,
                remaining_fraction: rem,
                remaining_percent: (rem * 1000.0).round() / 10.0,
                used_percent: ((1.0 - rem) * 1000.0).round() / 10.0,
                reset_time,
                reset_seconds,
            });
        }
    }

    let now_local = chrono::Local::now().format("%H:%M:%S").to_string();

    Ok(UsageSnapshot {
        installed,
        authenticated: true,
        auth_source,
        email,
        tier_name,
        paid_tier_name,
        project_id,
        active_sessions: count_active_sessions(),
        buckets,
        models,
        updated_at: now_local,
    })
}

#[tauri::command]
async fn get_usage(state: State<'_, Arc<AppState>>) -> Result<UsageSnapshot, String> {
    fetch_usage_internal(&state).await
}

#[tauri::command]
async fn set_tray_labels(
    state: State<'_, Arc<AppState>>,
    toggle: String,
    top: String,
    quit: String,
) -> Result<(), String> {
    let guard = state.tray_items.lock().await;
    if let Some(ref items) = *guard {
        let _ = items.toggle.set_text(toggle);
        let _ = items.top.set_text(top);
        let _ = items.quit.set_text(quit);
    }
    Ok(())
}

#[tauri::command]
async fn launch_antigravity(state: State<'_, Arc<AppState>>) -> Result<UsageSnapshot, String> {
    {
        let mut guard = state.cached_token.lock().await;
        *guard = None;
    }

    if load_stored_credential().is_err() {
        if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
            let base = PathBuf::from(&local_app_data);
            let cli_exe = base.join("agy").join("bin").join("agy.exe");
            let ide_exe = base
                .join("Programs")
                .join("Antigravity-IDE")
                .join("Antigravity IDE.exe");

            if cli_exe.is_file() {
                let _ = Command::new("cmd.exe")
                    .args(["/c", "start", "", &cli_exe.to_string_lossy()])
                    .spawn();
            } else if ide_exe.is_file() {
                let _ = Command::new(&ide_exe).spawn();
            }
        }
    }

    fetch_usage_internal(&state).await
}

fn main() {
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .build()
        .expect("HTTP_CLIENT_INIT_FAILED");

    let app_state = Arc::new(AppState {
        cached_token: Mutex::new(None),
        cached_project_id: Mutex::new(None),
        cached_oauth_client: Mutex::new(None),
        tray_items: Mutex::new(None),
        http,
    });

    let state_for_setup = Arc::clone(&app_state);

    tauri::Builder::default()
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            get_usage,
            launch_antigravity,
            set_tray_labels
        ])
        .setup(move |app| {
            let show_i = MenuItem::with_id(app, "toggle", "...", true, None::<&str>)?;
            let top_i = MenuItem::with_id(app, "top", "...", true, None::<&str>)?;
            let quit_i = MenuItem::with_id(app, "quit", "...", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_i, &top_i, &quit_i])?;

            if let Ok(mut guard) = state_for_setup.tray_items.try_lock() {
                *guard = Some(TrayItems {
                    toggle: show_i.clone(),
                    top: top_i.clone(),
                    quit: quit_i.clone(),
                });
            }

            let icon = app.default_window_icon().cloned().unwrap();

            let _tray = TrayIconBuilder::new()
                .icon(icon)
                .tooltip("Antigravity Quota Monitor")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "toggle" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let vis = w.is_visible().unwrap_or(true);
                            if vis {
                                let _ = w.hide();
                            } else {
                                let _ = w.show();
                                let _ = w.set_focus();
                            }
                        }
                    }
                    "top" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let cur = w.is_always_on_top().unwrap_or(true);
                            let _ = w.set_always_on_top(!cur);
                        }
                    }
                    "quit" => {
                        app.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(w) = app.get_webview_window("main") {
                            let vis = w.is_visible().unwrap_or(true);
                            if vis {
                                let _ = w.hide();
                            } else {
                                let _ = w.show();
                                let _ = w.set_focus();
                            }
                        }
                    }
                })
                .build(app)?;

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("TAURI_RUNTIME_ERROR");
}
