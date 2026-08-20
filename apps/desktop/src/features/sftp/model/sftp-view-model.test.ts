import { describe, expect, it } from 'vitest';

import { joinPath, nextSortState, selectRange, validateNameInput } from './sftp-view-model';

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
