import { afterEach, describe, expect, it } from 'vitest';

import type { ConnectionProfile } from '../../connections';
import { useSftpStore } from './sftp-store';

const profile = (overrides: Partial<ConnectionProfile> = {}): ConnectionProfile => ({
  id: 1,
  name: '生产服务器',
  host: 'old.example.com',
  port: 22,
  username: 'tester',
  authentication: 'ssh_agent',
  createdAt: 0,
  updatedAt: 0,
  remoteInitialPath: '/srv/old',
  ...overrides,
});

afterEach(() => {
  useSftpStore.setState({
    sessions: [],
    activeId: null,
    runningTransfers: {},
    selectionSummary: { scope: null, count: 0, totalSize: null },
  });
});

describe('sftp session store', () => {
  it('refreshes connection metadata and resets the path when the endpoint changes', () => {
    useSftpStore.getState().openSession(profile());
    useSftpStore.getState().setCurrentPath('1', '/srv/old/releases');

    useSftpStore
      .getState()
      .openSession(
        profile({ host: 'new.example.com', remoteInitialPath: '/srv/new', updatedAt: 1 })
      );

    expect(useSftpStore.getState().sessions[0]).toMatchObject({
      host: 'new.example.com',
      initialPath: '/srv/new',
      currentPath: '/srv/new',
    });
  });

  it('preserves the current directory when reopening an unchanged endpoint', () => {
    const connection = profile();
    useSftpStore.getState().openSession(connection);
    useSftpStore.getState().setCurrentPath('1', '/srv/old/releases');

    useSftpStore.getState().openSession({ ...connection, name: '重命名后的服务器' });

    expect(useSftpStore.getState().sessions[0]).toMatchObject({
      connectionName: '重命名后的服务器',
      currentPath: '/srv/old/releases',
      connectionAttempt: 2,
    });
  });

  it('creates a new connection attempt when retrying a failed session', () => {
    const connection = profile();
    useSftpStore.getState().openSession(connection);
    useSftpStore.getState().setSessionStatus('1', 'error', '用户拒绝读取钥匙串');

    useSftpStore.getState().openSession(connection);

    expect(useSftpStore.getState().sessions[0]).toMatchObject({
      connectionAttempt: 2,
      status: 'connecting',
      lastError: null,
    });
  });

  it('tracks running transfers by connection until they finish', () => {
    const store = useSftpStore.getState();
    store.trackTransfer('task-1', '1');
    store.trackTransfer('task-2', '2');

    expect(useSftpStore.getState().hasRunningTransfers(['1'])).toBe(true);
    expect(useSftpStore.getState().hasRunningTransfers(['3'])).toBe(false);

    useSftpStore.getState().finishTransfer('task-1');

    expect(useSftpStore.getState().hasRunningTransfers(['1'])).toBe(false);
    expect(useSftpStore.getState().hasRunningTransfers(['2'])).toBe(true);
  });
});
