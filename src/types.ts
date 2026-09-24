export type TaskStatus = 'pending' | 'uploading' | 'completed' | 'failed' | 'cancelled';

export interface FileInfo {
  path: string;
  name: string;
  size: number;
}

export interface UploadTask {
  id: string;
  file: FileInfo;
  alist_path: string;
  status: TaskStatus;
  progress: number;
  retry_count: number;
  error?: string;
  start_time?: string;
  end_time?: string;
  duration?: number;
  created_at: string;
  updated_at: string;
  speed: number;
  resolved: boolean;
  api_response?: string | null;
}

export interface BlockedFileRecord {
  file_path: string;
  file_name: string;
  file_size: number;
  reason: string;
  reasons?: BlockedReason[];
  is_directory?: boolean;
  blocked_at: string;
  target_path: string;
  resolved: boolean;
}

export type BlockedReason =
  | {
      kind: 'name_too_long';
      segment: string;
      actual_bytes: number;
      limit_bytes: number;
    }
  | {
      kind: 'file_too_large';
      actual_bytes: number;
      limit_bytes: number;
    }
  | {
      kind: 'target_path_too_long';
      segment: string;
      actual_bytes: number;
      limit_bytes: number;
    };

export interface AddToQueueResult {
  tasks: UploadTask[];
  warnings: string[];
}

export interface AlistConfig {
  base_url: string;
  token: string;
  username: string;
  password: string;
  auto_login: boolean;
  exe_path: string;
  kill_on_exit: boolean;
  run_in_background: boolean;
  use_system_proxy: boolean;
}

export interface DirItem {
  name: string;
  size: number;
  is_dir: boolean;
  modified?: string;
  sign?: string;
}

export interface ScheduledUpload {
  enabled: boolean;
  start_time: string;
  end_time: string;
  notify_on_start: boolean;
  notify_on_stop: boolean;
}

export interface NotificationConfig {
  enabled: boolean;
  webhook_url: string;
  channels: string[];
}

export interface UploadConfig {
  concurrency: number;
  max_retries: number;
  max_tasks_per_run: number;
  check_update_on_startup: boolean;
  auto_start_on_boot: boolean;
  progress_notify_enabled: boolean;
  progress_notify_interval: number;
  refresh_index_after_upload: boolean;
  rar_path: string;
  split_volume_mb: number;
  speed_limit: number;
  as_task: boolean;
  upload_method: string;
  last_alist_path: string;
  block_files_over_5gb: boolean;
  warn_files_over_4gb: boolean;
  file_exists_strategy: {
    strategy: string;
  };
  fail_action: string;
  show_progress: boolean;
  block_duplicate_file_upload: boolean;
  delete_source_after_compress: boolean;
 notify_on_complete: boolean;
 notify_feishu_on_queue_complete: boolean;
 shutdown_after_complete: boolean;
 shutdown_delay_minutes: number;
 minimize_on_close: boolean;
  schedule?: ScheduledUpload;
  notification?: NotificationConfig;
}

export interface HistoryConfig {
  retention_days: number;
  never_clean: boolean;
}

export interface HistoryPage {
  tasks: UploadTask[];
  total: number;
  page: number;
  page_size: number;
  total_pages: number;
  stats?: HistoryStats;
}

export interface HistoryStats {
  total: number;
  completed: number;
  failed: number;
  total_bytes: number;
  today_count: number;
  today_bytes: number;
  month_bytes: number;
  avg_speed: number;
}

export interface LogSyncConfig {
  enabled: boolean;
  base_url: string;
  username: string;
  password: string;
  token: string;
  target_path: string;
  sync_on_exit: boolean;
  sync_interval_minutes: number;
  use_system_proxy: boolean;
  last_sync_at?: string | null;
}

export interface LocalLogFileInfo {
  name: string;
  size: number;
  modified?: string;
}

export interface LogSyncResult {
  total: number;
  success: number;
  failed: number;
  details: string[];
  last_sync_at?: string | null;
}

export interface AppConfig {
  alist: AlistConfig;
  upload: UploadConfig;
  history: HistoryConfig;
  log_sync?: LogSyncConfig;
}

export const DEFAULT_APP_CONFIG: AppConfig = {
  alist: {
    base_url: 'http://127.0.0.1:5244',
    token: '',
    username: '',
    password: '',
    auto_login: true,
    exe_path: '',
    kill_on_exit: true,
    run_in_background: true,
    use_system_proxy: false,
  },
  upload: {
    concurrency: 1,
    max_retries: 5,
    max_tasks_per_run: 0,
    check_update_on_startup: true,
    auto_start_on_boot: false,
    progress_notify_enabled: false,
    progress_notify_interval: 30,
    refresh_index_after_upload: true,
    rar_path: 'C:\\Program Files\\WinRAR\\rar.exe',
    split_volume_mb: 2000,
    speed_limit: 0,
    as_task: true,
    upload_method: 'stream',
    last_alist_path: '/',
    block_files_over_5gb: true,
    warn_files_over_4gb: true,
    file_exists_strategy: {
      strategy: 'ask',
    },
    fail_action: 'stop',
    show_progress: false,
    block_duplicate_file_upload: true,
    delete_source_after_compress: true,
   notify_on_complete: false,
   notify_feishu_on_queue_complete: false,
   shutdown_after_complete: false,
   shutdown_delay_minutes: 10,
   minimize_on_close: true,
    schedule: {
      enabled: false,
      start_time: '03:00',
      end_time: '07:00',
      notify_on_start: false,
      notify_on_stop: false,
    },
    notification: {
      enabled: false,
      webhook_url: '',
      channels: ['feishu'],
    },
  },
  history: {
    retention_days: 30,
    never_clean: true,
  },
  log_sync: {
    enabled: false,
    base_url: '',
    username: '',
    password: '',
    token: '',
    target_path: '/本地磁盘/openlist-uploader-logs',
    sync_on_exit: true,
    sync_interval_minutes: 30,
    use_system_proxy: false,
    last_sync_at: null,
  },
};

export const normalizeAppConfig = (config?: Partial<AppConfig> | null): AppConfig => ({
  alist: {
    ...DEFAULT_APP_CONFIG.alist,
    ...config?.alist,
  },
  upload: {
    ...DEFAULT_APP_CONFIG.upload,
    ...config?.upload,
    file_exists_strategy: {
      ...DEFAULT_APP_CONFIG.upload.file_exists_strategy,
      ...config?.upload?.file_exists_strategy,
    },
    schedule: {
      ...DEFAULT_APP_CONFIG.upload.schedule!,
      ...config?.upload?.schedule,
    },
    notification: {
      ...DEFAULT_APP_CONFIG.upload.notification!,
      ...config?.upload?.notification,
    },
  },
  history: {
    ...DEFAULT_APP_CONFIG.history,
    ...config?.history,
  } as HistoryConfig,
  log_sync: {
    ...DEFAULT_APP_CONFIG.log_sync,
    ...config?.log_sync,
  } as LogSyncConfig,
});
