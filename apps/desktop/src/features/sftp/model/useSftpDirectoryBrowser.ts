/* eslint-disable react-hooks/refs, react-hooks/set-state-in-effect */
import { useCallback, useEffect, useRef, useState } from 'react';
import {
  isLocalFileApiAvailable,
  listLocalDir,
  listRemoteDir,
  type LocalDirectoryListing,
  type RemoteDirectoryListing,
} from '../api/sftp-client';
import { nextDirectoryLoading } from './sftp-directory-state';
import { getSftpErrorMessage } from './sftp-error';
import { refreshSavedFile } from './file-save-refresh';
import type { FileEditorSource } from './file-editor-store';

interface UseSftpDirectoryBrowserOptions {
  enabled: boolean;
  activeConnectionId?: string;
  activeConnectionAttempt: number;
  activeRemotePath: string;
  initialLocalPath: string;
  rememberLocalPath: (path: string) => void;
  setCurrentPath: (connectionId: string, path: string) => void;
  setSessionStatus: (
    connectionId: string,
    status: 'connecting' | 'connected' | 'error',
    lastError?: string | null
  ) => void;
}

export function useSftpDirectoryBrowser({
  enabled,
  activeConnectionId,
  activeConnectionAttempt,
  activeRemotePath,
  initialLocalPath,
  rememberLocalPath,
  setCurrentPath,
  setSessionStatus,
}: UseSftpDirectoryBrowserOptions) {
  const [localListing, setLocalListing] = useState<LocalDirectoryListing | null>(null);
  const [remoteListing, setRemoteListing] = useState<RemoteDirectoryListing | null>(null);
  const [localPath, setLocalPath] = useState<string | undefined>(initialLocalPath);
  const [remotePath, setRemotePath] = useState<string | undefined>();
  const [localLoading, setLocalLoading] = useState(false);
  const [remoteLoading, setRemoteLoading] = useState(false);
  const [localError, setLocalError] = useState('');
  const [remoteError, setRemoteError] = useState('');
  const localCacheRef = useRef<Map<string, LocalDirectoryListing>>(new Map());
  const remoteCacheRef = useRef<Map<string, RemoteDirectoryListing>>(new Map());
  const activeConnectionIdRef = useRef<string | null>(null);
  const localRequestIdRef = useRef(0);
  const remoteRequestIdRef = useRef(0);
  const handledConnectionAttemptsRef = useRef<Map<string, number>>(new Map());
  const browserSnapshotRef = useRef({
    enabled,
    connectionId: activeConnectionId,
    localPath,
    remotePath: activeRemotePath,
  });
  browserSnapshotRef.current = {
    enabled,
    connectionId: activeConnectionId,
    localPath,
    remotePath: activeRemotePath,
  };

  // 远程目录请求可能乱序返回，用 ref 保存当前连接，避免旧连接结果覆盖新连接页面。
  activeConnectionIdRef.current = activeConnectionId ?? null;

  const loadLocalDirectory = useCallback(
    async (path?: string, force = false) => {
      if (!enabled) return;
      const requestId = ++localRequestIdRef.current;
      const requestedPath = path || '~';
      setLocalPath(requestedPath);
      if (!isLocalFileApiAvailable()) {
        setLocalLoading((current) => nextDirectoryLoading(current, 'request-unavailable'));
        setLocalListing(null);
        setLocalError('当前环境不支持读取本地目录，请在 Tauri 客户端中使用。');
        return;
      }

      const cacheKey = requestedPath;
      if (!force) {
        const cached = localCacheRef.current.get(cacheKey);
        if (cached) {
          if (localRequestIdRef.current !== requestId) return;
          setLocalListing(cached);
          setLocalPath(cached.path);
          rememberLocalPath(cached.path);
          setLocalError('');
          setLocalLoading((current) => nextDirectoryLoading(current, 'cache-hit'));
          return;
        }
      }

      setLocalLoading((current) => nextDirectoryLoading(current, 'request-started'));
      setLocalError('');
      try {
        const listing = await listLocalDir(requestedPath);
        localCacheRef.current.set(cacheKey, listing);
        localCacheRef.current.set(listing.path, listing);
        if (localRequestIdRef.current !== requestId) return;
        setLocalListing(listing);
        setLocalPath(listing.path);
        rememberLocalPath(listing.path);
      } catch (err) {
        if (localRequestIdRef.current !== requestId) return;
        setLocalError(getSftpErrorMessage(err));
      } finally {
        if (localRequestIdRef.current === requestId) {
          setLocalLoading((current) => nextDirectoryLoading(current, 'current-request-settled'));
        }
      }
    },
    [enabled, rememberLocalPath]
  );

  const loadRemoteDirectory = useCallback(
    async (path?: string, force = false) => {
      if (!enabled) return;
      const connectionId = activeConnectionId;
      if (!connectionId) return;
      // 保存或传输完成的旧闭包不能启动另一连接的 loading，也不能抢占当前请求 ID。
      if (!browserSnapshotRef.current.enabled || activeConnectionIdRef.current !== connectionId)
        return;
      const requestId = ++remoteRequestIdRef.current;
      if (!isLocalFileApiAvailable()) {
        setRemoteLoading((current) => nextDirectoryLoading(current, 'request-unavailable'));
        setRemoteListing(null);
        const message = '当前环境不支持读取远程目录，请在 Tauri 客户端中使用。';
        setRemoteError(message);
        setSessionStatus(connectionId, 'error', message);
        return;
      }

      // 远程路径缓存必须带连接 ID，避免不同服务器相同路径互相污染。
      const cacheKey = `${connectionId}:${path || ''}`;
      if (!force) {
        const cached = remoteCacheRef.current.get(cacheKey);
        if (cached) {
          if (activeConnectionIdRef.current !== connectionId) return;
          if (remoteRequestIdRef.current !== requestId) return;
          setRemoteListing(cached);
          setRemotePath(cached.path);
          setCurrentPath(connectionId, cached.path);
          setSessionStatus(connectionId, 'connected');
          setRemoteError('');
          // 新请求命中缓存后，旧请求已失去清理 loading 的资格；当前分支必须接管。
          setRemoteLoading((current) => nextDirectoryLoading(current, 'cache-hit'));
          return;
        }
      }

      setRemoteLoading((current) => nextDirectoryLoading(current, 'request-started'));
      setRemoteError('');
      setSessionStatus(connectionId, 'connecting');
      try {
        const listing = await listRemoteDir(connectionId, path);
        remoteCacheRef.current.set(cacheKey, listing);
        remoteCacheRef.current.set(`${connectionId}:${listing.path}`, listing);
        if (
          activeConnectionIdRef.current !== connectionId ||
          remoteRequestIdRef.current !== requestId
        )
          return;
        setRemoteListing(listing);
        setRemotePath(listing.path);
        setCurrentPath(connectionId, listing.path);
        setSessionStatus(connectionId, 'connected');
      } catch (err) {
        if (
          activeConnectionIdRef.current !== connectionId ||
          remoteRequestIdRef.current !== requestId
        )
          return;
        const message = getSftpErrorMessage(err);
        setRemoteError(message);
        setSessionStatus(connectionId, 'error', message);
      } finally {
        if (
          activeConnectionIdRef.current === connectionId &&
          remoteRequestIdRef.current === requestId
        )
          setRemoteLoading((current) => nextDirectoryLoading(current, 'current-request-settled'));
      }
    },
    [activeConnectionId, enabled, setCurrentPath, setSessionStatus]
  );

  const invalidateLocalPath = useCallback((path: string) => {
    for (const [key, listing] of localCacheRef.current) {
      if (key === path || listing.path === path) localCacheRef.current.delete(key);
    }
  }, []);

  const invalidateRemotePath = useCallback((connectionId: string, path: string) => {
    for (const [key, listing] of remoteCacheRef.current) {
      if (key.startsWith(`${connectionId}:`) && listing.path === path)
        remoteCacheRef.current.delete(key);
    }
  }, []);

  // 回调身份稳定，执行时读取最新页面与加载函数，保存中的路由切换不改变通知目标。
  const loadersRef = useRef({ loadLocalDirectory, loadRemoteDirectory });
  loadersRef.current = { loadLocalDirectory, loadRemoteDirectory };
  const onFileSaved = useCallback(
    (source: FileEditorSource) => {
      refreshSavedFile(source, browserSnapshotRef.current, {
        invalidateLocalPath,
        invalidateRemotePath,
        ...loadersRef.current,
      });
    },
    [invalidateLocalPath, invalidateRemotePath]
  );

  useEffect(() => {
    if (!enabled) return;
    void loadLocalDirectory(localPath);
    // 首次加载和显式路径变更由 loadLocalDirectory 控制，避免刷新时重复请求。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [enabled]);

  useEffect(() => {
    if (!enabled) {
      // 终端页隐藏 SftpView 时使在途远程响应过期，并且不能留下常驻 loading。
      remoteRequestIdRef.current += 1;
      setRemoteLoading((current) => nextDirectoryLoading(current, 'request-unavailable'));
      return;
    }
    if (!activeConnectionId) {
      remoteRequestIdRef.current += 1;
      setRemoteLoading((current) => nextDirectoryLoading(current, 'request-unavailable'));
      setRemoteListing(null);
      setRemotePath(undefined);
      setRemoteError('');
      remoteCacheRef.current.clear();
      handledConnectionAttemptsRef.current.clear();
      return;
    }
    const previousAttempt = handledConnectionAttemptsRef.current.get(activeConnectionId);
    handledConnectionAttemptsRef.current.set(activeConnectionId, activeConnectionAttempt);
    if (previousAttempt !== undefined && previousAttempt !== activeConnectionAttempt) {
      // 同一数据库 ID 的目标配置可能已变，不能复用其它路径的旧服务器缓存。
      for (const key of remoteCacheRef.current.keys()) {
        if (key.startsWith(`${activeConnectionId}:`)) remoteCacheRef.current.delete(key);
      }
    }
    // 用户再次执行“连接”时必须越过旧缓存；路径规范化导致的后续渲染仍可使用缓存。
    void loadRemoteDirectory(
      activeRemotePath,
      previousAttempt !== undefined && previousAttempt !== activeConnectionAttempt
    );
  }, [activeConnectionAttempt, activeConnectionId, activeRemotePath, enabled, loadRemoteDirectory]);

  return {
    localListing,
    remoteListing,
    localPath,
    remotePath,
    localLoading,
    remoteLoading,
    localError,
    remoteError,
    loadLocalDirectory,
    loadRemoteDirectory,
    invalidateLocalPath,
    invalidateRemotePath,
    onFileSaved,
  };
}
