use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokvane_core::sync_crypto::generate_kdf_salt_hex;

use crate::credentials::CredentialManager;
use crate::error::{CollectorError, CollectorResult};
use crate::sync_client::{
    normalize_worker_url, set_sync_credentials, SyncBoxFuture, SYNC_BEARER_TOKEN_KEY,
    SYNC_CF_API_TOKEN_KEY, SYNC_E2EE_PASSPHRASE_KEY,
};

pub const DEFAULT_WORKER_SCRIPT_NAME: &str = "tokenvane-sync-worker";
pub const WORKER_COMPATIBILITY_DATE: &str = "2026-03-01";
const CF_API_BASE: &str = "https://api.cloudflare.com/client/v4";

pub const EMBEDDED_WORKER_INDEX_JS: &str =
    include_str!("../../../../services/sync-worker/bundle/index.js");
pub const EMBEDDED_WORKER_HUB_JS: &str =
    include_str!("../../../../services/sync-worker/bundle/hub.js");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProvisionMode {
    WranglerCli,
    CloudflareApi,
}

impl ProvisionMode {
    pub fn parse(raw: &str) -> CollectorResult<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "wrangler_cli" | "wrangler" => Ok(Self::WranglerCli),
            "cloudflare_api" | "api_token" => Ok(Self::CloudflareApi),
            other => Err(CollectorError::Compliance(format!(
                "部署方式 '{other}' 无效，请选择 wrangler_cli 或 cloudflare_api"
            ))),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::WranglerCli => "wrangler_cli",
            Self::CloudflareApi => "cloudflare_api",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudSyncProvisionRequest {
    pub mode: String,
    pub worker_name: Option<String>,
    pub bearer_token: Option<String>,
    pub e2ee_enabled: bool,
    pub e2ee_passphrase: Option<String>,
    pub cf_api_token: Option<String>,
    pub cf_account_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProvisionedWorkerArtifacts {
    pub mode: String,
    pub worker_name: String,
    pub worker_url: String,
    pub bearer_token: String,
    pub e2ee_passphrase: Option<String>,
}

/// Coarse-grained provisioning stage, reported to the UI while one-click setup runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProvisionStep {
    /// Preparing the temporary Worker bundle / resolving secrets.
    Preparing,
    /// `wrangler whoami` (first run may download Wrangler via npx).
    CheckingAuth,
    /// Waiting for the browser OAuth flow of `wrangler login`.
    BrowserLogin,
    /// Resolving the Cloudflare account and workers.dev subdomain via REST API.
    ResolvingAccount,
    /// Uploading the Worker script (Wrangler deploy or REST upload).
    Deploying,
    /// Writing the SYNC_TOKEN secret / enabling the workers.dev route.
    WritingSecret,
    /// Persisting credentials and sync config locally.
    Saving,
    /// Running the first push/pull against the new Worker.
    InitialSync,
}

/// Progress sink for [`provision_worker_with_progress`].
pub type ProvisionProgress<'a> = &'a (dyn Fn(ProvisionStep) + Send + Sync);

fn no_progress(_: ProvisionStep) {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WranglerCmdOutput {
    pub success: bool,
    pub status_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// Transport abstraction for Cloudflare v4 REST API requests (production reqwest or mock).
pub trait CloudflareApiTransport: Send + Sync {
    fn request<'a>(
        &'a self,
        method: &'a str,
        url: &'a str,
        bearer_token: &'a str,
        content_type: Option<&'a str>,
        body: Option<Vec<u8>>,
    ) -> SyncBoxFuture<'a, CollectorResult<(u16, String)>>;
}

/// Subprocess runner abstraction for Wrangler CLI commands (production OS process or mock).
pub trait WranglerCliRunner: Send + Sync {
    fn run_wrangler<'a>(
        &'a self,
        cwd: &'a Path,
        args: &'a [&'a str],
        stdin_data: Option<&'a [u8]>,
        interactive_login: bool,
        timeout: Duration,
    ) -> SyncBoxFuture<'a, CollectorResult<WranglerCmdOutput>>;
}

#[derive(Clone)]
pub struct ReqwestCfApiTransport {
    client: reqwest::Client,
}

impl ReqwestCfApiTransport {
    pub fn new() -> CollectorResult<Self> {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(45))
            .user_agent(concat!("TokenVane/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self { client })
    }
}

impl CloudflareApiTransport for ReqwestCfApiTransport {
    fn request<'a>(
        &'a self,
        method: &'a str,
        url: &'a str,
        bearer_token: &'a str,
        content_type: Option<&'a str>,
        body: Option<Vec<u8>>,
    ) -> SyncBoxFuture<'a, CollectorResult<(u16, String)>> {
        Box::pin(async move {
            let parsed_method = reqwest::Method::from_bytes(method.as_bytes()).map_err(|e| {
                CollectorError::Compliance(format!("无效 HTTP 方法 '{method}': {e}"))
            })?;
            let mut req = self.client.request(parsed_method, url).bearer_auth(bearer_token);
            if let Some(ct) = content_type {
                req = req.header("Content-Type", ct);
            }
            if let Some(bytes) = body {
                req = req.body(bytes);
            }
            let resp = req.send().await?;
            let status = resp.status().as_u16();
            let text = resp.text().await.unwrap_or_default();
            Ok((status, text))
        })
    }
}

#[derive(Default, Clone, Copy)]
pub struct SystemWranglerRunner;

impl WranglerCliRunner for SystemWranglerRunner {
    fn run_wrangler<'a>(
        &'a self,
        cwd: &'a Path,
        args: &'a [&'a str],
        stdin_data: Option<&'a [u8]>,
        interactive_login: bool,
        timeout: Duration,
    ) -> SyncBoxFuture<'a, CollectorResult<WranglerCmdOutput>> {
        let cwd_buf = cwd.to_path_buf();
        let owned_args: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
        let owned_stdin = stdin_data.map(|b| b.to_vec());

        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                run_wrangler_blocking(
                    &cwd_buf,
                    &owned_args,
                    owned_stdin.as_deref(),
                    interactive_login,
                    timeout,
                )
            })
            .await
            .map_err(|e| CollectorError::Compliance(format!("执行 Wrangler 任务异常: {e}")))?
        })
    }
}

#[derive(Debug, Clone)]
struct WranglerInvocation {
    program: PathBuf,
    prefix_args: Vec<&'static str>,
}

fn find_executable_in_paths(names: &[&str]) -> Option<PathBuf> {
    let mut search_dirs: Vec<PathBuf> = Vec::new();
    if let Some(path_var) = std::env::var_os("PATH") {
        search_dirs.extend(std::env::split_paths(&path_var));
    }
    #[cfg(windows)]
    if let Some(appdata) = std::env::var_os("APPDATA") {
        search_dirs.push(PathBuf::from(appdata).join("npm"));
    }

    for dir in search_dirs {
        for name in names {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn resolve_wrangler_invocation() -> CollectorResult<WranglerInvocation> {
    #[cfg(windows)]
    let wrangler_names = ["wrangler.cmd", "wrangler.exe", "wrangler"];
    #[cfg(not(windows))]
    let wrangler_names = ["wrangler"];

    if let Some(program) = find_executable_in_paths(&wrangler_names) {
        return Ok(WranglerInvocation {
            program,
            prefix_args: Vec::new(),
        });
    }

    #[cfg(windows)]
    let npx_names = ["npx.cmd", "npx.exe", "npx"];
    #[cfg(not(windows))]
    let npx_names = ["npx"];

    if let Some(program) = find_executable_in_paths(&npx_names) {
        return Ok(WranglerInvocation {
            program,
            prefix_args: vec!["--yes", "wrangler@latest"],
        });
    }

    Err(CollectorError::Compliance(
        "请先安装 Node.js 环境（包含 npx / wrangler），或切换至 Cloudflare API 令牌模式直接部署"
            .to_string(),
    ))
}

/// Build the Wrangler subprocess command.
///
/// The resolved program (e.g. `C:\Program Files\nodejs\npx.cmd`) is spawned
/// directly. Rust's standard library (>= 1.77) wraps `.cmd` / `.bat` targets in
/// `cmd.exe` with correct quoting and rejects unsafe arguments. Wrapping it
/// manually as `cmd.exe /d /s /c <program> ...` breaks whenever the program path
/// contains spaces: `/s` strips the quotes around the path and cmd then tries to
/// run `C:\Program`.
fn build_wrangler_command(
    inv: &WranglerInvocation,
    cwd: &Path,
    args: &[String],
    stdin_data: Option<&[u8]>,
    interactive_login: bool,
) -> std::process::Command {
    let mut cmd = std::process::Command::new(&inv.program);

    for prefix in &inv.prefix_args {
        cmd.arg(prefix);
    }
    for arg in args {
        cmd.arg(arg);
    }

    cmd.current_dir(cwd)
        .env("WRANGLER_SEND_METRICS", "false")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if interactive_login {
        // Wrangler refuses to open the browser OAuth flow when CI=true is present.
        cmd.env_remove("CI");
        cmd.stdin(Stdio::null());
    } else {
        cmd.env("CI", "true");
        if stdin_data.is_some() {
            cmd.stdin(Stdio::piped());
        } else {
            cmd.stdin(Stdio::null());
        }
    }

    crate::cli_process::configure_hidden_cli_command(&mut cmd);
    cmd
}

fn run_wrangler_blocking(
    cwd: &Path,
    args: &[String],
    stdin_data: Option<&[u8]>,
    interactive_login: bool,
    timeout: Duration,
) -> CollectorResult<WranglerCmdOutput> {
    let inv = resolve_wrangler_invocation()?;
    let mut cmd = build_wrangler_command(&inv, cwd, args, stdin_data, interactive_login);

    let mut child = cmd
        .spawn()
        .map_err(|e| CollectorError::Compliance(format!("启动 Wrangler CLI 失败: {e}")))?;
    // Never bind an interactive `wrangler login` browser launch to a kill-on-close job object.
    let job_guard = (!interactive_login).then(|| crate::cli_process::ChildProcessGuard::attach(&child));

    if let Some(bytes) = stdin_data {
        if let Some(mut stdin_pipe) = child.stdin.take() {
            stdin_pipe.write_all(bytes).map_err(|e| {
                CollectorError::Compliance(format!("向 Wrangler 写入密钥失败: {e}"))
            })?;
            let _ = stdin_pipe.flush();
        }
    }

    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();

    let (wait_res, out_bytes, err_bytes) = std::thread::scope(|scope| {
        let stdout_handle = scope.spawn(move || -> Vec<u8> {
            let mut buf = Vec::new();
            if let Some(mut reader) = stdout_pipe {
                let _ = reader.read_to_end(&mut buf);
            }
            buf
        });
        let stderr_handle = scope.spawn(move || -> Vec<u8> {
            let mut buf = Vec::new();
            if let Some(mut reader) = stderr_pipe {
                let _ = reader.read_to_end(&mut buf);
            }
            buf
        });

        let deadline = std::time::Instant::now() + timeout;
        let status = loop {
            match child.try_wait() {
                Ok(Some(st)) => break Ok(st),
                Ok(None) => {
                    if std::time::Instant::now() >= deadline {
                        if let Some(ref guard) = job_guard {
                            guard.terminate_tree();
                        }
                        let _ = child.kill();
                        let _ = child.wait();
                        break Err(CollectorError::Compliance(format!(
                            "Wrangler 执行超时（>{}s），请确认网络连通并重试",
                            timeout.as_secs()
                        )));
                    }
                    std::thread::sleep(Duration::from_millis(40));
                }
                Err(e) => {
                    if let Some(ref guard) = job_guard {
                        guard.terminate_tree();
                    }
                    let _ = child.kill();
                    let _ = child.wait();
                    break Err(CollectorError::Compliance(format!(
                        "等待 Wrangler 退出失败: {e}"
                    )));
                }
            }
        };

        let out_b = stdout_handle.join().unwrap_or_default();
        let err_b = stderr_handle.join().unwrap_or_default();
        (status, out_b, err_b)
    });

    drop(job_guard);

    let status = wait_res?;
    Ok(WranglerCmdOutput {
        success: status.success(),
        status_code: status.code(),
        stdout: String::from_utf8_lossy(&out_bytes).into_owned(),
        stderr: String::from_utf8_lossy(&err_bytes).into_owned(),
    })
}

pub fn normalize_worker_script_name(raw: Option<&str>) -> CollectorResult<String> {
    let Some(trimmed) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(DEFAULT_WORKER_SCRIPT_NAME.to_string());
    };
    let lower = trimmed.to_ascii_lowercase();
    if lower.len() > 63 {
        return Err(CollectorError::Compliance(
            "Worker 名称长度需控制在 63 个字符以内".to_string(),
        ));
    }
    let bytes = lower.as_bytes();
    let valid_edges = bytes.first().is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && bytes.last().is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit());
    let valid_chars = bytes
        .iter()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-');
    if !valid_edges || !valid_chars {
        return Err(CollectorError::Compliance(
            "Worker 名称仅允许小写字母、数字与连字符，且须以字母或数字开头和结尾".to_string(),
        ));
    }
    Ok(lower)
}

pub fn generate_sync_secret_hex(byte_len: usize) -> CollectorResult<String> {
    if byte_len == 0 {
        return Err(CollectorError::Compliance(
            "密钥字节长度必须大于 0".to_string(),
        ));
    }
    let target_hex_len = byte_len.saturating_mul(2);
    let mut out = String::with_capacity(target_hex_len + 32);
    while out.len() < target_hex_len {
        out.push_str(&generate_kdf_salt_hex()?);
    }
    out.truncate(target_hex_len);
    Ok(out)
}

fn resolve_sync_secrets(
    credentials: &dyn CredentialManager,
    req: &CloudSyncProvisionRequest,
) -> CollectorResult<(String, Option<String>)> {
    let bearer = if let Some(input) = req
        .bearer_token
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        input.to_string()
    } else if let Some(stored) = credentials
        .get_api_key(SYNC_BEARER_TOKEN_KEY)?
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        stored
    } else {
        generate_sync_secret_hex(24)?
    };

    let passphrase = if req.e2ee_enabled {
        if let Some(input) = req
            .e2ee_passphrase
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(input.to_string())
        } else if let Some(stored) = credentials
            .get_api_key(SYNC_E2EE_PASSPHRASE_KEY)?
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
        {
            Some(stored)
        } else {
            Some(generate_sync_secret_hex(16)?)
        }
    } else {
        None
    };

    Ok((bearer, passphrase))
}

fn strip_ansi_sequences(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' && chars.peek() == Some(&'[') {
            chars.next();
            for next in chars.by_ref() {
                if ('@'..='~').contains(&next) {
                    break;
                }
            }
            continue;
        }
        out.push(ch);
    }
    out
}

pub fn extract_deployed_workers_dev_url(output: &str, worker_name: &str) -> CollectorResult<String> {
    let cleaned = strip_ansi_sequences(output);
    let expected_prefix = format!("{}.", worker_name.to_ascii_lowercase());
    let mut preferred: Option<String> = None;
    let mut fallback: Option<String> = None;

    let mut search_from = 0;
    while let Some(rel_idx) = cleaned[search_from..].find("https://") {
        let abs_idx = search_from + rel_idx;
        let rest = &cleaned[abs_idx..];
        let end = rest
            .find(|c: char| {
                !(c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | ':' | '/'))
            })
            .unwrap_or(rest.len());
        let raw_candidate = rest[..end].trim_end_matches(['.', '/', ',', ';', ')', ']', '"', '\'']);
        if let Ok(parsed) = reqwest::Url::parse(raw_candidate) {
            if let Some(host) = parsed.host_str() {
                let lower_host = host.to_ascii_lowercase();
                if lower_host.ends_with(".workers.dev") {
                    let normalized = normalize_worker_url(Some(raw_candidate))?;
                    if lower_host.starts_with(&expected_prefix) {
                        preferred = Some(normalized);
                        break;
                    }
                    if fallback.is_none() {
                        fallback = Some(normalized);
                    }
                }
            }
        }
        search_from = abs_idx + "https://".len();
    }

    preferred.or(fallback).ok_or_else(|| {
        CollectorError::Compliance(
            "Wrangler 部署已完成但未能从输出识别 workers.dev 服务地址，请检查 Wrangler 日志".to_string(),
        )
    })
}

pub fn build_worker_multipart_body(
    boundary: &str,
    metadata_json: &str,
    index_js: &str,
    hub_js: &str,
) -> Vec<u8> {
    let mut body = Vec::with_capacity(
        metadata_json.len() + index_js.len() + hub_js.len() + boundary.len() * 4 + 256,
    );

    let push_part = |buf: &mut Vec<u8>, header: &str, content: &str| {
        buf.extend_from_slice(b"--");
        buf.extend_from_slice(boundary.as_bytes());
        buf.extend_from_slice(b"\r\n");
        buf.extend_from_slice(header.as_bytes());
        buf.extend_from_slice(b"\r\n\r\n");
        buf.extend_from_slice(content.as_bytes());
        buf.extend_from_slice(b"\r\n");
    };

    push_part(
        &mut body,
        "Content-Disposition: form-data; name=\"metadata\"\r\nContent-Type: application/json",
        metadata_json,
    );
    push_part(
        &mut body,
        "Content-Disposition: form-data; name=\"index.js\"; filename=\"index.js\"\r\nContent-Type: application/javascript+module",
        index_js,
    );
    push_part(
        &mut body,
        "Content-Disposition: form-data; name=\"hub.js\"; filename=\"hub.js\"\r\nContent-Type: application/javascript+module",
        hub_js,
    );

    body.extend_from_slice(b"--");
    body.extend_from_slice(boundary.as_bytes());
    body.extend_from_slice(b"--\r\n");
    body
}

fn build_worker_metadata_json(bearer_token: &str, include_migration: bool) -> CollectorResult<String> {
    let mut metadata = json!({
        "main_module": "index.js",
        "compatibility_date": WORKER_COMPATIBILITY_DATE,
        "bindings": [
            {
                "type": "durable_object_namespace",
                "name": "SYNC_HUB",
                "class_name": "SyncHubDO"
            },
            {
                "type": "secret_text",
                "name": "SYNC_TOKEN",
                "text": bearer_token
            }
        ]
    });

    if include_migration {
        if let Some(obj) = metadata.as_object_mut() {
            obj.insert(
                "migrations".to_string(),
                json!({
                    "new_tag": "v1",
                    "new_sqlite_classes": ["SyncHubDO"]
                }),
            );
        }
    }

    Ok(serde_json::to_string(&metadata)?)
}

fn parse_cf_v4_error(status: u16, body: &str) -> String {
    if let Ok(val) = serde_json::from_str::<Value>(body) {
        if let Some(first) = val
            .get("errors")
            .and_then(Value::as_array)
            .and_then(|arr| arr.first())
        {
            let code = first.get("code").and_then(Value::as_i64);
            let msg = first
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("Cloudflare API 请求异常");
            return match code {
                Some(c) => format!("HTTP {status}: [{c}] {msg}"),
                None => format!("HTTP {status}: {msg}"),
            };
        }
    }
    let snippet: String = body.trim().chars().take(200).collect();
    if snippet.is_empty() {
        format!("HTTP {status}")
    } else {
        format!("HTTP {status}: {snippet}")
    }
}

fn parse_cf_success_json(status: u16, body: &str) -> CollectorResult<Value> {
    if !(200..300).contains(&status) {
        return Err(CollectorError::Compliance(format!(
            "Cloudflare API 请求被拒绝 ({})",
            parse_cf_v4_error(status, body)
        )));
    }
    let val: Value = serde_json::from_str(body)?;
    if val.get("success").and_then(Value::as_bool) == Some(false) {
        return Err(CollectorError::Compliance(format!(
            "Cloudflare API 返回失败 ({})",
            parse_cf_v4_error(status, body)
        )));
    }
    Ok(val)
}

fn is_migration_precondition_error(status: u16, body: &str) -> bool {
    if (200..300).contains(&status) {
        return false;
    }
    let lower = body.to_ascii_lowercase();
    body.contains("10079") || lower.contains("migration") || lower.contains("already")
}

async fn provision_via_cloudflare_api(
    credentials: &dyn CredentialManager,
    transport: &dyn CloudflareApiTransport,
    req: &CloudSyncProvisionRequest,
    worker_name: &str,
    bearer_token: &str,
    progress: ProvisionProgress<'_>,
) -> CollectorResult<String> {
    progress(ProvisionStep::ResolvingAccount);
    let cf_token = if let Some(input) = req
        .cf_api_token
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        credentials.set_api_key(SYNC_CF_API_TOKEN_KEY, input)?;
        input.to_string()
    } else if let Some(stored) = credentials
        .get_api_key(SYNC_CF_API_TOKEN_KEY)?
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        stored
    } else {
        return Err(CollectorError::Compliance(
            "请填写具备 Workers 编辑权限的 Cloudflare API 令牌".to_string(),
        ));
    };

    // A1: Resolve Account ID
    let account_id = if let Some(explicit) = req
        .cf_account_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if !explicit
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(CollectorError::Compliance(
                "Cloudflare Account ID 仅支持字母、数字、下划线与连字符".to_string(),
            ));
        }
        explicit.to_string()
    } else {
        let (status, body) = transport
            .request(
                "GET",
                &format!("{CF_API_BASE}/accounts?per_page=5"),
                &cf_token,
                None,
                None,
            )
            .await?;
        let json = parse_cf_success_json(status, &body)?;
        json.get("result")
            .and_then(Value::as_array)
            .and_then(|arr| arr.first())
            .and_then(|acc| acc.get("id"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
            .ok_or_else(|| {
                CollectorError::Compliance(
                    "请确认 Cloudflare API 令牌已授予 Account:Read 与 Workers Scripts:Edit 权限，或手动填入 Account ID".to_string(),
                )
            })?
    };

    // A2: Discover or initialize workers.dev account subdomain
    let subdomain_url = format!("{CF_API_BASE}/accounts/{account_id}/workers/subdomain");
    let (sub_status, sub_body) = transport
        .request("GET", &subdomain_url, &cf_token, None, None)
        .await?;

    let existing_subdomain = if (200..300).contains(&sub_status) {
        parse_cf_success_json(sub_status, &sub_body)?
            .get("result")
            .and_then(|r| r.get("subdomain"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
    } else if sub_status == 404 {
        None
    } else {
        return Err(CollectorError::Compliance(format!(
            "读取 workers.dev 子域名失败 ({})",
            parse_cf_v4_error(sub_status, &sub_body)
        )));
    };

    let subdomain = match existing_subdomain {
        Some(sub) => sub,
        None => {
            let suffix: String = account_id
                .chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .take(8)
                .collect::<String>()
                .to_ascii_lowercase();
            let candidate = format!("tokenvane-{suffix}");
            let put_body = serde_json::to_vec(&json!({ "subdomain": candidate }))?;
            let (put_status, put_resp) = transport
                .request(
                    "PUT",
                    &subdomain_url,
                    &cf_token,
                    Some("application/json"),
                    Some(put_body),
                )
                .await?;
            if (200..300).contains(&put_status) {
                let put_json = parse_cf_success_json(put_status, &put_resp)?;
                put_json
                    .get("result")
                    .and_then(|r| r.get("subdomain"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(ToString::to_string)
                    .unwrap_or(candidate)
            } else {
                return Err(CollectorError::Compliance(
                    "请先在 Cloudflare 控制台 Workers & Pages 中激活一次默认 workers.dev 子域名后重试"
                        .to_string(),
                ));
            }
        }
    };

    // A3: Probe existing script migration state so repeated setup stays idempotent
    let settings_url =
        format!("{CF_API_BASE}/accounts/{account_id}/workers/scripts/{worker_name}/settings");
    let (settings_status, settings_body) = transport
        .request("GET", &settings_url, &cf_token, None, None)
        .await?;
    let already_migrated = if settings_status == 200 {
        serde_json::from_str::<Value>(&settings_body)
            .ok()
            .and_then(|v| {
                v.get("result")
                    .and_then(|r| r.get("migration_tag"))
                    .and_then(Value::as_str)
                    .map(|tag| !tag.trim().is_empty())
            })
            .unwrap_or(false)
    } else {
        false
    };

    // A4: Upload ES Module Worker + SyncHubDO + SYNC_TOKEN binding
    progress(ProvisionStep::Deploying);
    let script_url = format!("{CF_API_BASE}/accounts/{account_id}/workers/scripts/{worker_name}");
    let boundary = format!("----TokenVaneWorkerBoundary{}", generate_sync_secret_hex(8)?);
    let content_type = format!("multipart/form-data; boundary={boundary}");

    let include_initial_migration = !already_migrated;
    let first_meta = build_worker_metadata_json(bearer_token, include_initial_migration)?;
    let first_payload = build_worker_multipart_body(
        &boundary,
        &first_meta,
        EMBEDDED_WORKER_INDEX_JS,
        EMBEDDED_WORKER_HUB_JS,
    );

    let (mut upload_status, mut upload_body) = transport
        .request(
            "PUT",
            &script_url,
            &cf_token,
            Some(&content_type),
            Some(first_payload),
        )
        .await?;

    if include_initial_migration && is_migration_precondition_error(upload_status, &upload_body) {
        let retry_meta = build_worker_metadata_json(bearer_token, false)?;
        let retry_payload = build_worker_multipart_body(
            &boundary,
            &retry_meta,
            EMBEDDED_WORKER_INDEX_JS,
            EMBEDDED_WORKER_HUB_JS,
        );
        (upload_status, upload_body) = transport
            .request(
                "PUT",
                &script_url,
                &cf_token,
                Some(&content_type),
                Some(retry_payload),
            )
            .await?;
    }

    parse_cf_success_json(upload_status, &upload_body)?;

    // A5: Enable script-level workers.dev route
    progress(ProvisionStep::WritingSecret);
    let script_subdomain_url =
        format!("{CF_API_BASE}/accounts/{account_id}/workers/scripts/{worker_name}/subdomain");
    let enable_body = serde_json::to_vec(&json!({ "enabled": true }))?;
    let (enable_status, enable_resp) = transport
        .request(
            "POST",
            &script_subdomain_url,
            &cf_token,
            Some("application/json"),
            Some(enable_body),
        )
        .await?;
    parse_cf_success_json(enable_status, &enable_resp)?;

    normalize_worker_url(Some(&format!("https://{worker_name}.{subdomain}.workers.dev")))
}

struct TempProvisionDir(PathBuf);

impl TempProvisionDir {
    fn create() -> CollectorResult<Self> {
        let suffix = generate_sync_secret_hex(8)?;
        let dir = std::env::temp_dir().join(format!("tokenvane-worker-{suffix}"));
        std::fs::create_dir_all(&dir)?;
        Ok(Self(dir))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempProvisionDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn build_wrangler_toml(worker_name: &str) -> String {
    format!(
        r#"name = "{worker_name}"
main = "index.js"
compatibility_date = "{WORKER_COMPATIBILITY_DATE}"
workers_dev = true

[[durable_objects.bindings]]
name = "SYNC_HUB"
class_name = "SyncHubDO"

[[migrations]]
tag = "v1"
new_sqlite_classes = ["SyncHubDO"]
"#
    )
}

fn is_wrangler_authenticated(out: &WranglerCmdOutput) -> bool {
    if !out.success {
        return false;
    }
    let combined = format!("{}\n{}", out.stdout, out.stderr).to_ascii_lowercase();
    !combined.contains("not authenticated") && !combined.contains("run `wrangler login`")
}

fn summarize_wrangler_error(out: &WranglerCmdOutput) -> String {
    let combined = strip_ansi_sequences(&format!("{}\n{}", out.stderr, out.stdout));
    let trimmed = combined.trim();
    if trimmed.is_empty() {
        match out.status_code {
            Some(code) => format!("退出码 {code}"),
            None => "未知错误".to_string(),
        }
    } else {
        trimmed.chars().take(240).collect()
    }
}

async fn provision_via_wrangler_cli(
    runner: &dyn WranglerCliRunner,
    worker_name: &str,
    bearer_token: &str,
    progress: ProvisionProgress<'_>,
) -> CollectorResult<String> {
    let temp_dir = TempProvisionDir::create()?;
    std::fs::write(
        temp_dir.path().join("wrangler.toml"),
        build_wrangler_toml(worker_name),
    )?;
    std::fs::write(temp_dir.path().join("index.js"), EMBEDDED_WORKER_INDEX_JS)?;
    std::fs::write(temp_dir.path().join("hub.js"), EMBEDDED_WORKER_HUB_JS)?;

    // The first `npx wrangler@latest` run downloads Wrangler (tens of MB) before it
    // can answer, which easily exceeds 30s behind a proxy; allow a generous window.
    progress(ProvisionStep::CheckingAuth);
    let whoami = runner
        .run_wrangler(
            temp_dir.path(),
            &["whoami"],
            None,
            false,
            Duration::from_secs(240),
        )
        .await
        .map_err(|e| CollectorError::Compliance(format!("检查 Wrangler 登录状态失败：{e}")))?;

    if !is_wrangler_authenticated(&whoami) {
        progress(ProvisionStep::BrowserLogin);
        let login = runner
            .run_wrangler(
                temp_dir.path(),
                &["login"],
                None,
                true,
                Duration::from_secs(180),
            )
            .await?;
        if !login.success {
            return Err(CollectorError::Compliance(format!(
                "Wrangler 浏览器授权登录中止或超时（{}），可重试或切换至 Cloudflare API 令牌模式",
                summarize_wrangler_error(&login)
            )));
        }
    }

    progress(ProvisionStep::Deploying);
    let deploy = runner
        .run_wrangler(
            temp_dir.path(),
            &["deploy"],
            None,
            false,
            Duration::from_secs(240),
        )
        .await?;
    if !deploy.success {
        return Err(CollectorError::Compliance(format!(
            "Wrangler 部署 Worker 失败：{}",
            summarize_wrangler_error(&deploy)
        )));
    }

    let combined_deploy = format!("{}\n{}", deploy.stdout, deploy.stderr);
    let worker_url = extract_deployed_workers_dev_url(&combined_deploy, worker_name)?;

    progress(ProvisionStep::WritingSecret);
    let secret = runner
        .run_wrangler(
            temp_dir.path(),
            &["secret", "put", "SYNC_TOKEN"],
            Some(bearer_token.as_bytes()),
            false,
            Duration::from_secs(90),
        )
        .await?;
    if !secret.success {
        return Err(CollectorError::Compliance(format!(
            "写入 Worker SYNC_TOKEN 密钥失败：{}",
            summarize_wrangler_error(&secret)
        )));
    }

    Ok(worker_url)
}

pub async fn provision_worker(
    credentials: &dyn CredentialManager,
    req: &CloudSyncProvisionRequest,
    progress: ProvisionProgress<'_>,
) -> CollectorResult<ProvisionedWorkerArtifacts> {
    let cf_transport = ReqwestCfApiTransport::new()?;
    let wrangler_runner = SystemWranglerRunner;
    provision_worker_with_progress(credentials, &cf_transport, &wrangler_runner, req, progress)
        .await
}

pub async fn provision_worker_with(
    credentials: &dyn CredentialManager,
    cf_transport: &dyn CloudflareApiTransport,
    wrangler_runner: &dyn WranglerCliRunner,
    req: &CloudSyncProvisionRequest,
) -> CollectorResult<ProvisionedWorkerArtifacts> {
    provision_worker_with_progress(credentials, cf_transport, wrangler_runner, req, &no_progress)
        .await
}

pub async fn provision_worker_with_progress(
    credentials: &dyn CredentialManager,
    cf_transport: &dyn CloudflareApiTransport,
    wrangler_runner: &dyn WranglerCliRunner,
    req: &CloudSyncProvisionRequest,
    progress: ProvisionProgress<'_>,
) -> CollectorResult<ProvisionedWorkerArtifacts> {
    progress(ProvisionStep::Preparing);
    let mode = ProvisionMode::parse(&req.mode)?;
    let worker_name = normalize_worker_script_name(req.worker_name.as_deref())?;
    let (bearer_token, e2ee_passphrase) = resolve_sync_secrets(credentials, req)?;

    let worker_url = match mode {
        ProvisionMode::CloudflareApi => {
            provision_via_cloudflare_api(
                credentials,
                cf_transport,
                req,
                &worker_name,
                &bearer_token,
                progress,
            )
            .await?
        }
        ProvisionMode::WranglerCli => {
            provision_via_wrangler_cli(wrangler_runner, &worker_name, &bearer_token, progress)
                .await?
        }
    };
    progress(ProvisionStep::Saving);

    set_sync_credentials(
        credentials,
        Some(&bearer_token),
        e2ee_passphrase.as_deref(),
    )?;

    Ok(ProvisionedWorkerArtifacts {
        mode: mode.as_str().to_string(),
        worker_name,
        worker_url,
        bearer_token,
        e2ee_passphrase,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::credentials::MemoryCredentialStore;

    #[derive(Debug, Clone)]
    struct RecordedCfCall {
        method: String,
        url: String,
        bearer: String,
        content_type: Option<String>,
        body: Option<Vec<u8>>,
    }

    type CfMockHandler =
        Arc<dyn Fn(&str, &str, Option<&[u8]>) -> CollectorResult<(u16, String)> + Send + Sync>;
    type WranglerMockHandler = Arc<
        dyn Fn(&[String], Option<&[u8]>, bool) -> CollectorResult<WranglerCmdOutput> + Send + Sync,
    >;

    struct MockCfTransport {
        calls: Arc<Mutex<Vec<RecordedCfCall>>>,
        handler: CfMockHandler,
    }

    impl CloudflareApiTransport for MockCfTransport {
        fn request<'a>(
            &'a self,
            method: &'a str,
            url: &'a str,
            bearer_token: &'a str,
            content_type: Option<&'a str>,
            body: Option<Vec<u8>>,
        ) -> SyncBoxFuture<'a, CollectorResult<(u16, String)>> {
            let calls = Arc::clone(&self.calls);
            let handler = Arc::clone(&self.handler);
            Box::pin(async move {
                calls.lock().unwrap().push(RecordedCfCall {
                    method: method.to_string(),
                    url: url.to_string(),
                    bearer: bearer_token.to_string(),
                    content_type: content_type.map(ToString::to_string),
                    body: body.clone(),
                });
                (handler)(method, url, body.as_deref())
            })
        }
    }

    #[derive(Debug, Clone)]
    struct RecordedWranglerCall {
        args: Vec<String>,
        stdin_data: Option<Vec<u8>>,
        interactive_login: bool,
        has_wrangler_toml: bool,
        has_index_js: bool,
        has_hub_js: bool,
    }

    struct MockWranglerRunner {
        calls: Arc<Mutex<Vec<RecordedWranglerCall>>>,
        handler: WranglerMockHandler,
    }

    impl WranglerCliRunner for MockWranglerRunner {
        fn run_wrangler<'a>(
            &'a self,
            cwd: &'a Path,
            args: &'a [&'a str],
            stdin_data: Option<&'a [u8]>,
            interactive_login: bool,
            _timeout: Duration,
        ) -> SyncBoxFuture<'a, CollectorResult<WranglerCmdOutput>> {
            let calls = Arc::clone(&self.calls);
            let handler = Arc::clone(&self.handler);
            let owned_args: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
            let owned_stdin = stdin_data.map(|b| b.to_vec());
            let has_wrangler_toml = cwd.join("wrangler.toml").is_file();
            let has_index_js = cwd.join("index.js").is_file();
            let has_hub_js = cwd.join("hub.js").is_file();

            Box::pin(async move {
                calls.lock().unwrap().push(RecordedWranglerCall {
                    args: owned_args.clone(),
                    stdin_data: owned_stdin.clone(),
                    interactive_login,
                    has_wrangler_toml,
                    has_index_js,
                    has_hub_js,
                });
                (handler)(&owned_args, owned_stdin.as_deref(), interactive_login)
            })
        }
    }

    #[test]
    fn wrangler_command_spawns_cmd_wrapper_directly_without_manual_cmd_exe() {
        let inv = WranglerInvocation {
            program: PathBuf::from(r"C:\Program Files\nodejs\npx.cmd"),
            prefix_args: vec!["--yes", "wrangler@latest"],
        };
        let args = vec!["secret".to_string(), "put".to_string(), "SYNC_TOKEN".to_string()];
        let cmd = build_wrangler_command(&inv, Path::new("."), &args, Some(b"tok"), false);

        // The program path (with spaces) must be the spawned program itself, not an
        // argument to `cmd.exe /s /c`, which strips its quotes and runs `C:\Program`.
        assert_eq!(cmd.get_program(), inv.program.as_os_str());
        let got: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(got, ["--yes", "wrangler@latest", "secret", "put", "SYNC_TOKEN"]);
    }

    #[test]
    fn validates_worker_script_name_and_blocks_unsafe_input() {
        assert_eq!(
            normalize_worker_script_name(None).unwrap(),
            "tokenvane-sync-worker"
        );
        assert_eq!(
            normalize_worker_script_name(Some("  My-Sync-Hub-01 ")).unwrap(),
            "my-sync-hub-01"
        );
        assert!(normalize_worker_script_name(Some("-bad-start")).is_err());
        assert!(normalize_worker_script_name(Some("bad-end-")).is_err());
        assert!(normalize_worker_script_name(Some("bad;rm -rf")).is_err());
        assert!(normalize_worker_script_name(Some("bad_underscore")).is_err());
        assert!(normalize_worker_script_name(Some(&"a".repeat(64))).is_err());
    }

    #[test]
    fn extracts_workers_dev_url_across_ansi_and_multiline_output() {
        let noisy = "\x1b[32mUploaded\x1b[39m tokenvane-sync-worker (1.23 sec)\n\
                     \x1b[1mPublished\x1b[0m tokenvane-sync-worker\n  \
                     https://tokenvane-sync-worker.alice-demo.workers.dev.\n\
                     Current Version ID: 1234";
        assert_eq!(
            extract_deployed_workers_dev_url(noisy, "tokenvane-sync-worker").unwrap(),
            "https://tokenvane-sync-worker.alice-demo.workers.dev"
        );
        assert!(extract_deployed_workers_dev_url("no url here", "tokenvane-sync-worker").is_err());
    }

    #[tokio::test]
    async fn cloudflare_api_provisions_initial_worker_persists_secrets_and_handles_idempotent_retry() {
        let creds = MemoryCredentialStore::new();
        let cf_calls = Arc::new(Mutex::new(Vec::new()));
        let put_attempts = Arc::new(Mutex::new(0usize));

        let put_attempts_clone = Arc::clone(&put_attempts);
        let cf_transport = MockCfTransport {
            calls: Arc::clone(&cf_calls),
            handler: Arc::new(move |method, url, body| {
                if method == "GET" && url.ends_with("/accounts?per_page=5") {
                    return Ok((
                        200,
                        r#"{"success":true,"result":[{"id":"acc-1234567890"}]}"#.to_string(),
                    ));
                }
                if method == "GET" && url.ends_with("/workers/subdomain") {
                    return Ok((
                        200,
                        r#"{"success":true,"result":{"subdomain":"demo-sub"}}"#.to_string(),
                    ));
                }
                if method == "GET" && url.ends_with("/scripts/tokenvane-sync-worker/settings") {
                    return Ok((404, r#"{"success":false,"errors":[{"code":10007,"message":"not found"}]}"#.to_string()));
                }
                if method == "PUT" && url.ends_with("/scripts/tokenvane-sync-worker") {
                    let mut count = put_attempts_clone.lock().unwrap();
                    *count += 1;
                    let text = String::from_utf8_lossy(body.unwrap_or_default()).into_owned();
                    assert!(text.contains("name=\"metadata\""));
                    assert!(text.contains("name=\"index.js\"; filename=\"index.js\""));
                    assert!(text.contains("name=\"hub.js\"; filename=\"hub.js\""));
                    assert!(text.contains("application/javascript+module"));
                    assert!(text.contains("SyncHubDO"));
                    assert!(text.contains("SYNC_TOKEN"));
                    assert!(text.contains("\"new_sqlite_classes\":[\"SyncHubDO\"]"));
                    return Ok((200, r#"{"success":true,"result":{"id":"tokenvane-sync-worker"}}"#.to_string()));
                }
                if method == "POST" && url.ends_with("/scripts/tokenvane-sync-worker/subdomain") {
                    return Ok((200, r#"{"success":true,"result":{"enabled":true}}"#.to_string()));
                }
                panic!("Unexpected CF API call: {method} {url}");
            }),
        };

        let wrangler_runner = MockWranglerRunner {
            calls: Arc::new(Mutex::new(Vec::new())),
            handler: Arc::new(|_, _, _| panic!("Wrangler should not be called in cloudflare_api mode")),
        };

        let req = CloudSyncProvisionRequest {
            mode: "cloudflare_api".to_string(),
            worker_name: None,
            bearer_token: None,
            e2ee_enabled: true,
            e2ee_passphrase: None,
            cf_api_token: Some("cf-api-secret-token".to_string()),
            cf_account_id: None,
        };

        let res = provision_worker_with(&creds, &cf_transport, &wrangler_runner, &req)
            .await
            .expect("first api provision");
        assert_eq!(res.mode, "cloudflare_api");
        assert_eq!(res.worker_name, "tokenvane-sync-worker");
        assert_eq!(
            res.worker_url,
            "https://tokenvane-sync-worker.demo-sub.workers.dev"
        );
        assert_eq!(res.bearer_token.len(), 48);
        assert_eq!(res.e2ee_passphrase.as_ref().map(String::len), Some(32));
        assert_eq!(*put_attempts.lock().unwrap(), 1);
        assert_eq!(
            creds.get_api_key(SYNC_BEARER_TOKEN_KEY).unwrap().as_deref(),
            Some(res.bearer_token.as_str())
        );
        assert_eq!(
            creds.get_api_key(SYNC_E2EE_PASSPHRASE_KEY).unwrap().as_deref(),
            res.e2ee_passphrase.as_deref()
        );
        assert_eq!(
            creds.get_api_key(SYNC_CF_API_TOKEN_KEY).unwrap().as_deref(),
            Some("cf-api-secret-token")
        );
        {
            let first_cf_calls = cf_calls.lock().unwrap();
            assert_eq!(first_cf_calls.len(), 5);
            assert!(first_cf_calls.iter().all(|c| c.bearer == "cf-api-secret-token"));
            assert!(first_cf_calls[3]
                .content_type
                .as_deref()
                .is_some_and(|ct| ct.starts_with("multipart/form-data; boundary=")));
            assert_eq!(first_cf_calls[4].method, "POST");
            assert!(first_cf_calls[4].url.ends_with("/subdomain"));
            assert!(first_cf_calls[4].body.is_some());
        }

        // Second run: settings reports migration_tag = "v1" -> should omit migrations block,
        // and also verify 10079 fallback if settings didn't report it.
        let retry_calls = Arc::new(Mutex::new(Vec::new()));
        let retry_put_idx = Arc::new(Mutex::new(0usize));
        let retry_put_idx_clone = Arc::clone(&retry_put_idx);
        let cf_retry_transport = MockCfTransport {
            calls: Arc::clone(&retry_calls),
            handler: Arc::new(move |method, url, body| {
                if method == "GET" && url.ends_with("/accounts?per_page=5") {
                    return Ok((
                        200,
                        r#"{"success":true,"result":[{"id":"acc-1234567890"}]}"#.to_string(),
                    ));
                }
                if method == "GET" && url.ends_with("/workers/subdomain") {
                    return Ok((
                        200,
                        r#"{"success":true,"result":{"subdomain":"demo-sub"}}"#.to_string(),
                    ));
                }
                if method == "GET" && url.ends_with("/scripts/tokenvane-sync-worker/settings") {
                    return Ok((200, r#"{"success":true,"result":{}}"#.to_string()));
                }
                if method == "PUT" && url.ends_with("/scripts/tokenvane-sync-worker") {
                    let mut idx = retry_put_idx_clone.lock().unwrap();
                    *idx += 1;
                    let text = String::from_utf8_lossy(body.unwrap_or_default()).into_owned();
                    if *idx == 1 {
                        assert!(text.contains("\"migrations\""));
                        return Ok((
                            400,
                            r#"{"success":false,"errors":[{"code":10079,"message":"Migration tag precondition failed"}]}"#.to_string(),
                        ));
                    }
                    assert!(!text.contains("\"migrations\""));
                    return Ok((200, r#"{"success":true,"result":{"id":"tokenvane-sync-worker"}}"#.to_string()));
                }
                if method == "POST" && url.ends_with("/scripts/tokenvane-sync-worker/subdomain") {
                    return Ok((200, r#"{"success":true,"result":{"enabled":true}}"#.to_string()));
                }
                panic!("Unexpected CF API call: {method} {url}");
            }),
        };

        let redeploy_req = CloudSyncProvisionRequest {
            mode: "cloudflare_api".to_string(),
            worker_name: Some("tokenvane-sync-worker".to_string()),
            bearer_token: None,
            e2ee_enabled: true,
            e2ee_passphrase: None,
            cf_api_token: None, // Should reuse saved SYNC_CF_API_TOKEN_KEY
            cf_account_id: None,
        };

        let res2 = provision_worker_with(&creds, &cf_retry_transport, &wrangler_runner, &redeploy_req)
            .await
            .expect("redeploy with 10079 fallback");
        assert_eq!(*retry_put_idx.lock().unwrap(), 2);
        // Existing credentials should be preserved so already-paired mobile clients stay valid
        assert_eq!(res2.bearer_token, res.bearer_token);
        assert_eq!(res2.e2ee_passphrase, res.e2ee_passphrase);

        // Third run: settings already reports migration_tag = "v1" -> omits migrations on first PUT
        let migrated_puts = Arc::new(Mutex::new(0usize));
        let migrated_puts_clone = Arc::clone(&migrated_puts);
        let cf_migrated_transport = MockCfTransport {
            calls: Arc::new(Mutex::new(Vec::new())),
            handler: Arc::new(move |method, url, body| {
                if method == "GET" && url.ends_with("/accounts?per_page=5") {
                    return Ok((
                        200,
                        r#"{"success":true,"result":[{"id":"acc-1234567890"}]}"#.to_string(),
                    ));
                }
                if method == "GET" && url.ends_with("/workers/subdomain") {
                    return Ok((
                        200,
                        r#"{"success":true,"result":{"subdomain":"demo-sub"}}"#.to_string(),
                    ));
                }
                if method == "GET" && url.ends_with("/scripts/tokenvane-sync-worker/settings") {
                    return Ok((
                        200,
                        r#"{"success":true,"result":{"migration_tag":"v1"}}"#.to_string(),
                    ));
                }
                if method == "PUT" && url.ends_with("/scripts/tokenvane-sync-worker") {
                    *migrated_puts_clone.lock().unwrap() += 1;
                    let text = String::from_utf8_lossy(body.unwrap_or_default()).into_owned();
                    assert!(!text.contains("\"migrations\""));
                    return Ok((200, r#"{"success":true,"result":{"id":"tokenvane-sync-worker"}}"#.to_string()));
                }
                if method == "POST" && url.ends_with("/scripts/tokenvane-sync-worker/subdomain") {
                    return Ok((200, r#"{"success":true,"result":{"enabled":true}}"#.to_string()));
                }
                panic!("Unexpected CF API call: {method} {url}");
            }),
        };
        let res3 = provision_worker_with(&creds, &cf_migrated_transport, &wrangler_runner, &redeploy_req)
            .await
            .expect("redeploy skips migrations when migration_tag present");
        assert_eq!(*migrated_puts.lock().unwrap(), 1);
        assert_eq!(res3.worker_url, "https://tokenvane-sync-worker.demo-sub.workers.dev");
    }

    #[tokio::test]
    async fn wrangler_cli_skips_login_when_already_authenticated() {
        let creds = MemoryCredentialStore::new();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let wrangler_runner = MockWranglerRunner {
            calls: Arc::clone(&calls),
            handler: Arc::new(|args, stdin_data, interactive_login| {
                assert!(!interactive_login);
                match args.first().map(String::as_str).unwrap_or("") {
                    "whoami" => Ok(WranglerCmdOutput {
                        success: true,
                        status_code: Some(0),
                        stdout: "You are logged in with an OAuth Token!".to_string(),
                        stderr: String::new(),
                    }),
                    "deploy" => Ok(WranglerCmdOutput {
                        success: true,
                        status_code: Some(0),
                        stdout: "Published tokenvane-sync-worker\nhttps://tokenvane-sync-worker.ready.workers.dev\n"
                            .to_string(),
                        stderr: String::new(),
                    }),
                    "secret" => {
                        assert!(!stdin_data.unwrap_or_default().is_empty());
                        Ok(WranglerCmdOutput {
                            success: true,
                            status_code: Some(0),
                            stdout: "Success!".to_string(),
                            stderr: String::new(),
                        })
                    }
                    other => panic!("Unexpected command: {other}"),
                }
            }),
        };
        let cf_transport = MockCfTransport {
            calls: Arc::new(Mutex::new(Vec::new())),
            handler: Arc::new(|_, _, _| panic!("CF API should not be called")),
        };
        let req = CloudSyncProvisionRequest {
            mode: "wrangler_cli".to_string(),
            worker_name: None,
            bearer_token: None,
            e2ee_enabled: false,
            e2ee_passphrase: None,
            cf_api_token: None,
            cf_account_id: None,
        };
        let steps = Arc::new(Mutex::new(Vec::new()));
        let steps_sink = Arc::clone(&steps);
        let record = move |step: ProvisionStep| steps_sink.lock().unwrap().push(step);
        let res =
            provision_worker_with_progress(&creds, &cf_transport, &wrangler_runner, &req, &record)
                .await
                .expect("already logged in wrangler provision");
        assert_eq!(res.worker_url, "https://tokenvane-sync-worker.ready.workers.dev");
        assert_eq!(res.bearer_token.len(), 48);
        assert_eq!(calls.lock().unwrap().len(), 3);
        assert_eq!(
            *steps.lock().unwrap(),
            vec![
                ProvisionStep::Preparing,
                ProvisionStep::CheckingAuth,
                ProvisionStep::Deploying,
                ProvisionStep::WritingSecret,
                ProvisionStep::Saving,
            ]
        );
    }

    #[tokio::test]
    async fn wrangler_cli_handles_login_flow_deploy_and_stdin_secret_injection() {
        let creds = MemoryCredentialStore::new();
        let calls = Arc::new(Mutex::new(Vec::new()));

        let wrangler_runner = MockWranglerRunner {
            calls: Arc::clone(&calls),
            handler: Arc::new(|args, stdin_data, interactive_login| {
                let cmd = args.first().map(String::as_str).unwrap_or("");
                match cmd {
                    "whoami" => {
                        assert!(!interactive_login);
                        Ok(WranglerCmdOutput {
                            success: false,
                            status_code: Some(1),
                            stdout: String::new(),
                            stderr: "You are not authenticated. Please run `wrangler login`."
                                .to_string(),
                        })
                    }
                    "login" => {
                        assert!(interactive_login);
                        Ok(WranglerCmdOutput {
                            success: true,
                            status_code: Some(0),
                            stdout: "Successfully logged in.".to_string(),
                            stderr: String::new(),
                        })
                    }
                    "deploy" => {
                        assert!(!interactive_login);
                        Ok(WranglerCmdOutput {
                            success: true,
                            status_code: Some(0),
                            stdout: "Published my-sync\n  https://my-sync.bob-edge.workers.dev\n"
                                .to_string(),
                            stderr: String::new(),
                        })
                    }
                    "secret" => {
                        assert!(!interactive_login);
                        assert_eq!(args, ["secret", "put", "SYNC_TOKEN"]);
                        let secret = String::from_utf8(stdin_data.unwrap_or_default().to_vec())
                            .expect("utf8 stdin");
                        assert_eq!(secret, "custom-bearer-secret");
                        Ok(WranglerCmdOutput {
                            success: true,
                            status_code: Some(0),
                            stdout: "Success! Uploaded secret SYNC_TOKEN".to_string(),
                            stderr: String::new(),
                        })
                    }
                    other => panic!("Unexpected wrangler command: {other}"),
                }
            }),
        };

        let cf_transport = MockCfTransport {
            calls: Arc::new(Mutex::new(Vec::new())),
            handler: Arc::new(|_, _, _| panic!("CF API should not be called in wrangler_cli mode")),
        };

        let req = CloudSyncProvisionRequest {
            mode: "wrangler_cli".to_string(),
            worker_name: Some("my-sync".to_string()),
            bearer_token: Some("custom-bearer-secret".to_string()),
            e2ee_enabled: false,
            e2ee_passphrase: None,
            cf_api_token: None,
            cf_account_id: None,
        };

        let res = provision_worker_with(&creds, &cf_transport, &wrangler_runner, &req)
            .await
            .expect("wrangler provision");
        assert_eq!(res.mode, "wrangler_cli");
        assert_eq!(res.worker_name, "my-sync");
        assert_eq!(res.worker_url, "https://my-sync.bob-edge.workers.dev");
        assert_eq!(res.bearer_token, "custom-bearer-secret");
        assert_eq!(res.e2ee_passphrase, None);

        let recorded = calls.lock().unwrap();
        assert_eq!(recorded.len(), 4);
        assert!(recorded.iter().all(|c| c.has_wrangler_toml && c.has_index_js && c.has_hub_js));
        assert_eq!(recorded[0].args, vec!["whoami"]);
        assert!(!recorded[0].interactive_login);
        assert_eq!(recorded[1].args, vec!["login"]);
        assert!(recorded[1].interactive_login);
        assert_eq!(recorded[2].args, vec!["deploy"]);
        assert_eq!(recorded[3].args, vec!["secret", "put", "SYNC_TOKEN"]);
        assert_eq!(recorded[3].stdin_data.as_deref(), Some(b"custom-bearer-secret".as_slice()));
        assert_eq!(
            creds.get_api_key(SYNC_BEARER_TOKEN_KEY).unwrap().as_deref(),
            Some("custom-bearer-secret")
        );
    }
}
