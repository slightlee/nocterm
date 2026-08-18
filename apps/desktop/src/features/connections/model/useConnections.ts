import { useCallback, useEffect, useState } from 'react';

import { isDesktopRuntime } from '../../../shared/lib/tauri-runtime';
import { markCredentialBound, sortConnectionGroups } from './connection-list-model';
import { useConnectionPresenceStore } from './connection-presence-store';
import {
  createConnection as createConnectionRequest,
  deleteConnection as deleteConnectionRequest,
  listConnections,
  listConnectionGroups,
  normalizeConnectionError,
  reorderConnection,
  storeConnectionCredential,
  storeConnectionCredentialFile,
  upsertConnectionGroup,
  deleteConnectionGroup,
  updateConnection as updateConnectionRequest,
} from '../api/connection-client';
import type {
  AppError,
  ConnectionCreateRequest,
  ConnectionGroup,
  ConnectionProfile,
} from '../types/connection-types';

interface ConnectionState {
  connections: ConnectionProfile[];
  groups: ConnectionGroup[];
  loading: boolean;
  saving: boolean;
  error: AppError | null;
}

export function useConnections() {
  const setConnectionCount = useConnectionPresenceStore((state) => state.setConnectionCount);
  const [state, setState] = useState<ConnectionState>({
    connections: [],
    groups: [],
    loading: isDesktopRuntime(),
    saving: false,
    error: null,
  });

  const load = useCallback(async () => {
    // 浏览器预览只验证界面，不调用不存在的 Tauri IPC，也不伪造连接数据。
    if (!isDesktopRuntime()) return;
    setState((current) => ({ ...current, loading: true, error: null }));
    try {
      const [connections, groups] = await Promise.all([listConnections(), listConnectionGroups()]);
      setState((current) => ({ ...current, connections, groups, loading: false }));
      setConnectionCount(connections.length);
    } catch (error) {
      setState((current) => ({
        ...current,
        loading: false,
        error: normalizeConnectionError(error),
      }));
    }
  }, [setConnectionCount]);

  useEffect(() => {
    void load();
  }, [load]);

  const create = useCallback(async (request: ConnectionCreateRequest) => {
    if (!isDesktopRuntime()) {
      throw {
        code: 'DESKTOP_RUNTIME_REQUIRED',
        message: '请在 Nocterm 桌面客户端中保存连接',
        retryable: false,
      } satisfies AppError;
    }

    setState((current) => ({ ...current, saving: true, error: null }));
    let createdId: number | undefined;
    try {
      const created = await createConnectionRequest(request);
      createdId = created.id;
      const credentialKind =
        request.authentication === 'password'
          ? 'password'
          : request.authentication === 'private_key'
            ? 'private_key'
            : null;
      const secret = credentialKind === 'password' ? request.password : request.privateKey;
      if (credentialKind && secret) {
        await storeConnectionCredential(created.id, credentialKind, secret);
      } else if (created && request.privateKeyPath) {
        await storeConnectionCredentialFile(created.id, 'private_key', request.privateKeyPath);
      }
      const saved =
        (credentialKind && secret) || request.privateKeyPath
          ? markCredentialBound(created, credentialKind ?? 'private_key')
          : created;
      // 成功后直接使用后端返回对象更新列表，避免额外读取和时间窗口。
      setState((current) => ({
        ...current,
        connections: [saved, ...current.connections],
        saving: false,
      }));
      const presence = useConnectionPresenceStore.getState();
      presence.setConnectionCount(presence.connectionCount + 1);
      return saved;
    } catch (error) {
      if (createdId !== undefined) {
        // 凭据写入失败时回滚刚创建的连接，避免留下无法使用的半成品资料。
        await deleteConnectionRequest(createdId).catch(() => undefined);
      }
      const normalized = normalizeConnectionError(error);
      setState((current) => ({ ...current, saving: false, error: normalized }));
      throw normalized;
    }
  }, []);

  const update = useCallback(async (request: ConnectionCreateRequest) => {
    if (!isDesktopRuntime() || request.id === undefined) {
      throw {
        code: 'DESKTOP_RUNTIME_REQUIRED',
        message: '请在桌面客户端中更新连接',
        retryable: false,
      } satisfies AppError;
    }
    try {
      const updated = await updateConnectionRequest(request);
      const credentialKind =
        request.authentication === 'password'
          ? 'password'
          : request.authentication === 'private_key'
            ? 'private_key'
            : null;
      const secret = credentialKind === 'password' ? request.password : request.privateKey;
      const saved =
        (credentialKind && secret) || request.privateKeyPath
          ? markCredentialBound(updated, credentialKind ?? 'private_key')
          : updated;
      setState((current) => ({
        ...current,
        connections: current.connections.map((item) => (item.id === saved.id ? saved : item)),
      }));
      return saved;
    } catch (error) {
      const normalized = normalizeConnectionError(error);
      setState((current) => ({ ...current, error: normalized }));
      throw normalized;
    }
  }, []);

  const remove = useCallback(async (id: number) => {
    if (!isDesktopRuntime()) return;
    try {
      await deleteConnectionRequest(id);
      setState((current) => ({
        ...current,
        connections: current.connections.filter((item) => item.id !== id),
      }));
      const presence = useConnectionPresenceStore.getState();
      presence.setConnectionCount(presence.connectionCount - 1);
    } catch (error) {
      const normalized = normalizeConnectionError(error);
      setState((current) => ({ ...current, error: normalized }));
      throw normalized;
    }
  }, []);

  const reorder = useCallback(async (id: number, groupId: string | null, sortOrder: number) => {
    try {
      const updated = await reorderConnection(id, groupId, sortOrder);
      setState((current) => ({
        ...current,
        connections: current.connections.map((item) => (item.id === updated.id ? updated : item)),
      }));
      return updated;
    } catch (error) {
      const normalized = normalizeConnectionError(error);
      setState((current) => ({ ...current, error: normalized }));
      throw normalized;
    }
  }, []);

  const saveGroup = useCallback(async (group: ConnectionGroup) => {
    try {
      const saved = await upsertConnectionGroup(group);
      setState((current) => ({
        ...current,
        groups: sortConnectionGroups([
          ...current.groups.filter((item) => item.id !== saved.id),
          saved,
        ]),
      }));
      return saved;
    } catch (error) {
      const normalized = normalizeConnectionError(error);
      setState((current) => ({ ...current, error: normalized }));
      throw normalized;
    }
  }, []);

  const removeGroup = useCallback(async (id: string) => {
    try {
      await deleteConnectionGroup(id);
      setState((current) => ({
        ...current,
        groups: current.groups.filter((item) => item.id !== id),
        connections: current.connections.map((item) =>
          item.groupId === id ? { ...item, groupId: null, groupName: null } : item
        ),
      }));
    } catch (error) {
      const normalized = normalizeConnectionError(error);
      setState((current) => ({ ...current, error: normalized }));
      throw normalized;
    }
  }, []);

  return {
    ...state,
    create,
    update,
    remove,
    reorder,
    saveGroup,
    removeGroup,
    reload: load,
    desktopAvailable: isDesktopRuntime(),
  };
}
