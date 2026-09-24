pub use dashmap::DashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;

/// 115Crypt 加密驱动限制：单级目录名/文件名 UTF-8 字节数不超过 175
const NAME_BYTES_LIMIT: usize = 175;

/// 检查目标路径各级目录名和文件名是否超限，返回首个超限的段名
fn check_name_bytes_limit(path: &str) -> Option<String> {
    for segment in path.split('/').filter(|s| !s.is_empty()) {
        let bytes = segment.len();
        if bytes > NAME_BYTES_LIMIT {
            return Some(segment.to_string());
        }
    }
    None
}

struct FileValidation {
    blockers: Vec<BlockedReason>,
    warning: Option<String>,
}

fn blocked_reason_text(reason: &BlockedReason, file_name: &str) -> String {
    match reason {
        BlockedReason::NameTooLong {
            segment,
            actual_bytes,
            limit_bytes,
        } => format!(
            "文件名过长，115Crypt 限制 {} 字节，当前 {} 字节: {}",
            limit_bytes, actual_bytes, segment
        ),
        BlockedReason::TargetPathTooLong {
            segment,
            actual_bytes,
            limit_bytes,
        } => format!(
            "目标目录名过长，115Crypt 限制 {} 字节，当前 {} 字节: {}",
            limit_bytes, actual_bytes, segment
        ),
        BlockedReason::FileTooLarge {
            actual_bytes,
            limit_bytes,
        } => format!(
            "文件大小超限，115 网盘非会员单个文件最大支持 {}，当前 {}，{} 已阻止加入上传队列。",
            format_size_limit(*limit_bytes),
            format_file_size(*actual_bytes),
            file_name,
        ),
    }
}

fn format_size_limit(bytes: u64) -> String {
    if bytes % (1024 * 1024 * 1024) == 0 {
        format!("{}GB", bytes / (1024 * 1024 * 1024))
    } else {
        format!("{} 字节", bytes)
    }
}

fn format_file_size(bytes: u64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit_index = 0;
    while size >= 1024.0 && unit_index < units.len() - 1 {
        size /= 1024.0;
        unit_index += 1;
    }
    format!("{:.2} {}", size, units[unit_index])
}

fn format_blocked_reasons(reasons: &[BlockedReason], file_name: &str) -> String {
    reasons
        .iter()
        .map(|reason| blocked_reason_text(reason, file_name))
        .collect::<Vec<_>>()
        .join("\n")
}
use chrono::{DateTime, Utc};
use crate::models::*;
use crate::utils::storage::Storage;
use crate::utils::log::log;

pub const FOUR_GB: u64 = 4 * 1024 * 1024 * 1024;
pub const FIVE_GB: u64 = 5 * 1024 * 1024 * 1024;

pub struct QueueManager {
    pub queue: Arc<RwLock<QueueData>>,
    pub history: Arc<RwLock<HistoryData>>,
    pub config: Arc<RwLock<AppConfig>>,
    pub processing_tasks: Arc<DashMap<String, UploadTask>>,
   is_uploading: Arc<AtomicBool>,
   stop_after_current: Arc<AtomicBool>,
   tasks_uploaded_in_run: Arc<AtomicU32>,
   tasks_failed_in_run: Arc<AtomicU32>,
   shutdown_deadline: Arc<RwLock<Option<DateTime<Utc>>>>,
}

impl QueueManager {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let mut queue = Storage::load_queue().unwrap_or_default();
        let history = Storage::load_history().unwrap_or_default();
        let config = Storage::load_config().unwrap_or_default();

        // 恢复因异常退出而中断的上传任务：标记为失败，显示重试按钮
        let recovered = Self::recover_interrupted_tasks(&mut queue);
        if recovered > 0 {
            log(&format!("启动时发现 {} 个中断的上传任务，已标记为失败，可手动重试", recovered));
            Storage::save_queue(&queue)?;
        }

        Ok(Self {
            queue: Arc::new(RwLock::new(queue)),
            history: Arc::new(RwLock::new(history)),
            config: Arc::new(RwLock::new(config)),
            processing_tasks: Arc::new(DashMap::new()),
            is_uploading: Arc::new(AtomicBool::new(false)),
            stop_after_current: Arc::new(AtomicBool::new(false)),
           tasks_uploaded_in_run: Arc::new(AtomicU32::new(0)),
           tasks_failed_in_run: Arc::new(AtomicU32::new(0)),
           shutdown_deadline: Arc::new(RwLock::new(None)),
       })
    }

    pub async fn add_to_queue(&self, file_path: String, alist_path: String) -> Result<AddToQueueResult, Box<dyn std::error::Error>> {
        let mut added_tasks = Vec::new();
        let mut warnings = Vec::new();
        let target_root = normalize_alist_path(&alist_path);
        log(&format!("开始添加到上传队列: file_path={}, target_root={}", file_path, target_root));
        if is_root_alist_path(&target_root) {
            log(&format!("添加到上传队列被拦截: file_path={}, target_root=/, reason=根目录不是具体上传目录", file_path));
            return Err("请选择 Alist 中的具体目录后再添加文件，根目录 / 仅用于浏览存储入口".into());
        }

        // 去重检查：队列中已存在相同 file_path + alist_path 的待上传任务则跳过
        // 若开启 block_duplicate_file_upload，则同一文件不同目标路径也拦截
        {
            let queue = self.queue.read().await;
            let config = self.config.read().await;
            let block_dup = config.upload.block_duplicate_file_upload;
            drop(config);
            let already_exists = queue.tasks.iter().any(|t| {
                if t.status != TaskStatus::Pending {
                    return false;
                }
                if t.file.path == file_path && t.alist_path == target_root {
                    return true;
                }
                if block_dup && t.file.path == file_path {
                    return true;
                }
                false
            });
            if already_exists {
                let msg = if block_dup {
                    format!("文件已在队列中（可能目标路径不同），已跳过: file_path={}", file_path)
                } else {
                    format!("文件已在队列中，跳过重复添加: file_path={}, target={}", file_path, target_root)
                };
                log(&msg);
                return Ok(AddToQueueResult { tasks: vec![], warnings: vec!["文件已在队列中，已跳过".to_string()] });
            }
        }
        
        if crate::utils::fs::is_directory(&file_path) {
            let files = crate::utils::fs::collect_files_from_dir(&file_path)
                .map_err(|e| format!("收集文件夹文件失败: {}", e))?;
            log(&format!("检测到文件夹，递归收集完成: dir_path={}, file_count={}, target_root={}", file_path, files.len(), target_root));
            
            // 拖入文件夹时，远程也要保留文件夹名这一层，例如 /115Crypt/课本/...
            let folder_name = Path::new(&file_path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            let folder_target = folder_target_root(&target_root, &folder_name);
            log(&format!("文件夹名称已加入目标路径: folder_name={}, target_root={}, folder_target={}",
                Path::new(&file_path).file_name().and_then(|n| n.to_str()).unwrap_or(""),
                target_root,
                folder_target,
            ));

            // 校验目标路径各级目录名 UTF-8 字节数是否超限
            if let Some(over_name) = check_name_bytes_limit(&folder_target) {
                let over_bytes = over_name.len();
                let reasons = vec![BlockedReason::TargetPathTooLong {
                    actual_bytes: over_bytes,
                    limit_bytes: NAME_BYTES_LIMIT,
                    segment: over_name,
                }];
                let msg = format_blocked_reasons(&reasons, &folder_name);
                log(&format!("文件夹被名称长度拦截: folder_name={}, bytes={}", folder_name, over_bytes));
                self.record_blocked_file(
                    &file_path,
                    &folder_name,
                    0,
                    &reasons,
                    &folder_target,
                )
                .await;
                warnings.push(msg);
                return Ok(AddToQueueResult { tasks: added_tasks, warnings });
            }
            
            for file_info in files {
                let validation = self.validate_file(&file_info.name, file_info.size, None).await;
                if !validation.blockers.is_empty() {
                    let message = format_blocked_reasons(&validation.blockers, &file_info.name);
                    log(&format!(
                        "文件夹内文件被拦截: file_path={}, file_name={}, size={}B, reasons={}",
                        file_info.path,
                        file_info.name,
                        file_info.size,
                        message.replace('\n', " | ")
                    ));
                    self.record_blocked_file(
                        &file_info.path,
                        &file_info.name,
                        file_info.size,
                        &validation.blockers,
                        &folder_target,
                    )
                    .await;
                    warnings.push(message);
                    continue;
                }
                if let Some(warning) = validation.warning {
                    log(&format!(
                        "大文件风险提示: file_path={}, file_name={}, size={}B, warning={}",
                        file_info.path, file_info.name, file_info.size, warning
                    ));
                    warnings.push(warning);
                }

                let task = self.add_single_file_to_queue(&file_info, &folder_target).await?;
                added_tasks.push(task);
            }
        } else {
            let (size, name) = crate::utils::fs::get_file_info(&file_path)
                .await
                .map_err(|e| e.to_string())?;

            let validation = self
                .validate_file(&name, size, Some(&target_root))
                .await;
            if !validation.blockers.is_empty() {
                let message = format_blocked_reasons(&validation.blockers, &name);
                log(&format!(
                    "单文件被拦截: file_path={}, file_name={}, size={}B, reasons={}",
                    file_path,
                    name,
                    size,
                    message.replace('\n', " | ")
                ));
                self.record_blocked_file(
                    &file_path,
                    &name,
                    size,
                    &validation.blockers,
                    &target_root,
                )
                .await;
                warnings.push(message.clone());

                let has_name_blocker = validation
                    .blockers
                    .iter()
                    .any(|reason| matches!(reason, BlockedReason::NameTooLong { .. }));
                if has_name_blocker {
                    return Err(message.into());
                }
                return Ok(AddToQueueResult { tasks: added_tasks, warnings });
            }
            if let Some(warning) = validation.warning {
                log(&format!(
                    "大文件风险提示: file_path={}, file_name={}, size={}B, warning={}",
                    file_path, name, size, warning
                ));
                warnings.push(warning);
            }
            
            let mut task = UploadTask::new(file_path.clone(), target_root.clone());
            task.file.size = size;
            task.file.name = name;
            log(&format!("添加单文件任务: file_path={}, file_name={}, size={}B, target_dir={}", file_path, task.file.name, size, target_root));
            
            let mut queue = self.queue.write().await;
            let already_exists = queue.tasks.iter().any(|t| {
                t.status == TaskStatus::Pending
                    && t.file.path == file_path
                    && t.alist_path == target_root
            });
            if already_exists {
                log(&format!("文件已在队列中，跳过: file_path={}, target={}", file_path, target_root));
                return Ok(AddToQueueResult { tasks: added_tasks, warnings });
            }
            queue.tasks.push(task.clone());
            Storage::save_queue(&*queue)?;
            drop(queue);
            
            added_tasks.push(task);
        }

        if added_tasks.is_empty() && !warnings.is_empty() {
            return Err(warnings.join("\n").into());
        }
        
        Ok(AddToQueueResult { tasks: added_tasks, warnings })
    }

    async fn validate_file(
        &self,
        file_name: &str,
        size: u64,
        target_path: Option<&str>,
    ) -> FileValidation {
        let config = self.config.read().await;
        let block_files_over_5gb = config.upload.block_files_over_5gb;
        let warn_files_over_4gb = config.upload.warn_files_over_4gb;
        drop(config);

        let mut blockers = Vec::new();
        if let Some(over_name) = check_name_bytes_limit(file_name) {
            blockers.push(BlockedReason::NameTooLong {
                actual_bytes: over_name.len(),
                limit_bytes: NAME_BYTES_LIMIT,
                segment: over_name,
            });
        }

        if let Some(target_path) = target_path {
            if let Some(over_name) = check_name_bytes_limit(target_path) {
                blockers.push(BlockedReason::TargetPathTooLong {
                    actual_bytes: over_name.len(),
                    limit_bytes: NAME_BYTES_LIMIT,
                    segment: over_name,
                });
            }
        }

        if block_files_over_5gb && size > FIVE_GB {
            blockers.push(BlockedReason::FileTooLarge {
                actual_bytes: size,
                limit_bytes: FIVE_GB,
            });
        }

        let warning = if blockers
            .iter()
            .any(|reason| matches!(reason, BlockedReason::FileTooLarge { .. }))
        {
            None
        } else if warn_files_over_4gb && size > FOUR_GB {
            Some(format!(
                "{} 超过 4GB。大文件上传耗时较长，若 1 小时内未完成可能因 Token 过期导致失败。建议在上传带宽较好时上传，或先压缩/分卷处理。",
                file_name
            ))
        } else {
            None
        };

        FileValidation { blockers, warning }
    }

    async fn record_blocked_file(
        &self,
        file_path: &str,
        file_name: &str,
        file_size: u64,
        reasons: &[BlockedReason],
        target_path: &str,
    ) {
        let record = BlockedFileRecord {
            file_path: file_path.to_string(),
            file_name: file_name.to_string(),
            file_size,
            reason: format_blocked_reasons(reasons, file_name),
            reasons: reasons.to_vec(),
            is_directory: Path::new(file_path).is_dir(),
            blocked_at: chrono::Utc::now(),
            target_path: target_path.to_string(),
            resolved: false,
        };
        let mut data = Storage::load_blocked_files().unwrap_or_default();
        data.records.push(record);
        if let Err(e) = Storage::save_blocked_files(&data) {
            log(&format!("保存拦截记录失败: {}", e));
        }
    }
    
    async fn add_single_file_to_queue(&self, file_info: &crate::models::FileInfo, alist_path: &str) -> Result<UploadTask, Box<dyn std::error::Error>> {
        let target_path = if let Some(ref relative_path) = file_info.relative_path {
            build_target_dir(alist_path, relative_path)
        } else {
            normalize_alist_path(alist_path)
        };
        log(&format!("添加文件夹内文件任务: file_path={}, file_name={}, relative_path={}, target_dir={}", file_info.path, file_info.name, file_info.relative_path.as_deref().unwrap_or(""), target_path));
        
        let mut queue = self.queue.write().await;
        // 去重检查
        let already_exists = queue.tasks.iter().any(|t| {
            t.status == TaskStatus::Pending
                && t.file.path == file_info.path
                && t.alist_path == target_path
        });
        if already_exists {
            log(&format!("文件已在队列中，跳过: file_path={}, target={}", file_info.path, target_path));
            return Ok(UploadTask::new(file_info.path.clone(), target_path));
        }

        let mut task = UploadTask::new(file_info.path.clone(), target_path.clone());
        task.file.size = file_info.size;
        task.file.name = file_info.name.clone();
        task.file.relative_path = file_info.relative_path.clone();

        queue.tasks.push(task.clone());
        Storage::save_queue(&*queue)?;
        drop(queue);

        Ok(task)
    }

    pub async fn remove_from_queue(&self, task_id: String) -> Result<(), Box<dyn std::error::Error>> {
        let mut queue = self.queue.write().await;
        queue.tasks.retain(|t| t.id != task_id);
        Storage::save_queue(&*queue)?;
        Ok(())
    }

    pub async fn clear_queue(&self) -> Result<(), Box<dyn std::error::Error>> {
        let mut queue = self.queue.write().await;
        queue.tasks.clear();
        Storage::save_queue(&*queue)?;
        Ok(())
    }

    pub async fn get_history(&self) -> Vec<UploadTask> {
        let history = self.history.read().await;
        history.records.clone()
    }

    pub async fn clear_history(&self) -> Result<(), Box<dyn std::error::Error>> {
        let mut history = self.history.write().await;
        history.records.clear();
        Storage::save_history(&*history)?;
        Ok(())
    }

    pub async fn save_config(&self, config: AppConfig) -> Result<(), Box<dyn std::error::Error>> {
        let mut config_guard = self.config.write().await;
        *config_guard = config.clone();
        Storage::save_config(&config)?;
        Ok(())
    }

    pub async fn update_task(&self, task_id: String, task: UploadTask) -> Result<(), Box<dyn std::error::Error>> {
        let mut queue = self.queue.write().await;
        if let Some(existing) = queue.tasks.iter_mut().find(|t| t.id == task_id) {
            *existing = task;
        }
        Storage::save_queue(&*queue)?;
        Ok(())
    }

    pub async fn claim_next_pending_task(&self) -> Option<UploadTask> {
        let mut queue = self.queue.write().await;
        let task = queue.tasks.iter_mut()
            .find(|t| t.status == TaskStatus::Pending)?;

        task.mark_uploading();
        let claimed = task.clone();
        if let Err(error) = Storage::save_queue(&*queue) {
            log(&format!("抢占待上传任务后保存队列失败: task_id={}, file={}, error={}", claimed.id, claimed.file.name, error));
        }
        log(&format!("抢占待上传任务: task_id={}, file={}, alist_path={}", claimed.id, claimed.file.name, claimed.alist_path));
        Some(claimed)
    }

    pub async fn remove_completed_from_queue(&self, task_id: String) -> Result<(), Box<dyn std::error::Error>> {
        let mut queue = self.queue.write().await;
        queue.tasks.retain(|t| t.id != task_id);
        Storage::save_queue(&*queue)?;
        Ok(())
    }

    pub async fn add_to_history(&self, task: UploadTask) -> Result<(), Box<dyn std::error::Error>> {
        let mut history = self.history.write().await;
        // 去重：相同文件路径 + 目标路径的旧记录替换为新记录
        history.records.retain(|r| !(r.file.path == task.file.path && r.alist_path == task.alist_path));
        history.records.insert(0, task);
        
        // 按保留天数清理过期记录（never_clean 开启时跳过）
        let config = self.config.read().await;
        let never_clean = config.history.never_clean;
        let retention_days = config.history.retention_days;
        drop(config);
        if !never_clean {
            let cutoff = chrono::Utc::now() - chrono::Duration::days(retention_days as i64);
            history.records.retain(|r| {
                let ts = r.end_time.unwrap_or(r.created_at);
                ts > cutoff
            });
        }
        
        Storage::save_history(&*history)?;
        Ok(())
    }

    pub fn is_uploading(&self) -> bool {
        self.is_uploading.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn set_uploading(&self, value: bool) {
        self.is_uploading.store(value, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn clone_inner(&self) -> Arc<QueueManager> {
        Arc::new(Self {
            queue: Arc::clone(&self.queue),
            history: Arc::clone(&self.history),
            config: Arc::clone(&self.config),
            processing_tasks: Arc::clone(&self.processing_tasks),
            is_uploading: Arc::clone(&self.is_uploading),
            stop_after_current: Arc::clone(&self.stop_after_current),
           tasks_uploaded_in_run: Arc::clone(&self.tasks_uploaded_in_run),
           tasks_failed_in_run: Arc::clone(&self.tasks_failed_in_run),
           shutdown_deadline: Arc::clone(&self.shutdown_deadline),
       })
    }

    pub fn stop_after_current(&self) -> bool {
        self.stop_after_current.load(Ordering::SeqCst)
    }

    pub fn set_stop_after_current(&self, value: bool) {
        self.stop_after_current.store(value, Ordering::SeqCst);
    }

    pub fn tasks_uploaded_in_run(&self) -> u32 {
        self.tasks_uploaded_in_run.load(Ordering::SeqCst)
    }

   pub fn increment_tasks_uploaded(&self) {
       self.tasks_uploaded_in_run.fetch_add(1, Ordering::SeqCst);
   }

   pub fn tasks_failed_in_run(&self) -> u32 {
       self.tasks_failed_in_run.load(Ordering::SeqCst)
   }

   pub fn increment_tasks_failed(&self) {
       self.tasks_failed_in_run.fetch_add(1, Ordering::SeqCst);
   }

   pub fn reset_tasks_uploaded(&self) {
       self.tasks_uploaded_in_run.store(0, Ordering::SeqCst);
       self.tasks_failed_in_run.store(0, Ordering::SeqCst);
   }

    pub async fn set_shutdown_deadline(&self, deadline: DateTime<Utc>) {
        let mut guard = self.shutdown_deadline.write().await;
        *guard = Some(deadline);
    }

    pub async fn clear_shutdown_deadline(&self) {
        let mut guard = self.shutdown_deadline.write().await;
        *guard = None;
    }

    pub async fn get_shutdown_deadline(&self) -> Option<DateTime<Utc>> {
        let guard = self.shutdown_deadline.read().await;
        *guard
    }

    pub async fn mark_queue_failed(
        &self,
        file_name: String,
        error: String,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // 记录失败信息到日志
        log::error!(
            "队列因文件 '{}' 失败而停止: {}",
            file_name,
            error
        );
        
        Ok(())
     }
 }
 
impl QueueManager {
    /// 将队列中所有状态为 `Uploading` 的任务标记为 `Failed`，
    /// 原因是应用异常退出，原上传进程已丢失。
    /// 返回被恢复的任务数量。
    fn recover_interrupted_tasks(queue: &mut QueueData) -> usize {
        let mut count = 0;
        for task in &mut queue.tasks {
            if task.status == TaskStatus::Uploading {
                task.status = TaskStatus::Failed;
                task.error = Some("上传中断：应用异常退出，原上传进程已丢失，请点击重试重新上传".to_string());
                task.progress = 0;
                count += 1;
            }
        }
        count
    }
}

pub fn is_root_alist_path(path: &str) -> bool {
    normalize_alist_path(path) == "/"
}

fn normalize_alist_path(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() || trimmed == "/" {
        return "/".to_string();
    }

    let with_prefix = if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{}", trimmed)
    };

    with_prefix.trim_end_matches('/').to_string()
}

fn build_target_dir(root: &str, relative_path: &str) -> String {
    let root = normalize_alist_path(root);
    let relative_parent = Path::new(relative_path)
        .parent()
        .and_then(|parent| parent.to_str())
        .unwrap_or("")
        .trim_matches('/');

    if relative_parent.is_empty() {
        root
    } else if root == "/" {
        format!("/{}", relative_parent.replace('\\', "/"))
    } else {
        format!("{}/{}", root, relative_parent.replace('\\', "/"))
    }
}

fn folder_target_root(target_root: &str, folder_name: &str) -> String {
    if folder_name.is_empty() {
        return target_root.to_string();
    }

    let root = normalize_alist_path(target_root);
    if root == "/" {
        format!("/{}", folder_name)
    } else {
        format!("{}/{}", root, folder_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_target_dir_with_parent() {
        let result = build_target_dir("/115Crypt", "12\\102号\\IMG\\_8689.MOV");
        assert_eq!(result, "/115Crypt/12/102号/IMG");
    }

    #[test]
    fn test_build_target_dir_root() {
        let result = build_target_dir("/", "folder\\sub\\file.txt");
        assert_eq!(result, "/folder/sub");
    }

    #[test]
    fn test_build_target_dir_no_parent() {
        let result = build_target_dir("/115Crypt", "file.txt");
        assert_eq!(result, "/115Crypt");
    }

    #[test]
    fn test_build_target_dir_normalize_root() {
        let result = build_target_dir("/115Crypt/", "sub\\file.txt");
        assert_eq!(result, "/115Crypt/sub");
    }

    #[test]
    fn test_folder_target_keeps_folder_name() {
        let folder_target = folder_target_root("/115Crypt", "课本");
        assert_eq!(folder_target, "/115Crypt/课本");
    }

    #[test]
    fn test_folder_target_root() {
        let folder_target = folder_target_root("/", "课本");
        assert_eq!(folder_target, "/课本");
    }

    #[test]
    fn test_folder_target_empty_folder_name() {
        let folder_target = folder_target_root("/115Crypt", "");
        assert_eq!(folder_target, "/115Crypt");
    }

    #[test]
    fn test_folder_target_normalizes_root() {
        let folder_target = folder_target_root("/115Crypt/", "课本");
        assert_eq!(folder_target, "/115Crypt/课本");
    }

    #[test]
    fn test_build_target_dir_with_folder_root() {
        let folder_target = "/115Crypt/课本";
        let result = build_target_dir(&folder_target, "12\\102号\\IMG\\_8689.MOV");
        assert_eq!(result, "/115Crypt/课本/12/102号/IMG");
    }
}
