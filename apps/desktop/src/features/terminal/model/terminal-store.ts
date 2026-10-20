import { create } from 'zustand';

import type { ConnectionProfile } from '../../connections';

export type TerminalStatus = 'idle' | 'connecting' | 'connected' | 'closed' | 'error';
export type TerminalSessionId = number | string;

export interface RemoteTerminalSession extends Omit<ConnectionProfile, 'id'> {
  id: string;
  connectionId: number;
  kind: 'remote';
  terminalNumber: number;
}

export interface LocalTerminalSession {
  id: string;
  kind: 'local';
  name: string;
}

export type TerminalSession = RemoteTerminalSession | LocalTerminalSession;

interface TerminalState {
  sessions: TerminalSession[];
  activeId: TerminalSessionId | null;
  /** 每台服务器按最近使用顺序记录终端，关闭最近标签后仍能回退到次近标签。 */
  recentRemoteSessionIds: Record<string, string[]>;
  /**
   * 按会话记录的重连计数，用于强制重挂载对应终端组件。
   *
   * 必须逐会话隔离而不能用一个全局计数：全局计数一旦大于 0，
   * "当前活动标签用计数、其余标签用 0"的 key 规则会让每次切换标签同时改变
   * 新旧两个标签的 key，React 于是把两个终端都卸载重建——正在跑的 SSH 会话被关闭，
   * 交互输入的口令因租约归还而重新追问。
   */
  reconnectNonces: Record<string, number>;
  statuses: Record<string, TerminalStatus>;
  errors: Record<string, string | null>;
  status: TerminalStatus;
  error: string | null;
  openConnection: (connection: ConnectionProfile) => void;
  activateRecentConnection: (connection: ConnectionProfile) => void;
  openLocalSession: (id: string, name: string) => void;
  activateConnection: (id: TerminalSessionId) => void;
  closeConnection: (id?: TerminalSessionId) => void;
  closeConnectionSessions: (connectionId: number) => void;
  syncConnectionName: (connectionId: number, name: string) => void;
  reconnectConnection: (id: TerminalSessionId) => void;
  markSessionConnected: (id: TerminalSessionId) => void;
  setSessionStatus: (id: TerminalSessionId, status: TerminalStatus, error?: string | null) => void;
  setStatus: (status: TerminalStatus, error?: string | null) => void;
}

/** 远程标签拥有独立 UI 会话 ID；数据库连接 ID 只用于后端 SSH/SFTP 调用。 */
function createRemoteSession(
  connection: ConnectionProfile,
  sessions: TerminalSession[]
): RemoteTerminalSession {
  const terminalNumber =
    Math.max(
      0,
      ...sessions
        .filter(
          (session): session is RemoteTerminalSession =>
            session.kind === 'remote' && session.connectionId === connection.id
        )
        .map((session) => session.terminalNumber)
    ) + 1;

  return {
    ...connection,
    id: `remote:${connection.id}:${crypto.randomUUID()}`,
    connectionId: connection.id,
    kind: 'remote',
    terminalNumber,
  };
}

function omitSessionKeys<T>(record: Record<string, T>, removedIds: Set<string>) {
  return Object.fromEntries(Object.entries(record).filter(([key]) => !removedIds.has(key)));
}

function findLastRemoteSession(sessions: TerminalSession[], connectionId: number) {
  for (let index = sessions.length - 1; index >= 0; index -= 1) {
    const session = sessions[index];
    if (session?.kind === 'remote' && session.connectionId === connectionId) return session;
  }
  return undefined;
}

function rememberRemoteSession(
  recentIds: Record<string, string[]>,
  connectionId: number,
  sessionId: string
) {
  const key = String(connectionId);
  return {
    ...recentIds,
    [key]: [sessionId, ...(recentIds[key] ?? []).filter((id) => id !== sessionId)],
  };
}

/** 会话选择与 xterm 实例分离，避免不可序列化终端对象进入全局状态。 */
export const useTerminalStore = create<TerminalState>((set) => ({
  sessions: [],
  activeId: null,
  recentRemoteSessionIds: {},
  reconnectNonces: {},
  statuses: {},
  errors: {},
  status: 'idle',
  error: null,
  openConnection: (connection) =>
    set((current) => {
      const session = createRemoteSession(connection, current.sessions);
      return {
        sessions: [...current.sessions, session],
        activeId: session.id,
        recentRemoteSessionIds: rememberRemoteSession(
          current.recentRemoteSessionIds,
          connection.id,
          session.id
        ),
        status: 'connecting' as const,
        error: null,
        statuses: { ...current.statuses, [session.id]: 'connecting' as const },
        errors: { ...current.errors, [session.id]: null },
      };
    }),
  activateRecentConnection: (connection) =>
    set((current) => {
      const recentIds = current.recentRemoteSessionIds[String(connection.id)] ?? [];
      const session =
        recentIds
          .map((id) => current.sessions.find((item) => item.id === id))
          .find((item) => item?.kind === 'remote' && item.connectionId === connection.id) ??
        findLastRemoteSession(current.sessions, connection.id);

      if (session?.kind === 'remote') {
        return {
          activeId: session.id,
          recentRemoteSessionIds: rememberRemoteSession(
            current.recentRemoteSessionIds,
            connection.id,
            session.id
          ),
          status: current.statuses[session.id] ?? 'connecting',
          error: current.errors[session.id] ?? null,
        };
      }

      const created = createRemoteSession(connection, current.sessions);
      return {
        sessions: [...current.sessions, created],
        activeId: created.id,
        recentRemoteSessionIds: rememberRemoteSession(
          current.recentRemoteSessionIds,
          connection.id,
          created.id
        ),
        status: 'connecting' as const,
        error: null,
        statuses: { ...current.statuses, [created.id]: 'connecting' as const },
        errors: { ...current.errors, [created.id]: null },
      };
    }),
  openLocalSession: (id, name) =>
    set((current) => {
      const exists = current.sessions.some((item) => item.id === id);
      const sessionStatus = current.statuses[id] ?? 'connecting';
      return {
        sessions: exists
          ? current.sessions
          : [...current.sessions, { id, kind: 'local' as const, name }],
        activeId: id,
        status: sessionStatus,
        error: current.errors[id] ?? null,
        statuses: exists ? current.statuses : { ...current.statuses, [id]: 'connecting' },
        errors: exists ? current.errors : { ...current.errors, [id]: null },
      };
    }),
  activateConnection: (id) =>
    set((current) => {
      const session = current.sessions.find((item) => item.id === id);
      return {
        activeId: id,
        recentRemoteSessionIds:
          session?.kind === 'remote'
            ? rememberRemoteSession(
                current.recentRemoteSessionIds,
                session.connectionId,
                session.id
              )
            : current.recentRemoteSessionIds,
        status: current.statuses[String(id)] ?? 'connecting',
        error: current.errors[String(id)] ?? null,
      };
    }),
  closeConnection: (id) =>
    set((current) => {
      const targetId = id ?? current.activeId;
      const removedSession = current.sessions.find((item) => item.id === targetId);
      const sessions = current.sessions.filter((item) => item.id !== targetId);
      // 关闭后台标签不得抢走当前会话；只有活动标签被关闭时才选择相邻剩余会话。
      const nextId =
        current.activeId === targetId ? (sessions.at(-1)?.id ?? null) : current.activeId;
      const removedIds = new Set([String(targetId)]);
      const statuses = omitSessionKeys(current.statuses, removedIds);
      const errors = omitSessionKeys(current.errors, removedIds);
      // 会话已从列表移除，其重连计数不再有对应组件，留着只会让映射无界增长。
      const reconnectNonces = omitSessionKeys(current.reconnectNonces, removedIds);
      const recentRemoteSessionIds = { ...current.recentRemoteSessionIds };
      if (removedSession?.kind === 'remote') {
        const key = String(removedSession.connectionId);
        const remainingRecentIds = (recentRemoteSessionIds[key] ?? []).filter(
          (sessionId) => sessionId !== removedSession.id
        );
        if (remainingRecentIds.length > 0) recentRemoteSessionIds[key] = remainingRecentIds;
        else delete recentRemoteSessionIds[key];
      }
      return {
        sessions,
        activeId: nextId,
        recentRemoteSessionIds,
        status: nextId === null ? 'idle' : (statuses[nextId] ?? 'connecting'),
        error: nextId === null ? null : (errors[nextId] ?? null),
        statuses,
        errors,
        reconnectNonces,
      };
    }),
  closeConnectionSessions: (connectionId) =>
    set((current) => {
      const removedIds = new Set(
        current.sessions
          .filter((session) => session.kind === 'remote' && session.connectionId === connectionId)
          .map((session) => String(session.id))
      );
      if (removedIds.size === 0) return current;

      const sessions = current.sessions.filter((session) => !removedIds.has(String(session.id)));
      const activeRemoved = current.activeId !== null && removedIds.has(String(current.activeId));
      const nextId = activeRemoved ? (sessions.at(-1)?.id ?? null) : current.activeId;
      const statuses = omitSessionKeys(current.statuses, removedIds);
      const errors = omitSessionKeys(current.errors, removedIds);
      const recentRemoteSessionIds = { ...current.recentRemoteSessionIds };
      delete recentRemoteSessionIds[String(connectionId)];

      return {
        sessions,
        activeId: nextId,
        recentRemoteSessionIds,
        status: nextId === null ? 'idle' : (statuses[String(nextId)] ?? 'connecting'),
        error: nextId === null ? null : (errors[String(nextId)] ?? null),
        statuses,
        errors,
        reconnectNonces: omitSessionKeys(current.reconnectNonces, removedIds),
      };
    }),
  syncConnectionName: (connectionId, name) =>
    set((current) => {
      const sessions = current.sessions.map((session) =>
        session.kind === 'remote' && session.connectionId === connectionId
          ? { ...session, name }
          : session
      );
      return { sessions };
    }),
  reconnectConnection: (id) =>
    set((current) => ({
      activeId: id,
      // 只递增目标会话的计数，其他标签的 key 保持不变，避免连带重挂载。
      reconnectNonces: {
        ...current.reconnectNonces,
        [String(id)]: (current.reconnectNonces[String(id)] ?? 0) + 1,
      },
      status: 'connecting',
      error: null,
      statuses: { ...current.statuses, [String(id)]: 'connecting' as TerminalStatus },
      errors: { ...current.errors, [String(id)]: null },
    })),
  // 退出事件可能在 open IPC 返回前抵达，不能再用迟到的成功响应覆盖失败状态。
  markSessionConnected: (id) =>
    set((current) => {
      const currentStatus = current.statuses[id];
      if (currentStatus === 'closed' || currentStatus === 'error') return current;
      return {
        statuses: { ...current.statuses, [String(id)]: 'connected' as TerminalStatus },
        errors: { ...current.errors, [String(id)]: null },
        ...(current.activeId === id ? { status: 'connected' as const, error: null } : {}),
      };
    }),
  setSessionStatus: (id, status, error = null) =>
    set((current) => ({
      statuses: { ...current.statuses, [id]: status },
      errors: { ...current.errors, [id]: error },
      ...(current.activeId === id ? { status, error } : {}),
    })),
  setStatus: (status, error = null) => set({ status, error }),
}));
