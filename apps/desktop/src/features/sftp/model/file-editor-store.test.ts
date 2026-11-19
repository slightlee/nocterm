import { afterEach, describe, expect, it } from 'vitest';

import {
  canChangeEditorContent,
  editorDirtyAfterSave,
  fileEditorId,
  fileEditorName,
  useFileEditorStore,
  type FileEditorDraft,
} from './file-editor-store';

const draft = (overrides: Partial<FileEditorDraft> = {}): FileEditorDraft => ({
  id: 'remote:1:/home/jar/deploy.sh',
  name: 'deploy.sh',
  source: { kind: 'remote', path: '/home/jar/deploy.sh', connectionId: '1' },
  content: '',
  savedContent: '',
  baseline: { size: null, mtimeSeconds: null, mtimeNanos: null },
  ...overrides,
});

afterEach(() => {
  useFileEditorStore.setState({ editors: [], activeEditorId: null, closePending: false });
});

describe('file editor store', () => {
  it('derives collision-free ids from source identity', () => {
    // 本地与远端、不同连接、不同目录的同名文件互不碰撞。
    expect(fileEditorId({ kind: 'local', path: '/etc/hosts' })).toBe('local:/etc/hosts');
    expect(fileEditorId({ kind: 'remote', path: '/etc/hosts', connectionId: '1' })).toBe(
      'remote:1:/etc/hosts'
    );
    expect(fileEditorId({ kind: 'remote', path: '/etc/hosts', connectionId: '2' })).not.toBe(
      fileEditorId({ kind: 'remote', path: '/etc/hosts', connectionId: '1' })
    );
    expect(fileEditorId({ kind: 'remote', path: '/var/hosts', connectionId: '1' })).not.toBe(
      fileEditorId({ kind: 'remote', path: '/etc/hosts', connectionId: '1' })
    );
  });

  it('extracts the file name for the tab label', () => {
    expect(fileEditorName('/home/jar/deploy.sh')).toBe('deploy.sh');
    expect(fileEditorName('C:\\work\\config.yml')).toBe('config.yml');
    expect(fileEditorName('/')).toBe('/');
  });

  it('reopens the same source by activating the existing tab', () => {
    const store = useFileEditorStore.getState();
    store.openEditor(draft());
    store.patchEditor(draft().id, { content: '未保存内容', dirty: true });

    // 同来源重复打开激活已有标签；未保存内容不被覆盖。
    store.openEditor(draft());
    const editors = useFileEditorStore.getState().editors;
    expect(editors).toHaveLength(1);
    expect(editors[0]).toMatchObject({ content: '未保存内容', dirty: true });
    expect(useFileEditorStore.getState().activeEditorId).toBe(draft().id);
  });

  it('patches only the target editor and keeps others untouched', () => {
    const store = useFileEditorStore.getState();
    store.openEditor(draft());
    store.openEditor(
      draft({
        id: 'local:/Users/ming/config.yml',
        name: 'config.yml',
        source: { kind: 'local', path: '/Users/ming/config.yml' },
      })
    );

    store.patchEditor('remote:1:/home/jar/deploy.sh', { status: 'saving' });

    const [remote, local] = useFileEditorStore.getState().editors;
    expect(remote.status).toBe('saving');
    expect(local.status).toBe('loading');
  });

  it('closes one editor and falls back to the last survivor', () => {
    const store = useFileEditorStore.getState();
    store.openEditor(draft());
    store.openEditor(
      draft({
        id: 'local:/tmp/a.txt',
        name: 'a.txt',
        source: { kind: 'local', path: '/tmp/a.txt' },
      })
    );
    expect(useFileEditorStore.getState().activeEditorId).toBe('local:/tmp/a.txt');

    store.closeEditor('local:/tmp/a.txt');

    // 激活标签被关闭后回退到最后的幸存编辑器。
    expect(useFileEditorStore.getState().activeEditorId).toBe('remote:1:/home/jar/deploy.sh');

    store.closeEditor('remote:1:/home/jar/deploy.sh');
    expect(useFileEditorStore.getState().activeEditorId).toBeNull();
    expect(useFileEditorStore.getState().editors).toHaveLength(0);
  });

  it('batch close keeps activation when the active editor survives', () => {
    const store = useFileEditorStore.getState();
    store.openEditor(draft());
    store.openEditor(
      draft({
        id: 'local:/tmp/a.txt',
        name: 'a.txt',
        source: { kind: 'local', path: '/tmp/a.txt' },
      })
    );

    useFileEditorStore.getState().closeEditors(['remote:1:/home/jar/deploy.sh']);

    expect(useFileEditorStore.getState().editors.map((editor) => editor.id)).toEqual([
      'local:/tmp/a.txt',
    ]);
    // 激活的本地编辑器不在关闭集合内，保持激活。
    expect(useFileEditorStore.getState().activeEditorId).toBe('local:/tmp/a.txt');
  });

  it('allows edits during save and preserves post-save dirty state', () => {
    expect(canChangeEditorContent('ready')).toBe(true);
    expect(canChangeEditorContent('saving')).toBe(true);
    expect(canChangeEditorContent('loading')).toBe(false);

    // 上传的是快照 A；保存期间用户输入 B，完成后 B 仍必须标记为未保存。
    expect(editorDirtyAfterSave('content B', 'content A')).toBe(true);
    expect(editorDirtyAfterSave('content A', 'content A')).toBe(false);
  });
});
