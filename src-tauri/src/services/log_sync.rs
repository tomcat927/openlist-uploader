use reqwest::{Client, header::{HeaderMap, HeaderValue, AUTHORIZATION}};
use sha2::{Sha256, Digest};
use std::fs;
use std::path::PathBuf;
use std::time::Duration;
use crate::models::*;
use crate::utils::log::log;
use crate::utils::storage::Storage;

const ALIST_SALT: &str = "https://github.com/alist-org/alist";

#[derive(Debug, serde::Deserialize, Default)]
struct LogSyncResponse<T> {
    pub code: i32,
    pub message: String,
    #[serde(default)]
    pub data: Option<T>,
}

fn now_rfc3339() -> String {
    chrono::Local::now().to_rfc3339()
}

fn load_last_sync_at() -> Option<String> {
    Storage::load_config().ok()?.log_sync.last_sync_at
}

fn save_token_to_config(token: &str) {
    if let Ok(mut config) = Storage::load_config() {
        config.log_sync.token = token.to_string();
        let _ = Storage::save_config(&config);
    }
}

#[derive(Debug, serde::Deserialize, Default)]
struct LogSyncLoginResp {
    pub token: String,
}

fn hash_password(password: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(format!("{}-{}", password, ALIST_SALT).as_bytes());
    let result = hasher.finalize();
    result.iter().map(|b| format!("{:02x}", b)).collect()
}

fn get_app_dir() -> Option<PathBuf> {
    let mut app_dir = dirs::data_local_dir()?;
    app_dir.push("openlist-uploader");
    if !app_dir.exists() {
        let _ = fs::create_dir_all(&app_dir);
    }
    Some(app_dir)
}

fn get_local_log_files_list() -> Vec<(PathBuf, String, u64, Option<String>)> {
    let mut files = Vec::new();

    let app_dir = match get_app_dir() {
        Some(d) => d,
        None => return files,
    };

    let mut collect_entry = |entry: &std::path::Path, display_name: &str| {
        if let Ok(meta) = entry.metadata() {
            let size = meta.len();
            let modified = meta.modified().ok().map(|t| {
                let dt: chrono::DateTime<chrono::Local> = t.into();
                dt.to_rfc3339()
            });
            files.push((entry.to_path_buf(), display_name.to_string(), size, modified));
        }
    };

    let debug_log = app_dir.join("debug.log");
    if debug_log.exists() {
        collect_entry(&debug_log, "debug.log");
    }

    let startup_log = app_dir.join("startup.log");
    if startup_log.exists() {
        collect_entry(&startup_log, "startup.log");
    }

    let panic_log = app_dir.join("panic.log");
    if panic_log.exists() {
        collect_entry(&panic_log, "panic.log");
    }

    let logs_dir = app_dir.join("logs");
    if logs_dir.exists() {
        if let Ok(entries) = fs::read_dir(&logs_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        collect_entry(&path, name);
                    }
                }
            }
        }
    }

    files.sort_by(|a, b| a.1.cmp(&b.1));
    files
}

pub struct LogSyncClient {
    client: Client,
    base_url: String,
    token: String,
}

impl LogSyncClient {
    pub fn new(config: &LogSyncConfig) -> Self {
        let mut builder = Client::builder().timeout(Duration::from_secs(60));
        if !config.use_system_proxy {
            builder = builder.no_proxy();
        }
        let client = builder.build().unwrap_or_default();
        Self {
            client,
            base_url: config.base_url.trim_end_matches('/').to_string(),
            token: config.token.clone(),
        }
    }

    fn headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if !self.token.is_empty() {
            if let Ok(val) = HeaderValue::from_str(&self.token) {
                headers.insert(AUTHORIZATION, val);
            }
        }
        headers
    }

    pub async fn login(&self, username: &str, password: &str) -> Result<String, String> {
        let url = format!("{}/api/auth/login/hash", self.base_url);
        let hashed = hash_password(password);
        log(&format!("日志Alist登录: url={}, username={}", url, username));

        let body = serde_json::json!({
            "username": username,
            "password": hashed,
            "otp_code": ""
        });

        let resp = self.client
            .post(&url)
            .json(&body)
            .timeout(Duration::from_secs(15))
            .send()
            .await
            .map_err(|e| format!("登录请求失败: {}", e))?;

        let status = resp.status();
        let text = resp.text().await.map_err(|e| format!("读取响应失败: {}", e))?;
        log(&format!("日志Alist登录响应: status={}, body_length={}", status, text.len()));

        let resp: LogSyncResponse<LogSyncLoginResp> = serde_json::from_str(&text)
            .map_err(|e| format!("解析响应失败: {}; 原始: {}", e, text))?;

        if resp.code == 200 {
            match resp.data {
                Some(d) if !d.token.is_empty() => {
                    log("日志Alist登录成功");
                    Ok(d.token)
                }
                _ => Err("登录成功但未返回 token".into()),
            }
        } else {
            Err(format!("登录失败: code={}, message={}", resp.code, resp.message))
        }
    }

    pub async fn ensure_dir(&self, path: &str) -> Result<(), String> {
        let url = format!("{}/api/fs/mkdir", self.base_url);
        let body = serde_json::json!({ "path": path });
        let resp = self.client
            .post(&url)
            .headers(self.headers())
            .json(&body)
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| format!("创建目录请求失败: {}", e))?;

        let text = resp.text().await.map_err(|e| format!("读取响应失败: {}", e))?;
        let result: LogSyncResponse<serde_json::Value> = serde_json::from_str(&text)
            .unwrap_or(LogSyncResponse { code: -1, message: text.clone(), data: None });

        if result.code == 200 || result.message.contains("exists") || result.message.contains("exist") {
            log(&format!("日志目录已就绪: path={}", path));
            Ok(())
        } else {
            log(&format!("创建日志目录完成: path={}, code={}, message={}", path, result.code, result.message));
            Ok(())
        }
    }

    pub async fn upload_file(&self, local_path: &PathBuf, target_path: &str) -> Result<(), String> {
        let file_name = local_path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");

        let data = fs::read(local_path).map_err(|e| {
            format!("读取本地日志文件失败: file={}, error={}", file_name, e)
        })?;

        let encoded_path = percent_encoding::utf8_percent_encode(
            target_path,
            percent_encoding::NON_ALPHANUMERIC,
        ).to_string();

        let url = format!("{}/api/fs/put", self.base_url);
        let mut headers = self.headers();
        if let Ok(val) = HeaderValue::from_str(&encoded_path) {
            headers.insert("File-Path", val);
        }
        headers.insert("Content-Type", HeaderValue::from_static("application/octet-stream"));
        if let Ok(val) = HeaderValue::from_str(&data.len().to_string()) {
            headers.insert("Content-Length", val);
        }

        log(&format!("上传日志文件: file={}, size={}B, target={}", file_name, data.len(), target_path));

        let resp = self.client
            .put(&url)
            .headers(headers)
            .body(data)
            .timeout(Duration::from_secs(120))
            .send()
            .await
            .map_err(|e| format!("上传请求失败: file={}, error={}", file_name, e))?;

        let status = resp.status();
        let text = resp.text().await.map_err(|e| format!("读取上传响应失败: file={}, error={}", file_name, e))?;
        log(&format!("上传日志响应: file={}, status={}, body={}", file_name, status, text));

        let result: LogSyncResponse<serde_json::Value> = serde_json::from_str(&text)
            .unwrap_or(LogSyncResponse { code: -1, message: text.clone(), data: None });

        if result.code == 200 {
            log(&format!("日志文件上传成功: file={}", file_name));
            Ok(())
        } else {
            Err(format!("上传失败: file={}, code={}, message={}", file_name, result.code, result.message))
        }
    }
}

pub async fn sync_logs(config: &LogSyncConfig) -> LogSyncResult {
    let mut details = Vec::new();

    if config.base_url.is_empty() {
        details.push("日志Alist地址为空，请先配置".to_string());
        return LogSyncResult { total: 0, success: 0, failed: 0, details, last_sync_at: load_last_sync_at() };
    }

    let client = LogSyncClient::new(config);

    // token 为空时自动登录
    if config.token.is_empty() && !config.username.is_empty() && !config.password.is_empty() {
        match client.login(&config.username, &config.password).await {
            Ok(token) => {
                log("日志同步: 自动登录成功，token已缓存");
                save_token_to_config(&token);
                let client = LogSyncClient {
                    client: client.client.clone(),
                    base_url: client.base_url.clone(),
                    token,
                };
                return sync_logs_with_client(&client, config).await;
            }
            Err(e) => {
                details.push(format!("登录失败: {}", e));
                return LogSyncResult { total: 0, success: 0, failed: 0, details, last_sync_at: load_last_sync_at() };
            }
        }
    }

    let result = sync_logs_with_client(&client, config).await;

    // 如果失败且可能是 token 过期，尝试重新登录后重试一次
    if result.failed > 0 && result.success == 0 && !config.username.is_empty() && !config.password.is_empty() {
        let has_auth_error = result.details.iter().any(|d| {
            let dl = d.to_lowercase();
            dl.contains("401") || dl.contains("unauthorized") || dl.contains("token") || dl.contains("认证")
        });
        if has_auth_error {
            log("日志同步: 检测到可能的 token 过期，尝试重新登录");
            match client.login(&config.username, &config.password).await {
                Ok(new_token) => {
                    log("日志同步: 重新登录成功，token已更新");
                    save_token_to_config(&new_token);
                    let new_client = LogSyncClient {
                        client: client.client.clone(),
                        base_url: client.base_url.clone(),
                        token: new_token,
                    };
                    return sync_logs_with_client(&new_client, config).await;
                }
                Err(e) => {
                    log(&format!("日志同步: 重新登录失败: {}", e));
                }
            }
        }
    }

    result
}

async fn sync_logs_with_client(client: &LogSyncClient, config: &LogSyncConfig) -> LogSyncResult {
    let mut details = Vec::new();

    let target_path = config.target_path.trim_end_matches('/').to_string();

    if let Err(e) = client.ensure_dir(&target_path).await {
        details.push(format!("创建目录失败: {}", e));
        return LogSyncResult { total: 0, success: 0, failed: 0, details, last_sync_at: load_last_sync_at() };
    }

    let local_files = get_local_log_files_list();
    let total = local_files.len();
    if total == 0 {
        details.push("未找到本地日志文件".to_string());
        return LogSyncResult { total: 0, success: 0, failed: 0, details, last_sync_at: load_last_sync_at() };
    }

    let mut success = 0usize;
    let mut failed = 0usize;

    for (path, name, _size, _modified) in &local_files {
        let file_target = if target_path.ends_with('/') {
            format!("{}{}", target_path, name)
        } else {
            format!("{}/{}", target_path, name)
        };

        match client.upload_file(path, &file_target).await {
            Ok(()) => {
                success += 1;
                details.push(format!("✓ {}", name));
            }
            Err(e) => {
                failed += 1;
                details.push(format!("✗ {}: {}", name, e));
            }
        }
    }

    log(&format!("日志同步完成: total={}, success={}, failed={}", total, success, failed));

    // 持久化最近一次成功同步时间
    let last_sync_at = if success > 0 {
        let ts = now_rfc3339();
        if let Ok(mut config) = Storage::load_config() {
            config.log_sync.last_sync_at = Some(ts.clone());
            let _ = Storage::save_config(&config);
        }
        Some(ts)
    } else {
        load_last_sync_at()
    };

    LogSyncResult { total, success, failed, details, last_sync_at }
}

pub fn get_local_log_files() -> Vec<LocalLogFileInfo> {
    let files = get_local_log_files_list();
    files.into_iter().map(|(_path, name, size, modified)| {
        LocalLogFileInfo { name, size, modified }
    }).collect()
}

pub fn sync_on_exit_blocking(config: &LogSyncConfig) {
    if !config.enabled || !config.sync_on_exit || config.base_url.is_empty() {
        return;
    }

    log("退出时自动同步日志开始");

    let runtime = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            log(&format!("退出时创建runtime失败: {}", e));
            return;
        }
    };

    let result = runtime.block_on(sync_logs(config));
    log(&format!("退出时日志同步完成: total={}, success={}, failed={}", result.total, result.success, result.failed));
}

/// 启动定时同步后台任务（每隔 N 分钟同步一次日志）
/// 返回 None 表示未启用（enabled=false 或 interval=0）
pub fn spawn_interval_sync(config: LogSyncConfig) -> Option<()> {
    if !config.enabled || config.sync_interval_minutes == 0 || config.base_url.is_empty() {
        return None;
    }

    let interval_secs = (config.sync_interval_minutes as u64) * 60;
    log(&format!("启动日志定时同步: interval={}s, target={}", interval_secs, config.target_path));

    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(interval_secs)).await;
            log("定时同步日志触发");
            let result = sync_logs(&config).await;
            if result.failed > 0 {
                log(&format!("定时日志同步有失败: success={}, failed={}", result.success, result.failed));
            }
        }
    });

    Some(())
}
