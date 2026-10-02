use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use chrono::Local;

static LOG_MUTEX: Mutex<()> = Mutex::new(());
static LAST_LOG_DAY: Mutex<Option<chrono::NaiveDate>> = Mutex::new(None);

fn get_app_dir() -> Option<PathBuf> {
    let Some(mut app_dir) = dirs::data_local_dir() else {
        return None;
    };

    app_dir.push("openlist-uploader");

    if fs::create_dir_all(&app_dir).is_err() {
        return None;
    }

    Some(app_dir)
}

fn get_logs_dir() -> Option<PathBuf> {
    let mut logs_dir = get_app_dir()?;
    logs_dir.push("logs");

    if fs::create_dir_all(&logs_dir).is_err() {
        return None;
    }

    Some(logs_dir)
}

fn append_to_file(path: PathBuf, line: &str) {
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{}", line);
    }
}

fn append_line(line: &str) {
    if let Some(mut app_log_path) = get_app_dir() {
        app_log_path.push("debug.log");
        append_to_file(app_log_path, &line);
    }

    if let Some(mut daily_log_path) = get_logs_dir() {
        daily_log_path.push(format!("alist-{}.log", Local::now().format("%Y-%m-%d")));
        append_to_file(daily_log_path, &line);
    }
}

/// 汇总当前配置里的全部开关状态与关键数值（不含任何密钥/密码），供日志快照使用
fn build_config_snapshot_line(reason: &str) -> String {
    let config = crate::utils::storage::Storage::load_config().unwrap_or_default();
    let up = &config.upload;
    let al = &config.alist;
    let ls = &config.log_sync;
    let schedule = up.schedule.clone().unwrap_or_default();
    let notify_enabled = up.notification.as_ref().map(|n| n.enabled).unwrap_or(false);

    format!(
        "[配置快照 reason={reason}] \
        concurrency={concurrency}, max_retries={max_retries}, speed_limit={speed_limit}, max_tasks_per_run={max_tasks_per_run}, \
        as_task={as_task}, upload_method={upload_method}, fail_action={fail_action}, strategy={strategy}, \
        block_files_over_5gb={block_files_over_5gb}, warn_files_over_4gb={warn_files_over_4gb}, \
        block_duplicate_file_upload={block_duplicate_file_upload}, delete_source_after_compress={delete_source_after_compress}, \
        mark_uploaded_delete_prefix={mark_uploaded_delete_prefix}, refresh_index_after_upload={refresh_index_after_upload}, \
        split_volume_mb={split_volume_mb}, rar_path={rar_path}, \
        notify_on_complete={notify_on_complete}, feishu_queue_complete={feishu_queue_complete}, notify_webhook_enabled={notify_webhook_enabled}, \
        shutdown_after_complete={shutdown_after_complete}, shutdown_delay_minutes={shutdown_delay_minutes}, \
        check_update_on_startup={check_update_on_startup}, auto_start_on_boot={auto_start_on_boot}, start_silent={start_silent}, \
        progress_notify={progress_notify}, progress_notify_interval={progress_notify_interval}, minimize_on_close={minimize_on_close}, \
        alist_base_url={alist_base_url}, alist_auto_login={alist_auto_login}, kill_alist_on_exit={kill_alist_on_exit}, \
        alist_run_in_background={alist_run_in_background}, alist_use_system_proxy={alist_use_system_proxy}, \
        logsync_enabled={logsync_enabled}, logsync_sync_on_exit={logsync_sync_on_exit}, \
        logsync_interval_minutes={logsync_interval_minutes}, logsync_use_system_proxy={logsync_use_system_proxy}, \
        history_never_clean={history_never_clean}, history_retention_days={history_retention_days}, \
        schedule_enabled={schedule_enabled}, schedule_start={schedule_start}, schedule_end={schedule_end}, \
        alist_last_target={alist_last_target}",
        reason = reason,
        concurrency = up.concurrency,
        max_retries = up.max_retries,
        speed_limit = up.speed_limit,
        max_tasks_per_run = up.max_tasks_per_run,
        as_task = up.as_task,
        upload_method = up.upload_method,
        fail_action = up.fail_action,
        strategy = up.file_exists_strategy.value,
        block_files_over_5gb = up.block_files_over_5gb,
        warn_files_over_4gb = up.warn_files_over_4gb,
        block_duplicate_file_upload = up.block_duplicate_file_upload,
        delete_source_after_compress = up.delete_source_after_compress,
        mark_uploaded_delete_prefix = up.mark_uploaded_delete_prefix,
        refresh_index_after_upload = up.refresh_index_after_upload,
        split_volume_mb = up.split_volume_mb,
        rar_path = up.rar_path,
        notify_on_complete = up.notify_on_complete,
        feishu_queue_complete = up.notify_feishu_on_queue_complete,
        notify_webhook_enabled = notify_enabled,
        shutdown_after_complete = up.shutdown_after_complete,
        shutdown_delay_minutes = up.shutdown_delay_minutes,
        check_update_on_startup = up.check_update_on_startup,
        auto_start_on_boot = up.auto_start_on_boot,
        start_silent = up.start_silent,
        progress_notify = up.progress_notify_enabled,
        progress_notify_interval = up.progress_notify_interval,
        minimize_on_close = up.minimize_on_close,
        alist_base_url = al.base_url,
        alist_auto_login = al.auto_login,
        kill_alist_on_exit = al.kill_on_exit,
        alist_run_in_background = al.run_in_background,
        alist_use_system_proxy = al.use_system_proxy,
        logsync_enabled = ls.enabled,
        logsync_sync_on_exit = ls.sync_on_exit,
        logsync_interval_minutes = ls.sync_interval_minutes,
        logsync_use_system_proxy = ls.use_system_proxy,
        history_never_clean = config.history.never_clean,
        history_retention_days = config.history.retention_days,
        schedule_enabled = schedule.enabled,
        schedule_start = schedule.start_time,
        schedule_end = schedule.end_time,
        alist_last_target = up.last_alist_path,
    )
}

/// 把当前配置开关状态写进日志。reason 用于区分触发时机（startup / config_saved）。
/// 注意不要在持有 LOG_MUTEX 时调用之外的地方递归 log。
pub fn log_config_snapshot(reason: &str) {
    log(&build_config_snapshot_line(reason));
}

/// 每份按天日志文件的首条写入前，先落一条配置快照，保证每天日志都能对照当天的开关状态
fn maybe_append_daily_snapshot_locked(now: &chrono::DateTime<Local>) {
    let today = now.date_naive();
    let mut last = match LAST_LOG_DAY.lock() {
        Ok(guard) => guard,
        Err(_) => return,
    };
    let first_ever = last.is_none();
    if let Some(prev) = *last {
        if prev == today {
            return;
        }
    }
    *last = Some(today);
    // 进程首次写日志的当天由 startup 快照覆盖，避免重复
    if !first_ever {
        let timestamp = now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        append_line(&format!("[{}] {}", timestamp, build_config_snapshot_line("daily_rotate")));
    }
}

pub fn log(message: &str) {
    let now = Local::now();
    let timestamp = now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let line = format!("[{}] {}", timestamp, message);

    let _guard = LOG_MUTEX.lock().ok();
    maybe_append_daily_snapshot_locked(&now);
    append_line(&line);
}

pub fn log_error(message: &str, error: &dyn std::error::Error) {
    log(&format!("ERROR: {} - {}", message, error));
}

pub fn log_debug(message: &str) {
    log(&format!("DEBUG: {}", message));
}

pub fn log_info(message: &str) {
    log(&format!("INFO: {}", message));
}

pub fn log_warn(message: &str) {
    log(&format!("WARN: {}", message));
}

pub fn log_request(method: &str, url: &str, status: u16, duration_ms: u64) {
    log(&format!("REQUEST: {} {} -> {} ({}ms)", method, url, status, duration_ms));
}
