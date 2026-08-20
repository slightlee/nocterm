import { beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));
vi.mock('../../../shared/lib/tauri-runtime', () => ({ isDesktopRuntime: () => true }));

import { disconnectSftpSession, listRemoteDir, type RemoteDirectoryListing } from './sftp-client';

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

describe('SFTP client', () => {
  beforeEach(() => {
    mocks.invoke.mockReset();
  });

  it('shares an in-flight remote directory request and clears it after completion', async () => {
    const request = deferred<RemoteDirectoryListing>();
    mocks.invoke.mockReturnValue(request.promise);

    const first = listRemoteDir('7', '/srv');
    const duplicate = listRemoteDir('7', '/srv');

    expect(duplicate).toBe(first);
    expect(mocks.invoke).toHaveBeenCalledTimes(1);

    request.resolve({ path: '/srv', parent: '/', entries: [] });
    await first;

    mocks.invoke.mockResolvedValue({ path: '/srv', parent: '/', entries: [] });
    await listRemoteDir('7', '/srv');
    expect(mocks.invoke).toHaveBeenCalledTimes(2);
  });

  it('does not merge requests for different paths', async () => {
    mocks.invoke.mockResolvedValue({ path: '/srv', parent: '/', entries: [] });

    await Promise.all([listRemoteDir('7', '/srv'), listRemoteDir('7', '/var')]);

    expect(mocks.invoke).toHaveBeenCalledTimes(2);
  });

  it('disconnects the backend session with a numeric connection id', async () => {
    mocks.invoke.mockResolvedValue(undefined);

    await disconnectSftpSession('7');

    expect(mocks.invoke).toHaveBeenCalledWith('close_sftp_session', { connectionId: 7 });
  });
});
