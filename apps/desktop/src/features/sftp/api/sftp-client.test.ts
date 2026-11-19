import { beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));
vi.mock('../../../shared/lib/tauri-runtime', () => ({ isDesktopRuntime: () => true }));

import {
  disconnectSftpSession,
  listLocalDir,
  listRemoteDir,
  readLocalFile,
  readRemoteFile,
  saveLocalFile,
  saveRemoteFile,
  type LocalDirectoryListing,
  type RemoteDirectoryListing,
} from './sftp-client';

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

  it('shares an in-flight local directory request and clears it after completion', async () => {
    const request = deferred<LocalDirectoryListing>();
    mocks.invoke.mockReturnValue(request.promise);

    const first = listLocalDir('~/projects');
    const duplicate = listLocalDir('~/projects');

    expect(duplicate).toBe(first);
    expect(mocks.invoke).toHaveBeenCalledTimes(1);

    request.resolve({ path: '/Users/ming/projects', parent: '/Users/ming', entries: [] });
    await first;

    mocks.invoke.mockResolvedValue({
      path: '/Users/ming/projects',
      parent: '/Users/ming',
      entries: [],
    });
    await listLocalDir('~/projects');
    expect(mocks.invoke).toHaveBeenCalledTimes(2);
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

  it('reads a remote file with a numeric connection id', async () => {
    mocks.invoke.mockResolvedValue({ content: 'hello', size: 5, modifiedAt: 1000 });

    const result = await readRemoteFile('7', '/etc/hosts');

    expect(result).toEqual({ content: 'hello', size: 5, modifiedAt: 1000 });
    expect(mocks.invoke).toHaveBeenCalledWith('sftp_read_remote_file', {
      connectionId: 7,
      remotePath: '/etc/hosts',
    });
  });

  it('saves a remote file passing the baseline as nullable stat fields', async () => {
    mocks.invoke.mockResolvedValue({ size: 8, modifiedAt: 2000 });

    const result = await saveRemoteFile('7', '/etc/hosts', 'new text', {
      size: 5,
      modifiedAt: 1000,
      targetRevision: 'target-1',
    });

    expect(result).toEqual({ size: 8, modifiedAt: 2000 });
    expect(mocks.invoke).toHaveBeenCalledWith('sftp_write_remote_file', {
      connectionId: 7,
      remotePath: '/etc/hosts',
      content: 'new text',
      baselineSize: 5,
      baselineModifiedAt: 1000,
      targetRevision: 'target-1',
      force: false,
    });
  });

  it('forces an overwrite save when the conflict dialog resolves to overwrite', async () => {
    mocks.invoke.mockResolvedValue({ size: 8, modifiedAt: 2000 });

    await saveRemoteFile(
      '7',
      '/etc/hosts',
      'new text',
      { size: null, modifiedAt: null, targetRevision: 'target-1' },
      true
    );

    expect(mocks.invoke).toHaveBeenCalledWith('sftp_write_remote_file', {
      connectionId: 7,
      remotePath: '/etc/hosts',
      content: 'new text',
      baselineSize: null,
      baselineModifiedAt: null,
      targetRevision: 'target-1',
      force: true,
    });
  });

  it('reads a local file and keeps the backend identity path', async () => {
    mocks.invoke.mockResolvedValue({
      content: 'hello',
      identityPath: '/Users/ming/notes.txt',
      baseline: { size: 5, modifiedSeconds: 1000, modifiedNanos: 500 },
    });

    const result = await readLocalFile('~/notes.txt');

    expect(result.identityPath).toBe('/Users/ming/notes.txt');
    expect(result.baseline.modifiedNanos).toBe(500);
    expect(mocks.invoke).toHaveBeenCalledWith('sftp_read_local_file', { path: '~/notes.txt' });
  });

  it('saves a local file passing precise seconds and nanos baseline fields', async () => {
    mocks.invoke.mockResolvedValue({ size: 9, modifiedSeconds: 2000, modifiedNanos: 250 });

    const result = await saveLocalFile(
      '/Users/ming/notes.txt',
      'new text\n',
      { size: 5, modifiedSeconds: 1000, modifiedNanos: 500 },
      false
    );

    expect(result).toEqual({ size: 9, modifiedSeconds: 2000, modifiedNanos: 250 });
    expect(mocks.invoke).toHaveBeenCalledWith('sftp_write_local_file', {
      path: '/Users/ming/notes.txt',
      content: 'new text\n',
      baselineSize: 5,
      baselineModifiedSeconds: 1000,
      baselineModifiedNanos: 500,
      force: false,
    });
  });
});
