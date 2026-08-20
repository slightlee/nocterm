/* eslint-disable react-hooks/refs, react-hooks/set-state-in-effect */
import { useCallback, useEffect, useRef, useState } from 'react';
import {
  isLocalFileApiAvailable,
  listLocalDir,
  listRemoteDir,
  type LocalDirectoryListing,
  type RemoteDirectoryListing,
} from '../api/sftp-client';
import { getSftpErrorMessage } from './sftp-error';

interface UseSftpDirectoryBrowserOptions {
  activeConnectionId?: string;
  activeConnectionAttempt: number;
  activeRemotePath: string;
  setCurrentPath: (connectionId: string, path: string) => void;
  setSessionStatus: (
    connectionId: string,
    status: 'connecting' | 'connected' | 'error',
    lastError?: string | null
  ) => void;
}

export function useSftpDirectoryBrowser({
  activeConnectionId,
  activeConnectionAttempt,
  activeRemotePath,
  setCurrentPath,
  setSessionStatus,
}: UseSftpDirectoryBrowserOptions) {
  const [localListing, setLocalListing] = useState<LocalDirectoryListing | null>(null);
  const [remoteListing, setRemoteListing] = useState<RemoteDirectoryListing | null>(null);
  const [localPath, setLocalPath] = useState<string | undefined>();
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

  // 远程目录请求可能乱序返回，用 ref 保存当前连接，避免旧连接结果覆盖新连接页面。
  activeConnectionIdRef.current = activeConnectionId ?? null;

  const loadLocalDirectory = useCallback(async (path?: string, force = false) => {
    const requestId = ++localRequestIdRef.current;
    if (!isLocalFileApiAvailable()) {
      setLocalListing(null);
      setLocalError('当前环境不支持读取本地目录，请在 Tauri 客户端中使用。');
      return;
    }

    const cacheKey = path || '';
    if (!force) {
      const cached = localCacheRef.current.get(cacheKey);
      if (cached) {
        if (localRequestIdRef.current !== requestId) return;
        setLocalListing(cached);
        setLocalPath(cached.path);
        setLocalError('');
        return;
      }
    }

    setLocalLoading(true);
    setLocalError('');
    try {
      const listing = await listLocalDir(path);
      localCacheRef.current.set(cacheKey, listing);
      localCacheRef.current.set(listing.path, listing);
      if (localRequestIdRef.current !== requestId) return;
      setLocalListing(listing);
      setLocalPath(listing.path);
    } catch (err) {
      if (localRequestIdRef.current !== requestId) return;
      setLocalError(err instanceof Error ? err.message : String(err));
    } finally {
      if (localRequestIdRef.current === requestId) setLocalLoading(false);
    }
  }, []);

  const loadRemoteDirectory = useCallback(
    async (path?: string, force = false) => {
      const connectionId = activeConnectionId;
      if (!connectionId) return;
      const requestId = ++remoteRequestIdRef.current;
      if (!isLocalFileApiAvailable()) {
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
          return;
        }
      }

      setRemoteLoading(true);
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
          setRemoteLoading(false);
      }
    },
    [activeConnectionId, setCurrentPath, setSessionStatus]
  );

  const invalidateLocalPath = useCallback((path: string) => {
    localCacheRef.current.delete(path);
  }, []);

  const invalidateRemotePath = useCallback((connectionId: string, path: string) => {
    remoteCacheRef.current.delete(`${connectionId}:${path}`);
  }, []);

  useEffect(() => {
    void loadLocalDirectory(localPath);
    // 首次加载和显式路径变更由 loadLocalDirectory 控制，避免刷新时重复请求。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (!activeConnectionId) {
      remoteRequestIdRef.current += 1;
      setRemoteListing(null);
      setRemotePath(undefined);
      setRemoteError('');
      remoteCacheRef.current.clear();
      handledConnectionAttemptsRef.current.clear();
      return;
    }
    const previousAttempt = handledConnectionAttemptsRef.current.get(activeConnectionId);
    handledConnectionAttemptsRef.current.set(activeConnectionId, activeConnectionAttempt);
    // 用户再次执行“连接”时必须越过旧缓存；路径规范化导致的后续渲染仍可使用缓存。
    void loadRemoteDirectory(
      activeRemotePath,
      previousAttempt !== undefined && previousAttempt !== activeConnectionAttempt
    );
  }, [activeConnectionAttempt, activeConnectionId, activeRemotePath, loadRemoteDirectory]);

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
  };
}
