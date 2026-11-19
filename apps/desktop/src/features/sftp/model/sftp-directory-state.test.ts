import { describe, expect, it } from 'vitest';

import { nextDirectoryLoading } from './sftp-directory-state';

describe('directory loading state', () => {
  it('clears loading when a newer request hits cached data', () => {
    // 回归：旧请求先进入 loading，新请求命中缓存；旧请求已过期，不会再负责 finally。
    const loading = nextDirectoryLoading(false, 'request-started');
    expect(loading).toBe(true);
    expect(nextDirectoryLoading(loading, 'cache-hit')).toBe(false);
  });

  it('clears loading when runtime or connection is unavailable', () => {
    expect(nextDirectoryLoading(true, 'request-unavailable')).toBe(false);
    expect(nextDirectoryLoading(true, 'current-request-settled')).toBe(false);
  });
});
