import { describe, expect, it } from 'vitest';

import {
  getFileKindForName,
  isRemoteChangedError,
  joinPath,
  nextSortState,
  remoteParentPath,
  selectRange,
  sortRows,
  toLocalRows,
  validateNameInput,
} from './sftp-view-model';

describe('sftp view model', () => {
  it('hides active random staging files while preserving recovery backups', () => {
    const names = [
      '.config.nocterm-0123456789abcdef0123456789abcdef.part',
      '.config.nocterm-0123456789abcdef0123456789abcdef.backup',
      'config',
    ];
    const rows = toLocalRows({
      path: '/tmp',
      parent: '/',
      entries: names.map((name) => ({
        name,
        path: `/tmp/${name}`,
        isDir: false,
        size: 3,
        modifiedAt: 1,
      })),
    });
    expect(rows.map((row) => row.name)).toEqual(names.slice(1));
  });
  it('classifies common editable and output file names consistently', () => {
    expect(getFileKindForName('deploy.sh')).toBe('code');
    expect(getFileKindForName('config.yaml')).toBe('code');
    expect(getFileKindForName('nohup.out')).toBe('log');
    expect(getFileKindForName('.env.production')).toBe('env');
    expect(getFileKindForName('archive.tar.gz')).toBe('archive');
  });

  it('joins root and nested remote paths without duplicate separators', () => {
    expect(joinPath('/', 'docs')).toBe('/docs');
    expect(joinPath('/var/data/', 'report.txt')).toBe('/var/data/report.txt');
  });

  it('cycles sort direction while defaulting numeric columns to descending', () => {
    expect(nextSortState({ key: 'name', direction: 'asc' }, 'size')).toEqual({
      key: 'size',
      direction: 'desc',
    });
    expect(nextSortState({ key: 'size', direction: 'desc' }, 'size')).toEqual({
      key: 'size',
      direction: 'asc',
    });
  });

  it('keeps directories first for name sorting', () => {
    const rows = [
      { name: 'z-file.txt', isDir: false },
      { name: 'a-folder', isDir: true },
      { name: 'b-file.txt', isDir: false },
    ].map((row) => ({
      ...row,
      kind: row.isDir ? ('folder' as const) : ('file' as const),
      size: '-',
      sizeBytes: row.isDir ? null : 1,
      modified: '-',
      modifiedAt: null,
    }));

    expect(sortRows(rows, { key: 'name', direction: 'asc' }).map((row) => row.name)).toEqual([
      'a-folder',
      'b-file.txt',
      'z-file.txt',
    ]);
  });

  it('sorts files and directories together by modification time', () => {
    const rows = [
      { name: 'old-folder', isDir: true, modifiedAt: 10 },
      { name: 'new-file.txt', isDir: false, modifiedAt: 30 },
      { name: 'middle-folder', isDir: true, modifiedAt: 20 },
    ].map((row) => ({
      ...row,
      kind: row.isDir ? ('folder' as const) : ('file' as const),
      size: '-',
      sizeBytes: row.isDir ? null : 1,
      modified: '-',
    }));

    expect(sortRows(rows, { key: 'modified', direction: 'desc' }).map((row) => row.name)).toEqual([
      'new-file.txt',
      'middle-folder',
      'old-folder',
    ]);
  });

  it('keeps entries without modification time after known entries in both directions', () => {
    const rows = [
      { name: 'unknown.txt', modifiedAt: null },
      { name: 'known.txt', modifiedAt: 10 },
    ].map((row) => ({
      ...row,
      kind: 'file' as const,
      size: '-',
      sizeBytes: 1,
      modified: '-',
    }));

    expect(sortRows(rows, { key: 'modified', direction: 'asc' }).map((row) => row.name)).toEqual([
      'known.txt',
      'unknown.txt',
    ]);
    expect(sortRows(rows, { key: 'modified', direction: 'desc' }).map((row) => row.name)).toEqual([
      'known.txt',
      'unknown.txt',
    ]);
  });

  it('selects an inclusive range and rejects an unknown anchor', () => {
    const rows = [{ name: 'a' }, { name: 'b' }, { name: 'c' }].map((row) => ({
      ...row,
      kind: 'file' as const,
      size: '-',
      sizeBytes: 1,
      modified: '-',
      modifiedAt: null,
    }));
    expect([...selectRange(rows, 'a', 'c')!]).toEqual(['a', 'b', 'c']);
    expect(selectRange(rows, 'missing', 'c')).toBeNull();
  });

  it('rejects path separators in names', () => {
    expect(validateNameInput(' folder ')).toEqual({ name: 'folder' });
    expect(validateNameInput('a/b').error).toContain('路径分隔符');
    expect(validateNameInput('..').error).toContain('相对路径');
    expect(validateNameInput('tab\tname').error).toContain('Tab');
  });

  it('resolves the remote parent path for nested and root-level files', () => {
    expect(remoteParentPath('/etc/nginx/nginx.conf')).toBe('/etc/nginx');
    expect(remoteParentPath('/a.txt')).toBe('/');
    expect(remoteParentPath('/etc/hosts/')).toBe('/etc');
    // 无父目录的裸路径与根目录本身没有父目录，返回 null 交由调用方拒绝。
    expect(remoteParentPath('hosts')).toBeNull();
    expect(remoteParentPath('/')).toBeNull();
  });

  it('recognizes the remote-changed conflict error by its stable code', () => {
    expect(isRemoteChangedError({ code: 'SFTP_REMOTE_CHANGED', message: 'x' })).toBe(true);
    expect(isRemoteChangedError({ code: 'SFTP_REMOTE_WRITE_FAILED', message: 'x' })).toBe(false);
    expect(isRemoteChangedError(new Error('boom'))).toBe(false);
    expect(isRemoteChangedError('SFTP_REMOTE_CHANGED')).toBe(false);
  });
});
