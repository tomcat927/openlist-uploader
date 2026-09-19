import { useEffect, useState, useRef, useMemo, useCallback, Fragment } from 'react';
import { useAppStore } from './store/appStore';
import { open, ask } from '@tauri-apps/plugin-dialog';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getVersion } from '@tauri-apps/api/app';
import { isPermissionGranted, requestPermission, sendNotification } from '@tauri-apps/plugin-notification';
import { relaunch } from '@tauri-apps/plugin-process';
import { FolderPicker } from './components/FolderPicker';
import { DEFAULT_APP_CONFIG, normalizeAppConfig, type AppConfig, type BlockedFileRecord, type UploadTask, type LocalLogFileInfo, type LogSyncResult } from './types';
import './App.css';

const FOUR_GB = 4 * 1024 * 1024 * 1024;

function App() {
  const {
    queue,
    history,
    historyPage,
    config,
    configLoaded,
    isUploading,
    isLoading,
    alistConnected,
    alistServiceAvailable,
    alistChecking,
    isStopping,
    loadQueue,
    addToFileQueue,
    removeFromQueue,
    clearQueue,
    loadHistory,
    loadHistoryPage,
    clearHistory,
    loadConfig,
    saveConfig,
    startUpload,
    pauseUpload,
    retryUpload,
    testConnection,
    setIsUploading,
    startHealthCheck,
    stopHealthCheck,
    login,
  } = useAppStore();

  const [alistPath, setAlistPath] = useState('/');
  const [recentPaths, setRecentPaths] = useState<string[]>(() => {
    try {
      const stored = localStorage.getItem('openlist-uploader:recent-paths')
        ?? localStorage.getItem('alist-uploader:recent-paths');
      if (stored) {
        const parsed = JSON.parse(stored);
        localStorage.removeItem('alist-uploader:recent-paths');
        localStorage.setItem('openlist-uploader:recent-paths', JSON.stringify(parsed));
        return parsed;
      }
      return [];
    } catch {
      return [];
    }
  });

  const addRecentPath = useCallback((path: string) => {
    setRecentPaths(prev => {
      const filtered = prev.filter(p => p !== path);
      const next = [path, ...filtered].slice(0, 8);
      try {
        localStorage.setItem('openlist-uploader:recent-paths', JSON.stringify(next));
      } catch {}
      return next;
    });
  }, []);
  const [activeTab, setActiveTab] = useState<'queue' | 'history' | 'settings' | 'blocked'>('queue');
  const [configForm, setConfigForm] = useState<AppConfig>(DEFAULT_APP_CONFIG);
  const [blockedFiles, setBlockedFiles] = useState<BlockedFileRecord[]>([]);
  const [connectionStatus, setConnectionStatus] = useState<'idle' | 'success' | 'error'>('idle');
  const [connectionMessage, setConnectionMessage] = useState('');
  const [uploadPathError, setUploadPathError] = useState('');
  const [showPassword, setShowPassword] = useState(false);
  const [speedLimitCustomMode, setSpeedLimitCustomMode] = useState(false);
  const [speedLimitCustomText, setSpeedLimitCustomText] = useState('');
  const [downloadingUpdate, setDownloadingUpdate] = useState(false);
  const [testingAlist, setTestingAlist] = useState(false);
  const [downloadProgress, setDownloadProgress] = useState(0);
  const [downloadSizeText, setDownloadSizeText] = useState('');
  const [expandedTaskId, setExpandedTaskId] = useState<string | null>(null);
  const [expandedHistoryTaskId, setExpandedHistoryTaskId] = useState<string | null>(null);
  const [expandedBlockedIndex, setExpandedBlockedIndex] = useState<number | null>(null);
  const [compressingIndex, setCompressingIndex] = useState<number | null>(null);
  const [compressProgress, setCompressProgress] = useState(0);
  const [queueFilter, setQueueFilter] = useState<'all' | 'pending' | 'uploading'>('all');
  const [selectedTaskIds, setSelectedTaskIds] = useState<Set<string>>(new Set());
  const [queueSearchText, setQueueSearchText] = useState('');
  const [blockedSearchText, setBlockedSearchText] = useState('');
  const [blockedSortOrder, setBlockedSortOrder] = useState<'desc' | 'asc'>('desc');
  const [saveConfigStatus, setSaveConfigStatus] = useState<'idle' | 'saving' | 'success' | 'error'>('idle');
  const [saveConfigMessage, setSaveConfigMessage] = useState('');
  const [historyFilter, setHistoryFilter] = useState<'all' | 'completed' | 'failed'>('all');
  const [historySortOrder, setHistorySortOrder] = useState<'desc' | 'asc'>('desc');
  const [historySearchText, setHistorySearchText] = useState('');
  const [historyCurrentPage, setHistoryCurrentPage] = useState(1);
  const [historyPageSize] = useState(50);
  const [historyRetryStatus, setHistoryRetryStatus] = useState<Record<string, 'idle' | 'queued' | 'error'>>({});
  const [appVersion, setAppVersion] = useState('');
  const [shutdownDeadline, setShutdownDeadline] = useState<string | null>(null);
  const [shutdownCountdown, setShutdownCountdown] = useState('');
  const [logSyncStatus, setLogSyncStatus] = useState<'idle' | 'syncing' | 'success' | 'error'>('idle');
  const [logSyncMessage, setLogSyncMessage] = useState('');
  const [logSyncLoginStatus, setLogSyncLoginStatus] = useState<'idle' | 'success' | 'error'>('idle');
  const [logSyncLoginMessage, setLogSyncLoginMessage] = useState('');
  const [showLogSyncPassword, setShowLogSyncPassword] = useState(false);
  const [localLogFiles, setLocalLogFiles] = useState<LocalLogFileInfo[]>([]);
const historyRetryTimerRef = useRef<Record<string, number>>({});
  const autoLoginRef = useRef(false);
  const configInitializedRef = useRef(false);
  const startupUpdateCheckedRef = useRef(false);
  const alistPathRef = useRef('/');
  const savePathTimerRef = useRef<number | null>(null);
  const speedLimitSaveTimerRef = useRef<number | null>(null);
  const notifiedTaskIds = useRef<Set<string>>(new Set());

  const normalizeAlistPath = (path: string) => {
    const trimmed = path.trim();
    if (!trimmed || trimmed === '/') return '/';
    const withPrefix = trimmed.startsWith('/') ? trimmed : `/${trimmed}`;
    return withPrefix.replace(/\/+$/, '') || '/';
  };

  const loadBlockedFiles = async () => {
    try {
      const records = await invoke<BlockedFileRecord[]>('get_blocked_files');
      setBlockedFiles(records);
    } catch (error) {
      console.error('Failed to load blocked files:', error);
    }
  };

  const fetchHistoryPage = useCallback(async (page: number, filter: string, search: string, sort: string) => {
    try {
      await loadHistoryPage(page, historyPageSize, filter, search, sort);
      setHistoryCurrentPage(page);
    } catch (error) {
      console.error('Failed to load history page:', error);
    }
  }, [loadHistoryPage, historyPageSize]);

  const removeBlockedFile = async (index: number) => {
    await invoke('remove_blocked_file', { index });
    await loadBlockedFiles();
  };

  const resolveBlockedFile = async (index: number) => {
    await invoke('resolve_blocked_file', { index });
    await loadBlockedFiles();
  };

  const handleSplitCompress = async (record: BlockedFileRecord, index: number) => {
    setCompressingIndex(index);
    setCompressProgress(0);
    try {
      await writeClientLog(`开始分卷压缩: file=${record.file_path}, target=${record.target_path}`);
      const outDir = await invoke<string>('split_compress_file', { filePath: record.file_path });
      await writeClientLog(`分卷压缩完成: out_dir=${outDir}`);

      // 自动加入上传队列（分卷文件夹 → 原目标路径）
      const result = await addToFileQueue(outDir, record.target_path);
      if (result.warnings.length > 0) {
        window.alert(result.warnings.join('\n'));
      }

      // 标记拦截记录已处理
      await resolveBlockedFile(index);
      await writeClientLog(`分卷压缩并加入队列成功: file=${record.file_path}, out_dir=${outDir}`);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      window.alert(`分卷压缩失败: ${message}`);
      await writeClientLog(`分卷压缩失败: file=${record.file_path}, error=${message}`);
    } finally {
      setCompressingIndex(null);
    }
  };

  const handleRenameAndUpload = async (record: BlockedFileRecord, index: number) => {
    setCompressingIndex(index);
    try {
      await writeClientLog(`开始改名重传: file=${record.file_path}, target=${record.target_path}`);
      const newPath = await invoke<string>('rename_blocked_folder', { folderPath: record.file_path });
      await writeClientLog(`改名完成: new_path=${newPath}`);

      // 自动加入上传队列
      const result = await addToFileQueue(newPath, record.target_path);
      if (result.warnings.length > 0) {
        window.alert(result.warnings.join('\n'));
      }

      // 标记拦截记录已处理
      await resolveBlockedFile(index);
      await writeClientLog(`改名重传并加入队列成功: original=${record.file_path}, renamed=${newPath}`);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      window.alert(`改名重传失败: ${message}`);
      await writeClientLog(`改名重传失败: file=${record.file_path}, error=${message}`);
    } finally {
      setCompressingIndex(null);
    }
  };

  const clearBlockedFiles = async () => {
    await invoke('clear_blocked_files');
    setBlockedFiles([]);
  };

  const speedLimitToMBs = (bytesPerSec: number): number =>
    bytesPerSec === 0
      ? 0
      : Math.round(bytesPerSec / 1000000 * 100) / 100;

  const isRootAlistPath = (path: string) => normalizeAlistPath(path) === '/';

  const getRootPathMessage = () => '请选择 Alist 中的具体目录后再添加或上传文件，根目录 / 仅用于浏览存储入口。';

  const ensureConcreteAlistPath = async (path: string, action: string) => {
    if (!isRootAlistPath(path)) {
      setUploadPathError('');
      return true;
    }

    const message = getRootPathMessage();
    setUploadPathError(message);
    await writeClientLog(`${action} 被拦截: target_path=/, reason=根目录不是具体上传目录`);
    window.alert(message);
    return false;
  };

  useEffect(() => {
    loadQueue();
    fetchHistoryPage(1, 'all', '', 'desc');
    loadBlockedFiles();
    loadConfig();
    invoke<LocalLogFileInfo[]>('get_local_log_files').then(setLocalLogFiles).catch(() => {});

    // 启动心跳检测
    startHealthCheck();

    // 监听压缩进度事件
    const unlistenCompress = listen<number>('compress_progress', (event) => {
      setCompressProgress(event.payload);
    });
    return () => { unlistenCompress.then(fn => fn()); };
    
    // 监听文件拖拽事件
    let unlistenFn: (() => void) | null = null;

    const setupDragDrop = async () => {
      try {
        const unlisten = await getCurrentWebview().onDragDropEvent(async (event) => {
          if (event.payload.type === 'drop') {
            const paths = event.payload.paths;
            if (Array.isArray(paths)) {
              const targetPath = alistPathRef.current;
              if (!(await ensureConcreteAlistPath(targetPath, '拖拽添加文件'))) {
                return;
              }
              await writeClientLog(`拖拽添加文件: count=${paths.length}, target_path=${targetPath}`);

              // 拖拽时确认目标路径
              const confirmMsg = paths.length === 1
                ? `确认上传到以下目标路径？\n\n${targetPath}/`
                : `确认上传 ${paths.length} 个项目到以下目标路径？\n\n${targetPath}/`;
              const confirmed = await ask(confirmMsg, { title: '确认上传目标', kind: 'info' });
              if (!confirmed) {
                await writeClientLog('用户取消了拖拽上传');
                return;
              }
              for (const filePath of paths) {
                try {
                  const result = await addToFileQueue(filePath, targetPath);
                  if (result.warnings.length > 0) {
                    window.alert(result.warnings.join('\n'));
                    await writeClientLog(`拖拽添加文件触发大文件提示: warning_count=${result.warnings.length}`);
                  }
                  await writeClientLog(`拖拽文件已加入队列: file_path=${filePath}, target_path=${targetPath}`);
                } catch (error) {
                  const message = error instanceof Error ? error.message : String(error);
                  await writeClientLog(`拖拽文件加入队列失败: file_path=${filePath}, target_path=${targetPath}, error=${message}`);
                  window.alert(message);
                  console.error('Failed to add file:', error);
                }
              }
            }
          }
        });

        unlistenFn = unlisten;
      } catch (error) {
        console.error('Failed to setup drag drop:', error);
      }
    };

    setupDragDrop();

    return () => {
      stopHealthCheck();
      if (unlistenFn) {
        unlistenFn();
      }
      if (savePathTimerRef.current) {
        window.clearTimeout(savePathTimerRef.current);
      }
      Object.values(historyRetryTimerRef.current).forEach(window.clearTimeout);
    };
 }, [loadQueue, loadHistory, loadConfig, startHealthCheck, stopHealthCheck, addToFileQueue]);

  useEffect(() => {
    getVersion().then(setAppVersion).catch(() => {});
  }, []);

 useEffect(() => {
    const normalizedConfig = normalizeAppConfig(config);
    setConfigForm(normalizedConfig);
    // 判断当前上传限速值是否在预设选项中，否则启用自定义模式
    const mb_s = speedLimitToMBs(normalizedConfig.upload.speed_limit);
    const presets = [0, 1, 2, 5, 10];
    setSpeedLimitCustomMode(!presets.includes(mb_s));
    setSpeedLimitCustomText(mb_s === 0 ? '' : String(mb_s));

    if (configLoaded && !configInitializedRef.current) {
      const savedPath = normalizeAlistPath(normalizedConfig.upload.last_alist_path || '/');
      setAlistPath(savedPath);
      alistPathRef.current = savedPath;
      setUploadPathError(isRootAlistPath(savedPath) ? getRootPathMessage() : '');
      configInitializedRef.current = true;
      writeClientLog(`恢复上次上传目标目录: target_path=${savedPath}`);
    }
    
    // 自动登录逻辑 - 只执行一次
    const performAutoLogin = async () => {
      // 使用 ref 确保自动登录只执行一次
      if (autoLoginRef.current) {
        return;
      }
      
      if (configLoaded && normalizedConfig.alist.auto_login && 
          normalizedConfig.alist.username && 
          normalizedConfig.alist.password && 
          normalizedConfig.alist.base_url) {
        autoLoginRef.current = true;
        try {
          await writeClientLog(`自动登录: base_url=${normalizedConfig.alist.base_url}, username=${normalizedConfig.alist.username}`);
          // 如果配置了 exe_path，先加速等待 openlist 就绪
          if (normalizedConfig.alist.exe_path) {
            await writeClientLog('检测到 exe_path 配置，加速等待 Alist 就绪');
            await useAppStore.getState().fastCheckHealth();
          }
          await login(normalizedConfig.alist.base_url, normalizedConfig.alist.username, normalizedConfig.alist.password);
          await writeClientLog('自动登录成功');
        } catch (error) {
          const message = error instanceof Error ? error.message : String(error);
          await writeClientLog(`自动登录失败: ${message}`);
          console.error('Auto login failed:', error);
          // 登录失败后不重试，避免无限循环
        }
      }
    };

    performAutoLogin();
  }, [config, configLoaded]);

  // 启动时自动检查更新（可关闭，默认开启）
  useEffect(() => {
    if (!configLoaded || startupUpdateCheckedRef.current) return;
    startupUpdateCheckedRef.current = true;
    const normalizedConfig = normalizeAppConfig(config);
    if (!normalizedConfig.upload.check_update_on_startup) return;

    (async () => {
      try {
        const version = await invoke<string | null>('check_update_no_proxy');
        if (!version) return;
        const confirmed = await ask(`发现新版本 ${version}，是否下载安装？`, {
          title: '发现新版本',
          kind: 'info',
        });
        if (!confirmed) return;

        setDownloadingUpdate(true);
        setDownloadProgress(0);
        setDownloadSizeText('正在下载更新（不走系统代理）...');

        await invoke('download_and_install_update_no_proxy');

        setDownloadingUpdate(false);
        await writeClientLog('更新已安装，即将重启');
        await relaunch();
      } catch (error) {
        setDownloadingUpdate(false);
        const message = error instanceof Error ? error.message : String(error);
        await writeClientLog(`启动检查更新失败: ${message}`);
      }
    })();
  }, [configLoaded, config]);

  useEffect(() => {
    if (!isUploading) return;

    const intervalId = window.setInterval(async () => {
      try {
        await loadQueue();
        await fetchHistoryPage(historyCurrentPage, historyFilter, historySearchText, historySortOrder);
        const latestQueue = useAppStore.getState().queue;
        const backendUploading = await invoke<boolean>('get_is_uploading');
       if (!backendUploading) {
         setIsUploading(false);
         await writeClientLog('上传调度器已停止，前端关闭上传中状态');

          const shutdownState = await invoke<string | null>('get_shutdown_state');
          if (shutdownState) {
            setShutdownDeadline(shutdownState);
          }
       }

        // 检测新完成的任务，发送系统通知
        const notifyOnComplete = useAppStore.getState().config.upload.notify_on_complete;
        if (notifyOnComplete) {
          const perm = await isPermissionGranted();
          if (!perm) {
            await requestPermission();
          }
          for (const task of latestQueue) {
            if (task.status === 'completed' && !notifiedTaskIds.current.has(task.id)) {
              notifiedTaskIds.current.add(task.id);
              sendNotification({ title: '上传完成', body: `文件 ${task.file.name} 上传成功` });
            }
            if (task.status === 'failed' && !notifiedTaskIds.current.has(task.id)) {
              notifiedTaskIds.current.add(task.id);
              sendNotification({ title: '上传失败', body: `${task.file.name}: ${task.error || '未知错误'}` });
            }
          }
        }
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        await writeClientLog(`上传队列刷新失败: ${message}`);
      }
    }, 1000);

    return () => window.clearInterval(intervalId);
  }, [isUploading, loadQueue, historyCurrentPage, historyFilter, historySearchText, historySortOrder, fetchHistoryPage, setIsUploading]);

  // 关机倒计时
  useEffect(() => {
    if (!shutdownDeadline) return;

    const updateCountdown = () => {
      const deadline = new Date(shutdownDeadline).getTime();
      const remaining = deadline - Date.now();
      if (remaining <= 0) {
        setShutdownCountdown('正在关机...');
        return;
      }
      const minutes = Math.floor(remaining / 60000);
      const seconds = Math.floor((remaining % 60000) / 1000);
      setShutdownCountdown(`${minutes}:${seconds.toString().padStart(2, '0')}`);
    };

    updateCountdown();
    const intervalId = window.setInterval(updateCountdown, 1000);
    return () => window.clearInterval(intervalId);
  }, [shutdownDeadline]);

 const handleCancelShutdown = async () => {
   try {
     await invoke('cancel_shutdown');
     setShutdownDeadline(null);
     setShutdownCountdown('');
   } catch (error) {
     console.error('取消关机失败:', error);
   }
 };

  const handleOpenFileLocation = async (filePath: string) => {
    try {
      await invoke('open_file_location', { filePath });
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      window.alert(`打开文件所在目录失败: ${message}`);
    }
  };

 const persistAlistPath = (path: string) => {
    const normalizedPath = normalizeAlistPath(path);
    setAlistPath(normalizedPath);
    alistPathRef.current = normalizedPath;
    setUploadPathError(isRootAlistPath(normalizedPath) ? getRootPathMessage() : '');
    setConfigForm(current => normalizeAppConfig({
      ...current,
      upload: { ...current.upload, last_alist_path: normalizedPath },
    }));

    if (savePathTimerRef.current) {
      window.clearTimeout(savePathTimerRef.current);
    }

    savePathTimerRef.current = window.setTimeout(async () => {
      const latestConfig = normalizeAppConfig(useAppStore.getState().config);
      const nextConfig = normalizeAppConfig({
        ...latestConfig,
        upload: { ...latestConfig.upload, last_alist_path: alistPathRef.current },
      });
      try {
        await writeClientLog(`保存上传目标目录: target_path=${alistPathRef.current}`);
        await saveConfig(nextConfig);
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        await writeClientLog(`保存上传目标目录失败: target_path=${alistPathRef.current}, error=${message}`);
      }
    }, 500);
  };

  const handleSelectFiles = async () => {
    try {
      if (!(await ensureConcreteAlistPath(alistPathRef.current, '选择文件'))) {
        return;
      }

      const selected = await open({
        multiple: true,
      });
      
      if (selected) {
        const files = Array.isArray(selected) ? selected : [selected];
        const targetPath = alistPathRef.current;
        await writeClientLog(`文件选择添加队列: count=${files.length}, target_path=${targetPath}`);
        const warnings: string[] = [];
        for (const file of files) {
          const result = await addToFileQueue(file, targetPath);
          warnings.push(...result.warnings);
          await writeClientLog(`选择文件已加入队列: file_path=${file}, target_path=${targetPath}`);
        }
        if (warnings.length > 0) {
          window.alert(warnings.join('\n'));
          await writeClientLog(`选择文件触发大文件提示: warning_count=${warnings.length}`);
        }
      }
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      await writeClientLog(`选择文件加入队列失败: target_path=${alistPathRef.current}, error=${message}`);
      window.alert(message);
      console.error('Failed to select files:', error);
    }
  };

 const handleStartUpload = async () => {
   const rootTargetTask = queue.find(task => isRootAlistPath(task.alist_path));
   setShutdownDeadline(null);
   setShutdownCountdown('');
   if (rootTargetTask) {
      const message = `队列中存在目标目录为 / 的任务：${rootTargetTask.file.name}。请删除该任务后选择具体目录重新添加。`;
      setUploadPathError(message);
      await writeClientLog(`开始上传被拦截: task_id=${rootTargetTask.id}, file=${rootTargetTask.file.name}, target_path=/`);
      window.alert(message);
      return;
    }

    await startUpload();
  };

  const handlePauseUpload = async () => {
    await pauseUpload();
  };
  const handleHistoryResolve = async (task: UploadTask) => {
    try {
      await invoke('resolve_history_task', { taskId: task.id });
      await writeClientLog(`历史记录标记已处理: task_id=${task.id}, file=${task.file.path}`);
      await loadHistoryPage(historyCurrentPage, historyPageSize, historyFilter, historySearchText, historySortOrder);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      window.alert(`标记失败: ${message}`);
    }
  };

  const handleHistoryRetry = async (task: UploadTask) => {
    try {
      await writeClientLog(`历史记录重试: file=${task.file.path}, alist_path=${task.alist_path}`);
      const result = await addToFileQueue(task.file.path, task.alist_path);
      if (result.warnings.length > 0) {
        window.alert(result.warnings.join('\n'));
      }
      setHistoryRetryStatus(prev => ({ ...prev, [task.id]: 'queued' }));
      const timerId = window.setTimeout(() => {
        setHistoryRetryStatus(prev => {
          const next = { ...prev };
          delete next[task.id];
          return next;
        });
      }, 3000);
      historyRetryTimerRef.current[task.id] = timerId;
      await writeClientLog(`历史记录重试成功: file=${task.file.path}, alist_path=${task.alist_path}`);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      await writeClientLog(`历史记录重试失败: ${message}`);
      setHistoryRetryStatus(prev => ({ ...prev, [task.id]: 'error' }));
      window.alert(`重试失败: ${message}`);
      const timerId = window.setTimeout(() => {
        setHistoryRetryStatus(prev => {
          const next = { ...prev };
          delete next[task.id];
          return next;
        });
      }, 3000);
      historyRetryTimerRef.current[task.id] = timerId;
    }
  };

  const writeClientLog = async (message: string) => {
    try {
      console.log('[client_log]', message);
      await invoke('write_client_log', { message });
    } catch (error) {
      console.error('[client_log_failed]', error);
    }
  };

  const handleTestConnection = async () => {
    await writeClientLog(`点击测试连接: base_url=${configForm.alist.base_url}, username=${configForm.alist.username}, has_token=${Boolean(configForm.alist.token)}`);

    try {
      const success = await testConnection(configForm);
      setConnectionStatus(success ? 'success' : 'error');
      setConnectionMessage(success ? 'Token 有效，连接成功' : 'Token 无效或未登录，请先点击登录获取 Token');
      await writeClientLog(`测试连接完成: success=${success}`);
      setTimeout(() => setConnectionStatus('idle'), 5000);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setConnectionStatus('error');
      setConnectionMessage(message);
      await writeClientLog(`测试连接异常: ${message}`);
      setTimeout(() => setConnectionStatus('idle'), 5000);
    }
  };

  const handleSaveConfig = async () => {
    setSaveConfigStatus('saving');
    setSaveConfigMessage('');
    try {
      await saveConfig(normalizeAppConfig(configForm));
      setSaveConfigStatus('success');
      setSaveConfigMessage('配置已保存');
      await writeClientLog('配置保存成功');
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setSaveConfigStatus('error');
      setSaveConfigMessage(message);
      await writeClientLog(`配置保存失败: ${message}`);
    }
    setTimeout(() => {
      setSaveConfigStatus('idle');
      setSaveConfigMessage('');
    }, 3000);
  };

  const handleTestNotification = async () => {
    const notification = configForm.upload.notification;
    if (!notification || !notification.enabled) {
      window.alert('请先启用失败通知');
      return;
    }
    if (!notification.webhook_url.trim()) {
      window.alert('请先填写 Webhook URL');
      return;
    }
    await writeClientLog(`点击测试通知: has_webhook=${Boolean(notification.webhook_url)}, channels=${notification.channels.join(',')}`);
    try {
      await invoke('test_notification', { config: notification });
      await writeClientLog('测试通知发送成功');
      window.alert('测试通知发送成功，请检查飞书机器人消息');
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      await writeClientLog(`测试通知发送失败: ${message}`);
      window.alert(`测试通知发送失败: ${message}`);
    }
  };

  const handleCheckUpdate = async () => {
    await writeClientLog('点击检查更新');
    try {
      const version = await invoke<string | null>('check_update_no_proxy');
      if (version) {
        const confirmed = await ask(`发现新版本 ${version}，是否下载安装？`, {
          title: '发现新版本',
          kind: 'info',
        });
        if (!confirmed) {
          return;
        }

        setDownloadingUpdate(true);
        setDownloadProgress(0);
        setDownloadSizeText('正在下载更新（不走系统代理）...');

        await invoke('download_and_install_update_no_proxy');

        setDownloadingUpdate(false);
        await writeClientLog('更新已安装，即将重启');
        await relaunch();
      } else {
        window.alert('已是最新版本');
      }
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setDownloadingUpdate(false);
      await writeClientLog(`检查更新失败: ${message}`);
      window.alert(`检查更新失败: ${message}`);
    }
  };

  const loadLocalLogFiles = async () => {
    try {
      const files = await invoke<LocalLogFileInfo[]>('get_local_log_files');
      setLocalLogFiles(files);
    } catch (error) {
      console.error('Failed to load local log files:', error);
    }
  };

  const handleOpenLogFolder = async () => {
    try {
      const logDir = await invoke<string>('get_log_dir');
      const target = logDir.endsWith('\\') ? logDir : logDir + '\\';
      await invoke('open_file_location', { filePath: target });
      await writeClientLog(`打开日志目录: ${target}`);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      window.alert(`打开日志目录失败: ${message}`);
    }
  };

  const handleLogSyncLogin = async () => {
    const ls = configForm.log_sync;
    if (!ls || !ls.base_url || !ls.username || !ls.password) {
      setLogSyncLoginStatus('error');
      setLogSyncLoginMessage('请填写地址、用户名和密码');
      setTimeout(() => setLogSyncLoginStatus('idle'), 5000);
      return;
    }
    await writeClientLog(`日志同步登录: base_url=${ls.base_url}, username=${ls.username}`);
    try {
      const token = await invoke<string>('log_sync_login', { config: ls });
      setLogSyncLoginStatus('success');
      setLogSyncLoginMessage(`登录成功，Token 已缓存`);
      setConfigForm(prev => ({
        ...prev,
        log_sync: { ...prev.log_sync!, token },
      }));
      await writeClientLog('日志同步登录成功');
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setLogSyncLoginStatus('error');
      setLogSyncLoginMessage(message);
      await writeClientLog(`日志同步登录失败: ${message}`);
    }
    setTimeout(() => { setLogSyncLoginStatus('idle'); setLogSyncLoginMessage(''); }, 5000);
  };

  const handleSyncLogs = async () => {
    const ls = configForm.log_sync;
    if (!ls || !ls.base_url) {
      setLogSyncStatus('error');
      setLogSyncMessage('请先填写日志Alist服务地址');
      setTimeout(() => setLogSyncStatus('idle'), 5000);
      return;
    }
    setLogSyncStatus('syncing');
    setLogSyncMessage('正在同步...');
    await writeClientLog(`手动同步日志: base_url=${ls.base_url}, target_path=${ls.target_path}`);
    try {
      const result = await invoke<LogSyncResult>('sync_logs', { config: ls });
      if (result.failed === 0) {
        setLogSyncStatus('success');
        setLogSyncMessage(`同步完成: ${result.success}/${result.total} 个文件成功`);
      } else {
        setLogSyncStatus('error');
        setLogSyncMessage(`同步完成: 成功 ${result.success}/${result.total}，失败 ${result.failed}`);
      }
      await writeClientLog(`日志同步结果: total=${result.total}, success=${result.success}, failed=${result.failed}`);
      if (result.last_sync_at) {
        setConfigForm(prev => ({
          ...prev,
          log_sync: { ...prev.log_sync!, last_sync_at: result.last_sync_at },
        }));
      }
      await loadLocalLogFiles();
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setLogSyncStatus('error');
      setLogSyncMessage(`同步失败: ${message}`);
      await writeClientLog(`日志同步失败: ${message}`);
    }
    setTimeout(() => { setLogSyncStatus('idle'); setLogSyncMessage(''); }, 8000);
  };

  const formatFileSize = (bytes: number) => {
    const units = ['B', 'KB', 'MB', 'GB'];
    let size = bytes;
    let unitIndex = 0;
    while (size >= 1024 && unitIndex < units.length - 1) {
      size /= 1024;
      unitIndex++;
    }
    return `${size.toFixed(2)} ${units[unitIndex]}`;
  };

  const formatDuration = (seconds: number) => {
    if (seconds < 60) return `${seconds}s`;
    const m = Math.floor(seconds / 60);
    const s = seconds % 60;
    return s > 0 ? `${m}m ${s}s` : `${m}m`;
  };

  const formatSpeed = (bytesPerSec: number) => {
    if (bytesPerSec <= 0) return '';
    const units = ['B/s', 'KB/s', 'MB/s', 'GB/s'];
    let speed = bytesPerSec;
    let unitIndex = 0;
    while (speed >= 1024 && unitIndex < units.length - 1) {
      speed /= 1024;
      unitIndex++;
    }
    return `${speed.toFixed(2)} ${units[unitIndex]}`;
  };

  const totalUploadSpeed = queue
    .filter(t => t.status === 'uploading')
    .reduce((sum, t) => sum + (t.speed || 0), 0);

  const queueStats = useMemo(() => {
    const total = queue.length;
    const pending = queue.filter(t => t.status === 'pending');
    const uploading = queue.filter(t => t.status === 'uploading');
    const pendingBytes = pending.reduce((sum, t) => sum + (t.file.size || 0), 0);
    const uploadingBytes = uploading.reduce((sum, t) => sum + (t.file.size || 0), 0);
    const allBytes = queue.reduce((sum, t) => sum + (t.file.size || 0), 0);
    return { total, pending: pending.length, uploading: uploading.length, pendingBytes, uploadingBytes, allBytes };
  }, [queue]);

  const historyStats = useMemo(() => {
    const total = historyPage?.total ?? history.length;
    const completed = history.filter(t => t.status === 'completed');
    const failed = history.filter(t => t.status === 'failed');
    const successRate = total > 0 ? Math.round((completed.length / total) * 100) : 0;
    const totalBytes = completed.reduce((sum, t) => sum + (t.file.size || 0), 0);

    const now = new Date();
    const monthStart = new Date(now.getFullYear(), now.getMonth(), 1);
    const thisMonth = completed.filter(t => {
      if (!t.end_time) return false;
      return new Date(t.end_time) >= monthStart;
    });
    const monthBytes = thisMonth.reduce((sum, t) => sum + (t.file.size || 0), 0);

    const totalDuration = completed.reduce((sum, t) => sum + (t.duration || 0), 0);
    const avgSpeed = totalDuration > 0 ? totalBytes / totalDuration : 0;

    return { total, completed: completed.length, failed: failed.length, successRate, totalBytes, monthBytes, avgSpeed };
  }, [history, historyPage]);

  const formatDateTime = (isoString?: string) => {
    if (!isoString) return '-';
    return new Date(isoString).toLocaleString('zh-CN');
  };

  const hasRootTargetInQueue = queue.some(task => isRootAlistPath(task.alist_path));

  const filteredQueue = useMemo(() => {
    let result = queueFilter === 'all' ? queue : queue.filter(t => t.status === queueFilter);
    if (queueSearchText.trim()) {
      const q = queueSearchText.trim().toLowerCase();
      result = result.filter(t => t.file.name.toLowerCase().includes(q));
    }
    return result;
  }, [queue, queueFilter, queueSearchText]);

  const filteredBlockedFiles = useMemo(() => {
    let result = [...blockedFiles];
    if (blockedSearchText.trim()) {
      const q = blockedSearchText.trim().toLowerCase();
      result = result.filter(r => r.file_name.toLowerCase().includes(q) || r.file_path.toLowerCase().includes(q));
    }
    result.sort((a, b) => {
      const ta = new Date(a.blocked_at).getTime();
      const tb = new Date(b.blocked_at).getTime();
      return blockedSortOrder === 'desc' ? tb - ta : ta - tb;
    });
    // 已处理的记录排到后面
    result.sort((a, b) => Number(a.resolved) - Number(b.resolved));
    return result;
  }, [blockedFiles, blockedSearchText, blockedSortOrder]);

  const allFilteredSelected = filteredQueue.length > 0 && filteredQueue.every(t => selectedTaskIds.has(t.id));

  const handleBatchDelete = async () => {
    const ids = Array.from(selectedTaskIds);
    for (const id of ids) {
      await removeFromQueue(id);
    }
    setSelectedTaskIds(new Set());
  };

  const toggleSelect = (id: string) => {
    setSelectedTaskIds(prev => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const toggleSelectAll = () => {
    if (allFilteredSelected) {
      setSelectedTaskIds(new Set());
    } else {
      setSelectedTaskIds(new Set(filteredQueue.map(t => t.id)));
    }
  };

  if (isLoading) {
    return <div className="loading">加载中...</div>;
  }

  return (
    <div className="app">
      <header className="app-header">
        <h1>Openlist 上传管理器</h1>
        <nav className="app-nav">
          <button
            className={activeTab === 'queue' ? 'active' : ''}
            onClick={() => setActiveTab('queue')}
          >
            待上传队列 ({queue.length})
          </button>
          <button
            className={activeTab === 'history' ? 'active' : ''}
            onClick={() => setActiveTab('history')}
          >
            历史记录 ({history.length})
          </button>
          <button
            className={activeTab === 'blocked' ? 'active' : ''}
            onClick={() => { setActiveTab('blocked'); loadBlockedFiles(); }}
          >
            拦截记录 ({blockedFiles.length})
          </button>
          <button
            className={activeTab === 'settings' ? 'active' : ''}
            onClick={() => setActiveTab('settings')}
          >
            设置
          </button>
        </nav>
        <div className="header-status">
          <div className="alist-status">
            <span className={`status-dot ${alistChecking ? 'checking' : alistServiceAvailable && alistConnected ? 'connected' : alistServiceAvailable ? 'warning' : 'disconnected'}`} />
            <span className="status-label">
              {alistChecking ? '检测中...' : alistServiceAvailable && alistConnected ? 'Openlist 已连接' : alistServiceAvailable ? 'Openlist 已启动，未登录' : 'Openlist 服务不可用'}
            </span>
          </div>
          <span className={`status-indicator ${isUploading ? 'active' : ''}`}>
            {isUploading && totalUploadSpeed > 0
              ? `上传中 ${formatSpeed(totalUploadSpeed)}`
              : isUploading
                ? '上传中'
                : '空闲'}
          </span>
        </div>
      </header>

      <main className="app-content">
        {activeTab === 'queue' && (
          <div className="queue-tab">
            <div className="queue-toolbar">
              <div className="target-path-panel">
                <div className="target-path-header">
                  <span className="target-step">上传位置</span>
                </div>
                <div className="upload-path-bar">
                  <FolderPicker
                    value={alistPath}
                    onChange={persistAlistPath}
                    recentPaths={recentPaths}
                    onAddRecentPath={addRecentPath}
                  />
                </div>
                <div className="target-path-hint">
                  之后选择或拖拽的文件都会加入此目录；已加入队列的任务保留各自目标路径。
                </div>
                {uploadPathError && (
                  <div className="target-path-error">
                    {uploadPathError}
                  </div>
                )}
                {hasRootTargetInQueue && (
                  <div className="target-path-error">
                    队列中存在目标目录为 / 的任务，请删除后选择具体目录重新添加。
                  </div>
                )}
              </div>
              <div className="toolbar-actions">
                <button
                  onClick={handleSelectFiles}
                  disabled={isRootAlistPath(alistPath)}
                  title={isRootAlistPath(alistPath) ? '请先选择具体目录作为上传目标' : undefined}
                >
                  选择文件
                </button>
                {!isUploading ? (
                  <button 
                    onClick={handleStartUpload} 
                    disabled={queue.length === 0 || hasRootTargetInQueue}
                    className="primary"
                    title={hasRootTargetInQueue ? '队列中存在目标目录为 / 的任务，请删除后重新添加' : undefined}
                  >
                    开始上传
                  </button>
                ) : isStopping ? (
                  <button disabled className="warning disabled">
                    停止中...
                  </button>
                ) : (
                  <button onClick={handlePauseUpload} className="warning">
                    停止队列
                  </button>
                )}
                <button onClick={clearQueue} disabled={queue.length === 0 || isUploading}>
                  清空队列
                </button>
                {selectedTaskIds.size > 0 && (
                  <button onClick={handleBatchDelete} className="danger small">
                    删除选中 ({selectedTaskIds.size})
                  </button>
                )}
              </div>
              <div className="queue-filter">
                <button
                  type="button"
                  className={queueFilter === 'all' ? 'active' : ''}
                  onClick={() => setQueueFilter('all')}
                >
                  全部 ({queue.length})
                </button>
                <button
                  type="button"
                  className={queueFilter === 'pending' ? 'active' : ''}
                  onClick={() => setQueueFilter('pending')}
                >
                  等待中 ({queue.filter(t => t.status === 'pending').length})
                </button>
                <button
                  type="button"
                  className={queueFilter === 'uploading' ? 'active' : ''}
                  onClick={() => setQueueFilter('uploading')}
                >
                  上传中 ({queue.filter(t => t.status === 'uploading').length})
                </button>
                <input
                  type="text"
                  placeholder="搜索文件名..."
                  value={queueSearchText}
                  onChange={(e) => setQueueSearchText(e.target.value)}
                  onKeyDown={(e) => { if (e.key === 'Escape' && queueSearchText) { e.preventDefault(); setQueueSearchText(''); } }}
                  className="history-search-input"
                />
                {queueSearchText && (
                  <button
                    type="button"
                    className="history-search-clear"
                    onClick={() => setQueueSearchText('')}
                    title="清空搜索内容"
                  >
                    ✕
                  </button>
                )}
              </div>
              {isStopping && (
                <div className="stopping-notice">
                  等待当前文件上传完成后停止...
                </div>
              )}
              {queueStats.total > 0 && (
                <div className="stats-grid">
                  <div className="stat-card">
                    <span className="stat-value">{queueStats.total}</span>
                    <span className="stat-label">总任务数</span>
                  </div>
                  <div className="stat-card">
                    <span className="stat-value">{queueStats.pending}</span>
                    <span className="stat-label">等待中</span>
                  </div>
                  <div className="stat-card">
                    <span className="stat-value">{queueStats.uploading}</span>
                    <span className="stat-label">上传中</span>
                  </div>
                  <div className="stat-card">
                    <span className="stat-value">{formatFileSize(queueStats.allBytes)}</span>
                    <span className="stat-label">队列总大小</span>
                  </div>
                  <div className="stat-card">
                    <span className="stat-value">{formatFileSize(queueStats.pendingBytes)}</span>
                    <span className="stat-label">待上传大小</span>
                  </div>
                </div>
              )}
            </div>

            <div className="queue-list">
              {queue.length === 0 ? (
                <div className="empty-state">
                  <p>队列为空</p>
                  <p>当前目标目录：{alistPath}</p>
                  <p>先选择目标目录，再拖拽文件或点击"选择文件"添加</p>
                </div>
              ) : (
                <table>
                  <thead>
                    <tr>
                      <th><input type="checkbox" checked={allFilteredSelected} onChange={toggleSelectAll} /></th>
                      <th>文件名</th>
                      <th>大小</th>
                      <th>目标路径</th>
                      <th>状态</th>
                      <th>重试次数</th>
                      <th>添加时间</th>
                      <th>操作</th>
                    </tr>
                  </thead>
                  <tbody>
                    {filteredQueue.map(task => (
                      <Fragment key={task.id}>
                      <tr>
                        <td><input type="checkbox" checked={selectedTaskIds.has(task.id)} onChange={() => toggleSelect(task.id)} /></td>
                        <td>
                          <button
                            type="button"
                            className={`task-name-toggle ${expandedTaskId === task.id ? 'expanded' : ''}`}
                            onClick={() => setExpandedTaskId(expandedTaskId === task.id ? null : task.id)}
                            aria-expanded={expandedTaskId === task.id}
                          >
                            <span className="task-expand-icon" aria-hidden="true">▶</span>
                            <span className="task-file-name">{task.file.name}</span>
                          </button>
                        </td>
                        <td>{formatFileSize(task.file.size)}</td>
                        <td>
                          <span>{task.alist_path}</span>
                          {task.file.size > FOUR_GB && (
                            <span className="large-file-badge" title="该文件超过 4GB。大文件上传耗时较长，若 1 小时内未完成可能因 Token 过期导致失败。建议在上传带宽较好时上传，或先压缩/分卷处理。">
                              大文件风险
                            </span>
                          )}
                        </td>
                        <td>
                          <span className={`status-badge status-${task.status}`}>
                            {task.status === 'pending' && '等待中'}
                            {task.status === 'uploading' && `上传中 ${task.progress}%`}
                            {task.status === 'completed' && '已完成'}
                            {task.status === 'failed' && '失败'}
                            {task.status === 'cancelled' && '已取消'}
                          </span>
                        </td>
                        <td>{task.retry_count}</td>
                        <td>{formatDateTime(task.created_at)}</td>
                        <td>
                          {task.status === 'failed' && (
                            <button 
                              onClick={() => retryUpload(task.id)}
                              className="small"
                            >
                              重试
                            </button>
                          )}
                         <button 
                           onClick={() => removeFromQueue(task.id)}
                           className="small danger"
                         >
                           删除
                         </button>
                         <button
                           onClick={() => handleOpenFileLocation(task.file.path)}
                           className="small"
                           title="打开文件所在目录"
                         >
                           定位
                         </button>
                       </td>
                     </tr>
                      {expandedTaskId === task.id && (
                        <tr className="task-detail-row">
                          <td colSpan={8}>
                            <div className="task-path-detail">
                              <div>
                                <span className="task-path-label">本地路径</span>
                                <code title={task.file.path}>{task.file.path}</code>
                              </div>
                              <div>
                                <span className="task-path-label">OpenList 目标</span>
                                <code>{task.alist_path}</code>
                              </div>
                              <div>
                                <span className="task-path-label">添加时间</span>
                                <code>{formatDateTime(task.created_at)}</code>
                              </div>
                            </div>
                          </td>
                        </tr>
                      )}
                      </Fragment>
                    ))}
                  </tbody>
                </table>
              )}
            </div>
          </div>
        )}

        {activeTab === 'history' && (
          <div className="history-tab">
            <div className="history-toolbar">
              <div className="history-filter">
                <button
                  type="button"
                  className={historyFilter === 'all' ? 'active' : ''}
                  onClick={() => { setHistoryFilter('all'); fetchHistoryPage(1, 'all', historySearchText, historySortOrder); }}
                >
                  全部 ({historyPage?.total ?? 0})
                </button>
                <button
                  type="button"
                  className={historyFilter === 'completed' ? 'active' : ''}
                  onClick={() => { setHistoryFilter('completed'); fetchHistoryPage(1, 'completed', historySearchText, historySortOrder); }}
                >
                  成功 ({historyPage ? '↓' : '0'})
                </button>
                <button
                  type="button"
                  className={historyFilter === 'failed' ? 'active' : ''}
                  onClick={() => { setHistoryFilter('failed'); fetchHistoryPage(1, 'failed', historySearchText, historySortOrder); }}
                >
                  失败 ({historyPage ? '↓' : '0'})
                </button>
              </div>
              <div className="history-search-wrapper">
                <input
                  type="text"
                  placeholder="搜索文件名..."
                  value={historySearchText}
                  onChange={(e) => setHistorySearchText(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter') { fetchHistoryPage(1, historyFilter, historySearchText, historySortOrder); }
                    if (e.key === 'Escape' && historySearchText) {
                      e.preventDefault();
                      setHistorySearchText('');
                      fetchHistoryPage(1, historyFilter, '', historySortOrder);
                    }
                  }}
                  className="history-search-input"
                />
                {historySearchText && (
                  <button
                    type="button"
                    className="history-search-clear"
                    onClick={() => { setHistorySearchText(''); fetchHistoryPage(1, historyFilter, '', historySortOrder); }}
                    title="清空搜索内容"
                    aria-label="清空搜索内容"
                  >
                    ✕
                  </button>
                )}
              </div>
              <button type="button" onClick={clearHistory} disabled={history.length === 0}>
                清空历史
              </button>
            </div>

            {historyPage && historyPage.total > 0 && (
              <div className="history-stats">
                <div className="stat-card">
                  <span className="stat-value">{historyStats.total}</span>
                  <span className="stat-label">总上传数</span>
                </div>
                <div className="stat-card">
                  <span className="stat-value">{historyStats.successRate}%</span>
                  <span className="stat-label">成功率</span>
                </div>
                <div className="stat-card">
                  <span className="stat-value">{formatFileSize(historyStats.totalBytes)}</span>
                  <span className="stat-label">总上传量</span>
                </div>
                <div className="stat-card">
                  <span className="stat-value">{formatFileSize(historyStats.monthBytes)}</span>
                  <span className="stat-label">本月上传量</span>
                </div>
                <div className="stat-card">
                  <span className="stat-value">{formatSpeed(historyStats.avgSpeed)}</span>
                  <span className="stat-label">平均速度</span>
                </div>
              </div>
            )}

            <div className="history-list">
              {history.length === 0 ? (
                <div className="empty-state">
                  <p>{(historyPage?.total ?? 0) === 0 ? '暂无历史记录' : '当前筛选下暂无历史记录'}</p>
                </div>
              ) : (
                <table>
                  <thead>
                    <tr>
                      <th>文件名</th>
                      <th>大小</th>
                      <th>目标路径</th>
                      <th>状态</th>
                      <th
                        onClick={() => {
                          const next = historySortOrder === 'desc' ? 'asc' : 'desc';
                          setHistorySortOrder(next);
                          fetchHistoryPage(historyCurrentPage, historyFilter, historySearchText, next);
                        }}
                        style={{ cursor: 'pointer', userSelect: 'none' }}
                        title="点击切换排序"
                      >
                        完成时间 {historySortOrder === 'desc' ? '▼' : '▲'}
                      </th>
                      <th>耗时</th>
                      <th>操作</th>
                    </tr>
                  </thead>
                  <tbody>
                    {history.map(task => (
                      <Fragment key={task.id}>
                        <tr
                          onClick={() => setExpandedHistoryTaskId(expandedHistoryTaskId === task.id ? null : task.id)}
                          style={{ cursor: 'pointer' }}
                          className={task.resolved ? 'blocked-row-resolved' : ''}
                        >
                          <td><span className="task-file-name" title={task.file.name}>{task.file.name}</span></td>
                          <td>{formatFileSize(task.file.size)}</td>
                          <td>{task.alist_path}</td>
                          <td>
                            <span className={`status-badge status-${task.status} ${task.error ? 'with-error' : ''}`}>
                              {task.status === 'completed' ? '成功' : '失败'}
                              {task.error && `: ${task.error}`}
                            </span>
                            {task.resolved && (
                              <span className="status-badge status-completed" title="已手动处理（如通过网页等其他方式补传成功）">
                                已处理
                              </span>
                            )}
                          </td>
                          <td>{formatDateTime(task.end_time)}</td>
                          <td>{task.duration ? formatDuration(task.duration) : '-'}</td>
                          <td onClick={(e) => e.stopPropagation()}>
                            {task.status === 'failed' && !task.resolved && (
                              <button
                                type="button"
                                onClick={() => handleHistoryRetry(task)}
                                className={`small ${historyRetryStatus[task.id] === 'queued' ? 'queued' : ''}`}
                                disabled={historyRetryStatus[task.id] === 'queued'}
                              >
                                {historyRetryStatus[task.id] === 'queued' ? '已加入队列' : '重试'}
                              </button>
                            )}
                            {task.status === 'failed' && !task.resolved && (
                              <button
                                type="button"
                                onClick={() => handleHistoryResolve(task)}
                                className="small"
                                title="已通过其他方式（如网页上传）补传成功，不再需要重试"
                              >
                                标记已处理
                              </button>
                            )}
                            <button
                              type="button"
                              onClick={() => handleOpenFileLocation(task.file.path)}
                              className="small"
                              title="打开文件所在目录"
                            >
                              定位
                            </button>
                          </td>
                        </tr>
                        {expandedHistoryTaskId === task.id && (
                          <tr className="task-detail-row">
                          <td colSpan={8}>
                              <div className="task-path-detail">
                                <div>
                                  <span className="task-path-label">本地路径</span>
                                  <code title={task.file.path}>{task.file.path}</code>
                                </div>
                                <div>
                                  <span className="task-path-label">OpenList 目标</span>
                                  <code>{task.alist_path}</code>
                                </div>
                                {task.api_response && (
                                  <div>
                                    <span className="task-path-label">接口返回值</span>
                                    <code className="api-response-detail">{task.api_response}</code>
                                  </div>
                                )}
                              </div>
                            </td>
                          </tr>
                        )}
                      </Fragment>
                    ))}
                  </tbody>
                </table>
              )}
              {historyPage && historyPage.total_pages > 1 && (
                <div className="history-pagination">
                  <button
                    type="button"
                    disabled={historyCurrentPage <= 1}
                    onClick={() => fetchHistoryPage(historyCurrentPage - 1, historyFilter, historySearchText, historySortOrder)}
                  >
                    上一页
                  </button>
                  <span className="pagination-info">
                    第 {historyCurrentPage} / {historyPage.total_pages} 页（共 {historyPage.total} 条）
                  </span>
                  <button
                    type="button"
                    disabled={historyCurrentPage >= historyPage.total_pages}
                    onClick={() => fetchHistoryPage(historyCurrentPage + 1, historyFilter, historySearchText, historySortOrder)}
                  >
                    下一页
                  </button>
                </div>
              )}
            </div>
          </div>
        )}

        {activeTab === 'blocked' && (
          <div className="blocked-tab">
            <div className="tab-header">
              <h2>被拦截文件记录</h2>
              <input
                type="text"
                placeholder="搜索文件名或路径..."
                value={blockedSearchText}
                onChange={(e) => setBlockedSearchText(e.target.value)}
                className="history-search-input"
              />
              <button onClick={clearBlockedFiles} className="danger" disabled={blockedFiles.length === 0}>
                清空所有记录
              </button>
            </div>
            {filteredBlockedFiles.length === 0 ? (
              <p>{blockedFiles.length === 0 ? '暂无被拦截文件记录' : '当前筛选下暂无记录'}</p>
            ) : (
              <table>
                <thead>
                  <tr>
                    <th>文件名</th>
                    <th>文件大小</th>
                    <th>原因</th>
                    <th
                      onClick={() => setBlockedSortOrder(prev => prev === 'desc' ? 'asc' : 'desc')}
                      style={{ cursor: 'pointer', userSelect: 'none' }}
                      title="点击切换排序"
                    >
                      时间 {blockedSortOrder === 'desc' ? '▼' : '▲'}
                    </th>
                    <th>状态</th>
                    <th>操作</th>
                  </tr>
                </thead>
                <tbody>
                  {filteredBlockedFiles.map((record) => {
                    const realIndex = blockedFiles.indexOf(record);
                    return (
                    <Fragment key={realIndex}>
                    <tr className={record.resolved ? 'blocked-row-resolved' : ''}>
                      <td>
                        <button
                          type="button"
                          className={`task-name-toggle ${expandedBlockedIndex === realIndex ? 'expanded' : ''}`}
                          onClick={() => setExpandedBlockedIndex(expandedBlockedIndex === realIndex ? null : realIndex)}
                          aria-expanded={expandedBlockedIndex === realIndex}
                        >
                          <span className="task-expand-icon" aria-hidden="true">▶</span>
                          <span className="task-file-name">{record.file_name}</span>
                        </button>
                      </td>
                      <td>{formatFileSize(record.file_size)}</td>
                      <td title={record.reason}>{record.reason}</td>
                      <td>{new Date(record.blocked_at).toLocaleString()}</td>
                      <td>
                        {record.resolved ? (
                          <span className="status-badge status-completed">已处理</span>
                        ) : (
                          <span className="status-badge status-pending">待处理</span>
                        )}
                      </td>
                      <td>
                        {!record.resolved && (
                          <button
                            onClick={() => handleSplitCompress(record, realIndex)}
                            className="small primary"
                            disabled={compressingIndex === realIndex}
                            title="自动分卷压缩并加入上传队列"
                          >
                            {compressingIndex === realIndex ? `压缩中 ${compressProgress}%` : '分卷压缩'}
                          </button>
                        )}
                        {!record.resolved && record.file_path.endsWith('-dir') && (
                          <button
                            onClick={() => handleRenameAndUpload(record, realIndex)}
                            className="small primary"
                            disabled={compressingIndex === realIndex}
                            title="自动截断文件夹名和分卷文件名，创建存根后加入上传队列"
                          >
                            {compressingIndex === realIndex ? '处理中...' : '改名重传'}
                          </button>
                        )}
                        {!record.resolved && (
                          <button
                            onClick={() => resolveBlockedFile(realIndex)}
                            className="small"
                            title="已将该文件分卷压缩并重新上传"
                          >
                            标记已处理
                          </button>
                        )}
                        <button onClick={() => removeBlockedFile(realIndex)} className="small danger">
                          删除
                        </button>
                        <button
                          onClick={() => handleOpenFileLocation(record.file_path)}
                          className="small"
                          title="打开文件所在目录"
                        >
                          定位
                        </button>
                      </td>
                    </tr>
                    {expandedBlockedIndex === realIndex && (
                      <tr className="task-detail-row">
                        <td colSpan={6}>
                          <div className="task-path-detail">
                            <div>
                              <span className="task-path-label">本地路径</span>
                              <code title={record.file_path}>{record.file_path}</code>
                            </div>
                            <div>
                              <span className="task-path-label">OpenList 目标</span>
                              <code>{record.target_path || '-'}</code>
                            </div>
                          </div>
                        </td>
                      </tr>
                    )}
                    </Fragment>
                    );
                  })}
                </tbody>
              </table>
            )}
          </div>
        )}

        {activeTab === 'settings' && (
          <div className="settings-tab">
            <div className="settings-section">
              <h3>Alist 账号配置</h3>
              <div className="form-group">
                <label>服务地址:</label>
                <input
                  type="text"
                  value={configForm.alist.base_url}
                  onChange={(e) => setConfigForm({
                    ...configForm,
                    alist: { ...configForm.alist, base_url: e.target.value }
                  })}
                  placeholder="http://127.0.0.1:5244"
                />
              </div>
              <div className="form-group">
                <label>用户名:</label>
                <input
                  type="text"
                  value={configForm.alist.username}
                  onChange={(e) => setConfigForm({
                    ...configForm,
                    alist: { ...configForm.alist, username: e.target.value }
                  })}
                  placeholder="填写你的 Alist 账号"
                />
              </div>
              <div className="form-group">
                <label>密码:</label>
                <div className="password-input-wrapper">
                  <input
                    type={showPassword ? "text" : "password"}
                    value={configForm.alist.password}
                    onChange={(e) => setConfigForm({
                      ...configForm,
                      alist: { ...configForm.alist, password: e.target.value }
                    })}
                    placeholder="填写你的 Alist 密码"
                  />
                  <button
                    type="button"
                    className="password-toggle"
                    onClick={() => setShowPassword(!showPassword)}
                    aria-label={showPassword ? '隐藏密码' : '显示密码'}
                  >
                    <span className={`eye-icon ${showPassword ? 'visible' : ''}`} />
                  </button>
                </div>
              </div>
              <div className="form-group checkbox-group">
                <input
                  type="checkbox"
                  id="autoLogin"
                  checked={configForm.alist.auto_login}
                  onChange={(e) => setConfigForm({
                    ...configForm,
                    alist: { ...configForm.alist, auto_login: e.target.checked }
                  })}
                />
                <label htmlFor="autoLogin">启动时自动登录</label>
              </div>
              <div className="form-group">
                <label>Alist 路径:</label>
                <input
                  type="text"
                  value={configForm.alist.exe_path}
                  onChange={(e) => setConfigForm({
                    ...configForm,
                    alist: { ...configForm.alist, exe_path: e.target.value }
                  })}
                  placeholder="C:\\alist\\alist.exe（留空则不自动启动）"
                />
              </div>
              <div className="form-group">
                <button
                  onClick={async () => {
                    if (testingAlist) return;
                    setTestingAlist(true);
                    await writeClientLog('点击测试启动 Alist');
                    try {
                      // 先保存配置，确保 exe_path 已持久化
                      await saveConfig(normalizeAppConfig(configForm));
                      const result = await invoke<string>('test_start_alist');
                      await writeClientLog(`测试启动 Alist 结果: ${result}`);
                      window.alert(result);
                      // 如果启动成功，触发加速健康检测
                      if (result.includes('已就绪') || result.includes('已在运行')) {
                        useAppStore.getState().fastCheckHealth();
                      }
                    } catch (error) {
                      const message = error instanceof Error ? error.message : String(error);
                      await writeClientLog(`测试启动 Alist 失败: ${message}`);
                      window.alert(`启动失败: ${message}`);
                    } finally {
                      setTestingAlist(false);
                    }
                  }}
                  className="secondary small"
                  disabled={!configForm.alist.exe_path || testingAlist}
                >
                  {testingAlist ? '正在启动...' : '测试启动 Alist'}
                </button>
                <span className="field-hint">点击前会自动保存配置；启动后等待最多 30 秒检测就绪</span>
              </div>
              <div className="form-group checkbox-group">
                <input
                  type="checkbox"
                  id="killAlistOnExit"
                  checked={configForm.alist.kill_on_exit}
                  onChange={(e) => setConfigForm({
                    ...configForm,
                    alist: { ...configForm.alist, kill_on_exit: e.target.checked }
                  })}
                />
                <label htmlFor="killAlistOnExit">退出时关闭 Alist</label>
              </div>
              <div className="form-group checkbox-group">
                <input
                  type="checkbox"
                  id="runInBackground"
                  checked={configForm.alist.run_in_background}
                  onChange={(e) => setConfigForm({
                    ...configForm,
                    alist: { ...configForm.alist, run_in_background: e.target.checked }
                  })}
                />
                <label htmlFor="runInBackground">后台启动 Alist（隐藏控制台窗口）</label>
              </div>
              <div className="form-group checkbox-group">
                <input
                  type="checkbox"
                  id="useSystemProxy"
                  checked={configForm.alist.use_system_proxy}
                  onChange={(e) => setConfigForm({
                    ...configForm,
                    alist: { ...configForm.alist, use_system_proxy: e.target.checked }
                  })}
                />
                <label htmlFor="useSystemProxy">使用系统代理（远程 Alist 才需要，本地默认关闭）</label>
              </div>
              <div className="toolbar-actions">
                <button 
                  onClick={async () => {
                    await writeClientLog(`点击登录获取 Token: base_url=${configForm.alist.base_url}, username=${configForm.alist.username}, password_length=${configForm.alist.password.length}`);

                    try {
                      await useAppStore.getState().login(
                        configForm.alist.base_url,
                        configForm.alist.username,
                        configForm.alist.password
                      );
                      setConnectionStatus('success');
                      setConnectionMessage('登录成功，Token 已保存');
                      await writeClientLog('登录获取 Token 成功');
                      setTimeout(() => setConnectionStatus('idle'), 5000);
                    } catch (error) {
                      const message = error instanceof Error ? error.message : String(error);
                      setConnectionStatus('error');
                      setConnectionMessage(message);
                      await writeClientLog(`登录获取 Token 失败: ${message}`);
                      setTimeout(() => setConnectionStatus('idle'), 5000);
                    }
                  }} 
                  className="secondary"
                  disabled={!configForm.alist.username || !configForm.alist.password}
                >
                  登录获取 Token
                </button>
                <button onClick={handleTestConnection} className="secondary">
                  测试连接
                </button>
              </div>
              {connectionStatus === 'success' && (
                <span className="test-result success">
                  {connectionMessage || (configForm.alist.token ? 'Token 已缓存' : '连接成功')}
                </span>
              )}
              {connectionStatus === 'error' && (
                <span className="test-result error">{connectionMessage || '认证失败，请检查账号密码'}</span>
              )}
              {configForm.alist.token && (
                <div className="token-info">
                  <span className="token-label">Token（自动缓存，无需修改）:</span>
                  <code>{configForm.alist.token.substring(0, 20)}...</code>
                </div>
              )}
            </div>

            <div className="settings-section">
              <h3>上传配置</h3>
              <div className="form-group">
                <label>并发数:</label>
                <input
                  type="number"
                  min="1"
                  max="10"
                  value={configForm.upload.concurrency}
                  onChange={(e) => setConfigForm({
                    ...configForm,
                    upload: { ...configForm.upload, concurrency: parseInt(e.target.value) || 1 }
                  })}
                />
              </div>
              <div className="form-group">
                <label>最大重试次数:</label>
                <input
                  type="number"
                  min="0"
                  max="10"
                  value={configForm.upload.max_retries}
                  onChange={(e) => setConfigForm({
                    ...configForm,
                    upload: { ...configForm.upload, max_retries: parseInt(e.target.value) || 0 }
                  })}
                />
              </div>
              <div className="form-group">
                <label>每轮最大上传任务数:</label>
                <input
                  type="number"
                  min="0"
                  value={configForm.upload.max_tasks_per_run}
                  onChange={(e) => setConfigForm({
                    ...configForm,
                    upload: { ...configForm.upload, max_tasks_per_run: parseInt(e.target.value) || 0 }
                  })}
                />
                <span className="field-hint">0 表示不限，达到数量后本轮自动停止</span>
              </div>
              <div className="form-group checkbox-group">
                <input
                  type="checkbox"
                  id="refreshIndexAfterUpload"
                  checked={configForm.upload.refresh_index_after_upload}
                  onChange={(e) => setConfigForm({
                    ...configForm,
                    upload: { ...configForm.upload, refresh_index_after_upload: e.target.checked }
                  })}
                />
                <label htmlFor="refreshIndexAfterUpload">上传成功后刷新目标目录触发索引更新</label>
              </div>
              <div className="form-group">
                <label>上传限速 MB/s:</label>
                <div className="speed-limit-control">
                  <select
                    value={speedLimitCustomMode ? -1 : (() => {
                      const mb_s = speedLimitToMBs(configForm.upload.speed_limit);
                      const presets = [0, 1, 2, 5, 10];
                      return presets.includes(mb_s) ? mb_s : -1;
                    })()}
                    onChange={(e) => {
                      const val = parseFloat(e.target.value);
                      if (val === -1) {
                        if (!speedLimitCustomMode) {
                          const mb_s = speedLimitToMBs(configForm.upload.speed_limit);
                          setSpeedLimitCustomText(mb_s === 0 ? '' : String(mb_s));
                        }
                        setSpeedLimitCustomMode(true);
                        return;
                      }
                      setSpeedLimitCustomMode(false);
                      setSpeedLimitCustomText('');
                      const bytesPerSec = val === 0 ? 0 : Math.round(val * 1000000);
                      const newConfig = { ...configForm, upload: { ...configForm.upload, speed_limit: bytesPerSec } };
                      setConfigForm(newConfig);
                      if (speedLimitSaveTimerRef.current) window.clearTimeout(speedLimitSaveTimerRef.current);
                      speedLimitSaveTimerRef.current = window.setTimeout(() => {
                        saveConfig(newConfig);
                      }, 500);
                    }}
                  >
                    <option value={0}>不限速</option>
                    <option value={1}>1 MB/s</option>
                    <option value={2}>2 MB/s</option>
                    <option value={5}>5 MB/s</option>
                    <option value={10}>10 MB/s</option>
                    <option value={-1}>自定义</option>
                  </select>
                  {speedLimitCustomMode && (
                    <input
                      type="number"
                      min="0"
                      max="1000"
                      step="0.1"
                      value={speedLimitCustomText}
                      onChange={(e) => {
                        setSpeedLimitCustomText(e.target.value);
                        const v = parseFloat(e.target.value);
                        const bytesPerSec = Number.isFinite(v) && v > 0 ? Math.round(v * 1000000) : 0;
                        const newConfig = { ...configForm, upload: { ...configForm.upload, speed_limit: bytesPerSec } };
                        setConfigForm(newConfig);
                        if (speedLimitSaveTimerRef.current) window.clearTimeout(speedLimitSaveTimerRef.current);
                        speedLimitSaveTimerRef.current = window.setTimeout(() => {
                          saveConfig(newConfig);
                        }, 500);
                      }}
                    />
                  )}
                </div>
                <span className="field-hint">通过 AList API 控制 AList → 云盘的上传速度，0 表示不限速</span>
              </div>
              <div className="form-group checkbox-group">
                <input
                  type="checkbox"
                  id="asTask"
                  checked={configForm.upload.as_task}
                  onChange={(e) => setConfigForm({
                    ...configForm,
                    upload: { ...configForm.upload, as_task: e.target.checked }
                  })}
                />
                <label htmlFor="asTask">使用 Alist 后台任务模式</label>
              </div>
              <div className="form-group checkbox-group">
                <input
                  type="checkbox"
                  id="showProgress"
                  checked={configForm.upload.show_progress}
                  onChange={(e) => setConfigForm({
                    ...configForm,
                    upload: { ...configForm.upload, show_progress: e.target.checked }
                  })}
                />
<label htmlFor="showProgress">显示上传进度</label>
              </div>
              <div className="form-group checkbox-group">
                <input
                  type="checkbox"
                  id="refreshIndexAfterUpload"
                  checked={configForm.upload.refresh_index_after_upload ?? true}
                  onChange={(e) => setConfigForm({
                    ...configForm,
                    upload: { ...configForm.upload, refresh_index_after_upload: e.target.checked }
                  })}
                />
                <label htmlFor="refreshIndexAfterUpload">上传成功后刷新目录索引（OpenList 增量索引，便于搜索新文件）</label>
              </div>
              <div className="form-group">
                <label>WinRAR (rar.exe) 路径:</label>
                <input
                  type="text"
                  value={configForm.upload.rar_path || ''}
                  onChange={(e) => setConfigForm({
                    ...configForm,
                    upload: { ...configForm.upload, rar_path: e.target.value }
                  })}
                  placeholder="C:\Program Files\WinRAR\rar.exe"
                />
                <span className="field-hint">用于被拦截的大文件分卷压缩</span>
              </div>
              <div className="form-group">
                <label>分卷大小 (MB):</label>
                <input
                  type="number"
                  min="100"
                  max="10240"
                  value={configForm.upload.split_volume_mb ?? 2000}
                  onChange={(e) => setConfigForm({
                    ...configForm,
                    upload: { ...configForm.upload, split_volume_mb: parseInt(e.target.value) || 2000 }
                  })}
                />
                <span className="field-hint">每卷大小，默认 2000MB（115 网盘非会员单文件限制 5GB）</span>
              </div>
              <div className="form-group checkbox-group">
                <input
                  type="checkbox"
                  id="blockDuplicateFileUpload"
                  checked={configForm.upload.block_duplicate_file_upload}
                  onChange={(e) => setConfigForm({
                    ...configForm,
                    upload: { ...configForm.upload, block_duplicate_file_upload: e.target.checked }
                  })}
                />
                <label htmlFor="blockDuplicateFileUpload">拦截同一文件重复添加（不同目标路径也拦截）</label>
              </div>

              <div className="form-group">
                <label>上传失败后行为:</label>
                <select
                  value={configForm.upload.fail_action || 'stop'}
                  onChange={(e) => setConfigForm({
                    ...configForm,
                    upload: { ...configForm.upload, fail_action: e.target.value }
                  })}
                >
                  <option value="stop">停止整个队列（默认）</option>
                  <option value="skip">跳过失败文件，继续上传</option>
                </select>
              </div>

              <div className="form-group checkbox-group">
                <input
                  type="checkbox"
                  id="notifyOnComplete"
                  checked={configForm.upload.notify_on_complete}
                  onChange={(e) => setConfigForm({
                    ...configForm,
                    upload: { ...configForm.upload, notify_on_complete: e.target.checked }
                  })}
                />
                <label htmlFor="notifyOnComplete">上传完成后发送系统通知</label>
              </div>

              <div className="form-group checkbox-group">
                <input
                  type="checkbox"
                  id="minimizeOnClose"
                  checked={configForm.upload.minimize_on_close}
                  onChange={(e) => setConfigForm({
                    ...configForm,
                    upload: { ...configForm.upload, minimize_on_close: e.target.checked }
                  })}
                />
                <label htmlFor="minimizeOnClose">关闭窗口时最小化到托盘，不退出程序</label>
              </div>

              <div className="form-group checkbox-group">
                <input
                  type="checkbox"
                  id="blockFilesOver5gb"
                  checked={configForm.upload.block_files_over_5gb}
                  onChange={(e) => setConfigForm({
                    ...configForm,
                    upload: { ...configForm.upload, block_files_over_5gb: e.target.checked }
                  })}
                />
                <label htmlFor="blockFilesOver5gb">拦截超过 5GB 的单个文件（适用于 115 网盘非会员限制）</label>
              </div>

              <div className="form-group checkbox-group">
                <input
                  type="checkbox"
                  id="warnFilesOver4gb"
                  checked={configForm.upload.warn_files_over_4gb}
                  onChange={(e) => setConfigForm({
                    ...configForm,
                    upload: { ...configForm.upload, warn_files_over_4gb: e.target.checked }
                  })}
                />
                <label htmlFor="warnFilesOver4gb">对超过 4GB 的文件显示上传失败风险提示</label>
              </div>

              <div className="large-file-notice">
                115 网盘非会员单个文件最大支持 5GB；超过 4GB 的文件上传耗时较长，若 1 小时内未完成可能因 Token 过期导致失败。
              </div>
              
              <div className="form-group radio-group">
                <label>上传方式:</label>
                <div className="radio-options">
                  <label className="radio-label">
                    <input
                      type="radio"
                      name="uploadMethod"
                      value="stream"
                      checked={configForm.upload.upload_method === 'stream'}
                      onChange={(e) => setConfigForm({
                        ...configForm,
                        upload: { ...configForm.upload, upload_method: e.target.value }
                      })}
                    />
                    Stream（流式上传）
                  </label>
                  <label className="radio-label">
                    <input
                      type="radio"
                      name="uploadMethod"
                      value="form"
                      checked={configForm.upload.upload_method === 'form'}
                      onChange={(e) => setConfigForm({
                        ...configForm,
                        upload: { ...configForm.upload, upload_method: e.target.value }
                      })}
                    />
                    Form（表单上传）
                  </label>
                </div>
              </div>
              
              <div className="form-group checkbox-group">
                <input
                  type="checkbox"
                  id="enableNotification"
                  checked={configForm.upload.notification?.enabled || false}
                  onChange={(e) => setConfigForm({
                    ...configForm,
                    upload: { 
                      ...configForm.upload, 
                      notification: {
                        enabled: e.target.checked,
                        webhook_url: configForm.upload.notification?.webhook_url || "",
                        channels: configForm.upload.notification?.channels || ["feishu"],
                      }
                    }
                  })}
                />
                <label htmlFor="enableNotification">启用失败通知</label>
              </div>
              
              {(configForm.upload.notification?.enabled || false) && (
                <div className="notification-settings">
                  <div className="form-group">
                    <label>Webhook URL:</label>
                    <input
                      type="text"
                      placeholder="https://open.feishu.cn/open-apis/bot/v2/hook/..."
                      value={configForm.upload.notification?.webhook_url || ""}
                      onChange={(e) => setConfigForm({
                        ...configForm,
                        upload: { 
                          ...configForm.upload, 
                          notification: {
                            enabled: configForm.upload.notification?.enabled || false,
                            webhook_url: e.target.value,
                            channels: configForm.upload.notification?.channels || ["feishu"],
                          }
                        }
                      })}
                    />
                  </div>
                 <div className="notification-notice">
                   ℹ️ 当文件上传失败超过重试阈值时，将发送通知并停止队列
                 </div>
                 <div className="form-group checkbox-group">
                   <input
                     type="checkbox"
                     id="notifyFeishuOnQueueComplete"
                     checked={configForm.upload.notify_feishu_on_queue_complete || false}
                     onChange={(e) => setConfigForm({
                       ...configForm,
                       upload: {
                         ...configForm.upload,
                         notify_feishu_on_queue_complete: e.target.checked
                       }
                     })}
                   />
                   <label htmlFor="notifyFeishuOnQueueComplete">队列上传完成后发送飞书通知</label>
                 </div>
                 <div className="settings-actions">
                    <button onClick={handleTestNotification} className="secondary small">
                      发送测试通知
                    </button>
                  </div>
                </div>
              )}
              
              <div className="form-group checkbox-group">
                <input
                  type="checkbox"
                  id="shutdownAfterComplete"
                  checked={configForm.upload.shutdown_after_complete || false}
                  onChange={(e) => setConfigForm({
                    ...configForm,
                    upload: {
                      ...configForm.upload,
                      shutdown_after_complete: e.target.checked
                    }
                  })}
                />
                <label htmlFor="shutdownAfterComplete">上传完成后关闭电脑</label>
              </div>
              {configForm.upload.shutdown_after_complete && (
                <div className="form-group">
                  <label>关机延迟:</label>
                  <select
                    value={configForm.upload.shutdown_delay_minutes}
                    onChange={(e) => setConfigForm({
                      ...configForm,
                      upload: {
                        ...configForm.upload,
                        shutdown_delay_minutes: Number(e.target.value)
                      }
                    })}
                  >
                    <option value={10}>10 分钟</option>
                    <option value={30}>30 分钟</option>
                    <option value={60}>1 小时</option>
                  </select>
                </div>
              )}

              <div className="form-group checkbox-group">
                <input
                  type="checkbox"
                  id="enableSchedule"
                  checked={configForm.upload.schedule?.enabled || false}
                  onChange={(e) => setConfigForm({
                    ...configForm,
                    upload: { 
                      ...configForm.upload, 
                      schedule: {
                        enabled: e.target.checked,
                        start_time: configForm.upload.schedule?.start_time || "03:00",
                        end_time: configForm.upload.schedule?.end_time || "07:00",
                        notify_on_start: configForm.upload.schedule?.notify_on_start || false,
                        notify_on_stop: configForm.upload.schedule?.notify_on_stop || false,
                      }
                    }
                  })}
                />
                <label htmlFor="enableSchedule">启用定时上传</label>
              </div>
              
              {(configForm.upload.schedule?.enabled || false) && (
                <div className="schedule-settings">
                  <div className="form-group">
                    <label>开始时间:</label>
                    <input
                      type="time"
                      value={configForm.upload.schedule?.start_time || "03:00"}
                      onChange={(e) => setConfigForm({
                        ...configForm,
                        upload: { 
                          ...configForm.upload, 
                          schedule: {
                            enabled: configForm.upload.schedule?.enabled || false,
                            start_time: e.target.value,
                            end_time: configForm.upload.schedule?.end_time || "07:00",
                            notify_on_start: configForm.upload.schedule?.notify_on_start || false,
                            notify_on_stop: configForm.upload.schedule?.notify_on_stop || false,
                          }
                        }
                      })}
                    />
                  </div>
                  <div className="form-group">
                    <label>结束时间:</label>
                    <input
                      type="time"
                      value={configForm.upload.schedule?.end_time || "07:00"}
                      onChange={(e) => setConfigForm({
                        ...configForm,
                        upload: { 
                          ...configForm.upload, 
                          schedule: {
                            enabled: configForm.upload.schedule?.enabled || false,
                            start_time: configForm.upload.schedule?.start_time || "03:00",
                            end_time: e.target.value,
                            notify_on_start: configForm.upload.schedule?.notify_on_start || false,
                            notify_on_stop: configForm.upload.schedule?.notify_on_stop || false,
                          }
                        }
                      })}
                    />
                  </div>
                  <div className="form-group checkbox-group">
                    <input
                      type="checkbox"
                      id="notifyOnStart"
                      checked={configForm.upload.schedule?.notify_on_start || false}
                      onChange={(e) => setConfigForm({
                        ...configForm,
                        upload: { 
                          ...configForm.upload, 
                          schedule: {
                            enabled: configForm.upload.schedule?.enabled || false,
                            start_time: configForm.upload.schedule?.start_time || "03:00",
                            end_time: configForm.upload.schedule?.end_time || "07:00",
                            notify_on_start: e.target.checked,
                            notify_on_stop: configForm.upload.schedule?.notify_on_stop || false,
                          }
                        }
                      })}
                    />
                    <label htmlFor="notifyOnStart">开始时发送飞书通知</label>
                  </div>
                  <div className="form-group checkbox-group">
                    <input
                      type="checkbox"
                      id="notifyOnStop"
                      checked={configForm.upload.schedule?.notify_on_stop || false}
                      onChange={(e) => setConfigForm({
                        ...configForm,
                        upload: { 
                          ...configForm.upload, 
                          schedule: {
                            enabled: configForm.upload.schedule?.enabled || false,
                            start_time: configForm.upload.schedule?.start_time || "03:00",
                            end_time: configForm.upload.schedule?.end_time || "07:00",
                            notify_on_start: configForm.upload.schedule?.notify_on_start || false,
                            notify_on_stop: e.target.checked,
                          }
                        }
                      })}
                    />
                    <label htmlFor="notifyOnStop">结束时发送飞书通知</label>
                  </div>
                  <div className="schedule-notice">
                    ℹ️ 到开始时间自动上传，到结束时间等待当前任务完成后停止
                  </div>
                </div>
              )}
            </div>

            <div className="settings-section">
              <h3>历史记录</h3>
              <div className="form-group checkbox-group">
                <input
                  type="checkbox"
                  id="historyNeverClean"
                  checked={configForm.history.never_clean}
                  onChange={(e) => setConfigForm({
                    ...configForm,
                    history: { ...configForm.history, never_clean: e.target.checked }
                  })}
                />
                <label htmlFor="historyNeverClean">永久保留历史记录（不按天数清理）</label>
              </div>
              <div className="form-group">
                <label>历史记录保留天数:</label>
                <input
                  type="number"
                  min="1"
                  max="365"
                  value={configForm.history.retention_days}
                  disabled={configForm.history.never_clean}
                  onChange={(e) => setConfigForm({
                    ...configForm,
                    history: { ...configForm.history, retention_days: parseInt(e.target.value) || 30 }
                  })}
                />
                <span className="field-hint">开启永久保留后此选项不生效</span>
              </div>
            </div>

            <div className="settings-section">
              <h3>日志同步</h3>
              <div className="form-group checkbox-group">
                <input
                  type="checkbox"
                  id="enableLogSync"
                  checked={configForm.log_sync?.enabled || false}
                  onChange={(e) => setConfigForm({
                    ...configForm,
                    log_sync: { ...configForm.log_sync!, enabled: e.target.checked }
                  })}
                />
                <label htmlFor="enableLogSync">启用日志同步到远程 Alist</label>
              </div>

              {(configForm.log_sync?.enabled || false) && (
                <div className="log-sync-settings">
                  <div className="form-group">
                    <label>日志 Alist 服务地址:</label>
                    <input
                      type="text"
                      value={configForm.log_sync?.base_url || ''}
                      onChange={(e) => setConfigForm({
                        ...configForm,
                        log_sync: { ...configForm.log_sync!, base_url: e.target.value }
                      })}
                      placeholder="http://119.91.136.173:5245"
                    />
                  </div>
                  <div className="form-group">
                    <label>用户名:</label>
                    <input
                      type="text"
                      value={configForm.log_sync?.username || ''}
                      onChange={(e) => setConfigForm({
                        ...configForm,
                        log_sync: { ...configForm.log_sync!, username: e.target.value }
                      })}
                      placeholder="日志 Alist 管理员用户名"
                    />
                  </div>
                  <div className="form-group">
                    <label>密码:</label>
                    <div className="password-input-wrapper">
                      <input
                        type={showLogSyncPassword ? "text" : "password"}
                        value={configForm.log_sync?.password || ''}
                        onChange={(e) => setConfigForm({
                          ...configForm,
                          log_sync: { ...configForm.log_sync!, password: e.target.value }
                        })}
                        placeholder="日志 Alist 密码"
                      />
                      <button
                        type="button"
                        className="password-toggle"
                        onClick={() => setShowLogSyncPassword(!showLogSyncPassword)}
                        aria-label={showLogSyncPassword ? '隐藏密码' : '显示密码'}
                      >
                        <span className={`eye-icon ${showLogSyncPassword ? 'visible' : ''}`} />
                      </button>
                    </div>
                  </div>
                  <div className="form-group">
                    <label>日志存放目录:</label>
                    <input
                      type="text"
                      value={configForm.log_sync?.target_path || ''}
                      onChange={(e) => setConfigForm({
                        ...configForm,
                        log_sync: { ...configForm.log_sync!, target_path: e.target.value }
                      })}
                      placeholder="/本地磁盘/openlist-uploader-logs"
                    />
                    <span className="field-hint">日志文件将上传到此目录（覆盖写）</span>
                  </div>
                  <div className="form-group checkbox-group">
                    <input
                      type="checkbox"
                      id="logSyncOnExit"
                      checked={configForm.log_sync?.sync_on_exit || false}
                      onChange={(e) => setConfigForm({
                        ...configForm,
                        log_sync: { ...configForm.log_sync!, sync_on_exit: e.target.checked }
                      })}
                    />
                    <label htmlFor="logSyncOnExit">退出程序时自动同步日志</label>
                  </div>
                  <div className="form-group">
                    <label>定时同步间隔（分钟）:</label>
                    <input
                      type="number"
                      min="0"
                      max="1440"
                      value={configForm.log_sync?.sync_interval_minutes ?? 30}
                      onChange={(e) => setConfigForm({
                        ...configForm,
                        log_sync: { ...configForm.log_sync!, sync_interval_minutes: parseInt(e.target.value) || 0 }
                      })}
                    />
                    <span className="field-hint">0 = 关闭定时同步；30 = 每 30 分钟自动同步一次。断电/崩溃时最多丢失一个间隔内的日志</span>
                  </div>
                  <div className="form-group checkbox-group">
                    <input
                      type="checkbox"
                      id="logSyncUseProxy"
                      checked={configForm.log_sync?.use_system_proxy || false}
                      onChange={(e) => setConfigForm({
                        ...configForm,
                        log_sync: { ...configForm.log_sync!, use_system_proxy: e.target.checked }
                      })}
                    />
                    <label htmlFor="logSyncUseProxy">使用系统代理（远程 Alist 才需要）</label>
                  </div>
                  <div className="toolbar-actions">
                    <button
                      onClick={handleLogSyncLogin}
                      className="secondary"
                      disabled={!configForm.log_sync?.base_url || !configForm.log_sync?.username || !configForm.log_sync?.password}
                    >
                      登录测试
                    </button>
                    <button
                      onClick={handleSyncLogs}
                      className="secondary"
                      disabled={logSyncStatus === 'syncing' || !configForm.log_sync?.base_url}
                    >
                      {logSyncStatus === 'syncing' ? '同步中...' : '立即同步日志'}
                    </button>
                    <button
                      onClick={loadLocalLogFiles}
                      className="secondary small"
                    >
                      刷新本地日志列表
                    </button>
                    <button
                      onClick={handleOpenLogFolder}
                      className="secondary small"
                      title="打开本地日志所在文件夹"
                    >
                      打开日志目录
                    </button>
                  </div>
                  {logSyncLoginStatus === 'success' && (
                    <span className="test-result success">{logSyncLoginMessage}</span>
                  )}
                  {logSyncLoginStatus === 'error' && (
                    <span className="test-result error">{logSyncLoginMessage}</span>
                  )}
                  {logSyncStatus === 'success' && (
                    <span className="test-result success">{logSyncMessage}</span>
                  )}
                  {logSyncStatus === 'error' && (
                    <span className="test-result error">{logSyncMessage}</span>
                  )}
                  {configForm.log_sync?.last_sync_at && (
                    <div className="token-info">
                      <span className="token-label">最近同步时间:</span>
                      <code>{formatDateTime(configForm.log_sync.last_sync_at)}</code>
                    </div>
                  )}
                  {configForm.log_sync?.token && (
                    <div className="token-info">
                      <span className="token-label">Token（自动缓存）:</span>
                      <code>{configForm.log_sync.token.substring(0, 20)}...</code>
                    </div>
                  )}
                  {localLogFiles.length > 0 && (
                    <div className="local-log-files">
                      <h4>本地日志文件</h4>
                      <table className="log-files-table">
                        <thead>
                          <tr>
                            <th>文件名</th>
                            <th>大小</th>
                            <th>修改时间</th>
                          </tr>
                        </thead>
                        <tbody>
                          {localLogFiles.map((f, i) => (
                            <tr key={i}>
                              <td>{f.name}</td>
                              <td>{formatFileSize(f.size)}</td>
                              <td>{f.modified ? formatDateTime(f.modified) : '-'}</td>
                            </tr>
                          ))}
                        </tbody>
                      </table>
                    </div>
                  )}
                  <div className="notification-notice">
                    日志同步到独立的远程 Alist 实例，不影响上传用的 Alist。日志文件覆盖写，开发端可通过 Alist Web 界面或 API 拉取分析。
                  </div>
                </div>
              )}
            </div>

            <div className="settings-actions">
              <button onClick={handleCheckUpdate} className="secondary">
                检查更新
              </button>
              <button onClick={handleSaveConfig} className="primary" disabled={saveConfigStatus === 'saving'}>
                {saveConfigStatus === 'saving' ? '保存中...' : '保存配置'}
              </button>
              {saveConfigStatus === 'success' && (
                <span className="test-result success">{saveConfigMessage}</span>
              )}
             {saveConfigStatus === 'error' && (
               <span className="test-result error">{saveConfigMessage}</span>
             )}
           </div>
           <div className="settings-version">
              当前版本：{appVersion}
            </div>
            <div className="form-group checkbox-group">
              <input
                type="checkbox"
                id="checkUpdateOnStartup"
                checked={configForm.upload.check_update_on_startup}
                onChange={(e) => setConfigForm({
                  ...configForm,
                  upload: { ...configForm.upload, check_update_on_startup: e.target.checked }
                })}
              />
              <label htmlFor="checkUpdateOnStartup">启动时自动检查更新</label>
              </div>
              <div className="form-group checkbox-group">
                <input
                  type="checkbox"
                  id="autoStartOnBoot"
                  checked={configForm.upload.auto_start_on_boot}
                  onChange={async (e) => {
                    const enabled = e.target.checked;
                    setConfigForm({ ...configForm, upload: { ...configForm.upload, auto_start_on_boot: enabled } });
                    try {
                      await invoke('set_autostart', { enabled });
                      await writeClientLog(`开机自启${enabled ? '已开启' : '已关闭'}`);
                    } catch (error) {
                      const msg = error instanceof Error ? error.message : String(error);
                      await writeClientLog(`设置开机自启失败: ${msg}`);
                      window.alert(`设置开机自启失败: ${msg}`);
                      setConfigForm({ ...configForm, upload: { ...configForm.upload, auto_start_on_boot: !enabled } });
                    }
                  }}
                />
                <label htmlFor="autoStartOnBoot">开机自启动（异常重启后自动恢复）</label>
              </div>
              <div className="form-group checkbox-group">
                <input
                  type="checkbox"
                  id="progressNotifyEnabled"
                  checked={configForm.upload.progress_notify_enabled}
                  onChange={(e) => setConfigForm({
                    ...configForm,
                    upload: { ...configForm.upload, progress_notify_enabled: e.target.checked }
                  })}
                />
                <label htmlFor="progressNotifyEnabled">上传期间定时发送进度通知</label>
              </div>
              {configForm.upload.progress_notify_enabled && (
                <div className="form-group">
                  <label>通知间隔（分钟）:</label>
                  <input
                    type="number"
                    min="1"
                    max="1440"
                    value={configForm.upload.progress_notify_interval}
                    onChange={(e) => setConfigForm({
                      ...configForm,
                      upload: { ...configForm.upload, progress_notify_interval: parseInt(e.target.value) || 30 }
                    })}
                  />
                </div>
              )}
          </div>
       )}
      </main>
      {downloadingUpdate && (
        <div className="update-download-overlay">
          <div className="update-download-dialog">
            <h3>正在下载更新</h3>
            <div className="update-progress-track">
              <div className="update-progress-fill" style={{ width: `${downloadProgress}%` }} />
            </div>
            <div className="update-progress-info">
              <span>{downloadProgress}%</span>
              {downloadSizeText && <span>{downloadSizeText}</span>}
            </div>
          </div>
       </div>
     )}
      {shutdownDeadline && (
        <div className="shutdown-overlay" onClick={(e) => e.stopPropagation()}>
          <div className="shutdown-dialog">
            <h3>电脑即将关机</h3>
            <p className="shutdown-countdown">{shutdownCountdown}</p>
            <p className="shutdown-hint">上传队列已完成，电脑将自动关闭</p>
            <button className="shutdown-cancel-btn" onClick={handleCancelShutdown}>
              取消关机
            </button>
          </div>
        </div>
      )}
   </div>
 );
}

export default App;
