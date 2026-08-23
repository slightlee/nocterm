import { useEffect, useMemo, useState } from 'react';

import { ConnectionCreateDialog } from './ConnectionCreateDialog';
import { ConnectionBackupMenu } from './ConnectionBackupMenu';
import { ConnectionGroup } from './ConnectionGroup';
import { ConnectionItem } from './ConnectionItem';
import { ContextMenu } from '../../../shared/components/ContextMenu';
import { isDesktopRuntime } from '../../../shared/lib/tauri-runtime';
import {
  bindConnectionPrivateKeyFile,
  importConnectionBackup,
  normalizeConnectionError,
  pickConnectionBackup,
  pickSshConfig,
  resolveLocalUsername,
  resolveSshConfigPrivateKeyPath,
  saveConnectionBackup,
} from '../api/connection-client';
import { createConnectionBackup, parseConnectionBackup } from '../model/connection-backup';
import { useGroupDragSort } from '../model/use-group-drag-sort';
import { useConnectionDragSort } from '../model/use-connection-drag-sort';
import { useConnections } from '../model/useConnections';
import { parseSshConfigConnections } from '../model/ssh-config-import';
import {
  connectionRequiresReconnect,
  getNextGroupName,
  groupConnections,
  isConnectionReady,
  resolveGroupDropSortOrder,
  UNGROUPED_CONNECTION_GROUP_ID,
} from '../model/connection-list-model';
import type { ConnectionProfile } from '../types/connection-types';
import { useTerminalStore } from '../../terminal';
import styles from './ConnectionList.module.css';
import { onConnectionDialogRequested } from '../model/connection-events';
import { disconnectSftpSession, getSftpErrorMessage, useSftpStore } from '../../sftp';

const ACTION_FEEDBACK_TIMEOUT_MS = 5_000;

/** 连接侧栏沿用旧客户端的搜索、创建和列表布局。 */
export function ConnectionList({ mode = 'terminal' }: { mode?: 'terminal' | 'sftp' }) {
  const [query, setQuery] = useState('');
  const [dialogOpen, setDialogOpen] = useState(false);
  const [newGroupId, setNewGroupId] = useState<string | undefined>();
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [editing, setEditing] = useState<ConnectionProfile | null>(null);
  const [editingGroupId, setEditingGroupId] = useState<string | null>(null);
  const [collapsedGroups, setCollapsedGroups] = useState<Set<string>>(new Set());
  const [backupBusy, setBackupBusy] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const [actionMessage, setActionMessage] = useState<string | null>(null);
  const {
    connections,
    groups,
    loading,
    saving,
    error,
    create,
    update,
    remove,
    reorder,
    saveGroup,
    removeGroup,
    reload,
  } = useConnections();
  const openConnection = useTerminalStore((state) => state.openConnection);
  const activateConnection = useTerminalStore((state) => state.activateConnection);
  const sessions = useTerminalStore((state) => state.sessions);
  const terminalStatuses = useTerminalStore((state) => state.statuses);
  const terminalErrors = useTerminalStore((state) => state.errors);
  const closeConnection = useTerminalStore((state) => state.closeConnection);
  const sftpSessions = useSftpStore((state) => state.sessions);
  const openSftpSession = useSftpStore((state) => state.openSession);
  const activateSftpSession = useSftpStore((state) => state.setActive);
  const closeSftpSession = useSftpStore((state) => state.closeSession);
  const filtered = useMemo(() => {
    const normalized = query.trim().toLocaleLowerCase();
    if (!normalized) return connections;
    return connections.filter((connection) =>
      [connection.name, connection.host, connection.username].some((value) =>
        value.toLocaleLowerCase().includes(normalized)
      )
    );
  }, [connections, query]);

  const groupedConnections = useMemo(() => groupConnections(filtered, groups), [filtered, groups]);
  const sftpSessionsByConnectionId = useMemo(
    () => new Map(sftpSessions.map((session) => [session.connectionId, session])),
    [sftpSessions]
  );

  const createGroup = async () => {
    if (!requireDesktopRuntime()) return;
    clearActionFeedback();
    try {
      const saved = await saveGroup({
        id: `group-${crypto.randomUUID()}`,
        name: getNextGroupName(groups),
        sortOrder: groups.length,
      });
      setEditingGroupId(saved.id);
    } catch (groupError) {
      setActionError(`创建分组失败：${normalizeConnectionError(groupError).message}`);
    }
  };

  const renameGroup = async (groupId: string, nextName: string) => {
    const group = groups.find((item) => item.id === groupId);
    if (!group) return;
    const name = nextName.trim();
    if (!name || name === group.name) return;
    await saveGroup({ ...group, name });
    setEditingGroupId(null);
  };

  const requireDesktopRuntime = () => {
    if (isDesktopRuntime()) return true;
    setActionMessage(null);
    setActionError('请在 Nocterm 桌面客户端中使用备份与恢复');
    return false;
  };

  const clearActionFeedback = () => {
    setActionError(null);
    setActionMessage(null);
  };

  const disconnectSftpConnection = async (connectionId: number) => {
    clearActionFeedback();
    try {
      await disconnectSftpSession(String(connectionId));
      closeSftpSession(String(connectionId));
    } catch (disconnectError) {
      setActionError(`断开 SFTP 失败：${getSftpErrorMessage(disconnectError)}`);
    }
  };

  useEffect(() => {
    if (!actionError && !actionMessage) return;
    const timeout = window.setTimeout(() => {
      setActionError(null);
      setActionMessage(null);
    }, ACTION_FEEDBACK_TIMEOUT_MS);
    return () => window.clearTimeout(timeout);
  }, [actionError, actionMessage]);

  const handleExportConnections = async () => {
    if (!requireDesktopRuntime()) return;
    clearActionFeedback();
    setBackupBusy(true);
    try {
      const defaultPath = `nocterm-connections-${new Date().toISOString().slice(0, 10)}.json`;
      const saved = await saveConnectionBackup(
        createConnectionBackup(connections, groups),
        defaultPath
      );
      if (!saved) return;
      setActionMessage('连接配置已导出。敏感凭据不会写入备份文件。');
    } catch (backupError) {
      setActionError(`导出连接配置失败：${normalizeConnectionError(backupError).message}`);
    } finally {
      setBackupBusy(false);
    }
  };

  const handleImportConnections = async () => {
    if (!requireDesktopRuntime()) return;
    clearActionFeedback();
    setBackupBusy(true);
    try {
      const content = await pickConnectionBackup();
      if (!content) return;
      const result = await importConnectionBackup(parseConnectionBackup(content));
      await reload();
      setActionMessage(
        `已导入 ${result.connections} 个连接、${result.groups} 个分组。敏感凭据需要在本机重新绑定。`
      );
    } catch (backupError) {
      setActionError(`导入连接配置失败：${normalizeConnectionError(backupError).message}`);
    } finally {
      setBackupBusy(false);
    }
  };

  const handleImportSshConfig = async () => {
    if (!requireDesktopRuntime()) return;
    clearActionFeedback();
    setBackupBusy(true);
    try {
      const selected = await pickSshConfig();
      if (!selected) return;
      const group = groups.find((item) => item.name === 'SSH Config');
      const groupId = group?.id ?? `group-${crypto.randomUUID()}`;
      const imported = parseSshConfigConnections(
        selected.content,
        groupId,
        selected.path,
        await resolveLocalUsername()
      );
      if (imported.length === 0) throw new Error('未找到可导入的 Host 配置');

      const result = await importConnectionBackup({
        groups: group ? [] : [{ id: groupId, name: 'SSH Config', sortOrder: groups.length }],
        connections: imported.map((connection) => ({
          sourceId: connection.sourceId,
          name: connection.name,
          host: connection.host,
          port: connection.port,
          username: connection.username,
          authentication: connection.authentication,
          groupId: connection.groupId,
          remark: connection.remark,
          remoteInitialPath: connection.remoteInitialPath,
          icon: connection.icon,
          sortOrder: connection.sortOrder,
          credentialKind: connection.credentialKind,
          credentialStatus: connection.credentialStatus,
        })),
      });
      const importedIds = new Map(
        result.importedConnections.map((connection) => [connection.sourceId, connection.id])
      );
      let unboundPrivateKeys = 0;
      for (const connection of imported) {
        if (!connection.privateKeyPath) continue;
        const connectionId = importedIds.get(connection.sourceId);
        const privateKeyPath = await resolveSshConfigPrivateKeyPath(connection.privateKeyPath);
        if (connectionId === undefined || !privateKeyPath) {
          unboundPrivateKeys += 1;
          continue;
        }
        try {
          await bindConnectionPrivateKeyFile(connectionId, privateKeyPath);
        } catch {
          // 导入主体已由事务提交；单个本机私钥不可用时保留连接并提示用户手动绑定。
          unboundPrivateKeys += 1;
        }
      }
      await reload();
      setActionMessage(
        unboundPrivateKeys > 0
          ? `已从 SSH Config 导入 ${result.connections} 个连接，${unboundPrivateKeys} 个私钥需要手动绑定。`
          : `已从 SSH Config 导入 ${result.connections} 个连接。`
      );
    } catch (backupError) {
      setActionError(`导入 SSH Config 失败：${normalizeConnectionError(backupError).message}`);
    } finally {
      setBackupBusy(false);
    }
  };

  const cloneConnection = async (connection: ConnectionProfile) => {
    await create({
      name: `${connection.name} (副本)`,
      host: connection.host,
      port: connection.port,
      username: connection.username,
      authentication: connection.authentication,
      groupId: connection.groupId ?? undefined,
      remark: connection.remark ?? undefined,
      remoteInitialPath: connection.remoteInitialPath ?? undefined,
      icon: connection.icon ?? undefined,
    });
  };

  const openTerminalConnection = (connection: ConnectionProfile) => {
    setSelectedId(connection.id);
    if (!isConnectionReady(connection)) {
      setEditing(connection);
      setDialogOpen(true);
      return;
    }
    if (mode === 'sftp') openSftpSession(connection);
    else openConnection(connection);
  };

  const credentialNotice =
    editing && !isConnectionReady(editing)
      ? editing.authentication === 'private_key'
        ? '此连接尚未绑定私钥，请选择私钥文件并保存后再连接。'
        : 'SSH Agent 尚未提供可用身份，请确认代理已启动并已加载密钥。'
      : undefined;

  const moveGroup = async (
    sourceGroupId: string,
    targetGroupId: string,
    position: 'before' | 'after'
  ) => {
    const group = groups.find((item) => item.id === sourceGroupId);
    const sortOrder = resolveGroupDropSortOrder(groups, sourceGroupId, targetGroupId, position);
    if (!group || sortOrder === null) return;
    clearActionFeedback();
    await saveGroup({ ...group, sortOrder });
  };

  const {
    beginDrag: beginGroupDrag,
    draggingGroupId,
    dropTarget: groupDropTarget,
    shouldSuppressClick: shouldSuppressGroupClick,
  } = useGroupDragSort({
    onMove: moveGroup,
    onError: (groupError) => {
      setActionMessage(null);
      setActionError(`调整分组顺序失败：${normalizeConnectionError(groupError).message}`);
    },
  });

  const {
    beginDrag: beginConnectionDrag,
    draggingConnectionId,
    dropTarget: connectionDropTarget,
    shouldSuppressClick: shouldSuppressConnectionClick,
    visibleGroups,
  } = useConnectionDragSort({
    groups: groupedConnections,
    onMove: async (connectionId, groupId, sortOrder) => {
      await reorder(connectionId, groupId, sortOrder);
    },
    onError: (connectionError) => {
      setActionMessage(null);
      setActionError(`移动连接失败：${normalizeConnectionError(connectionError).message}`);
    },
  });

  useEffect(
    () =>
      onConnectionDialogRequested(() => {
        setEditing(null);
        setNewGroupId(undefined);
        setDialogOpen(true);
      }),
    []
  );

  return (
    <section className={styles.list}>
      <header className={styles.header}>
        <label className={styles.searchWrap}>
          <svg className={styles.searchIcon} viewBox="0 0 24 24" aria-hidden="true">
            <circle cx="11" cy="11" r="7" />
            <path d="m20 20-4-4" />
          </svg>
          <input
            aria-label="搜索连接"
            className={styles.searchInput}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="搜索名称、主机或用户"
            value={query}
          />
        </label>
        <button
          aria-label="新建远程连接"
          className={styles.addBtn}
          onClick={() => {
            setEditing(null);
            setNewGroupId(undefined);
            setDialogOpen(true);
          }}
          title="新建远程连接"
          type="button"
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path d="M12 5v14M5 12h14" />
          </svg>
        </button>
      </header>

      {error ? (
        <p className={styles.inlineError}>
          <span>{error.message}</span>
          {error.retryable ? (
            <button onClick={() => void reload()} type="button">
              重试
            </button>
          ) : null}
        </p>
      ) : null}

      {actionError ? <p className={styles.inlineError}>{actionError}</p> : null}
      {actionMessage ? <p className={styles.inlineMessage}>{actionMessage}</p> : null}

      <ContextMenu
        items={[
          {
            label: '新建分组',
            onSelect: () => void createGroup(),
            icon: (
              <svg viewBox="0 0 24 24" aria-hidden="true">
                <path d="M3 7.5A2.5 2.5 0 0 1 5.5 5H10l2 2.5h6.5A2.5 2.5 0 0 1 21 10v7a3 3 0 0 1-3 3h-13A2.5 2.5 0 0 1 3 17z" />
              </svg>
            ),
          },
          {
            label: '新建远程连接',
            onSelect: () => {
              setEditing(null);
              setNewGroupId(undefined);
              setDialogOpen(true);
            },
            icon: (
              <svg viewBox="0 0 24 24" aria-hidden="true">
                <rect x="5" y="4" width="14" height="16" rx="2" />
                <path d="M8 8h4M8 13h2" />
              </svg>
            ),
          },
        ]}
      >
        <div className={styles.scrollContent}>
          {loading ? (
            <div className={styles.empty}>
              <span className={styles.emptyText}>正在加载连接…</span>
            </div>
          ) : filtered.length === 0 && (Boolean(query) || groups.length === 0) ? (
            <div className={styles.empty}>
              <svg className={styles.emptyIcon} viewBox="0 0 24 24" aria-hidden="true">
                <path d="M10 13a5 5 0 0 0 7.5.5l3-3a5 5 0 0 0-7-7l-1.7 1.7" />
                <path d="M14 11a5 5 0 0 0-7.5-.5l-3 3a5 5 0 0 0 7 7l1.7-1.7" />
              </svg>
              <span className={styles.emptyText}>{query ? '没有匹配的连接' : '暂无连接'}</span>
              <span className={styles.emptyHint}>
                {query ? '尝试使用其他名称或主机地址' : '点击右上角 +，或从中间空态添加远程服务器'}
              </span>
            </div>
          ) : (
            visibleGroups.map((group) => (
              <ConnectionGroup
                canManage={group.id !== UNGROUPED_CONNECTION_GROUP_ID}
                count={group.connections.length}
                dropActive={
                  connectionDropTarget?.groupId === group.id &&
                  connectionDropTarget.position === 'end'
                }
                editing={editingGroupId === group.id}
                expanded={!collapsedGroups.has(group.id)}
                groupDragging={draggingGroupId === group.id}
                groupDropPosition={
                  groupDropTarget?.id === group.id ? groupDropTarget.position : undefined
                }
                id={group.id}
                key={group.id}
                name={group.name}
                onCreate={() => {
                  setEditing(null);
                  setNewGroupId(group.id);
                  setDialogOpen(true);
                }}
                onDelete={() => {
                  if (window.confirm(`删除分组“${group.name}”？连接会移动到未分组`)) {
                    void removeGroup(group.id);
                  }
                }}
                onGroupPointerDown={(event) => {
                  beginGroupDrag(group.id, event);
                }}
                onRename={(name) => void renameGroup(group.id, name)}
                onRenameCancel={() => setEditingGroupId(null)}
                onStartRename={() => setEditingGroupId(group.id)}
                onToggle={() => {
                  if (shouldSuppressGroupClick()) return;
                  setCollapsedGroups((current) => {
                    const next = new Set(current);
                    if (next.has(group.id)) next.delete(group.id);
                    else next.add(group.id);
                    return next;
                  });
                }}
              >
                {group.connections.map((connection) => (
                  <ConnectionItem
                    connection={connection}
                    key={connection.id}
                    onSelect={() => {
                      if (shouldSuppressConnectionClick()) return;
                      setSelectedId(connection.id);
                      if (mode === 'sftp') {
                        if (
                          sftpSessions.some(
                            (session) => session.connectionId === String(connection.id)
                          )
                        ) {
                          activateSftpSession(String(connection.id));
                        }
                      } else if (sessions.some((session) => session.id === connection.id)) {
                        activateConnection(connection.id);
                      }
                    }}
                    onOpen={() => openTerminalConnection(connection)}
                    onDisconnect={() => {
                      if (mode === 'sftp') {
                        void disconnectSftpConnection(connection.id);
                      } else {
                        closeConnection(connection.id);
                      }
                    }}
                    onEdit={() => {
                      setEditing(connection);
                      setDialogOpen(true);
                    }}
                    onCopyAddress={() => {
                      if (!navigator.clipboard) return;
                      void navigator.clipboard
                        .writeText(`${connection.host}:${connection.port}`)
                        .catch(() => undefined);
                    }}
                    onDelete={() => {
                      if (window.confirm(`确定删除连接“${connection.name}”吗？`)) {
                        void remove(connection.id)
                          .then(() => {
                            closeConnection(connection.id);
                            closeSftpSession(String(connection.id));
                          })
                          .catch(() => undefined);
                      }
                    }}
                    onClone={() => void cloneConnection(connection)}
                    connectionStatus={
                      mode === 'sftp'
                        ? sftpSessionsByConnectionId.get(String(connection.id))?.status ===
                          'connected'
                          ? 'connected'
                          : sftpSessionsByConnectionId.get(String(connection.id))?.status ===
                              'connecting'
                            ? 'connecting'
                            : sftpSessionsByConnectionId.get(String(connection.id))?.status ===
                                'error'
                              ? 'error'
                              : 'closed'
                        : terminalStatuses[connection.id]
                    }
                    connectionError={
                      mode === 'sftp'
                        ? sftpSessionsByConnectionId.get(String(connection.id))?.lastError
                        : terminalErrors[connection.id]
                    }
                    selected={selectedId === connection.id}
                    dragging={draggingConnectionId === connection.id}
                    dropPosition={
                      connectionDropTarget?.connectionId === connection.id
                        ? connectionDropTarget.position
                        : null
                    }
                    onDragPointerDown={(event) => beginConnectionDrag(connection.id, event)}
                  />
                ))}
              </ConnectionGroup>
            ))
          )}
        </div>
      </ContextMenu>

      <footer className={styles.footer}>
        <ConnectionBackupMenu
          busy={backupBusy}
          onExportConnections={() => void handleExportConnections()}
          onImportConnections={() => void handleImportConnections()}
          onImportSshConfig={() => void handleImportSshConfig()}
        />
      </footer>

      {dialogOpen ? (
        <ConnectionCreateDialog
          key={editing?.id ?? newGroupId ?? 'new'}
          onClose={() => {
            setEditing(null);
            setNewGroupId(undefined);
            setDialogOpen(false);
          }}
          onCreate={async (request) => {
            if (editing) {
              const requiresReconnect = connectionRequiresReconnect(editing, request);
              await update({ ...request, id: editing.id });
              if (requiresReconnect && sessions.some((session) => session.id === editing.id)) {
                closeConnection(editing.id);
              }
            } else await create(request);
            setEditing(null);
            setNewGroupId(undefined);
          }}
          editing={Boolean(editing)}
          initialValues={
            editing
              ? {
                  name: editing.name,
                  host: editing.host,
                  port: String(editing.port),
                  username: editing.username,
                  authentication: editing.authentication,
                  groupId: editing.groupId ?? undefined,
                  remoteInitialPath: editing.remoteInitialPath ?? undefined,
                  remark: editing.remark ?? undefined,
                  icon: editing.icon ?? undefined,
                  // 回填已绑定的私钥路径，否则编辑其它字段后保存会把绑定清空。
                  privateKeyPath: editing.privateKeyPath ?? undefined,
                }
              : { groupId: newGroupId }
          }
          saving={saving}
          groups={groups}
          credentialNotice={credentialNotice}
          credentialBound={editing?.credentialStatus === 'bound'}
        />
      ) : null}
    </section>
  );
}
