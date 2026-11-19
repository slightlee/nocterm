import { describe, expect, it, vi } from 'vitest';
import { fileParentPath, refreshSavedFile } from './file-save-refresh';

const actions = () => ({
  invalidateLocalPath: vi.fn(),
  invalidateRemotePath: vi.fn(),
  loadLocalDirectory: vi.fn().mockResolvedValue(undefined),
  loadRemoteDirectory: vi.fn().mockResolvedValue(undefined),
});
describe('directory refresh after file save', () => {
  it('invalidates A without starting loading on B with the same path', () => {
    const effects = actions();
    refreshSavedFile(
      { kind: 'remote', connectionId: 'A', path: '/srv/config' },
      { enabled: true, connectionId: 'B', remotePath: '/srv', localPath: '/tmp' },
      effects
    );
    expect(effects.invalidateRemotePath).toHaveBeenCalledWith('A', '/srv');
    expect(effects.loadRemoteDirectory).not.toHaveBeenCalled();
  });
  it('refreshes the current directory for local saves but only invalidates hidden pages', () => {
    const effects = actions();
    const source = { kind: 'local' as const, path: 'C:\\work\\note' };
    refreshSavedFile(source, { enabled: true, remotePath: '/', localPath: 'C:\\work' }, effects);
    expect(effects.loadLocalDirectory).toHaveBeenCalledWith('C:\\work', true);
    effects.loadLocalDirectory.mockClear();
    refreshSavedFile(source, { enabled: false, remotePath: '/', localPath: 'C:\\work' }, effects);
    expect(effects.invalidateLocalPath).toHaveBeenCalledTimes(2);
    expect(effects.loadLocalDirectory).not.toHaveBeenCalled();
  });
  it('preserves root directory separators', () => {
    expect(fileParentPath('/note')).toBe('/');
    expect(fileParentPath('C:\\note')).toBe('C:\\');
  });
});
