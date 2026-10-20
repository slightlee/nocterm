import { useEffect, useMemo, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import { ConnectionCreateDialog } from './ConnectionCreateDialog';
import { ConnectionGroup } from './ConnectionGroup';
import { ConnectionItem } from './ConnectionItem';
import { ContextMenu } from '../../../shared/components/ContextMenu';
import { isDesktopRuntime } from '../../../shared/lib/tauri-runtime';
import { normalizeConnectionError } from '../api/connection-client';
import { useGroupDragSort } from '../model/use-group-drag-sort';
import { useConnectionDragSort } from '../model/use-connection-drag-sort';
import { useConnections } from '../model/useConnections';
import {
  connectionRequiresReconnect,
  getNextGroupName,
  groupConnections,
  isConnectionReady,
  resolveGroupDropSortOrder,
  UNGROUPED_CONNECTION_GROUP_ID,
} from '../model/connection-list-model';
import type { ConnectionProfile } from '../types/connection-types';
import { useTerminalStore, type TerminalSession, type TerminalSessionId } from '../../terminal';
import styles from './ConnectionList.module.css';
import { onConnectionDialogRequested } from '../model/connection-events';
import { useSftpStore } from '../../sftp';

const ACTION_FEEDBACK_TIMEOUT_MS = 5_000;
type RemoteTerminalSession = Extract<TerminalSession, { kind: 'remote' }>;

/** 连接侧栏集中承载本地终端入口、服务器检索和远程资源管理。 */
export function ConnectionList({ mode = 'terminal' }: { mode?: 'terminal' | 'sftp' }) {
  const navigate = useNavigate();
  const [query, setQuery] = useState('');
  const [dialogOpen, setDialogOpen] = useState(false);
  const [newGroupId, setNewGroupId] = useState<string | undefined>();
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [editing, setEditing] = useState<ConnectionProfile | null>(null);
  const [editingGroupId, setEditingGroupId] = useState<string | null>(null);
  const [collapsedGroups, setCollapsedGroups] = useState<Set<string>>(new Set());
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
  const openLocalSession = useTerminalStore((state) => state.openLocalSession);
  const activateRecentConnection = useTerminalStore((state) => state.activateRecentConnection);
  const activateTerminalSession = useTerminalStore((state) => state.activateConnection);
  const sessions = useTerminalStore((state) => state.sessions);
  const activeTerminalId = useTerminalStore((state) => state.activeId);
  const terminalStatuses = useTerminalStore((state) => state.statuses);
  const terminalErrors = useTerminalStore((state) => state.errors);
  const closeConnectionSessions = useTerminalStore((state) => state.closeConnectionSessions);
  const syncTerminalConnectionName = useTerminalStore((state) => state.syncConnectionName);
  const sftpSessions = useSftpStore((state) => state.sessions);
  const activeSftpId = useSftpStore((state) => state.activeId);
  const openSftpSession = useSftpStore((state) => state.openSession);
  const activateSftpSession = useSftpStore((state) => state.setActive);
  const syncSftpConnectionName = useSftpStore((state) => state.syncConnectionName);
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
  const terminalSessionsByConnectionId = useMemo(() => {
    const grouped = new Map<number, RemoteTerminalSession[]>();
    for (const session of sessions) {
      if (session.kind !== 'remote') continue;
      const connectionSessions = grouped.get(session.connectionId) ?? [];
      connectionSessions.push(session);
      grouped.set(session.connectionId, connectionSessions);
    }
    return grouped;
  }, [sessions]);

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
    setActionError('请在 Nocterm 桌面客户端中管理服务器连接');
    return false;
  };

  const clearActionFeedback = () => {
    setActionError(null);
    setActionMessage(null);
  };

  useEffect(() => {
    if (!actionError && !actionMessage) return;
    const timeout = window.setTimeout(() => {
      setActionError(null);
      setActionMessage(null);
    }, ACTION_FEEDBACK_TIMEOUT_MS);
    return () => window.clearTimeout(timeout);
  }, [actionError, actionMessage]);

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

  const prepareConnection = (connection: ConnectionProfile) => {
    setSelectedId(connection.id);
    if (!isConnectionReady(connection)) {
      setEditing(connection);
      setDialogOpen(true);
      return false;
    }
    return true;
  };

  const activateServer = (connection: ConnectionProfile) => {
    if (!prepareConnection(connection)) return;
    activateRecentConnection(connection);
    navigate('/connections');
  };

  const openTerminalShortcut = (connection: ConnectionProfile) => {
    if (!prepareConnection(connection)) return;
    openConnection(connection);
    navigate('/connections');
  };

  const openFilesShortcut = (connection: ConnectionProfile) => {
    if (!prepareConnection(connection)) return;
    const connectionId = String(connection.id);
    // 切回已有文件会话时保留当前目录、筛选和传输上下文。
    if (sftpSessionsByConnectionId.has(connectionId)) activateSftpSession(connectionId);
    else openSftpSession(connection);
    navigate('/sftp');
  };

  const createLocalTerminal = () => {
    const localCount = sessions.filter((session) => session.kind === 'local').length;
    const name = localCount === 0 ? '本地终端' : `本地终端 ${localCount + 1}`;
    openLocalSession(`local:extra:${crypto.randomUUID()}`, name);
    navigate('/connections');
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
            aria-label="搜索服务器"
            className={styles.searchInput}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="搜索服务器…"
            value={query}
          />
        </label>
      </header>

      <div className={styles.localActions}>
        <button
          className={styles.localTerminalButton}
          onClick={createLocalTerminal}
          title="直接新建一个本地终端"
          type="button"
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path d="m4 7 5 5-5 5M11 17h5M19 4v7M15.5 7.5h7" />
          </svg>
          <span>新建本地终端</span>
        </button>
      </div>

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
          <div className={styles.serverHeading}>
            <span>服务器</span>
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
          </div>
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
                {query
                  ? '尝试使用其他名称或主机地址'
                  : '点击“服务器”右侧 +，或从中间空态添加远程服务器'}
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
                    }}
                    onOpen={() => activateServer(connection)}
                    onOpenTerminal={() => openTerminalShortcut(connection)}
                    onActivateTerminalSession={(sessionId: TerminalSessionId) => {
                      activateTerminalSession(sessionId);
                      navigate('/connections');
                    }}
                    onOpenFiles={() => openFilesShortcut(connection)}
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
                            closeConnectionSessions(connection.id);
                            closeSftpSession(String(connection.id));
                          })
                          .catch(() => undefined);
                      }
                    }}
                    onClone={() => void cloneConnection(connection)}
                    connectionStatus={(() => {
                      if (mode === 'sftp') {
                        const status = sftpSessionsByConnectionId.get(
                          String(connection.id)
                        )?.status;
                        return status === 'connected' ||
                          status === 'connecting' ||
                          status === 'error'
                          ? status
                          : 'closed';
                      }
                      const remoteSessions =
                        terminalSessionsByConnectionId.get(connection.id) ?? [];
                      const states = remoteSessions.map(
                        (session) => terminalStatuses[session.id] ?? 'connecting'
                      );
                      if (states.includes('connected')) return 'connected';
                      if (states.includes('connecting')) return 'connecting';
                      if (states.includes('error')) return 'error';
                      return states.length > 0 ? 'closed' : undefined;
                    })()}
                    connectionError={(() => {
                      if (mode === 'sftp') {
                        return sftpSessionsByConnectionId.get(String(connection.id))?.lastError;
                      }
                      const failedSessions = (
                        terminalSessionsByConnectionId.get(connection.id) ?? []
                      ).filter((session) => terminalStatuses[session.id] === 'error');
                      const failedSession = failedSessions[failedSessions.length - 1];
                      return failedSession ? (terminalErrors[failedSession.id] ?? null) : null;
                    })()}
                    selected={selectedId === connection.id}
                    terminalActive={
                      mode === 'terminal' &&
                      (terminalSessionsByConnectionId.get(connection.id) ?? []).some(
                        (session) => session.id === activeTerminalId
                      )
                    }
                    terminalSessions={terminalSessionsByConnectionId.get(connection.id) ?? []}
                    activeTerminalId={activeTerminalId}
                    terminalStatuses={terminalStatuses}
                    terminalErrors={terminalErrors}
                    fileSession={sftpSessionsByConnectionId.get(String(connection.id)) ?? null}
                    filesActive={mode === 'sftp' && activeSftpId === String(connection.id)}
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
              const saved = await update({ ...request, id: editing.id });
              syncTerminalConnectionName(saved.id, saved.name);
              syncSftpConnectionName(String(saved.id), saved.name);
              if (
                requiresReconnect &&
                sessions.some(
                  (session) => session.kind === 'remote' && session.connectionId === editing.id
                )
              ) {
                closeConnectionSessions(editing.id);
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
