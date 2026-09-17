import { useState, useEffect, useRef, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { AppConfig, DirItem } from '../types';

interface FolderPickerProps {
  value: string;
  onChange: (path: string) => void;
  recentPaths: string[];
  onAddRecentPath: (path: string) => void;
  disabled?: boolean;
}

function normalizePath(path: string): string {
  const trimmed = path.trim();
  if (!trimmed || trimmed === '/') return '/';
  const withPrefix = trimmed.startsWith('/') ? trimmed : `/${trimmed}`;
  return withPrefix.replace(/\/+$/, '') || '/';
}

function getPathSegments(path: string): { name: string; path: string }[] {
  if (path === '/') return [{ name: '根目录', path: '/' }];
  const parts = path.split('/').filter(Boolean);
  const segments: { name: string; path: string }[] = [{ name: '根目录', path: '/' }];
  let current = '';
  for (const part of parts) {
    current = current === '' ? `/${part}` : `${current}/${part}`;
    segments.push({ name: part, path: current });
  }
  return segments;
}

export function FolderPicker({ value, onChange, recentPaths, onAddRecentPath, disabled }: FolderPickerProps) {
  const [isOpen, setIsOpen] = useState(false);
  const [browsingPath, setBrowsingPath] = useState('/');
  const [folders, setFolders] = useState<DirItem[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState('');
  const [searchQuery, setSearchQuery] = useState('');
  const [showManualInput, setShowManualInput] = useState(false);
  const [manualDraft, setManualDraft] = useState('');
  const [manualError, setManualError] = useState('');
  const [manualVerifying, setManualVerifying] = useState(false);
  const [showCreateInput, setShowCreateInput] = useState(false);
  const [createDraft, setCreateDraft] = useState('');
  const [creating, setCreating] = useState(false);
  const [createError, setCreateError] = useState('');

  const searchInputRef = useRef<HTMLInputElement>(null);
  const manualInputRef = useRef<HTMLInputElement>(null);
  const createInputRef = useRef<HTMLInputElement>(null);

  const isRoot = browsingPath === '/';
  const isValueRoot = normalizePath(value) === '/';

  const loadFolders = useCallback(async (path: string) => {
    setIsLoading(true);
    setError('');
    try {
      await invoke('write_client_log', { message: `打开 Alist 目录浏览: path=${path}` });
      const config = await invoke<AppConfig>('get_config');
      const alistConfig = config.alist;

      if (!alistConfig.base_url) {
        setError('请先在设置中配置 Alist 服务地址');
        await invoke('write_client_log', { message: 'Alist 目录浏览失败: base_url 为空' });
        return;
      }

      const rawItems = await invoke<string>('alist_list_dir', { config, path });
      const items = JSON.parse(rawItems || '[]') as DirItem[];
      const directories = items.filter(item => item.is_dir);
      await invoke('write_client_log', { message: `Alist 目录浏览成功: path=${path}, item_count=${items.length}, dir_count=${directories.length}` });
      setFolders(directories);
      setBrowsingPath(path);
    } catch (err: any) {
      const message = err instanceof Error ? err.message : String(err);
      setError(message);
      await invoke('write_client_log', { message: `Alist 目录浏览异常: path=${path}, error=${message}` });
    } finally {
      setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    if (isOpen) {
      const startPath = normalizePath(value || '/');
      setBrowsingPath(startPath);
      setSearchQuery('');
      setShowManualInput(false);
      setManualDraft('');
      setManualError('');
      setShowCreateInput(false);
      setCreateDraft('');
      setCreateError('');
      loadFolders(startPath);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isOpen]);

  useEffect(() => {
    if (!isOpen) return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        setIsOpen(false);
      }
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [isOpen]);

  useEffect(() => {
    if (isOpen && !isLoading) {
      const timer = setTimeout(() => {
        searchInputRef.current?.focus();
      }, 100);
      return () => clearTimeout(timer);
    }
  }, [isOpen, isLoading]);

  const handleOpen = () => setIsOpen(true);

  const handleEnterFolder = (folderName: string) => {
    const newPath = browsingPath === '/' ? `/${folderName}` : `${browsingPath}/${folderName}`;
    setSearchQuery('');
    loadFolders(newPath);
  };

  const handleNavigateTo = (path: string) => {
    setSearchQuery('');
    loadFolders(path);
  };

  const handleConfirm = () => {
    if (isRoot || isLoading || error) return;
    onChange(browsingPath);
    onAddRecentPath(browsingPath);
    setIsOpen(false);
  };

  const handleRetry = () => {
    loadFolders(browsingPath);
  };

  const handleManualVerify = async () => {
    const normalized = normalizePath(manualDraft);
    if (normalized === '/') {
      setManualError('根目录不能作为上传目标，请输入具体目录');
      return;
    }
    setManualVerifying(true);
    setManualError('');
    try {
      const config = await invoke<AppConfig>('get_config');
      if (!config.alist.base_url) {
        setManualError('请先在设置中配置 Alist 服务地址');
        return;
      }
      const rawItems = await invoke<string>('alist_list_dir', { config, path: normalized });
      const items = JSON.parse(rawItems || '[]') as DirItem[];
      await invoke('write_client_log', { message: `手输路径验证成功: path=${normalized}, item_count=${items.length}` });
      setBrowsingPath(normalized);
      setFolders(items.filter(item => item.is_dir));
      setSearchQuery('');
      setShowManualInput(false);
      setManualDraft('');
    } catch (err: any) {
      const message = err instanceof Error ? err.message : String(err);
      setManualError(`目录不存在或无法访问：${message}`);
      await invoke('write_client_log', { message: `手输路径验证失败: path=${normalized}, error=${message}` });
    } finally {
      setManualVerifying(false);
    }
  };

  const handleToggleCreate = () => {
    setShowCreateInput(!showCreateInput);
    setCreateError('');
    setCreateDraft('');
    if (!showCreateInput) {
      setTimeout(() => createInputRef.current?.focus(), 50);
    }
  };

  const handleCreateFolder = async () => {
    const name = createDraft.trim();
    if (!name) {
      setCreateError('请输入文件夹名称');
      return;
    }
    if (name.includes('/') || name.includes('\\') || name === '.' || name === '..') {
      setCreateError('文件夹名称不能包含 / \\ 等非法字符');
      return;
    }
    // 115Crypt 加密驱动限制：目录名称 UTF-8 字节数不超过 175（实测）
    const nameBytes = new TextEncoder().encode(name).length;
    if (nameBytes > 175) {
      setCreateError(`文件夹名称过长，115Crypt 限制 175 字节，当前 ${nameBytes} 字节（${name.length} 个字符）`);
      return;
    }
    setCreating(true);
    setCreateError('');
    try {
      const config = await invoke<AppConfig>('get_config');
      if (!config.alist.base_url) {
        setCreateError('请先在设置中配置 Alist 服务地址');
        return;
      }
      const newPath = browsingPath === '/' ? `/${name}` : `${browsingPath}/${name}`;
      await invoke('alist_mkdir', { config, path: newPath });
      await invoke('write_client_log', { message: `Alist 创建文件夹成功: path=${newPath}` });
      setShowCreateInput(false);
      setCreateDraft('');
      // 创建成功后直接进入新目录，方便继续上传
      await loadFolders(newPath);
    } catch (err: any) {
      const message = err instanceof Error ? err.message : String(err);
      setCreateError(message);
      await invoke('write_client_log', { message: `Alist 创建文件夹失败: path=${browsingPath}/${name}, error=${message}` });
    } finally {
      setCreating(false);
    }
  };

  const filteredFolders = searchQuery
    ? folders.filter(f => f.name.toLowerCase().includes(searchQuery.toLowerCase()))
    : folders;

  const segments = getPathSegments(browsingPath);
  const canConfirm = !isRoot && !isLoading && !error;

  return (
    <>
      <button
        type="button"
        className={`folder-picker-trigger ${isValueRoot ? 'is-root' : ''}`}
        onClick={handleOpen}
        disabled={disabled}
      >
        <span className="folder-picker-trigger-icon">📁</span>
        <span className="folder-picker-trigger-path">{value || '/'}</span>
        <span className="folder-picker-trigger-btn">更改</span>
      </button>

      {isOpen && (
        <div className="folder-picker-overlay" onClick={() => setIsOpen(false)}>
          <div
            className="folder-picker-modal"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="folder-picker-titlebar">
              <h3>选择上传目录</h3>
              <button
                type="button"
                className="folder-picker-close"
                onClick={() => setIsOpen(false)}
                aria-label="关闭"
              >
                ✕
              </button>
            </div>

            <div className="folder-picker-toolbar">
              <div className="folder-picker-breadcrumb">
                {segments.map((seg, idx) => (
                  <span key={seg.path} className="breadcrumb-segment">
                    {idx > 0 && <span className="breadcrumb-separator">/</span>}
                    <button
                      type="button"
                      className={`breadcrumb-link ${seg.path === browsingPath ? 'current' : ''}`}
                      onClick={() => handleNavigateTo(seg.path)}
                      disabled={isLoading}
                    >
                      {seg.name}
                    </button>
                  </span>
                ))}
              </div>
              <div className="folder-picker-toolbar-actions">
                <button
                  type="button"
                  className="small secondary"
                  onClick={handleRetry}
                  disabled={isLoading}
                  title="刷新当前目录"
                >
                  刷新
                </button>
                <button
                  type="button"
                  className="small secondary"
                  onClick={() => {
                    setShowManualInput(!showManualInput);
                    setManualError('');
                    if (!showManualInput) {
                      setTimeout(() => manualInputRef.current?.focus(), 50);
                    }
                  }}
                  disabled={isLoading}
                >
                  {showManualInput ? '取消输入' : '输入路径'}
                </button>
                <button
                  type="button"
                  className="small secondary"
                  onClick={handleToggleCreate}
                  disabled={isLoading || isRoot}
                  title={isRoot ? '根目录下不建议直接新建' : '在当前目录下新建文件夹'}
                >
                  {showCreateInput ? '取消新建' : '新建文件夹'}
                </button>
              </div>
            </div>

            {showManualInput && (
              <div className="folder-picker-manual">
                <input
                  ref={manualInputRef}
                  type="text"
                  value={manualDraft}
                  onChange={(e) => setManualDraft(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter') {
                      e.preventDefault();
                      handleManualVerify();
                    }
                  }}
                  placeholder="/云盘/目标目录"
                  disabled={manualVerifying}
                />
                <button
                  type="button"
                  className="primary small"
                  onClick={handleManualVerify}
                  disabled={manualVerifying || !manualDraft.trim()}
                >
                  {manualVerifying ? '验证中...' : '验证并跳转'}
                </button>
                {manualError && <div className="manual-error">{manualError}</div>}
              </div>
            )}

            {showCreateInput && (
              <div className="folder-picker-manual">
                <input
                  ref={createInputRef}
                  type="text"
                  value={createDraft}
                  onChange={(e) => setCreateDraft(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter') {
                      e.preventDefault();
                      handleCreateFolder();
                    }
                  }}
                  placeholder="新文件夹名称"
                  disabled={creating}
                />
                <button
                  type="button"
                  className="primary small"
                  onClick={handleCreateFolder}
                  disabled={creating || !createDraft.trim()}
                >
                  {creating ? '创建中...' : '创建'}
                </button>
                {createError && <div className="manual-error">{createError}</div>}
              </div>
            )}

            <div className="folder-picker-search">
              <input
                ref={searchInputRef}
                type="text"
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
                placeholder="筛选当前目录..."
                disabled={isLoading || !!error}
              />
            </div>

            {recentPaths.length > 0 && browsingPath === '/' && !searchQuery && !showManualInput && (
              <div className="folder-picker-recent">
                <span className="recent-label">最近使用</span>
                <div className="recent-list">
                  {recentPaths.map((rp) => (
                    <button
                      key={rp}
                      type="button"
                      className="recent-item"
                      onClick={() => handleNavigateTo(rp)}
                      disabled={isLoading}
                    >
                      {rp}
                    </button>
                  ))}
                </div>
              </div>
            )}

            <div className="folder-picker-list">
              {isLoading ? (
                <div className="folder-picker-status">加载中...</div>
              ) : error ? (
                <div className="folder-picker-status error">
                  <span>{error}</span>
                  <button type="button" className="small" onClick={handleRetry}>重试</button>
                </div>
              ) : filteredFolders.length === 0 ? (
                <div className="folder-picker-status">
                  {searchQuery ? '没有匹配的子目录' : '此目录下没有子目录'}
                </div>
              ) : (
                <div className="folder-list">
                  {filteredFolders.map((folder) => (
                    <div
                      key={folder.name}
                      className="folder-item"
                      onClick={() => handleEnterFolder(folder.name)}
                    >
                      <span className="folder-icon">📁</span>
                      <span className="folder-name">{folder.name}</span>
                      <span className="folder-enter">›</span>
                    </div>
                  ))}
                </div>
              )}
            </div>

            <div className="folder-picker-footer">
              <div className="folder-picker-current">
                <span className="footer-label">当前目录：</span>
                <code>{browsingPath}</code>
              </div>
              <div className="folder-picker-footer-actions">
                <button type="button" className="secondary" onClick={() => setIsOpen(false)}>取消</button>
                <button
                  type="button"
                  className="primary"
                  onClick={handleConfirm}
                  disabled={!canConfirm}
                  title={isRoot ? '根目录仅用于浏览，请选择具体子目录' : undefined}
                >
                  上传到此目录
                </button>
              </div>
            </div>
          </div>
        </div>
      )}
    </>
  );
}
