import { beforeEach, describe, expect, it, vi } from 'vitest';
import * as api from '../api/sftp-client';
import { createFileEditorController } from './file-editor-controller';
import { fileEditorId, useFileEditorStore, type FileEditorSource } from './file-editor-store';

vi.mock('../api/sftp-client', () => ({
  readLocalFile: vi.fn(),
  readRemoteFile: vi.fn(),
  saveLocalFile: vi.fn(),
  saveRemoteFile: vi.fn(),
}));
const source: FileEditorSource = { kind: 'remote', connectionId: 'A', path: '/srv/config' };
const baseline = { size: 3, modifiedAt: 1, targetRevision: 'revision-A' };
const hooks = { onFileSaved: vi.fn(), showToast: vi.fn() };
const controller = () => createFileEditorController(hooks);
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

beforeEach(() => {
  vi.resetAllMocks();
  useFileEditorStore.setState({ editors: [], activeEditorId: null, closePending: false });
  vi.mocked(api.readRemoteFile).mockResolvedValue({ content: 'old', ...baseline });
});

describe('file editor requests', () => {
  it('keeps edits made during save and notifies the original source', async () => {
    const pending = deferred<api.RemoteSaveResult>();
    vi.mocked(api.saveRemoteFile).mockReturnValue(pending.promise);
    const model = controller();
    await model.openFile(source);
    const id = fileEditorId(source);
    model.changeContent(id, 'first');
    const saving = model.saveFile(id);
    model.changeContent(id, 'second');
    pending.resolve({ ...baseline, size: 5, modifiedAt: 2 });
    await saving;
    expect(useFileEditorStore.getState().editors[0]).toMatchObject({
      content: 'second',
      savedContent: 'first',
      dirty: true,
      status: 'ready',
    });
    expect(hooks.onFileSaved).toHaveBeenCalledWith(source);
    expect(api.saveRemoteFile).toHaveBeenCalledWith('A', '/srv/config', 'first', baseline, false);
  });

  it('ignores a closed local request even when backend identity changes', async () => {
    const pending = deferred<api.LocalFileContent>();
    vi.mocked(api.readLocalFile).mockReturnValue(pending.promise);
    const model = controller();
    const local: FileEditorSource = { kind: 'local', path: '~/note' };
    const opening = model.openFile(local);
    useFileEditorStore.getState().closeEditors([fileEditorId(local)]);
    pending.resolve({
      content: 'late',
      identityPath: '/home/user/note',
      baseline: { size: 4, modifiedSeconds: 1, modifiedNanos: 0 },
    });
    await opening;
    expect(useFileEditorStore.getState().editors).toHaveLength(0);
  });

  it('does not let a late read replace a reopened file', async () => {
    const old = deferred<api.RemoteFileContent>();
    vi.mocked(api.readRemoteFile).mockReturnValueOnce(old.promise);
    const model = controller();
    const opening = model.openFile(source);
    useFileEditorStore.getState().closeEditors([fileEditorId(source)]);
    await model.openFile(source);
    model.changeContent(fileEditorId(source), 'draft');
    old.resolve({ content: 'stale', ...baseline });
    await opening;
    expect(useFileEditorStore.getState().editors[0].content).toBe('draft');
  });

  it('keeps content when the backend rejects a changed target and blocks forced retries', async () => {
    vi.mocked(api.saveRemoteFile).mockRejectedValue({ code: 'SFTP_REMOTE_TARGET_CHANGED' });
    const model = controller();
    await model.openFile(source);
    model.changeContent(fileEditorId(source), 'draft');
    await model.saveFile(fileEditorId(source));
    await model.saveFile(fileEditorId(source), true);
    expect(api.saveRemoteFile).toHaveBeenCalledTimes(1);
    expect(useFileEditorStore.getState().editors[0]).toMatchObject({
      content: 'draft',
      dirty: true,
      targetInvalidated: true,
    });
  });

  it('notifies local saves so the directory cache can be invalidated', async () => {
    const local: FileEditorSource = { kind: 'local', path: '/tmp/note' };
    const version = { size: 3, modifiedSeconds: 1, modifiedNanos: 0 };
    vi.mocked(api.readLocalFile).mockResolvedValue({
      content: 'old',
      identityPath: local.path,
      baseline: version,
    });
    vi.mocked(api.saveLocalFile).mockResolvedValue(version);
    const model = controller();
    await model.openFile(local);
    model.changeContent(fileEditorId(local), 'new');
    await model.saveFile(fileEditorId(local));
    expect(hooks.onFileSaved).toHaveBeenCalledWith(local);
  });
});
