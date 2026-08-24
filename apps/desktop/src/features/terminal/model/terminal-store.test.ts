import { afterEach, describe, expect, it } from 'vitest';

import { useTerminalStore } from './terminal-store';
import type { ConnectionProfile } from '../../connections';

const profile = (id: number): ConnectionProfile => ({
  id,
  name: `连接 ${id}`,
  host: '127.0.0.1',
  port: 22,
  username: 'tester',
  authentication: 'ssh_agent',
  createdAt: 0,
  updatedAt: 0,
});

afterEach(() => {
  useTerminalStore.setState({
    sessions: [],
    activeId: null,
    reconnectNonces: {},
    statuses: {},
    errors: {},
    status: 'idle',
    error: null,
  });
});

describe('terminal session store', () => {
  it('keeps status isolated when switching sessions', () => {
    const first = profile(1);
    const second = profile(2);
    useTerminalStore.getState().openConnection(first);
    useTerminalStore.getState().setSessionStatus(first.id, 'connected');
    useTerminalStore.getState().openConnection(second);
    useTerminalStore.getState().setSessionStatus(second.id, 'error', '连接失败');

    useTerminalStore.getState().activateConnection(first.id);

    expect(useTerminalStore.getState().status).toBe('connected');
    expect(useTerminalStore.getState().statuses).toEqual({ 1: 'connected', 2: 'error' });
  });

  it('removes closed session state when a tab closes', () => {
    const first = profile(1);
    useTerminalStore.getState().openConnection(first);
    useTerminalStore.getState().setSessionStatus(first.id, 'closed');
    useTerminalStore.getState().closeConnection(first.id);

    expect(useTerminalStore.getState().sessions).toEqual([]);
    expect(useTerminalStore.getState().statuses).toEqual({});
    expect(useTerminalStore.getState().status).toBe('idle');
  });

  it('keeps the active session when a background tab closes', () => {
    const first = profile(1);
    const second = profile(2);
    useTerminalStore.getState().openConnection(first);
    useTerminalStore.getState().setSessionStatus(first.id, 'connected');
    useTerminalStore.getState().openConnection(second);
    useTerminalStore.getState().activateConnection(first.id);

    useTerminalStore.getState().closeConnection(second.id);

    expect(useTerminalStore.getState().activeId).toBe(first.id);
    expect(useTerminalStore.getState().status).toBe('connected');
  });

  it('activates an existing session without resetting its status', () => {
    const first = profile(1);
    useTerminalStore.getState().openConnection(first);
    useTerminalStore.getState().setSessionStatus(first.id, 'connected');
    useTerminalStore.getState().openConnection(first);

    expect(useTerminalStore.getState().sessions).toHaveLength(1);
    expect(useTerminalStore.getState().status).toBe('connected');
  });

  it('creates independent local terminal sessions', () => {
    useTerminalStore.getState().openLocalSession('local:default', '本地终端');
    useTerminalStore.getState().setSessionStatus('local:default', 'connected');
    useTerminalStore.getState().openLocalSession('local:extra:1', '本地终端 2');

    expect(useTerminalStore.getState().sessions).toEqual([
      { id: 'local:default', kind: 'local', name: '本地终端' },
      { id: 'local:extra:1', kind: 'local', name: '本地终端 2' },
    ]);
    expect(useTerminalStore.getState().activeId).toBe('local:extra:1');
    expect(useTerminalStore.getState().statuses).toEqual({
      'local:default': 'connected',
      'local:extra:1': 'connecting',
    });
  });

  it('does not overwrite an early terminal exit with a late open response', () => {
    const first = profile(1);
    useTerminalStore.getState().openConnection(first);
    useTerminalStore.getState().setSessionStatus(first.id, 'error', '连接被拒绝');

    useTerminalStore.getState().markSessionConnected(first.id);

    expect(useTerminalStore.getState().statuses[first.id]).toBe('error');
    expect(useTerminalStore.getState().errors[first.id]).toBe('连接被拒绝');
  });

  it('bumps only the reconnected session so other tabs keep their mount key', () => {
    const first = profile(1);
    const second = profile(2);
    useTerminalStore.getState().openConnection(first);
    useTerminalStore.getState().openConnection(second);

    useTerminalStore.getState().reconnectConnection(first.id);
    // 切回另一个标签不得改变任何计数，否则活着的会话会被 React 卸载重建。
    useTerminalStore.getState().activateConnection(second.id);

    expect(useTerminalStore.getState().reconnectNonces).toEqual({ 1: 1 });
  });

  it('drops the reconnect counter together with the closed session', () => {
    const first = profile(1);
    useTerminalStore.getState().openConnection(first);
    useTerminalStore.getState().reconnectConnection(first.id);
    useTerminalStore.getState().closeConnection(first.id);

    expect(useTerminalStore.getState().reconnectNonces).toEqual({});
  });
});
