import { describe, expect, it } from 'vitest';

import {
  joinPath,
  nextSortState,
  selectRange,
  sortRows,
  validateNameInput,
} from './sftp-view-model';

describe('sftp view model', () => {
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
});
