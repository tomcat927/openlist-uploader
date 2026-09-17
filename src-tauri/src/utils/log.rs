use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use chrono::Local;

static LOG_MUTEX: Mutex<()> = Mutex::new(());

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

pub fn log(message: &str) {
    let _guard = LOG_MUTEX.lock().ok();
    let timestamp = Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let line = format!("[{}] {}", timestamp, message);

    if let Some(mut app_log_path) = get_app_dir() {
        app_log_path.push("debug.log");
        append_to_file(app_log_path, &line);
    }

    if let Some(mut daily_log_path) = get_logs_dir() {
        daily_log_path.push(format!("alist-{}.log", Local::now().format("%Y-%m-%d")));
        append_to_file(daily_log_path, &line);
    }
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
