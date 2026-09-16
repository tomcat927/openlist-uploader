use std::sync::{Arc, Mutex};
use std::time::Duration;
use chrono::{Local, Timelike};
use tokio::time::sleep;
use crate::models::ScheduledUpload;
use crate::services::queue_manager::QueueManager;
use crate::services::upload_scheduler::{UploadScheduler, ScheduleStats};

pub struct ScheduleManager {
    queue_manager: Arc<QueueManager>,
    last_start_event: Mutex<Option<String>>,
    last_stop_event: Mutex<Option<String>>,
}

impl ScheduleManager {
    pub fn new(queue_manager: Arc<QueueManager>) -> Self {
        Self {
            queue_manager,
            last_start_event: Mutex::new(None),
            last_stop_event: Mutex::new(None),
        }
    }

    async fn send_schedule_notification_if_enabled(
        &self,
        schedule: &ScheduledUpload,
        event_type: &str,
    ) {
        let enabled = match event_type {
            "start" => schedule.notify_on_start,
            "stop" => schedule.notify_on_stop,
            _ => return,
        };
        if !enabled {
            return;
        }

        let config = self.queue_manager.config.read().await;
        let notification = config.upload.notification.clone().filter(|n| n.enabled && !n.webhook_url.is_empty());
        drop(config);

        let Some(notification) = notification else { return };

        let event_key = format!("{}|{}", Local::now().format("%Y-%m-%d"), event_type);
        let should_send = {
            let mut last_event = if event_type == "start" {
                self.last_start_event.lock().unwrap()
            } else {
                self.last_stop_event.lock().unwrap()
            };
            if *last_event == Some(event_key.clone()) {
                false
            } else {
                *last_event = Some(event_key);
                true
            }
        };

        if should_send {
            let stats = self.collect_schedule_stats().await;
            UploadScheduler::send_schedule_notification(
                &notification,
                event_type,
                &schedule.start_time,
                &schedule.end_time,
                stats,
            ).await;
        }
    }

    async fn collect_schedule_stats(&self) -> ScheduleStats {
        let queue = self.queue_manager.queue.read().await;
        let pending_count = queue.tasks.iter().filter(|t| t.status == crate::models::TaskStatus::Pending).count();
        let pending_bytes = queue.tasks.iter()
            .filter(|t| t.status == crate::models::TaskStatus::Pending)
            .map(|t| t.file.size)
            .sum();
        let uploading_count = queue.tasks.iter().filter(|t| t.status == crate::models::TaskStatus::Uploading).count();
        drop(queue);

        let uploaded_count = self.queue_manager.tasks_uploaded_in_run();
        let failed_count = self.queue_manager.tasks_failed_in_run();
        let history = self.queue_manager.history.read().await;
        let uploaded_bytes = history.records.iter()
            .take(uploaded_count as usize)
            .filter(|t| t.status == crate::models::TaskStatus::Completed)
            .map(|t| t.file.size)
            .sum();
        drop(history);

        ScheduleStats {
            pending_count,
            pending_bytes,
            uploaded_count,
            uploaded_bytes,
            failed_count,
            uploading_count,
        }
    }

    pub async fn start_schedule_monitor(&self) {
        loop {
            sleep(Duration::from_secs(30)).await; // 每 30 秒检查一次

            let config = self.queue_manager.config.read().await;
            let schedule = match &config.upload.schedule {
                Some(s) if s.enabled => Some(s.clone()),
                _ => None,
            };
            drop(config);
            
            let Some(schedule) = schedule else { continue };

            let now = Local::now();
            let current_time = format!("{:02}:{:02}", now.hour(), now.minute());

            // 检查是否到达开始时间
            if current_time == schedule.start_time {
                // 确保不在上传中
                if !self.queue_manager.is_uploading() {
                    log::info!("定时上传时间到：{}", schedule.start_time);
                    
                    // 检查队列是否有任务
                    let queue = self.queue_manager.queue.read().await;
                    let has_pending = queue.tasks.iter().any(|t| t.status == crate::models::TaskStatus::Pending);
                    drop(queue);

                    if has_pending {
                        // 重置停止标志
                        self.queue_manager.set_stop_after_current(false);
                        
                        // 启动上传
                        let scheduler = UploadScheduler::new(self.queue_manager.clone_inner());
                        tokio::spawn(async move {
                            scheduler.start_scheduler().await;
                        });

                        self.send_schedule_notification_if_enabled(&schedule, "start").await;
                    }
                }
            }

            // 检查是否到达结束时间
            if current_time == schedule.end_time {
                log::info!("定时上传结束时间到：{}", schedule.end_time);
                // 设置停止标志（等待当前任务完成）
                self.queue_manager.set_stop_after_current(true);

                // 队列已全部完成时跳过"到点结束"通知（完成通知已由队列完成通知发出，避免重复打扰）
                let has_remaining = {
                    let queue = self.queue_manager.queue.read().await;
                    queue.tasks.iter().any(|t| {
                        t.status == crate::models::TaskStatus::Pending
                            || t.status == crate::models::TaskStatus::Uploading
                    })
                };
                if has_remaining {
                    self.send_schedule_notification_if_enabled(&schedule, "stop").await;
                } else {
                    log::info!("定时上传结束时间到，但队列已全部完成，跳过到点结束通知");
                }
            }
        }
    }
}
