import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import { DEFAULT_APP_CONFIG, normalizeAppConfig, type AddToQueueResult, type UploadTask, type AppConfig, type TaskStatus, type HistoryPage, type CleanupRecord, type CleanupItemResult, type RequeueFailedResult } from '../types';

interface AppState {
  queue: UploadTask[];
  history: UploadTask[];
  historyPage: HistoryPage | null;
  cleanupRecords: CleanupRecord[];
  config: AppConfig;
  isUploading: boolean;
  isLoading: boolean;
  configLoaded: boolean;
  alistConnected: boolean;
  alistServiceAvailable: boolean;
  alistChecking: boolean;
  isStopping: boolean;

  // Actions
  loadQueue: () => Promise<void>;
  addToFileQueue: (filePath: string, alistPath: string) => Promise<AddToQueueResult>;
  removeFromQueue: (taskId: string) => Promise<void>;
  clearQueue: () => Promise<void>;
  loadHistory: () => Promise<void>;
  loadHistoryPage: (page: number, pageSize: number, statusFilter: string, searchText: string, sortOrder: string) => Promise<void>;
  clearHistory: () => Promise<void>;
  loadCleanupList: () => Promise<void>;
  cleanupItems: (ids: string[]) => Promise<CleanupItemResult[]>;
  dismissCleanupItem: (id: string) => Promise<void>;
  retryMarkCleanupItem: (id: string) => Promise<string>;
  requeueFailedTasks: () => Promise<RequeueFailedResult>;
  loadConfig: () => Promise<void>;
  saveConfig: (config: AppConfig) => Promise<void>;
  startUpload: () => Promise<void>;
  pauseUpload: () => Promise<void>;
  retryUpload: (taskId: string) => Promise<void>;
  testConnection: (config?: AppConfig) => Promise<boolean>;
  checkHealth: () => Promise<void>;
  setIsUploading: (value: boolean) => void;
  setIsStopping: (value: boolean) => void;
  startHealthCheck: () => void;
  stopHealthCheck: () => void;
  fastCheckHealth: () => Promise<void>;
  login: (baseUrl: string, username: string, password: string) => Promise<void>;
}

let healthCheckInterval: ReturnType<typeof setInterval> | null = null;

export const useAppStore = create<AppState>((set, get) => ({
  queue: [],
  history: [],
  historyPage: null,
  cleanupRecords: [],
  config: DEFAULT_APP_CONFIG,
  isUploading: false,
  isLoading: true,
  configLoaded: false,
  alistConnected: false,
  alistServiceAvailable: false,
  alistChecking: false,
  isStopping: false,

  loadQueue: async () => {
    try {
      const queue = await invoke<UploadTask[]>('get_queue');
      set({ queue, isLoading: false });
    } catch (error) {
      console.error('Failed to load queue:', error);
      set({ isLoading: false });
    }
  },

  addToFileQueue: async (filePath, alistPath) => {
    const result = await invoke<AddToQueueResult>('add_to_queue', { filePath, alistPath });
    set(state => ({ queue: [...state.queue, ...result.tasks] }));
    return result;
  },

  removeFromQueue: async (taskId) => {
    await invoke('remove_from_queue', { taskId });
    set(state => ({
      queue: state.queue.filter(t => t.id !== taskId)
    }));
  },

  clearQueue: async () => {
    await invoke('clear_queue');
    set({ queue: [] });
  },

  loadHistory: async () => {
    const history = await invoke<UploadTask[]>('get_history');
    set({ history });
  },

  loadHistoryPage: async (page, pageSize, statusFilter, searchText, sortOrder) => {
    const result = await invoke<HistoryPage>('get_history_page', {
      page,
      pageSize,
      statusFilter: statusFilter || null,
      searchText: searchText || null,
      sortOrder: sortOrder || null,
    });
    set({ historyPage: result, history: result.tasks });
  },

  clearHistory: async () => {
    await invoke('clear_history');
    set({ history: [] });
  },

  loadCleanupList: async () => {
    try {
      const records = await invoke<CleanupRecord[]>('get_cleanup_list');
      set({ cleanupRecords: records });
    } catch (error) {
      console.error('Failed to load cleanup list:', error);
    }
  },

  cleanupItems: async (ids) => {
    const results = await invoke<CleanupItemResult[]>('cleanup_items', { ids });
    set(state => ({
      cleanupRecords: state.cleanupRecords.filter(r => !results.some(res => res.id === r.id && res.success))
    }));
    return results;
  },

  dismissCleanupItem: async (id) => {
    await invoke('dismiss_cleanup_item', { id });
    set(state => ({
      cleanupRecords: state.cleanupRecords.filter(r => r.id !== id)
    }));
  },

  retryMarkCleanupItem: async (id) => {
    const newPath = await invoke<string>('retry_mark_cleanup_item', { id });
    set(state => ({
      cleanupRecords: state.cleanupRecords.map(r =>
        r.id === id ? { ...r, path: newPath, marked: true } : r
      )
    }));
    return newPath;
  },

  requeueFailedTasks: async () => {
    const result = await invoke<RequeueFailedResult>('requeue_failed_history_tasks');
    if (result.requeued > 0) {
      const queue = await invoke<UploadTask[]>('get_queue');
      set({ queue });
    }
    return result;
  },

  loadConfig: async () => {
    try {
      const config = await invoke<AppConfig>('get_config');
      set({ config: normalizeAppConfig(config), configLoaded: true });
    } catch (error) {
      console.error('Failed to load config:', error);
      set({ config: DEFAULT_APP_CONFIG, configLoaded: true });
    }
  },

  saveConfig: async (config) => {
    const normalizedConfig = normalizeAppConfig(config);
    await invoke('save_config', { config: normalizedConfig });
    set({ config: normalizedConfig, configLoaded: true });
  },

  startUpload: async () => {
    await invoke('start_upload');
    set({ isUploading: true });
  },

  pauseUpload: async () => {
    set({ isStopping: true });
    await invoke('stop_after_current');
    // 开始轮询检查是否已停止
    const checkStopped = async () => {
      const queue = await invoke<UploadTask[]>('get_queue');
      const uploadingTasks = queue.filter(t => t.status === 'uploading');
      if (uploadingTasks.length === 0) {
        set({ isUploading: false, isStopping: false });
      } else {
        setTimeout(checkStopped, 1000);
      }
    };
    checkStopped();
  },

  retryUpload: async (taskId) => {
    await invoke('retry_upload', { taskId });
    set(state => ({
      queue: state.queue.map(t => 
        t.id === taskId 
          ? { ...t, status: 'pending' as TaskStatus, retry_count: 0, error: undefined, progress: 0 }
          : t
      )
    }));
  },

  testConnection: async (config) => {
    const targetConfig = normalizeAppConfig(config ?? get().config);
    return await invoke<boolean>('test_alist_connection', { config: targetConfig });
  },

  setIsUploading: (value) => {
    set({ isUploading: value });
  },

  setIsStopping: (value) => {
    set({ isStopping: value });
  },

  checkHealth: async () => {
    const state = get();
    if (!state.config) return;

    try {
      const serviceAvailable = await invoke<boolean>('check_health', { config: state.config });

      // 同步后端上传状态（定时上传/异常自愈场景后端可能自行启动调度器）
      try {
        const backendUploading = await invoke<boolean>('get_is_uploading');
        if (backendUploading !== state.isUploading) {
          set({ isUploading: backendUploading });
        }
      } catch {
        // 忽略状态同步失败
      }

      if (serviceAvailable) {
        const loggedIn = await invoke<boolean>('test_alist_connection', { config: state.config });
        set({ alistServiceAvailable: true, alistConnected: loggedIn, alistChecking: false });
      } else {
        set({ alistServiceAvailable: false, alistConnected: false, alistChecking: false });
      }
    } catch (error) {
      console.error('Health check failed:', error);
      set({ alistServiceAvailable: false, alistConnected: false, alistChecking: false });
    }
  },

  login: async (baseUrl: string, username: string, password: string) => {
    console.log('[login] 开始登录流程:', { baseUrl, username });
    await invoke<string>('alist_login', { baseUrl, username, password });
    console.log('[login] 登录成功');
    await get().loadConfig();
    set({ alistConnected: true });
  },

  startHealthCheck: () => {
    if (healthCheckInterval) return;
    
    // 立即检查一次
    get().checkHealth();
    
    // 每 30 秒检查一次（常态）
    healthCheckInterval = setInterval(() => {
      get().checkHealth();
    }, 30000);
  },

  fastCheckHealth: async () => {
    // 加速检测：每 3 秒检查一次，直到服务就绪或达到 40 次（约 120 秒）
    for (let i = 0; i < 40; i++) {
      await get().checkHealth();
      const state = get();
      if (state.alistServiceAvailable) break;
      await new Promise(resolve => setTimeout(resolve, 3000));
    }
  },

  stopHealthCheck: () => {
    if (healthCheckInterval) {
      clearInterval(healthCheckInterval);
      healthCheckInterval = null;
    }
  },
}));
