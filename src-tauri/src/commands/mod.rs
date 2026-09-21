use tauri::{State, Emitter};
use crate::models::*;
use crate::services::queue_manager::QueueManager;
use crate::services::alist_client::AlistClient;
use crate::utils::storage::Storage;
use crate::utils::log::log;
use std::sync::Arc;
use tokio::sync::Mutex;

/// 分卷压缩串行队列：同一时间只允许一个 rar.exe 进程（CPU/IO 密集，并发反而更慢）
static SPLIT_COMPRESS_MUTEX: std::sync::OnceLock<Arc<Mutex<()>>> = std::sync::OnceLock::new();

/// 压缩事件载荷：带文件路径标识，前端按记录分别显示进度
#[derive(Clone, serde::Serialize)]
struct CompressEvent {
    file_path: String,
    percent: u8,
}

fn split_compress_lock() -> Arc<Mutex<()>> {
    SPLIT_COMPRESS_MUTEX.get_or_init(|| Arc::new(Mutex::new(()))).clone()
}

#[tauri::command]
pub async fn get_queue(queue_manager: State<'_, QueueManager>) -> Result<Vec<UploadTask>, String> {
    let queue = queue_manager.queue.read().await;
    Ok(queue.tasks.clone())
}

#[tauri::command]
pub async fn add_to_queue(
    queue_manager: State<'_, QueueManager>,
    file_path: String,
    alist_path: String,
) -> Result<AddToQueueResult, String> {
    log(&format!("添加文件到队列: file_path={}, alist_path={}", file_path, alist_path));
    let result = queue_manager
        .add_to_queue(file_path, alist_path)
        .await
        .map_err(|e| e.to_string())?;
    log(&format!("添加到队列完成: 共 {} 个任务, warning_count={}", result.tasks.len(), result.warnings.len()));
    Ok(result)
}

#[tauri::command]
pub async fn remove_from_queue(
    queue_manager: State<'_, QueueManager>,
    task_id: String,
) -> Result<(), String> {
    queue_manager
        .remove_from_queue(task_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn clear_queue(queue_manager: State<'_, QueueManager>) -> Result<(), String> {
    queue_manager
        .clear_queue()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_history(queue_manager: State<'_, QueueManager>) -> Result<Vec<UploadTask>, String> {
    Ok(queue_manager.get_history().await)
}

#[tauri::command]
pub async fn get_history_page(
    queue_manager: State<'_, QueueManager>,
    page: usize,
    page_size: usize,
    status_filter: Option<String>,
    search_text: Option<String>,
    sort_order: Option<String>,
) -> Result<HistoryPage, String> {
    let history = queue_manager.history.read().await;
    let mut records: Vec<UploadTask> = history.records.clone();
    drop(history);

    if let Some(ref filter) = status_filter {
        if filter != "all" {
            records.retain(|t| filter == "completed" && t.status == TaskStatus::Completed
                || filter == "failed" && t.status == TaskStatus::Failed);
        }
    }

    if let Some(ref text) = search_text {
        let q = text.trim().to_lowercase();
        if !q.is_empty() {
            records.retain(|t| t.file.name.to_lowercase().contains(&q));
        }
    }

    records.sort_by(|a, b| {
        let ta = a.end_time.unwrap_or(a.created_at);
        let tb = b.end_time.unwrap_or(b.created_at);
        if sort_order.as_deref() == Some("asc") { ta.cmp(&tb) } else { tb.cmp(&ta) }
    });

    let total = records.len();
    let page_size = page_size.max(1);
    let total_pages = (total + page_size - 1) / page_size;
    let start = page.saturating_sub(1) * page_size;
    let tasks: Vec<UploadTask> = records.into_iter().skip(start).take(page_size).collect();

    Ok(HistoryPage { tasks, total, page, page_size, total_pages })
}

#[tauri::command]
pub async fn resolve_history_task(
    queue_manager: State<'_, QueueManager>,
    task_id: String,
) -> Result<(), String> {
    let mut history = queue_manager.history.write().await;
    let record = history
        .records
        .iter_mut()
        .find(|r| r.id == task_id)
        .ok_or_else(|| format!("历史记录不存在: {}", task_id))?;
    record.resolved = true;
    record.updated_at = chrono::Utc::now();
    crate::utils::storage::Storage::save_history(&*history).map_err(|e| e.to_string())?;
    log(&format!("历史记录已标记为已处理: task_id={}", task_id));
    Ok(())
}

#[tauri::command]
pub async fn clear_history(queue_manager: State<'_, QueueManager>) -> Result<(), String> {
    queue_manager
        .clear_history()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_config() -> Result<AppConfig, String> {
    let config = Storage::load_config().map_err(|e| {
        log(&format!("读取磁盘配置失败: {}", e));
        e.to_string()
    })?;
    log(&format!("读取磁盘配置: base_url={}, username={}, has_token={}, password_length={}, auto_login={}, exe_path={}, kill_on_exit={}, last_alist_path={}", config.alist.base_url, config.alist.username, !config.alist.token.is_empty(), config.alist.password.len(), config.alist.auto_login, config.alist.exe_path, config.alist.kill_on_exit, config.upload.last_alist_path));
    Ok(config)
}

#[tauri::command]
pub async fn save_config(queue_manager: State<'_, QueueManager>, config: AppConfig) -> Result<(), String> {
    log(&format!("收到保存配置请求: base_url={}, username={}, has_token={}, password_length={}, auto_login={}, last_alist_path={}", config.alist.base_url, config.alist.username, !config.alist.token.is_empty(), config.alist.password.len(), config.alist.auto_login, config.upload.last_alist_path));
    queue_manager.save_config(config).await.map_err(|e| {
        log(&format!("保存配置失败: {}", e));
        e.to_string()
    })?;
    log("配置保存成功");
    Ok(())
}

#[tauri::command]
pub async fn start_upload(queue_manager: State<'_, QueueManager>) -> Result<(), String> {
    log("收到开始上传请求");
    let scheduler = 
        crate::services::upload_scheduler::UploadScheduler::new(queue_manager.inner().clone_inner());
    
    tokio::spawn(async move {
        scheduler.start_scheduler().await;
    });

    Ok(())
}

#[tauri::command]
pub fn get_is_uploading(queue_manager: State<'_, QueueManager>) -> bool {
    queue_manager.is_uploading()
}

#[tauri::command]
pub async fn pause_upload(queue_manager: State<'_, QueueManager>) -> Result<(), String> {
    queue_manager.set_stop_after_current(true);
    Ok(())
}

#[tauri::command]
pub async fn stop_after_current(queue_manager: State<'_, QueueManager>) -> Result<(), String> {
    queue_manager.set_stop_after_current(true);
    Ok(())
}

#[tauri::command]
pub async fn retry_upload(
    queue_manager: State<'_, QueueManager>,
    task_id: String,
) -> Result<(), String> {
    let mut queue = queue_manager.queue.write().await;
    if let Some(task) = queue.tasks.iter_mut().find(|t| t.id == task_id) {
        task.status = TaskStatus::Pending;
        task.retry_count = 0;
        task.error = None;
        task.progress = 0;
        task.updated_at = chrono::Utc::now();
    }
    
    crate::utils::storage::Storage::save_queue(&*queue)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn test_alist_connection(config: AppConfig) -> Result<bool, String> {
    log(&format!("收到测试连接请求: base_url={}, username={}, has_token={}", config.alist.base_url, config.alist.username, !config.alist.token.is_empty()));

    let client = AlistClient::new(config.alist.base_url, config.alist.token, config.alist.use_system_proxy);

    let result = client
        .test_connection()
        .await
        .map_err(|e| {
            log(&format!("测试连接异常: {}", e));
            e.to_string()
        })?;

    log(&format!("测试连接结果: {}", result));
    Ok(result)
}

#[tauri::command]
pub async fn get_file_info(path: String) -> Result<FileInfo, String> {
    let (size, name) = crate::utils::fs::get_file_info(&path)
        .await
        .map_err(|e| e.to_string())?;

    Ok(FileInfo { path, name, size, relative_path: None })
}

#[tauri::command]
pub async fn get_data_path() -> Result<String, String> {
    Ok(Storage::get_data_path()
        .to_string_lossy()
        .to_string())
}

#[tauri::command]
pub async fn get_log_dir() -> Result<String, String> {
    let mut path = dirs::data_local_dir()
        .ok_or_else(|| "无法获取本地数据目录".to_string())?;
    path.push("openlist-uploader");
    if !path.exists() {
        std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
    }
    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn check_health(config: AppConfig) -> Result<bool, String> {
    log(&format!("收到服务健康检查请求: base_url={}", config.alist.base_url));

    let client = AlistClient::new(config.alist.base_url, config.alist.token, config.alist.use_system_proxy);

    let result = client
        .check_service_available()
        .await
        .map_err(|e| {
            log(&format!("服务健康检查异常: {}", e));
            e.to_string()
        })?;

    log(&format!("服务健康检查结果: {}", result));
    Ok(result)
}

#[tauri::command]
pub async fn alist_login(
    queue_manager: State<'_, QueueManager>,
    base_url: String,
    username: String,
    password: String,
) -> Result<String, String> {
    let normalized_base_url = base_url.trim_end_matches('/').to_string();
    log(&format!("开始 Alist 登录流程: base_url={}, username={}, password_length={}", normalized_base_url, username, password.len()));

    if normalized_base_url.is_empty() {
        log("Alist 登录失败: 服务地址为空");
        return Err("服务地址不能为空".to_string());
    }
    if username.trim().is_empty() {
        log("Alist 登录失败: 用户名为空");
        return Err("用户名不能为空".to_string());
    }
    if password.is_empty() {
        log("Alist 登录失败: 密码为空");
        return Err("密码不能为空".to_string());
    }

    let config = queue_manager.config.read().await;
    let use_proxy = config.alist.use_system_proxy;
    drop(config);

    let client = AlistClient::new(normalized_base_url.clone(), String::new(), use_proxy);

    let token = client
        .login(&username, &password)
        .await
        .map_err(|e| {
            log(&format!("Alist 登录请求失败: base_url={}, username={}, error={}", normalized_base_url, username, e));
            e.to_string()
        })?;

    log(&format!("Alist 登录成功: username={}, token_length={}", username, token.len()));

    let mut config = Storage::load_config().map_err(|e| {
        log(&format!("读取配置失败，无法保存登录信息: {}", e));
        e.to_string()
    })?;

    config.alist.base_url = normalized_base_url.clone();
    config.alist.token = token.clone();
    config.alist.username = username.clone();
    config.alist.password = password;

    queue_manager.save_config(config.clone()).await.map_err(|e| {
        log(&format!("保存登录配置失败: base_url={}, username={}, has_token={}, error={}", normalized_base_url, username, !token.is_empty(), e));
        e.to_string()
    })?;

    log(&format!("登录配置已持久化: base_url={}, username={}, has_token={}, password_length={}", normalized_base_url, username, !token.is_empty(), config.alist.password.len()));
    Ok(token)
}

#[tauri::command]
pub async fn write_client_log(message: String) -> Result<(), String> {
    log(&format!("前端事件: {}", message));
    Ok(())
}

#[tauri::command]
pub async fn test_notification(config: NotificationConfig) -> Result<(), String> {
    log(&format!("收到测试通知请求: channels={:?}, has_webhook={}", config.channels, !config.webhook_url.is_empty()));

    if config.webhook_url.trim().is_empty() {
        return Err("Webhook URL 不能为空".to_string());
    }
    if config.channels.is_empty() {
        return Err("未配置通知渠道".to_string());
    }

    crate::services::upload_scheduler::UploadScheduler::test_notification(&config)
        .await
        .map_err(|e| {
            log(&format!("发送测试通知失败: {}", e));
            e
        })?;

    log("测试通知发送成功");
    Ok(())
}

#[tauri::command]
pub async fn get_blocked_files() -> Result<Vec<BlockedFileRecord>, String> {
    let data = crate::utils::storage::Storage::load_blocked_files().map_err(|e| e.to_string())?;
    Ok(data.records)
}

#[tauri::command]
pub async fn remove_blocked_file(index: usize) -> Result<(), String> {
    let mut data = crate::utils::storage::Storage::load_blocked_files().map_err(|e| e.to_string())?;
    if index < data.records.len() {
        data.records.remove(index);
        crate::utils::storage::Storage::save_blocked_files(&data).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn resolve_blocked_file(index: usize) -> Result<(), String> {
    let mut data = crate::utils::storage::Storage::load_blocked_files().map_err(|e| e.to_string())?;
    if index < data.records.len() {
        data.records[index].resolved = true;
        crate::utils::storage::Storage::save_blocked_files(&data).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn clear_blocked_files() -> Result<(), String> {
    crate::utils::storage::Storage::save_blocked_files(&crate::models::BlockedFileData::default()).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn alist_list_dir(config: AppConfig, path: String) -> Result<String, String> {
    log(&format!("收到 Alist 目录列表请求: base_url={}, username={}, has_token={}, path={}", config.alist.base_url, config.alist.username, !config.alist.token.is_empty(), path));
    let client = AlistClient::new(config.alist.base_url, config.alist.token, config.alist.use_system_proxy);
    
    let items = client
        .list_directory(&path)
        .await
        .map_err(|e| {
            log(&format!("Alist 目录列表失败: path={}, error={}", path, e));
            e.to_string()
        })?;
    
    log(&format!("Alist 目录列表成功: path={}, item_count={}", path, items.len()));
    serde_json::to_string(&items).map_err(|e| {
        log(&format!("序列化 Alist 目录列表失败: path={}, error={}", path, e));
        e.to_string()
    })
}

#[tauri::command]
pub async fn alist_mkdir(config: AppConfig, path: String) -> Result<(), String> {
    log(&format!("收到 Alist 创建目录请求: base_url={}, path={}", config.alist.base_url, path));
    let client = AlistClient::new(config.alist.base_url, config.alist.token, config.alist.use_system_proxy);

    client
        .mkdir(&path)
        .await
        .map_err(|e| {
            log(&format!("Alist 创建目录失败: path={}, error={}", path, e));
            e.to_string()
        })?;

    log(&format!("Alist 创建目录成功: path={}", path));
    Ok(())
}

#[tauri::command]
pub async fn get_shutdown_state(
    queue_manager: State<'_, QueueManager>,
) -> Result<Option<String>, String> {
    let deadline = queue_manager.get_shutdown_deadline().await;
    Ok(deadline.map(|d| d.to_rfc3339()))
}

#[tauri::command]
pub async fn cancel_shutdown(
    queue_manager: State<'_, QueueManager>,
) -> Result<(), String> {
    match std::process::Command::new("shutdown")
        .args(["/a"])
        .spawn()
    {
        Ok(_) => {
            queue_manager.clear_shutdown_deadline().await;
            log("已取消定时关机");
            Ok(())
        }
        Err(e) => {
            log(&format!("取消关机失败: {}", e));
            Err(e.to_string())
        }
    }
}

#[tauri::command]
pub async fn open_file_location(file_path: String) -> Result<(), String> {
    let path = std::path::Path::new(&file_path);
    if !path.exists() {
        return Err(format!("文件不存在: {}", file_path));
    }

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer.exe")
            .args(["/select,", &file_path])
            .spawn()
            .map_err(|e| {
                log(&format!("打开文件所在目录失败: path={}, error={}", file_path, e));
                e.to_string()
            })?;
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .args(["-R", &file_path])
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "linux")]
    {
        let parent = path.parent().unwrap_or(std::path::Path::new("."));
        std::process::Command::new("xdg-open")
            .arg(parent)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    log(&format!("已打开文件所在目录: {}", file_path));
    Ok(())
}

#[tauri::command]
pub async fn test_start_alist(queue_manager: State<'_, QueueManager>) -> Result<String, String> {
    use std::process::Command;

    let config = queue_manager.config.read().await;
    let exe_path = config.alist.exe_path.clone();
    let base_url = config.alist.base_url.clone();
    let run_in_background = config.alist.run_in_background;
    drop(config);

    log(&format!("测试启动 Alist: exe_path='{}'", exe_path));

    if exe_path.is_empty() {
        let msg = "Alist 路径未配置，请先在设置页填写";
        log(msg);
        return Err(msg.to_string());
    }

    let path = std::path::Path::new(&exe_path);
    if !path.exists() {
        let msg = format!("文件不存在: {}，请检查路径是否正确", exe_path);
        log(&msg);
        return Err(msg);
    }

    // 使用 openlist.exe 所在目录作为工作目录，确保读到原有的 data/config.json
    let working_dir = path.parent().map(|p| p.to_path_buf());

    // 先检测是否已在运行
    let already_running = reqwest::Client::new()
        .get(format!("{}/ping", base_url.trim_end_matches('/')))
        .timeout(std::time::Duration::from_secs(2))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false);

    if already_running {
        let msg = format!("Alist 已在运行 ({} 返回 pong)", base_url);
        log(&msg);
        return Ok(msg);
    }

    log(&format!("尝试启动: {} server (working_dir={:?})", exe_path, working_dir));
    let mut cmd = Command::new(&exe_path);
    cmd.arg("server");
    if let Some(ref dir) = working_dir {
        cmd.current_dir(dir);
    }
    if run_in_background {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
    }
    match cmd.spawn() {
        Ok(child) => {
            let pid = child.id();
            *crate::ALIST_CHILD_PID.lock().unwrap() = Some(pid);
            *crate::ALIST_EXE_PATH.lock().unwrap() = Some(exe_path.clone());
            let msg = format!("Alist 进程已启动, pid={}", pid);
            log(&msg);

            // 轮询检测是否启动成功，最多等 30 秒
            let mut healthy = false;
            for i in 1..=30 {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                healthy = reqwest::Client::new()
                    .get(format!("{}/ping", base_url.trim_end_matches('/')))
                    .timeout(std::time::Duration::from_secs(3))
                    .send()
                    .await
                    .map(|r| r.status().is_success())
                    .unwrap_or(false);
                if healthy {
                    let msg2 = format!("Alist 启动成功，{} 已就绪 (pid={}, 等待 {}s)", base_url, pid, i);
                    log(&msg2);
                    return Ok(msg2);
                }
                log(&format!("等待 Alist 就绪: {}/s", i));
            }

            let msg2 = format!("进程已启动 (pid={})，但 {} 30 秒内未就绪，请检查 Alist 配置或端口", pid, base_url);
            log(&msg2);
            Ok(msg2)
        }
        Err(e) => {
            let msg = format!("启动 Alist 失败: {}", e);
            log(&msg);
            Err(msg)
        }
    }
}

#[tauri::command]
pub async fn check_update_no_proxy(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_updater::UpdaterExt;

    log("检查更新（不走系统代理）");

    let update = app
        .updater_builder()
        .no_proxy()
        .build()
        .map_err(|e| {
            log(&format!("构建 updater 失败: {}", e));
            e.to_string()
        })?
        .check()
        .await
        .map_err(|e| {
            log(&format!("检查更新失败: {}", e));
            e.to_string()
        })?;

    match update {
        Some(update) => {
            log(&format!("发现新版本: {} (当前: {})", update.version, update.current_version));
            Ok(Some(update.version))
        }
        None => {
            log("已是最新版本");
            Ok(None)
        }
    }
}

#[tauri::command]
pub async fn download_and_install_update_no_proxy(app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_updater::UpdaterExt;

    log("下载并安装更新（不走系统代理）");

    let update = app
        .updater_builder()
        .no_proxy()
        .build()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| {
            log(&format!("获取更新失败: {}", e));
            e.to_string()
        })?;

    let Some(update) = update else {
        return Err("已是最新版本".to_string());
    };

    log(&format!("下载更新: version={}", update.version));
    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|e| {
            log(&format!("安装更新失败: {}", e));
            e.to_string()
        })?;

    log("更新已安装，即将重启");
    app.restart();
}

#[tauri::command]
pub async fn set_autostart(
    app: tauri::AppHandle,
    enabled: bool,
) -> Result<(), String> {
    use tauri_plugin_autostart::ManagerExt;

    let autostart_manager = app.autolaunch();

    if enabled {
        // 强制刷新：先 disable 再 enable，确保注册表路径与当前 exe 一致
        // （改名升级后旧路径残留会导致自启动静默失败）
        let was_enabled = autostart_manager.is_enabled().unwrap_or(false);
        if was_enabled {
            let _ = autostart_manager.disable();
        }
        autostart_manager.enable().map_err(|e| {
            log(&format!("开启开机自启失败: {}", e));
            e.to_string()
        })?;
        log(&format!("开机自启已开启（was_enabled={}，注册表路径已刷新）", was_enabled));
    } else {
        if autostart_manager.is_enabled().unwrap_or(false) {
            autostart_manager.disable().map_err(|e| {
                log(&format!("关闭开机自启失败: {}", e));
                e.to_string()
            })?;
            log("开机自启已关闭");
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn is_autostart_enabled(app: tauri::AppHandle) -> Result<bool, String> {
    use tauri_plugin_autostart::ManagerExt;
    Ok(app.autolaunch().is_enabled().unwrap_or(false))
}

#[tauri::command]
pub async fn log_sync_login(config: LogSyncConfig) -> Result<String, String> {
    log(&format!("日志同步登录: base_url={}, username={}", config.base_url, config.username));

    if config.base_url.is_empty() {
        return Err("日志Alist服务地址不能为空".to_string());
    }
    if config.username.is_empty() {
        return Err("用户名不能为空".to_string());
    }
    if config.password.is_empty() {
        return Err("密码不能为空".to_string());
    }

    let client = crate::services::log_sync::LogSyncClient::new(&config);
    let token = client
        .login(&config.username, &config.password)
        .await
        .map_err(|e| {
            log(&format!("日志同步登录失败: base_url={}, error={}", config.base_url, e));
            e
        })?;

    let mut full_config = Storage::load_config().map_err(|e| e.to_string())?;
    full_config.log_sync.token = token.clone();
    full_config.log_sync.base_url = config.base_url.clone();
    full_config.log_sync.username = config.username.clone();
    full_config.log_sync.password = config.password.clone();
    Storage::save_config(&full_config).map_err(|e| e.to_string())?;

    log(&format!("日志同步登录成功，token已保存: base_url={}", config.base_url));
    Ok(token)
}

#[tauri::command]
pub async fn sync_logs(config: LogSyncConfig) -> Result<LogSyncResult, String> {
    log(&format!("手动同步日志: base_url={}, target_path={}", config.base_url, config.target_path));

    let result = crate::services::log_sync::sync_logs(&config).await;
    log(&format!("日志同步结果: total={}, success={}, failed={}", result.total, result.success, result.failed));
    Ok(result)
}

#[tauri::command]
pub async fn get_local_log_files() -> Result<Vec<LocalLogFileInfo>, String> {
    Ok(crate::services::log_sync::get_local_log_files())
}

#[tauri::command]
pub async fn split_compress_file(
    app: tauri::AppHandle,
    queue_manager: State<'_, QueueManager>,
    file_path: String,
) -> Result<String, String> {
    use std::path::Path;
    use std::io::Read;

    let config = queue_manager.config.read().await;
    let rar_path = config.upload.rar_path.clone();
    let volume_mb = config.upload.split_volume_mb;
    drop(config);

    log(&format!("开始分卷压缩: file_path={}, rar={}, volume={}MB", file_path, rar_path, volume_mb));

    // 串行锁：等待其他压缩任务完成（排队执行，不并发）
    let lock_arc = split_compress_lock();
    let _lock = lock_arc.lock().await;
    log(&format!("获得压缩串行锁，开始压缩: {}", file_path));
    let _ = app.emit("compress_started", CompressEvent { file_path: file_path.clone(), percent: 0 });

    // 检查 rar.exe 是否存在
    if !Path::new(&rar_path).exists() {
        let msg = format!("找不到 WinRAR: {}，请在设置页配置 rar.exe 路径", rar_path);
        log(&msg);
        return Err(msg);
    }

    // 检查源文件是否存在
    let src_path = Path::new(&file_path);
    if !src_path.exists() {
        return Err(format!("文件不存在: {}", file_path));
    }

    let file_name = src_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");
    let file_stem = src_path
        .file_stem()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");
    let parent_dir = src_path.parent().ok_or("无法获取源文件父目录")?;

    // 输出目录：源文件旁/完整文件名-dir/
    let out_dir = parent_dir.join(format!("{}-dir", file_name));

    // 检测输出目录是否已有 .rar 分卷文件——若有，说明该文件此前已分卷压缩过，
    // 再次 rar a 会向同一目录写入，导致分卷损坏/产物异常累积（如6GB源文件压出9GB）。
    // 拒绝重复压缩，要求用户先清理旧产物或使用"改名重传"。
    if out_dir.exists() {
        let existing_parts: Vec<String> = std::fs::read_dir(&out_dir)
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .filter_map(|e| e.file_name().to_str().map(|s| s.to_string()))
                    .filter(|name| name.ends_with(".rar"))
                    .collect()
            })
            .unwrap_or_default();
        if !existing_parts.is_empty() {
            let msg = format!(
                "输出目录已存在 {} 个 .rar 分卷文件，疑似此前压缩产物：\n{}\n请先删除该目录或对拦截记录使用"改名重传"，不要重复分卷压缩。",
                existing_parts.len(),
                existing_parts.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n")
            );
            log(&msg);
            return Err(msg);
        }
    }

    if !out_dir.exists() {
        std::fs::create_dir_all(&out_dir).map_err(|e| format!("创建输出目录失败: {}", e))?;
    }

    let rar_base = out_dir.join(format!("{}.rar", file_stem));

    // rar a -v2000m -m1 -ep3 "输出\文件名.rar" "源文件"
    let volume_arg = format!("-v{}m", volume_mb);

    #[cfg(windows)]
    use std::os::windows::process::CommandExt;
    #[cfg(windows)]
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    let mut cmd = std::process::Command::new(&rar_path);
    cmd.arg("a")
        .arg(&volume_arg)
        .arg("-m1")
        .arg("-ep3")
        .arg(&rar_base)
        .arg(&file_path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    #[cfg(windows)]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    log(&format!("执行分卷压缩: rar={} args=a {} -m1 -ep3 {} {}", rar_path, volume_arg, rar_base.display(), file_path));

    // 压缩前诊断：rar.exe 元信息、rarreg.key 是否存在、输出目录是否已存在、源文件大小
    {
        let rar_meta = std::fs::metadata(&rar_path);
        let rarreg = std::path::Path::new(&rar_path).with_file_name("rarreg.key");
        let src_meta = std::fs::metadata(&file_path);
        log(&format!(
            "压缩前诊断: rar_exists={}, rar_size={:?}, rar_modified={:?}, rarreg_exists={}, out_dir_exists={}, src_exists={}, src_size={:?}",
            rar_meta.is_ok(),
            rar_meta.as_ref().ok().map(|m| m.len()),
            rar_meta.as_ref().ok().and_then(|m| m.modified().ok()),
            std::path::Path::new(&rarreg).exists(),
            out_dir.exists(),
            src_meta.is_ok(),
            src_meta.as_ref().ok().map(|m| m.len()),
        ));
    }

    let mut child = cmd.spawn()
        .map_err(|e| format!("启动 rar.exe 失败: {}", e))?;

    // 逐字符读取 stdout，解析百分比（保留原始字节用于 GBK 解码，避免 from_utf8_lossy 丢失中文）
    let mut stdout = child.stdout.take().ok_or("无法获取 rar stdout")?;
    let mut stderr = child.stderr.take().ok_or("无法获取 rar stderr")?;
    let mut stdout_bytes: Vec<u8> = Vec::new();
    let mut stderr_bytes: Vec<u8> = Vec::new();
    let mut line_buf = String::new();
    let mut buf = [0u8; 4096];
    loop {
        match stdout.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                stdout_bytes.extend_from_slice(&buf[..n]);
                let chunk = String::from_utf8_lossy(&buf[..n]);
                line_buf.push_str(&chunk);
                // rar 用 \r 刷新进度，按 \r 和 \n 分割
                while let Some(pos) = line_buf.find(|c| c == '\r' || c == '\n') {
                    let line = line_buf[..pos].trim().to_string();
                    if !line.is_empty() {
                        if let Some(pct) = parse_rar_progress(&line) {
                            let _ = app.emit("compress_progress", CompressEvent { file_path: file_path.clone(), percent: pct });
                            log(&format!("压缩进度: {}%", pct));
                        }
                    }
                    line_buf = line_buf[pos+1..].to_string();
                }
            }
            Err(e) => {
                log(&format!("读取 rar stdout 失败: {}", e));
                break;
            }
        }
    }

    // 读 stderr 原始字节
    {
        let mut sbuf = Vec::new();
        std::io::Read::read_to_end(&mut stderr, &mut sbuf).ok();
        stderr_bytes.extend_from_slice(sbuf);
    }

    let status = child.wait()
        .map_err(|e| format!("等待 rar.exe 结束失败: {}", e))?;

    // 最终进度 100%
    let _ = app.emit("compress_progress", CompressEvent { file_path: file_path.clone(), percent: 100 });

    // GBK 解码 rar 输出（rar.exe 在中文 Windows 输出 GBK 编码）
    let stdout_gbk = String::from_utf8(stdout_bytes.clone()).unwrap_or_else(|_| {
        // 不是合法 UTF-8，尝试 GBK 解码
        decode_gbk(&stdout_bytes)
    });
    let stderr_gbk = String::from_utf8(stderr_bytes.clone()).unwrap_or_else(|_| {
        decode_gbk(&stderr_bytes)
    });

    if !stdout_gbk.is_empty() {
        log(&format!("rar stdout (GBK):\n{}", stdout_gbk));
    }
    if !stderr_gbk.is_empty() {
        log(&format!("rar stderr (GBK):\n{}", stderr_gbk));
    }

    if !status.success() {
        let code = status.code().unwrap_or(-1);
        // 失败时额外输出 hex 转储，便于离线分析编码问题
        log(&format!("rar stdout hex: {}", hex_dump(&stdout_bytes)));
        log(&format!("rar stderr hex: {}", hex_dump(&stderr_bytes)));
        let msg = format!("rar.exe 返回错误码 {}: {}", code, stderr_gbk);
        log(&msg);
        return Err(msg);
    }

    // 列出生成的分卷文件
    let mut parts: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&out_dir) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if name.ends_with(".rar") {
                    parts.push(name.to_string());
                }
            }
        }
    }
    parts.sort();

    let out_dir_str = out_dir.to_string_lossy().to_string();
    log(&format!("分卷压缩完成: file={}, out_dir={}, parts={:?}", file_name, out_dir_str, parts));
    Ok(out_dir_str)
}

/// 将超长名称的分卷文件夹重命名为短名，创建 TXT 存根，重命名内部 part 文件
#[tauri::command]
pub async fn rename_blocked_folder(
    folder_path: String,
) -> Result<String, String> {
    use std::path::Path;
    use std::io::Write;

    let src_path = Path::new(&folder_path);
    if !src_path.exists() || !src_path.is_dir() {
        return Err(format!("文件夹不存在或不是目录: {}", folder_path));
    }

    let original_name = src_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();
    let original_bytes = original_name.len();
    let parent_dir = src_path.parent().ok_or("无法获取父目录")?;

    // 截断到 50 字符 + "-dir"
    let base_name = if original_name.ends_with("-dir") {
        original_name[..original_name.len() - 4].to_string()
    } else {
        original_name.clone()
    };
    let truncated: String = base_name.chars().take(50).collect();
    let new_name = format!("{}-dir", truncated);
    let new_path = parent_dir.join(&new_name);

    if new_path == *src_path {
        return Err("新名称与原名称相同，无需改名".into());
    }

    // 如果目标已存在，加序号
    let mut final_path = new_path.clone();
    let mut suffix = 1;
    while final_path.exists() {
        final_path = parent_dir.join(format!("{}-{}-dir", truncated, suffix));
        suffix += 1;
    }
    let final_name = final_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(&new_name)
        .to_string();

    // 先收集内部 part 文件列表（重命名文件夹前）
    let mut part_files: Vec<(std::path::PathBuf, String)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(src_path) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(name) = entry.file_name().to_str() {
                if name.ends_with(".rar") {
                    part_files.push((path, name.to_string()));
                }
            }
        }
    }

    // 创建 TXT 存根（在原文件夹内）
    let txt_path = src_path.join("原名.txt");
    let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let txt_content = format!(
        "原始文件名: {}\n原始字节数: {} 字节\n改名原因: 超过 115Crypt 加密驱动 175 字节限制\n新文件夹名: {}\n改名时间: {}\n源路径: {}",
        original_name, original_bytes, final_name, timestamp, folder_path
    );
    match std::fs::File::create(&txt_path) {
        Ok(mut f) => {
            let _ = f.write_all(txt_content.as_bytes());
            log(&format!("已创建改名存根: {}", txt_path.display()));
        }
        Err(e) => {
            log(&format!("创建改名存根失败: {}", e));
        }
    }

    // 重命名 part 文件（先重命名内部文件，再重命名文件夹）
    let new_base = if final_name.ends_with("-dir") {
        final_name[..final_name.len() - 4].to_string()
    } else {
        final_name.clone()
    };

    for (path, name) in &part_files {
        // 原名格式：xxx.part1.rar → 新名：新base.part1.rar
        if let Some(part_suffix) = extract_part_suffix(name) {
            let new_file_name = format!("{}.{}", new_base, part_suffix);
            let new_file_path = src_path.join(&new_file_name);
            if let Err(e) = std::fs::rename(path, &new_file_path) {
                log(&format!("重命名 part 文件失败: {} -> {}, error={}", name, new_file_name, e));
            } else {
                log(&format!("重命名 part 文件: {} -> {}", name, new_file_name));
            }
        }
    }

    // 重命名文件夹
    std::fs::rename(src_path, &final_path).map_err(|e| format!("重命名文件夹失败: {}", e))?;
    log(&format!("文件夹已重命名: {} -> {}", folder_path, final_path.display()));

    Ok(final_path.to_string_lossy().to_string())
}

/// 从文件名提取 part 后缀（如 "xxx.part1.rar" → "part1.rar"）
fn extract_part_suffix(name: &str) -> Option<String> {
    let lower = name.to_lowercase();
    if let Some(pos) = lower.find(".part") {
        Some(name[pos..].to_string())
    } else {
        None
    }
}

/// 从 rar.exe 输出行解析百分比
/// rar 输出格式如 "Creating archive xxx.rar" 或 "Adding  file.mp4    45%"
fn parse_rar_progress(line: &str) -> Option<u8> {
    // 找末尾的 N% 模式
    let trimmed = line.trim();
    if let Some(pos) = trimmed.rfind('%') {
        // 往前找数字
        let before = &trimmed[..pos];
        let num_start = before.rfind(|c: char| !c.is_ascii_digit())?;
        let num_str = &before[num_start+1..];
        if let Ok(n) = num_str.parse::<u8>() {
            if n <= 100 {
                return Some(n);
            }
        }
    }
    None
}

/// GBK 解码字节数组为字符串（rar.exe 在中文 Windows 输出 GBK 编码）
fn decode_gbk(bytes: &[u8]) -> String {
    encoding_rs::GBK.decode(bytes).0.into_owned()
}

/// 将字节数组转为 hex 字符串，便于离线分析编码问题
fn hex_dump(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    // 限制长度，避免日志过大
    if s.len() > 4096 {
        format!("{}...(truncated, total={}B)", &s[..4096], bytes.len())
    } else {
        s
    }
}
