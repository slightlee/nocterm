import { create } from 'zustand';

import type { ConnectionProfile } from '../../connections';

export type TerminalStatus = 'idle' | 'connecting' | 'connected' | 'closed' | 'error';
export type TerminalSessionId = number | string;

export interface RemoteTerminalSession extends ConnectionProfile {
  kind: 'remote';
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
  reconnectNonce: number;
  statuses: Record<string, TerminalStatus>;
  errors: Record<string, string | null>;
  status: TerminalStatus;
  error: string | null;
  openConnection: (connection: ConnectionProfile) => void;
  openLocalSession: (id: string, name: string) => void;
  activateConnection: (id: TerminalSessionId) => void;
  closeConnection: (id?: TerminalSessionId) => void;
  reconnectConnection: (id: TerminalSessionId) => void;
  markSessionConnected: (id: TerminalSessionId) => void;
  setSessionStatus: (id: TerminalSessionId, status: TerminalStatus, error?: string | null) => void;
  setStatus: (status: TerminalStatus, error?: string | null) => void;
}

/** 会话选择与 xterm 实例分离，避免不可序列化终端对象进入全局状态。 */
export const useTerminalStore = create<TerminalState>((set) => ({
  sessions: [],
  activeId: null,
  reconnectNonce: 0,
  statuses: {},
  errors: {},
  status: 'idle',
  error: null,
  openConnection: (connection) =>
    set((current) => {
      const exists = current.sessions.some((item) => item.id === connection.id);
      const sessionStatus = current.statuses[connection.id] ?? 'connecting';
      return {
        sessions: exists
          ? current.sessions
          : [...current.sessions, { ...connection, kind: 'remote' as const }],
        activeId: connection.id,
        status: sessionStatus,
        error: current.errors[connection.id] ?? null,
        statuses: exists
          ? current.statuses
          : { ...current.statuses, [connection.id]: 'connecting' },
        errors: exists ? current.errors : { ...current.errors, [connection.id]: null },
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
    set((current) => ({
      activeId: id,
      status: current.statuses[id] ?? 'connecting',
      error: current.errors[id] ?? null,
    })),
  closeConnection: (id) =>
    set((current) => {
      const targetId = id ?? current.activeId;
      const sessions = current.sessions.filter((item) => item.id !== targetId);
      const nextId = sessions.at(-1)?.id ?? null;
      const statuses = Object.fromEntries(
        Object.entries(current.statuses).filter(([key]) => key !== String(targetId))
      );
      const errors = Object.fromEntries(
        Object.entries(current.errors).filter(([key]) => key !== String(targetId))
      );
      return {
        sessions,
        activeId: nextId,
        status: nextId === null ? 'idle' : (statuses[nextId] ?? 'connecting'),
        error: nextId === null ? null : (errors[nextId] ?? null),
        statuses,
        errors,
      };
    }),
  reconnectConnection: (id) =>
    set((current) => ({
      activeId: id,
      reconnectNonce: current.reconnectNonce + 1,
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
