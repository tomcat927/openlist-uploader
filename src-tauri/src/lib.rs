pub mod models;
pub mod services;
pub mod commands;
pub mod utils;
use tauri::Manager;
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use std::process::Command;
use crate::utils::storage::Storage;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

fn append_log(file_name: &str, message: &str) {
    use std::fs::{self, OpenOptions};
    use std::io::Write;

    let Some(mut log_dir) = dirs::data_local_dir() else {
        return;
    };

    log_dir.push("alist-uploader");

    if fs::create_dir_all(&log_dir).is_err() {
        return;
    }

    let log_path = log_dir.join(file_name);
    let timestamp = chrono::Local::now().to_rfc3339();

    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(log_path) {
        let _ = writeln!(file, "[{timestamp}] {message}");
    }
}

fn install_panic_hook() {
    std::panic::set_hook(Box::new(|panic_info| {
        append_log("panic.log", &format!("{panic_info}"));
        append_log("startup.log", &format!("panic: {panic_info}"));
    }));
}

/// 运行标记文件路径：正常退出时删除，异常终止时残留
fn running_marker_path() -> Option<std::path::PathBuf> {
    let mut dir = dirs::data_local_dir()?;
    dir.push("alist-uploader");
    Some(dir.join("running.marker"))
}

/// 启动时写入运行标记
fn write_running_marker() {
    if let Some(path) = running_marker_path() {
        let ts = chrono::Local::now().to_rfc3339();
        let _ = std::fs::write(&path, ts);
    }
}

/// 正常退出时删除运行标记
fn clear_running_marker() {
    if let Some(path) = running_marker_path() {
        let _ = std::fs::remove_file(&path);
    }
}

/// 检测上次是否异常终止（marker 残留即上次未走正常退出）
fn detect_abnormal_exit() -> bool {
    running_marker_path()
        .map(|p| p.exists())
        .unwrap_or(false)
}

// 全局存储 Alist 可执行文件路径和子进程 PID，供退出时关闭使用
pub static ALIST_EXE_PATH: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
pub static ALIST_CHILD_PID: std::sync::Mutex<Option<u32>> = std::sync::Mutex::new(None);

/// 退出时关闭 Alist 进程
fn kill_alist_on_exit() {
    let exe_path = ALIST_EXE_PATH.lock().unwrap().clone();
    if let Some(exe) = exe_path {
        // 尝试通过 taskkill 关闭由本应用启动的 Alist 进程
        let pid_opt = ALIST_CHILD_PID.lock().unwrap().clone();
        if let Some(pid) = pid_opt {
            let _ = Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .output();
            append_log("startup.log", &format!("退出时已关闭 Alist 进程, pid={}", pid));
        } else {
            // 如果没有 PID（可能是配置变更前启动的），用 exe 名做备用
            let exe_name = std::path::Path::new(&exe)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("alist.exe");
            let _ = Command::new("taskkill")
                .args(["/IM", exe_name, "/T", "/F"])
                .output();
            append_log("startup.log", &format!("退出时按名称关闭 Alist: {}", exe_name));
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    install_panic_hook();
    append_log("startup.log", "application startup begin");
    crate::utils::log::log(&format!("application startup begin; version={}, build_marker=state-free-login-config-v2", env!("CARGO_PKG_VERSION")));

    // 异常退出检测：marker 残留说明上次未走正常退出流程（断电/强杀/崩溃）
    let last_exit_abnormal = detect_abnormal_exit();
    if last_exit_abnormal {
        append_log("startup.log", "检测到上次异常退出（运行标记残留）");
        crate::utils::log::log("检测到上次异常退出: 运行标记残留，可能为断电/强杀/崩溃");
    }
    write_running_marker();

    let queue_manager = match crate::services::queue_manager::QueueManager::new() {
        Ok(manager) => manager,
        Err(error) => {
            append_log("startup.log", &format!("failed to initialize queue manager: {error:?}"));
            panic!("无法初始化队列管理器: {error:?}");
        }
    };
    let qm_for_setup = queue_manager.clone_inner();
    let abnormal_for_setup = last_exit_abnormal;

    append_log("startup.log", "queue manager initialized");
    crate::utils::log::log("queue manager initialized; managed_type=QueueManager");

    let app = tauri::Builder::default()
        .manage(queue_manager)
        .setup(move |app| {
            append_log("startup.log", "tauri setup begin");
            crate::utils::log::log("tauri setup begin; schedule manager starting");

            // 日志定时同步
            {
                let qm_for_logsync = qm_for_setup.clone_inner();
                let config = qm_for_logsync.config.blocking_read();
                let log_sync_config = config.log_sync.clone();
                drop(config);
                if crate::services::log_sync::spawn_interval_sync(log_sync_config).is_some() {
                    append_log("startup.log", "日志定时同步已启动");
                }
            }

            let schedule_manager = crate::services::schedule_manager::ScheduleManager::new(qm_for_setup.clone_inner());
            tauri::async_runtime::spawn(async move {
                append_log("startup.log", "schedule monitor started");
                schedule_manager.start_schedule_monitor().await;
                append_log("startup.log", "schedule monitor stopped");
            });

            // 如果配置了 Alist 可执行文件路径，启动时自动启动 Alist
            let qm = app.state::<crate::services::queue_manager::QueueManager>();
            let config = qm.config.blocking_read();
            let alist_exe = config.alist.exe_path.clone();
            let base_url = config.alist.base_url.clone();
            let kill_on_exit = config.alist.kill_on_exit;
            let run_in_background = config.alist.run_in_background;
            append_log("startup.log", &format!("Alist 自动启动检查: exe_path='{}', kill_on_exit={}, run_in_background={}", alist_exe, kill_on_exit, run_in_background));
            drop(config);

            if !alist_exe.is_empty() && kill_on_exit {
                // 存到全局供退出时使用
                *crate::ALIST_EXE_PATH.lock().unwrap() = Some(alist_exe.clone());
            }

            if !alist_exe.is_empty() {
                append_log("startup.log", &format!("exe_path 非空，进入自动启动逻辑: {}", alist_exe));
                // 检查文件是否存在
                let path_exists = std::path::Path::new(&alist_exe).exists();
                append_log("startup.log", &format!("Alist 可执行文件是否存在: {}", path_exists));
                if !path_exists {
                    append_log("startup.log", &format!("警告: 文件 {} 不存在，无法启动", alist_exe));
                }

                tauri::async_runtime::spawn(async move {
                    // 先检测 Alist 是否已在运行
                    append_log("startup.log", &format!("检测 Alist 是否已在运行: GET {}/ping", base_url.trim_end_matches('/')));
                    let already_running = reqwest::Client::new()
                        .get(format!("{}/ping", base_url.trim_end_matches('/')))
                        .timeout(std::time::Duration::from_secs(2))
                        .send()
                        .await
                        .map(|r| {
                            append_log("startup.log", &format!("ping 响应状态: {}", r.status()));
                            r.status().is_success()
                        })
                        .unwrap_or_else(|e| {
                            append_log("startup.log", &format!("ping 请求失败: {}", e));
                            false
                        });

                    if already_running {
                        append_log("startup.log", "Alist 已在运行，跳过自动启动");
                        return;
                    }

                    append_log("startup.log", &format!("正在启动 Alist: {} server", alist_exe));
                    let alist_path = std::path::Path::new(&alist_exe);
                    let working_dir = alist_path.parent();
                    let mut cmd = Command::new(&alist_exe);
                    cmd.arg("server");
                    if let Some(dir) = working_dir {
                        cmd.current_dir(dir);
                    }
                    if run_in_background {
                        #[cfg(windows)]
                        {
                            cmd.creation_flags(CREATE_NO_WINDOW);
                        }
                    }
                    match cmd.spawn() {
                        Ok(child) => {
                            let pid = child.id();
                            append_log("startup.log", &format!("Alist 进程已启动, pid={}", pid));
                            *crate::ALIST_CHILD_PID.lock().unwrap() = Some(pid);
                        }
                        Err(e) => {
                            append_log("startup.log", &format!("启动 Alist 失败: {}", e));
                        }
                    }
                });
            } else {
                append_log("startup.log", "exe_path 为空，跳过自动启动");
            }

            // 恢复中断任务：上次异常退出时中断的任务，等 Alist 就绪后检查是否已传完，
            // 传完的入历史，未传完的重新排队；异常退出场景自动恢复上传调度器
            {
                let qm_for_recover = qm_for_setup.clone_inner();
                let was_abnormal = abnormal_for_setup;
                tauri::async_runtime::spawn(async move {
                    const INTERRUPTED_MARKER: &str = "上传中断";

                    let is_interrupted = |t: &crate::models::UploadTask| {
                        t.status == crate::models::TaskStatus::Uploading
                            || (t.status == crate::models::TaskStatus::Failed
                                && t.error.as_deref().map_or(false, |e| e.contains(INTERRUPTED_MARKER)))
                    };

                    // 找出中断任务
                    let has_interrupted = {
                        let queue = qm_for_recover.queue.read().await;
                        queue.tasks.iter().any(|t| is_interrupted(t))
                    };

                    if !has_interrupted && !was_abnormal {
                        return;
                    }

                    // 有中断任务时等待 Alist 就绪（最多 60 秒），确保 check_file_exists 可靠
                    let (alist_base_url, alist_token, use_proxy) = {
                        let config = qm_for_recover.config.read().await;
                        (config.alist.base_url.clone(), config.alist.token.clone(), config.alist.use_system_proxy)
                    };

                    let mut alist_ready = false;
                    if has_interrupted {
                        let ping_client = reqwest::Client::new();
                        for _ in 0..30 {
                            let ok = ping_client
                                .get(format!("{}/ping", alist_base_url.trim_end_matches('/')))
                                .timeout(std::time::Duration::from_secs(2))
                                .send()
                                .await
                                .map(|r| r.status().is_success())
                                .unwrap_or(false);
                            if ok {
                                alist_ready = true;
                                break;
                            }
                            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                        }
                        crate::utils::log::log(&format!("中断任务恢复检查: alist_ready={}", alist_ready));
                    }

                    let alist_client = crate::services::alist_client::AlistClient::new(alist_base_url, alist_token, use_proxy);
                    let mut recovered = 0usize;
                    let mut skipped = 0usize;
                    let mut skip_ids: Vec<String> = Vec::new();
                    {
                        let mut queue = qm_for_recover.queue.write().await;
                        for task in queue.tasks.iter_mut() {
                            if !is_interrupted(task) {
                                continue;
                            }
                            append_log("startup.log", &format!("恢复中断任务: file={}, alist_path={}", task.file.name, task.alist_path));

                            if alist_ready {
                                // 检查文件是否已在 Alist 上传完成
                                let exists = alist_client.check_file_exists(&task.alist_path, &task.file.name).await.unwrap_or(false);
                                if exists {
                                    append_log("startup.log", &format!("文件已在 Alist 上存在，跳过: file={}", task.file.name));
                                    task.mark_completed();
                                    let done = task.clone();
                                    let _ = qm_for_recover.add_to_history(done).await;
                                    skip_ids.push(task.id.clone());
                                    skipped += 1;
                                    continue;
                                }
                            }

                            // 未传完，重新排队
                            task.status = crate::models::TaskStatus::Pending;
                            task.error = None;
                            task.progress = 0;
                            task.speed = 0;
                            recovered += 1;
                        }
                        if !skip_ids.is_empty() {
                            queue.tasks.retain(|t| !skip_ids.contains(&t.id));
                        }
                        let _ = crate::utils::storage::Storage::save_queue(&*queue);
                    }

                    let total = recovered + skipped;
                    crate::utils::log::log(&format!("中断任务恢复完成: total={}, recovered={}, skipped={}", total, recovered, skipped));
                    append_log("startup.log", &format!("中断任务恢复: {} 个重新上传，{} 个已存在跳过", recovered, skipped));

                    // 异常退出告警 + 自动恢复上传
                    if was_abnormal {
                        let reason = if total > 0 {
                            format!("⚠️ 程序异常退出告警\n上次运行被强制终止（断电/蓝屏/强杀），中断了 {} 个上传任务\n已自动恢复: {} 个重新上传，{} 个已传完跳过\n{}",
                                total, recovered, skipped,
                                if recovered > 0 { "上传调度器已自动启动" } else { "无需重新上传" })
                        } else {
                            "⚠️ 程序异常退出告警\n上次运行被强制终止（断电/蓝屏/强杀）\n本次启动未发现中断的上传任务\n建议检查电脑供电稳定性".to_string()
                        };
                        crate::utils::log::log(&format!("异常退出告警已触发: total={}", total));
                        append_log("startup.log", &format!("异常退出告警: {}", reason.replace('\n', " | ")));

                        let config = qm_for_recover.config.read().await;
                        if let Some(notification) = &config.upload.notification {
                            if notification.enabled && !notification.webhook_url.is_empty() {
                                crate::services::upload_scheduler::UploadScheduler::send_text_notification(notification, &reason).await;
                            }
                        }

                        // 自愈：异常退出且有任务重新排队，自动启动上传调度器
                        if recovered > 0 {
                            crate::utils::log::log("检测到异常退出且有待恢复任务，自动启动上传调度器");
                            append_log("startup.log", "自愈: 自动启动上传调度器");
                            let scheduler = crate::services::upload_scheduler::UploadScheduler::new(qm_for_recover.clone_inner());
                            tauri::async_runtime::spawn(async move {
                                scheduler.start_scheduler().await;
                            });
                        }
                    } else if total > 0 {
                        // 正常重启但有中断任务（如定时上传时段重启），通知但不自动启动
                        let config = qm_for_recover.config.read().await;
                        if let Some(notification) = &config.upload.notification {
                            if notification.enabled && !notification.webhook_url.is_empty() {
                                let msg = format!("系统重启恢复通知: 检测到 {} 个中断任务，{} 个重新排队，{} 个已传完跳过（可在队列页手动开始上传）", total, recovered, skipped);
                                crate::services::upload_scheduler::UploadScheduler::send_text_notification(&notification, &msg).await;
                            }
                        }
                    }
                });
            }

            // 创建系统托盘图标，用于窗口最小化到托盘后恢复
            let img = image::load_from_memory(include_bytes!("../icons/icon.png"))
                .expect("加载托盘图标失败")
                .to_rgba8();
            let (width, height) = img.dimensions();
            let icon = Image::new_owned(img.into_raw(), width, height);
            let show = MenuItem::with_id(app, "show", "显示窗口", true, None::<&str>)?;
            let devtools = MenuItem::with_id(app, "devtools", "开发者工具", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &devtools, &quit])?;
            TrayIconBuilder::with_id("main-tray")
                .icon(icon)
                .tooltip("alist-uploader")
                .menu(&menu)
                .on_menu_event(|app_handle, event| {
                    match event.id.as_ref() {
                        "show" => {
                            if let Some(window) = app_handle.get_webview_window("main") {
                                let _ = window.unminimize();
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                        "devtools" => {
                            if let Some(window) = app_handle.get_webview_window("main") {
                                let _ = window.unminimize();
                                let _ = window.show();
                                let _ = window.set_focus();
                                window.open_devtools();
                            }
                        }
                        "quit" => {
                            app_handle.exit(0);
                        }
                        _ => {}
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    if let tauri::tray::TrayIconEvent::Click {
                        button: tauri::tray::MouseButton::Left,
                        button_state: tauri::tray::MouseButtonState::Up,
                        ..
                    } = event
                    {
                        if let Some(window) = tray.app_handle().get_webview_window("main") {
                            let _ = window.unminimize();
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })
                .build(app)?;

            append_log("startup.log", "tauri setup complete");
            crate::utils::log::log("tauri setup complete; QueueManager should be managed");
            Ok(())
        })
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, Some(vec!["--autostart"])))
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if let Some(qm) = window.try_state::<crate::services::queue_manager::QueueManager>() {
                    let config = qm.config.blocking_read();
                    if config.upload.minimize_on_close {
                        api.prevent_close();
                        let _ = window.hide();
                    }
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            crate::commands::get_queue,
            crate::commands::add_to_queue,
            crate::commands::remove_from_queue,
            crate::commands::clear_queue,
            crate::commands::get_history,
           crate::commands::get_history_page,
            crate::commands::clear_history,
            crate::commands::get_config,
            crate::commands::save_config,
            crate::commands::start_upload,
            crate::commands::pause_upload,
            crate::commands::get_is_uploading,
            crate::commands::stop_after_current,
            crate::commands::retry_upload,
            crate::commands::test_alist_connection,
            crate::commands::get_file_info,
            crate::commands::get_data_path,
            crate::commands::check_health,
            crate::commands::alist_login,
            crate::commands::write_client_log,
            crate::commands::test_notification,
            crate::commands::alist_list_dir,
            crate::commands::alist_mkdir,
           crate::commands::get_blocked_files,
           crate::commands::remove_blocked_file,
           crate::commands::resolve_blocked_file,
           crate::commands::clear_blocked_files,
           crate::commands::get_shutdown_state,
           crate::commands::cancel_shutdown,
           crate::commands::open_file_location,
           crate::commands::test_start_alist,
           crate::commands::check_update_no_proxy,
           crate::commands::download_and_install_update_no_proxy,
           crate::commands::set_autostart,
           crate::commands::is_autostart_enabled,
           crate::commands::log_sync_login,
           crate::commands::sync_logs,
           crate::commands::get_local_log_files,
       ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|_app_handle, event| {
        if let tauri::RunEvent::Exit = event {
            clear_running_marker();
            let config = Storage::load_config().unwrap_or_default();
            crate::services::log_sync::sync_on_exit_blocking(&config.log_sync);
            kill_alist_on_exit();
        }
    });

    // 兜底：确保退出时关闭 Alist
    kill_alist_on_exit();
}

