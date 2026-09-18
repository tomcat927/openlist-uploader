use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    Uploading,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileInfo {
    pub path: String,
    pub name: String,
    pub size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relative_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadTask {
    pub id: String,
    pub file: FileInfo,
    pub alist_path: String,
    pub status: TaskStatus,
    pub progress: u8,
    pub retry_count: u32,
    pub error: Option<String>,
    pub start_time: Option<DateTime<Utc>>,
    pub end_time: Option<DateTime<Utc>>,
    pub duration: Option<u64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// 上传速度（字节/秒），仅 status=uploading 时有效，前端用于显示
    #[serde(default)]
    pub speed: u64,
    /// 用户已手动处理（如已通过网页等其他方式补传成功），仅失败记录展示用
    #[serde(default)]
    pub resolved: bool,
    /// 上传失败时 OpenList API 的完整返回值（原始 JSON body），用于排查问题
    #[serde(default)]
    pub api_response: Option<String>,
    /// 上一次轮询的进度百分比（0.0-100.0），仅内存使用，不持久化
    #[serde(skip)]
    pub prev_progress: f64,
    /// 上一次轮询的时间戳，仅内存使用，不持久化
    #[serde(skip)]
    pub prev_ts: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddToQueueResult {
    pub tasks: Vec<UploadTask>,
    pub warnings: Vec<String>,
}

impl UploadTask {
    pub fn new(file_path: String, alist_path: String) -> Self {
        let file_name = file_path
            .split('/')
            .last()
            .or_else(|| file_path.split('\\').last())
            .unwrap_or("unknown")
            .to_string();

        Self {
            id: Uuid::new_v4().to_string(),
            file: FileInfo {
                path: file_path,
                name: file_name,
                size: 0,
                relative_path: None,
            },
            alist_path,
            status: TaskStatus::Pending,
            progress: 0,
            retry_count: 0,
            error: None,
            start_time: None,
            end_time: None,
            duration: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            speed: 0,
            resolved: false,
            api_response: None,
            prev_progress: 0.0,
            prev_ts: None,
        }
    }

    pub fn update_status(&mut self, status: TaskStatus) {
        self.status = status;
        self.updated_at = Utc::now();
    }

    pub fn mark_uploading(&mut self) {
        self.status = TaskStatus::Uploading;
        self.start_time = Some(Utc::now());
        self.updated_at = Utc::now();
        self.error = None;
        self.speed = 0;
        self.prev_progress = 0.0;
        self.prev_ts = None;
    }

    pub fn mark_completed(&mut self) {
        self.status = TaskStatus::Completed;
        self.end_time = Some(Utc::now());
        self.progress = 100;
        self.updated_at = Utc::now();
        self.speed = 0;
        if let Some(start) = self.start_time {
            self.duration = Some((Utc::now() - start).num_seconds() as u64);
        }
    }

    pub fn mark_failed(&mut self, error: String) {
        self.status = TaskStatus::Failed;
        self.end_time = Some(Utc::now());
        self.error = Some(error);
        self.updated_at = Utc::now();
        self.speed = 0;
        if let Some(start) = self.start_time {
            self.duration = Some((Utc::now() - start).num_seconds() as u64);
        }
    }

    pub fn increment_retry(&mut self) {
        self.retry_count += 1;
        self.status = TaskStatus::Pending;
        self.updated_at = Utc::now();
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlistConfig {
    pub base_url: String,
    pub token: String,
    pub username: String,
    pub password: String,
    #[serde(default = "default_auto_login")]
    pub auto_login: bool,
    /// Alist 可执行文件路径，填写后启动本软件时自动启动 Alist
    #[serde(default)]
    pub exe_path: String,
    /// 退出本软件时同时关闭 Alist 进程
    #[serde(default = "default_true")]
    pub kill_on_exit: bool,
    /// 后台启动 Alist（隐藏控制台窗口），默认开启
    #[serde(default = "default_true")]
    pub run_in_background: bool,
    /// 是否使用系统代理（默认关闭，本地通信通常不需要代理）
    #[serde(default)]
    pub use_system_proxy: bool,
}

impl Default for AlistConfig {
    fn default() -> Self {
        Self {
            base_url: "http://127.0.0.1:5244".to_string(),
            token: String::new(),
            username: String::new(),
            password: String::new(),
            auto_login: true,
            exe_path: String::new(),
            kill_on_exit: true,
            run_in_background: true,
            use_system_proxy: false,
        }
    }
}

fn default_auto_login() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileExistsStrategy {
    #[serde(rename = "strategy")]
    pub value: String, // "ask", "overwrite", "skip", "rename"
}

impl Default for FileExistsStrategy {
    fn default() -> Self {
        Self {
            value: "ask".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledUpload {
    pub enabled: bool,
    pub start_time: String, // "HH:MM" format
    pub end_time: String,   // "HH:MM" format
    #[serde(default)]
    pub notify_on_start: bool,
    #[serde(default)]
    pub notify_on_stop: bool,
}

impl Default for ScheduledUpload {
    fn default() -> Self {
        Self {
            enabled: false,
            start_time: "03:00".to_string(),
            end_time: "07:00".to_string(),
            notify_on_start: false,
            notify_on_stop: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadConfig {
    pub concurrency: u8,
    pub max_retries: u32,
    /// 上传限速（字节/秒），0 表示不限速
    #[serde(default)]
    pub speed_limit: u64,
    pub as_task: bool,
    #[serde(default = "default_upload_method")]
    pub upload_method: String,
    #[serde(default = "default_alist_path")]
    pub last_alist_path: String,
    #[serde(default = "default_true")]
    pub block_files_over_5gb: bool,
    #[serde(default = "default_true")]
    pub warn_files_over_4gb: bool,
    pub file_exists_strategy: FileExistsStrategy,
    /// 上传失败后行为："stop"（停止队列，默认）或 "skip"（跳过继续）
    #[serde(default = "default_fail_action")]
    pub fail_action: String,
    pub show_progress: bool,
    /// 拦截同一文件重复添加到不同目标路径（默认开启）
    #[serde(default = "default_true")]
    pub block_duplicate_file_upload: bool,
   #[serde(default)]
   pub notify_on_complete: bool,
   #[serde(default)]
   pub notify_feishu_on_queue_complete: bool,
   #[serde(default)]
   pub shutdown_after_complete: bool,
   #[serde(default = "default_shutdown_delay_minutes")]
   pub shutdown_delay_minutes: u32,
   #[serde(default)]
   pub minimize_on_close: bool,
    /// 每轮上传任务数上限，0 表示不限
    #[serde(default)]
    pub max_tasks_per_run: u32,
    /// 启动时自动检查更新（默认开启）
    #[serde(default = "default_true")]
    pub check_update_on_startup: bool,
    /// 开机自启动（默认关闭）
    #[serde(default)]
    pub auto_start_on_boot: bool,
    /// 上传期间定时发送进度通知（默认关闭）
    #[serde(default)]
    pub progress_notify_enabled: bool,
    /// 进度通知间隔（分钟，默认 30）
    #[serde(default = "default_progress_notify_interval")]
    pub progress_notify_interval: u32,
    /// 上传成功后刷新目标目录触发 OpenList 增量索引（默认开启）
    #[serde(default = "default_true")]
    pub refresh_index_after_upload: bool,
    /// WinRAR (rar.exe) 路径，用于大文件分卷压缩
    #[serde(default = "default_rar_path")]
    pub rar_path: String,
    /// 分卷压缩每卷大小（MB，默认 2000）
    #[serde(default = "default_volume_mb")]
    pub split_volume_mb: u64,
    pub schedule: Option<ScheduledUpload>,
    pub notification: Option<NotificationConfig>,
}

fn default_progress_notify_interval() -> u32 {
    30
}

fn default_rar_path() -> String {
    "C:\\Program Files\\WinRAR\\rar.exe".to_string()
}

fn default_volume_mb() -> u64 {
    2000
}

fn default_upload_method() -> String {
    "stream".to_string()
}

fn default_shutdown_delay_minutes() -> u32 {
    10
}

fn default_alist_path() -> String {
    "/".to_string()
}

fn default_true() -> bool {
    true
}

fn default_fail_action() -> String {
    "stop".to_string()
}

impl Default for UploadConfig {
    fn default() -> Self {
        Self {
            concurrency: 1,
            max_retries: 5,
            speed_limit: 0,
            as_task: true,
            upload_method: "stream".to_string(),
            last_alist_path: "/".to_string(),
            block_files_over_5gb: true,
            warn_files_over_4gb: true,
            file_exists_strategy: FileExistsStrategy::default(),
            fail_action: "stop".to_string(),
            show_progress: false,
            block_duplicate_file_upload: true,
           notify_on_complete: false,
           notify_feishu_on_queue_complete: false,
           shutdown_after_complete: false,
           shutdown_delay_minutes: 10,
           minimize_on_close: true,
            max_tasks_per_run: 0,
            check_update_on_startup: true,
            auto_start_on_boot: false,
            progress_notify_enabled: false,
            progress_notify_interval: 30,
            refresh_index_after_upload: true,
            rar_path: default_rar_path(),
            split_volume_mb: default_volume_mb(),
            schedule: Some(ScheduledUpload::default()),
            notification: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationConfig {
    pub enabled: bool,
    pub webhook_url: String,
    pub channels: Vec<String>, // "feishu", "dingtalk", etc.
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            webhook_url: String::new(),
            channels: vec!["feishu".to_string()],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockedFileRecord {
    pub file_path: String,
    pub file_name: String,
    pub file_size: u64,
    pub reason: String,
    pub blocked_at: DateTime<Utc>,
    /// openlist 目标路径（拦截时用户选择的目录）
    #[serde(default)]
    pub target_path: String,
    /// 用户已标记为已分卷处理/已解决
    #[serde(default)]
    pub resolved: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockedFileData {
    pub records: Vec<BlockedFileRecord>,
}

impl Default for BlockedFileData {
    fn default() -> Self {
        Self { records: vec![] }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryConfig {
    #[serde(default = "default_history_retention_days")]
    pub retention_days: u32,
    /// 永久保留历史记录（不按天数清理）
    #[serde(default = "default_true")]
    pub never_clean: bool,
}

fn default_history_retention_days() -> u32 {
    30
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self { retention_days: 30, never_clean: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogSyncConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub token: String,
    #[serde(default = "default_log_sync_target_path")]
    pub target_path: String,
    #[serde(default = "default_true")]
    pub sync_on_exit: bool,
    /// 定时同步间隔（分钟），0 表示关闭定时同步
    #[serde(default = "default_log_sync_interval")]
    pub sync_interval_minutes: u32,
    #[serde(default)]
    pub use_system_proxy: bool,
    /// 最近一次同步成功时间（RFC3339），用于前端展示
    #[serde(default)]
    pub last_sync_at: Option<String>,
}

fn default_log_sync_interval() -> u32 {
    30
}

fn default_log_sync_target_path() -> String {
    "/本地磁盘/openlist-uploader-logs".to_string()
}

impl Default for LogSyncConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            base_url: String::new(),
            username: String::new(),
            password: String::new(),
            token: String::new(),
            target_path: default_log_sync_target_path(),
            sync_on_exit: true,
            sync_interval_minutes: default_log_sync_interval(),
            use_system_proxy: false,
            last_sync_at: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalLogFileInfo {
    pub name: String,
    pub size: u64,
    pub modified: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogSyncResult {
    pub total: usize,
    pub success: usize,
    pub failed: usize,
    pub details: Vec<String>,
    /// 最近一次同步成功时间（RFC3339），None 表示从未成功同步
    #[serde(default)]
    pub last_sync_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryPage {
    pub tasks: Vec<UploadTask>,
    pub total: usize,
    pub page: usize,
    pub page_size: usize,
    pub total_pages: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub alist: AlistConfig,
    pub upload: UploadConfig,
    pub history: HistoryConfig,
    #[serde(default)]
    pub log_sync: LogSyncConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            alist: AlistConfig::default(),
            upload: UploadConfig::default(),
            history: HistoryConfig::default(),
            log_sync: LogSyncConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueData {
    pub tasks: Vec<UploadTask>,
    pub version: u32,
}

impl Default for QueueData {
    fn default() -> Self {
        Self {
            tasks: Vec::new(),
            version: 1,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryData {
    pub records: Vec<UploadTask>,
    pub version: u32,
}

impl Default for HistoryData {
    fn default() -> Self {
        Self {
            records: Vec::new(),
            version: 1,
        }
    }
}
