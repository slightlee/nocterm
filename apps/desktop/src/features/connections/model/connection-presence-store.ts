import { create } from 'zustand';

interface ConnectionPresenceState {
  connectionCount: number;
  setConnectionCount: (connectionCount: number) => void;
}

/** 终端空态需要知道连接资料是否存在，但不应重复拥有连接列表数据。 */
export const useConnectionPresenceStore = create<ConnectionPresenceState>((set) => ({
  connectionCount: 0,
  setConnectionCount: (connectionCount) => set({ connectionCount: Math.max(0, connectionCount) }),
}));
