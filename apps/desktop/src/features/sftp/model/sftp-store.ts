import { create } from 'zustand';
import type { ConnectionProfile } from '../../connections';

type SftpSessionStatus = 'connecting' | 'connected' | 'error';

interface SftpSession {
  connectionId: string;
  connectionName: string;
  host: string;
  username: string;
  port: number;
  initialPath: string;
  currentPath: string;
  connectionAttempt: number;
  status: SftpSessionStatus;
  lastError: string | null;
}

interface SftpState {
  sessions: SftpSession[];
  activeId: string | null;
  runningTransfers: Record<string, string>;
  selectionSummary: {
    scope: 'local' | 'remote' | null;
    count: number;
    totalSize: number | null;
  };
  openSession: (connection: ConnectionProfile) => void;
  setCurrentPath: (connectionId: string, path: string) => void;
  setSessionStatus: (
    connectionId: string,
    status: SftpSessionStatus,
    lastError?: string | null
  ) => void;
  setSelectionSummary: (summary: SftpState['selectionSummary']) => void;
  setActive: (connectionId: string) => void;
  closeSession: (connectionId: string) => void;
  trackTransfer: (taskId: string, connectionId: string) => void;
  finishTransfer: (taskId: string) => void;
  hasRunningTransfers: (connectionIds: readonly string[]) => boolean;
}

function toSftpSession(connection: ConnectionProfile): SftpSession {
  return {
    connectionId: String(connection.id),
    connectionName: connection.name,
    host: connection.host,
    username: connection.username,
    port: connection.port,
    initialPath: connection.remoteInitialPath || '/',
    currentPath: connection.remoteInitialPath || '/',
    connectionAttempt: 1,
    status: 'connecting',
    lastError: null,
  };
}

export const useSftpStore = create<SftpState>()((set, get) => ({
  sessions: [],
  activeId: null,
  runningTransfers: {},
  selectionSummary: {
    scope: null,
    count: 0,
    totalSize: null,
  },

  openSession: (connection) => {
    const nextSession = toSftpSession(connection);
    const connectionId = String(connection.id);
    set((state) => {
      const exists = state.sessions.some((session) => session.connectionId === connectionId);

      return {
        activeId: connectionId,
        sessions: exists
          ? state.sessions.map((session) =>
              session.connectionId === connectionId
                ? {
                    ...session,
                    connectionName: nextSession.connectionName,
                    host: nextSession.host,
                    username: nextSession.username,
                    port: nextSession.port,
                    initialPath: nextSession.initialPath,
                    currentPath:
                      session.host !== nextSession.host ||
                      session.username !== nextSession.username ||
                      session.port !== nextSession.port
                        ? nextSession.initialPath
                        : session.currentPath,
                    connectionAttempt: session.connectionAttempt + 1,
                    status: 'connecting',
                    lastError: null,
                  }
                : session
            )
          : [...state.sessions, nextSession],
        selectionSummary: { scope: null, count: 0, totalSize: null },
      };
    });
  },

  setCurrentPath: (connectionId, path) => {
    set((state) => ({
      sessions: state.sessions.map((session) =>
        session.connectionId === connectionId ? { ...session, currentPath: path } : session
      ),
    }));
  },

  setSessionStatus: (connectionId, status, lastError = null) => {
    set((state) => ({
      sessions: state.sessions.map((session) =>
        session.connectionId === connectionId ? { ...session, status, lastError } : session
      ),
    }));
  },

  setSelectionSummary: (selectionSummary) => set({ selectionSummary }),

  setActive: (connectionId) => set({ activeId: connectionId }),

  closeSession: (connectionId) => {
    const { sessions, activeId } = get();
    const remaining = sessions.filter((session) => session.connectionId !== connectionId);
    const nextActiveId =
      activeId === connectionId
        ? (remaining[remaining.length - 1]?.connectionId ?? null)
        : activeId;

    set({
      sessions: remaining,
      activeId: nextActiveId,
      selectionSummary: { scope: null, count: 0, totalSize: null },
    });
  },

  // 传输状态跨路由保留，用户离开文件页后返回仍能获得安全关闭提示。
  trackTransfer: (taskId, connectionId) =>
    set((state) => ({
      runningTransfers: { ...state.runningTransfers, [taskId]: connectionId },
    })),

  finishTransfer: (taskId) =>
    set((state) => {
      if (!(taskId in state.runningTransfers)) return state;
      const runningTransfers = { ...state.runningTransfers };
      delete runningTransfers[taskId];
      return { runningTransfers };
    }),

  hasRunningTransfers: (connectionIds) => {
    const targets = new Set(connectionIds);
    return Object.values(get().runningTransfers).some((connectionId) => targets.has(connectionId));
  },
}));
