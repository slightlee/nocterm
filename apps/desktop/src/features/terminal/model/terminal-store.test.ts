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

function openRemote(connection: ConnectionProfile) {
  const store = useTerminalStore.getState();
  store.openConnection(connection);
  const session = useTerminalStore
    .getState()
    .sessions.slice()
    .reverse()
    .find((item) => item.kind === 'remote' && item.connectionId === connection.id);
  if (!session || session.kind !== 'remote') throw new Error('远程终端未创建');
  return session;
}

afterEach(() => {
  useTerminalStore.setState({
    sessions: [],
    activeId: null,
    recentRemoteSessionIds: {},
    reconnectNonces: {},
    statuses: {},
    errors: {},
    status: 'idle',
    error: null,
  });
});

describe('terminal session store', () => {
  it('creates independent remote sessions each time a server terminal is opened', () => {
    const connection = profile(1);
    const first = openRemote(connection);
    const second = openRemote(connection);
    useTerminalStore.getState().setSessionStatus(first.id, 'connected');
    useTerminalStore.getState().setSessionStatus(second.id, 'error', '连接失败');

    expect(first.id).not.toBe(second.id);
    expect([first.terminalNumber, second.terminalNumber]).toEqual([1, 2]);
    expect(useTerminalStore.getState().sessions).toHaveLength(2);
    expect(useTerminalStore.getState().statuses).toEqual({
      [first.id]: 'connected',
      [second.id]: 'error',
    });

    useTerminalStore.getState().closeConnection(first.id);
    expect(useTerminalStore.getState().sessions).toEqual([second]);
    expect(useTerminalStore.getState().statuses).toEqual({ [second.id]: 'error' });
  });

  it('returns to the most recently activated terminal for the selected server', () => {
    const connection = profile(1);
    const first = openRemote(connection);
    const second = openRemote(connection);
    useTerminalStore.getState().activateConnection(first.id);
    useTerminalStore.getState().activateRecentConnection(connection);

    expect(useTerminalStore.getState().activeId).toBe(first.id);
    expect(useTerminalStore.getState().sessions).toHaveLength(2);

    useTerminalStore.getState().activateConnection(second.id);
    useTerminalStore.getState().activateRecentConnection(connection);

    expect(useTerminalStore.getState().activeId).toBe(second.id);
    expect(useTerminalStore.getState().sessions).toHaveLength(2);
  });

  it('does not reactivate a closed terminal remembered for a server', () => {
    const connection = profile(1);
    const first = openRemote(connection);
    const second = openRemote(connection);
    useTerminalStore.getState().activateConnection(first.id);
    useTerminalStore.getState().closeConnection(first.id);
    useTerminalStore.getState().activateRecentConnection(connection);

    expect(useTerminalStore.getState().activeId).toBe(second.id);
    expect(useTerminalStore.getState().sessions).toEqual([second]);
  });

  it('creates the first terminal when activating a server without an open terminal', () => {
    const connection = profile(1);
    useTerminalStore.getState().activateRecentConnection(connection);
    const first = useTerminalStore.getState().sessions[0];
    useTerminalStore.getState().activateRecentConnection(connection);

    expect(first?.kind).toBe('remote');
    expect(useTerminalStore.getState().sessions).toHaveLength(1);
    expect(useTerminalStore.getState().activeId).toBe(first?.id);
  });

  it('falls back to the next most recently used terminal after one closes', () => {
    const connection = profile(1);
    const first = openRemote(connection);
    const second = openRemote(connection);
    useTerminalStore.getState().activateConnection(first.id);
    const otherServer = openRemote(profile(2));
    useTerminalStore.getState().closeConnection(first.id);
    useTerminalStore.getState().activateRecentConnection(connection);

    expect(useTerminalStore.getState().activeId).toBe(second.id);
    expect(useTerminalStore.getState().sessions).toEqual([second, otherServer]);
  });

  it('keeps status isolated when switching between different servers', () => {
    const first = openRemote(profile(1));
    const second = openRemote(profile(2));
    useTerminalStore.getState().setSessionStatus(first.id, 'connected');
    useTerminalStore.getState().setSessionStatus(second.id, 'error', '连接失败');

    useTerminalStore.getState().activateConnection(first.id);

    expect(useTerminalStore.getState().status).toBe('connected');
    expect(useTerminalStore.getState().statuses).toEqual({
      [first.id]: 'connected',
      [second.id]: 'error',
    });
  });

  it('removes closed session state when a tab closes', () => {
    const first = openRemote(profile(1));
    useTerminalStore.getState().setSessionStatus(first.id, 'closed');
    useTerminalStore.getState().closeConnection(first.id);

    expect(useTerminalStore.getState().sessions).toEqual([]);
    expect(useTerminalStore.getState().statuses).toEqual({});
    expect(useTerminalStore.getState().recentRemoteSessionIds).toEqual({});
    expect(useTerminalStore.getState().status).toBe('idle');
  });

  it('keeps the active session when a background tab closes', () => {
    const first = openRemote(profile(1));
    const second = openRemote(profile(2));
    useTerminalStore.getState().setSessionStatus(first.id, 'connected');
    useTerminalStore.getState().activateConnection(first.id);

    useTerminalStore.getState().closeConnection(second.id);

    expect(useTerminalStore.getState().activeId).toBe(first.id);
    expect(useTerminalStore.getState().status).toBe('connected');
  });

  it('closes every terminal for one server without affecting other servers', () => {
    const first = openRemote(profile(1));
    openRemote(profile(1));
    const otherServer = openRemote(profile(2));
    useTerminalStore.getState().closeConnectionSessions(1);

    expect(useTerminalStore.getState().sessions).toEqual([otherServer]);
    expect(useTerminalStore.getState().statuses).toEqual({ [otherServer.id]: 'connecting' });
    expect(useTerminalStore.getState().recentRemoteSessionIds).toEqual({
      '2': [otherServer.id],
    });
    expect(useTerminalStore.getState().sessions).not.toContain(first);
  });

  it('syncs a renamed connection across its open terminal labels', () => {
    openRemote(profile(1));
    openRemote(profile(1));
    const otherServer = openRemote(profile(2));

    useTerminalStore.getState().syncConnectionName(1, '生产服务器');

    expect(
      useTerminalStore
        .getState()
        .sessions.filter((session) => session.kind === 'remote' && session.connectionId === 1)
        .map((session) => session.name)
    ).toEqual(['生产服务器', '生产服务器']);
    expect(otherServer.name).toBe('连接 2');
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
    const first = openRemote(profile(1));
    useTerminalStore.getState().setSessionStatus(first.id, 'error', '连接被拒绝');

    useTerminalStore.getState().markSessionConnected(first.id);

    expect(useTerminalStore.getState().statuses[first.id]).toBe('error');
    expect(useTerminalStore.getState().errors[first.id]).toBe('连接被拒绝');
  });

  it('bumps only the reconnected session so other tabs keep their mount key', () => {
    const first = openRemote(profile(1));
    const second = openRemote(profile(2));

    useTerminalStore.getState().reconnectConnection(first.id);
    useTerminalStore.getState().activateConnection(second.id);

    expect(useTerminalStore.getState().reconnectNonces).toEqual({ [first.id]: 1 });
  });

  it('drops the reconnect counter together with the closed session', () => {
    const first = openRemote(profile(1));
    useTerminalStore.getState().reconnectConnection(first.id);
    useTerminalStore.getState().closeConnection(first.id);

    expect(useTerminalStore.getState().reconnectNonces).toEqual({});
  });
});
