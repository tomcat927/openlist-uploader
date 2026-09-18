use futures_util::stream::Stream;
use futures_util::StreamExt;
use reqwest::{Client, multipart, header::{HeaderMap, HeaderValue, AUTHORIZATION}};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio_util::bytes::Bytes;
use crate::services::rate_limiter::RateLimiter;
use crate::utils::log::log;

#[derive(Error, Debug)]
pub enum AlistError {
    #[error("HTTP 请求失败：{0}")]
    Request(#[from] reqwest::Error),
    #[error("Alist API 错误：{0}")]
    Api(String),
    #[error("Alist API 错误：{message}\n原始响应：{raw}")]
    ApiWithResponse { message: String, raw: String },
    #[error("文件已存在")]
    FileExists,
    #[error("认证失败")]
    AuthFailed,
    #[error("服务不可用")]
    ServiceUnavailable,
    #[error("IO 错误：{0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, serde::Deserialize)]
pub struct AlistResponse<T> {
    pub code: i32,
    pub message: String,
    #[serde(default)]
    pub data: Option<T>,
}

#[derive(Debug, serde::Deserialize, Default)]
pub struct AlistUserResp {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub username: String,
}

#[derive(Debug, serde::Deserialize, Default)]
pub struct LoginResp {
    pub token: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct AlistTaskResp {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub state: i32, // 0: pending, 1: running, 2: succeeded, 3: cancelled, 4: error
    #[serde(default)]
    pub progress: f64,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub error: String,
}

#[derive(Debug, serde::Deserialize, Default)]
pub struct AlistTaskListResp {
    pub tasks: Vec<AlistTaskResp>,
    pub total: i32,
}

pub struct AlistClient {
    client: Client,
    base_url: String,
    token: String,
}

impl AlistClient {
    pub fn new(base_url: String, token: String, use_proxy: bool) -> Self {
        let mut builder = Client::builder()
            .timeout(Duration::from_secs(3600));

        if !use_proxy {
            builder = builder.no_proxy();
        }

        let client = builder.build().unwrap();

        Self {
            client,
            base_url,
            token,
        }
    }

    pub async fn login(&self, username: &str, password: &str) -> Result<String, AlistError> {
        let base_url = self.base_url.trim_end_matches('/');
        log(&format!("尝试登录 Alist: base_url={}, username={}, password_length={}", base_url, username, password.len()));

        let url = format!("{}/api/auth/login", base_url);

        let body = serde_json::json!({
            "username": username,
            "password": password
        });

        log(&format!("发送登录请求到: {}", url));

        let response = match self.client
            .post(&url)
            .json(&body)
            .timeout(Duration::from_secs(10))
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(err) => {
                log(&format!("登录请求失败: {}", err));
                return Err(AlistError::Request(err));
            }
        };

        let status = response.status();
        log(&format!("收到登录响应状态码: {}", status));

        let response_text = match response.text().await {
            Ok(text) => text,
            Err(err) => {
                log(&format!("读取登录响应正文失败: {}", err));
                return Err(AlistError::Api(format!("读取响应失败: {}", err)));
            }
        };
        log(&format!("登录响应正文已读取: status={}, body_length={}", status, response_text.len()));

        let resp: AlistResponse<LoginResp> = match serde_json::from_str(&response_text) {
            Ok(r) => r,
            Err(err) => {
                log(&format!("解析登录响应失败: status={}, error={}, body={}", status, err, response_text));
                return Err(AlistError::Api(format!("解析响应失败: {}; 原始响应: {}", err, response_text)));
            }
        };

        log(&format!("收到登录响应: http_status={}, code={}, message={}", status, resp.code, resp.message));

        if resp.code == 200 {
            match resp.data {
                Some(d) => {
                    log(&format!("登录成功，已获取 token，token_length={}", d.token.len()));
                    Ok(d.token)
                },
                None => {
                    log("登录成功但响应中未返回 token");
                    Err(AlistError::Api("登录成功但未返回 token".into()))
                }
            }
        } else {
            let msg = format!("登录失败: code={}, message={}", resp.code, resp.message);
            log(&msg);
            Err(AlistError::Api(resp.message))
        }
    }

    fn headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        
        if !self.token.is_empty() {
            let auth_value = HeaderValue::from_str(&self.token);
            if let Ok(val) = auth_value {
                headers.insert(AUTHORIZATION, val);
            }
        }
        
        headers
    }

    pub async fn check_service_available(&self) -> Result<bool, AlistError> {
        let url = format!("{}/ping", self.base_url.trim_end_matches('/'));

        let response = self.client
            .get(&url)
            .timeout(Duration::from_secs(5))
            .send()
            .await;

        match response {
            Ok(resp) => Ok(resp.status().is_success()),
            Err(_) => Ok(false),
        }
    }

    pub async fn test_connection(&self) -> Result<bool, AlistError> {
        let url = format!("{}/api/me", self.base_url.trim_end_matches('/'));
        log(&format!("测试 Token 连接: url={}, has_token={}", url, !self.token.is_empty()));

        if self.token.is_empty() {
            log("测试 Token 连接失败: token 为空");
            return Ok(false);
        }

        let response = self.client
            .get(&url)
            .headers(self.headers())
            .timeout(Duration::from_secs(10))
            .send()
            .await?;

        let status = response.status();
        let response_text = response.text().await.map_err(|e| {
            log(&format!("读取 /api/me 响应失败: status={}, error={}", status, e));
            AlistError::Api(format!("读取响应失败: {}", e))
        })?;

        log(&format!("/api/me 响应: status={}, body_length={}", status, response_text.len()));

        if !status.is_success() {
            log(&format!("/api/me HTTP 状态失败: status={}, body={}", status, response_text));
            return Ok(false);
        }

        let resp: AlistResponse<serde_json::Value> = serde_json::from_str(&response_text).map_err(|e| {
            log(&format!("解析 /api/me 响应失败: status={}, error={}, body={}", status, e, response_text));
            AlistError::Api(format!("解析 /api/me 响应失败: {}; 原始响应: {}", e, response_text))
        })?;

        log(&format!("/api/me 解析结果: code={}, message={}, has_data={}", resp.code, resp.message, resp.data.is_some()));
        Ok(resp.code == 200)
    }

    pub async fn get_current_user(&self) -> Result<AlistUserResp, AlistError> {
        let url = format!("{}/api/me", self.base_url);
        
        let response = self.client
            .get(&url)
            .headers(self.headers())
            .send()
            .await?;

        let resp: AlistResponse<AlistUserResp> = response.json().await?;
        
        if resp.code == 200 {
            resp.data.ok_or_else(|| AlistError::Api("无用户数据".into()))
        } else {
            Err(AlistError::Api(resp.message))
        }
    }

    pub async fn check_file_exists(&self, path: &str, filename: &str) -> Result<bool, AlistError> {
        let url = format!("{}/api/fs/list", self.base_url);
        
        let body = serde_json::json!({
            "path": path
        });

        let response = self.client
            .post(&url)
            .headers(self.headers())
            .json(&body)
            .send()
            .await?;

        let resp: AlistResponse<serde_json::Value> = response.json().await?;
        
        if resp.code == 200 {
            if let Some(data) = resp.data {
                if let Some(content) = data.get("content") {
                    if let Some(files) = content.as_array() {
                        return Ok(files.iter().any(|f| {
                            f.get("name")
                                .and_then(|n| n.as_str())
                                .map(|n| n == filename)
                                .unwrap_or(false)
                        }));
                    }
                }
            }
            Ok(false)
        } else {
            // 404 可能意味着目录不存在，文件肯定不存在
            Ok(false)
        }
    }

    /// 设置 AList 服务端上传限速（控制 AList → 云盘的上传速度）
    /// kb_per_sec: KB/s，-1 表示不限速
    pub async fn set_server_upload_limit(&self, kb_per_sec: i64) -> Result<(), AlistError> {
        let url = format!("{}/api/admin/setting/save", self.base_url.trim_end_matches('/'));
        log(&format!("设置 AList 服务端上传限速: {} KB/s", if kb_per_sec < 0 { "不限速".to_string() } else { kb_per_sec.to_string() }));

        let body = serde_json::json!([{
            "key": "max_server_upload_speed",
            "value": kb_per_sec.to_string()
        }]);

        let response = self.client
            .post(&url)
            .headers(self.headers())
            .json(&body)
            .timeout(Duration::from_secs(10))
            .send()
            .await?;

        let status = response.status();
        let response_text = response.text().await.map_err(|e| {
            log(&format!("读取限速设置响应失败: status={}, error={}", status, e));
            AlistError::Api(format!("读取响应失败: {}", e))
        })?;

        log(&format!("限速设置响应: status={}, body_length={}", status, response_text.len()));

        let resp: AlistResponse<serde_json::Value> = serde_json::from_str(&response_text).map_err(|e| {
            log(&format!("解析限速设置响应失败: error={}, body={}", e, response_text));
            AlistError::Api(format!("解析响应失败: {}", e))
        })?;

        if resp.code == 200 {
            log("AList 服务端上传限速设置成功");
            Ok(())
        } else {
            log(&format!("AList 服务端上传限速设置失败: code={}, message={}", resp.code, resp.message));
            Err(AlistError::Api(format!("限速设置失败: {}", resp.message)))
        }
    }

    pub async fn upload_file(
        &self,
        file_path: &str,
        alist_path: &str,
        as_task: bool,
        upload_method: &str,
        rate_limiter: Option<Arc<RateLimiter>>,
    ) -> Result<Option<String>, AlistError> {
        if is_root_alist_path(alist_path) {
            log(&format!("Alist 上传请求被拦截: file_path={}, alist_path=/, reason=根目录不是具体上传目录", file_path));
            return Err(AlistError::Api("上传目标目录不能为根目录 /，请选择 Alist 中的具体目录".to_string()));
        }

        let file_name = file_name_from_path(file_path);
        let target_path = join_alist_path(alist_path, &file_name);
        log(&format!("打开文件准备上传: file_path={}, file_name={}", file_path, file_name));
        
        let file = tokio::fs::File::open(file_path).await?;
        let file_len = file.metadata().await?.len();
        let speed_limit_label = rate_limiter.as_ref().map_or("unlimited".to_string(), |_| "limited".to_string());
        log(&format!("文件已打开: file_name={}, size={}B, target_path={}, as_task={}, method={}, speed_limit={}", file_name, file_len, target_path, as_task, upload_method, speed_limit_label));

        let mut headers = self.headers();
        headers.insert("File-Path", target_path.parse().unwrap());
        // 云端已存在同名文件时，OpenList 返回 403 "file exists"，不传输（详见 OpenList fsup.go）
        headers.insert("Overwrite", "false".parse().unwrap());
        if as_task {
            headers.insert("As-Task", "true".parse().unwrap());
        }

        let url;
        let response;

        if upload_method == "form" {
            // 表单上传
            url = format!("{}/api/fs/form", self.base_url);
            let file_part = multipart::Part::stream_with_length(
                reqwest::Body::wrap_stream(Self::rate_limited_stream(file, rate_limiter)),
                file_len,
            ).file_name(file_name.clone());
            let form = multipart::Form::new().part("file", file_part);

            log(&format!("发送表单上传请求: url={}, file_name={}, size={}B", url, file_name, file_len));
            response = self.client
                .put(&url)
                .headers(headers)
                .multipart(form)
                .send()
                .await;
        } else {
            // 默认流式上传
            url = format!("{}/api/fs/put", self.base_url);
            headers.insert("Content-Length", file_len.to_string().parse().unwrap());
            let body = reqwest::Body::wrap_stream(Self::rate_limited_stream(file, rate_limiter));

            log(&format!("发送流式上传请求: url={}, file_name={}, size={}B", url, file_name, file_len));
            response = self.client
                .put(&url)
                .headers(headers)
                .body(body)
                .send()
                .await;
        }

        let response = response.map_err(|e| {
            log(&format!("上传请求失败: file_name={}, error={}", file_name, e));
            e
        })?;
        
        let status = response.status();
        log(&format!("上传响应: file_name={}, status={}", file_name, status));

        let raw_body = response.text().await.map_err(|e| {
            log(&format!("读取上传响应失败: file_name={}, error={}", file_name, e));
            AlistError::Request(e)
        })?;

        let resp: AlistResponse<serde_json::Value> = serde_json::from_str(&raw_body).map_err(|e| {
            let raw = raw_body.clone();
            log(&format!("解析上传响应失败: file_name={}, error={}, raw={}", file_name, e, raw));
            AlistError::ApiWithResponse {
                message: format!("解析响应失败: {}", e),
                raw,
            }
        })?;

        log(&format!("上传响应解析: file_name={}, code={}, message={}", file_name, resp.code, resp.message));

        if resp.code == 200 {
            log(&format!("上传请求已提交: file_name={}, as_task={}", file_name, as_task));
            if let Some(data) = &resp.data {
                if let Some(task) = data.get("task") {
                    let task_id = task.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    let task_name = task.get("name").and_then(|v| v.as_str()).unwrap_or("unknown");
                    log(&format!("后台任务已创建: task_id={}, task_name={}", task_id, task_name));
                    if !task_id.is_empty() {
                        return Ok(Some(task_id.to_string()));
                    }
                }
            }
            Ok(None)
        } else if resp.message.contains("file exists") {
            // Overwrite: false 时服务端检测到云端已有同名文件（HTTP 403）
            log(&format!("云端已存在同名文件，跳过传输: file_name={}, target={}", file_name, target_path));
            Err(AlistError::FileExists)
        } else {
            log(&format!("上传失败: file_name={}, code={}, message={}, raw={}", file_name, resp.code, resp.message, raw_body));
            Err(AlistError::ApiWithResponse {
                message: format!("上传失败: code={}, message={}", resp.code, resp.message),
                raw: raw_body.clone(),
            })
        }
    }

    fn rate_limited_stream(
        file: tokio::fs::File,
        rate_limiter: Option<Arc<RateLimiter>>,
    ) -> Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>> {
        let stream = tokio_util::io::ReaderStream::new(file);
        let Some(rate_limiter) = rate_limiter else {
            return Box::pin(stream);
        };

        let limited = stream.then(move |chunk| {
            let rate_limiter = Arc::clone(&rate_limiter);
            async move {
                if let Ok(ref bytes) = chunk {
                    rate_limiter.wait_for_bytes(bytes.len() as u64).await;
                }
                chunk
            }
        });

        Box::pin(limited)
    }

    pub async fn get_upload_tasks(&self) -> Result<Vec<AlistTaskResp>, AlistError> {
        let url = format!("{}/api/task/upload/undone", self.base_url.trim_end_matches('/'));
        log(&format!("查询 Alist 未完成上传任务: url={}", url));

        let response = self.client
            .get(&url)
            .headers(self.headers())
            .send()
            .await?;

        let status = response.status();
        let response_text = response.text().await.map_err(|e| {
            log(&format!("读取 Alist 未完成上传任务响应失败: status={}, error={}", status, e));
            AlistError::Api(format!("读取任务列表响应失败: {}", e))
        })?;

        log(&format!("Alist 未完成上传任务响应: status={}, body_length={}", status, response_text.len()));

        let resp: AlistResponse<serde_json::Value> = serde_json::from_str(&response_text).map_err(|e| {
            log(&format!("解析 Alist 未完成上传任务响应失败: status={}, error={}, body={}", status, e, response_text));
            AlistError::Api(format!("解析任务列表响应失败: {}; 原始响应: {}", e, response_text))
        })?;

        if resp.code == 200 {
            let Some(data) = resp.data else {
                return Ok(Vec::new());
            };

            if data.is_array() {
                serde_json::from_value(data).map_err(|e| {
                    log(&format!("解析 Alist 未完成上传任务数组失败: error={}", e));
                    AlistError::Api(format!("解析任务数组失败: {}", e))
                })
            } else if let Some(tasks) = data.get("tasks") {
                serde_json::from_value(tasks.clone()).map_err(|e| {
                    log(&format!("解析 Alist 未完成上传任务 tasks 字段失败: error={}", e));
                    AlistError::Api(format!("解析任务数组失败: {}", e))
                })
            } else {
                log(&format!("Alist 未完成上传任务响应 data 格式未知: data={}", data));
                Ok(Vec::new())
            }
        } else {
            Err(AlistError::Api(resp.message))
        }
    }

    /// 查询全部上传任务（含已失败），用于任务从 undone 消失后查错误信息
    pub async fn get_all_upload_tasks(&self) -> Result<Vec<AlistTaskResp>, AlistError> {
        let url = format!("{}/api/task/upload", self.base_url.trim_end_matches('/'));
        log(&format!("查询 Alist 全部上传任务: url={}", url));

        let response = self.client
            .get(&url)
            .headers(self.headers())
            .send()
            .await?;

        let status = response.status();
        let response_text = response.text().await.map_err(|e| {
            log(&format!("读取 Alist 全部上传任务响应失败: status={}, error={}", status, e));
            AlistError::Api(format!("读取任务列表响应失败: {}", e))
        })?;

        log(&format!("Alist 全部上传任务响应: status={}, body_length={}", status, response_text.len()));

        let resp: AlistResponse<serde_json::Value> = serde_json::from_str(&response_text).map_err(|e| {
            log(&format!("解析 Alist 全部上传任务响应失败: status={}, error={}, body={}", status, e, response_text));
            AlistError::Api(format!("解析任务列表响应失败: {}; 原始响应: {}", e, response_text))
        })?;

        if resp.code == 200 {
            let Some(data) = resp.data else {
                return Ok(Vec::new());
            };
            if data.is_array() {
                serde_json::from_value(data).map_err(|e| {
                    AlistError::Api(format!("解析任务数组失败: {}", e))
                })
            } else if let Some(tasks) = data.get("tasks") {
                serde_json::from_value(tasks.clone()).map_err(|e| {
                    AlistError::Api(format!("解析任务数组失败: {}", e))
                })
            } else {
                Ok(Vec::new())
            }
        } else {
            Err(AlistError::Api(resp.message))
        }
    }

    pub async fn get_task_progress(&self, task_id: &str) -> Result<AlistTaskResp, AlistError> {
        let tasks = self.get_upload_tasks().await?;
        
        tasks.into_iter()
            .find(|t| t.id == task_id)
            .ok_or_else(|| AlistError::Api(format!("任务 {} 未找到", task_id)))
    }

    pub async fn list_directory(&self, path: &str) -> Result<Vec<DirItem>, AlistError> {
        let url = format!("{}/api/fs/list", self.base_url.trim_end_matches('/'));
        log(&format!("请求 Alist 目录列表: url={}, path={}, has_token={}", url, path, !self.token.is_empty()));
        
        let body = serde_json::json!({
            "path": path
        });

        let response = self.client
            .post(&url)
            .headers(self.headers())
            .json(&body)
            .timeout(Duration::from_secs(10))
            .send()
            .await?;

        let status = response.status();
        let response_text = response.text().await.map_err(|e| {
            log(&format!("读取 Alist 目录列表响应失败: status={}, error={}", status, e));
            AlistError::Api(format!("读取目录响应失败: {}", e))
        })?;
        log(&format!("Alist 目录列表响应: status={}, body_length={}", status, response_text.len()));

        if !status.is_success() {
            log(&format!("Alist 目录列表 HTTP 状态失败: status={}, body={}", status, response_text));
            return Err(AlistError::Api(format!("目录列表请求失败: HTTP {}", status)));
        }

        let resp: AlistResponse<ListResp> = serde_json::from_str(&response_text).map_err(|e| {
            log(&format!("解析 Alist 目录列表响应失败: status={}, error={}, body={}", status, e, response_text));
            AlistError::Api(format!("解析目录列表响应失败: {}; 原始响应: {}", e, response_text))
        })?;
        
        log(&format!("Alist 目录列表解析结果: code={}, message={}, has_data={}", resp.code, resp.message, resp.data.is_some()));
        if resp.code == 200 {
            let content = resp
                .data
                .and_then(|d| d.content)
                .unwrap_or_default();
            log(&format!("Alist 目录列表内容: path={}, item_count={}", path, content.len()));
            Ok(content)
        } else {
            Err(AlistError::Api(resp.message))
        }
    }

    /// 强制刷新目录（refresh=true 穿透缓存），触发 OpenList 增量索引更新
    pub async fn refresh_directory(&self, path: &str) -> Result<(), AlistError> {
        let url = format!("{}/api/fs/list", self.base_url.trim_end_matches('/'));
        log(&format!("刷新目录触发增量索引: path={}", path));

        let body = serde_json::json!({
            "path": path,
            "refresh": true,
            "page": 1,
            "per_page": 1
        });

        let response = self.client
            .post(&url)
            .headers(self.headers())
            .json(&body)
            .timeout(Duration::from_secs(30))
            .send()
            .await?;

        let status = response.status();
        let response_text = response.text().await.map_err(|e| {
            log(&format!("读取刷新目录响应失败: status={}, error={}", status, e));
            AlistError::Api(format!("读取刷新目录响应失败: {}", e))
        })?;

        let resp: AlistResponse<serde_json::Value> = serde_json::from_str(&response_text)
            .unwrap_or(AlistResponse { code: -1, message: response_text.clone(), data: None });

        if resp.code == 200 {
            log(&format!("刷新目录成功（增量索引已触发）: path={}", path));
            Ok(())
        } else {
            log(&format!("刷新目录失败: path={}, code={}, message={}", path, resp.code, resp.message));
            Err(AlistError::Api(resp.message))
        }
    }

    pub async fn mkdir(&self, path: &str) -> Result<(), AlistError> {        let url = format!("{}/api/fs/mkdir", self.base_url.trim_end_matches('/'));
        log(&format!("请求 Alist 创建目录: url={}, path={}, has_token={}", url, path, !self.token.is_empty()));

        let body = serde_json::json!({
            "path": path
        });

        let response = self.client
            .post(&url)
            .headers(self.headers())
            .json(&body)
            .timeout(Duration::from_secs(10))
            .send()
            .await?;

        let status = response.status();
        let response_text = response.text().await.map_err(|e| {
            log(&format!("读取 Alist 创建目录响应失败: status={}, error={}", status, e));
            AlistError::Api(format!("读取创建目录响应失败: {}", e))
        })?;

        log(&format!("Alist 创建目录响应: status={}, body={}", status, response_text));

        let resp: AlistResponse<serde_json::Value> = serde_json::from_str(&response_text).unwrap_or(AlistResponse {
            code: -1,
            message: response_text.clone(),
            data: None,
        });

        if resp.code == 200 || resp.message.contains("exist") {
            log(&format!("Alist 目录创建成功或已存在: path={}", path));
            Ok(())
        } else {
            Err(AlistError::Api(format!("创建目录失败: {}", resp.message)))
        }
    }
}

fn is_root_alist_path(path: &str) -> bool {
    let trimmed = path.trim().trim_end_matches('/');
    trimmed.is_empty()
}

fn join_alist_path(dir: &str, file_name: &str) -> String {
    let normalized_dir = match dir.trim() {
        "" | "/" => "/".to_string(),
        value if value.starts_with('/') => value.trim_end_matches('/').to_string(),
        value => format!("/{}", value.trim_end_matches('/')),
    };

    if normalized_dir == "/" {
        format!("/{}", file_name.trim_start_matches('/'))
    } else {
        format!("{}/{}", normalized_dir, file_name.trim_start_matches('/'))
    }
}

fn file_name_from_path(file_path: &str) -> String {
    file_path
        .rsplit(|c| c == '/' || c == '\\')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or("unknown")
        .to_string()
}

#[derive(Debug, serde::Deserialize, Default)]
pub struct ListResp {
    #[serde(default)]
    pub content: Option<Vec<DirItem>>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DirItem {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub is_dir: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sign: Option<String>,
}
