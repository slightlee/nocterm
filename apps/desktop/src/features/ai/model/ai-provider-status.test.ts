import { describe, expect, it } from 'vitest';

import { providerStatus } from './ai-provider-status';

describe('AI Provider status', () => {
  it('shows only actionable discovery failures', () => {
    expect(providerStatus({ error: 'not-installed' }, true)).toEqual({
      label: '未安装',
      tone: 'muted',
    });
    expect(providerStatus({ error: 'unavailable' }, true)).toEqual({
      label: '目录读取失败',
      tone: 'error',
    });
    expect(providerStatus({ access: 'loginRequired' }, true)).toEqual({
      label: '需要登录',
      tone: 'error',
    });
  });

  it('describes non-default routes without claiming they are authenticated', () => {
    expect(providerStatus({ access: 'customEndpoint' }, true)).toEqual({
      label: '自定义端点',
      tone: 'muted',
    });
    expect(providerStatus({ access: 'externalProvider' }, true)).toEqual({
      label: '云平台接入',
      tone: 'muted',
    });
  });

  it('omits normal, unknown, loading and browser-only states', () => {
    expect(providerStatus({ refreshedAt: Date.now() }, true)).toBeUndefined();
    expect(providerStatus({}, true)).toBeUndefined();
    expect(providerStatus({ access: 'customEndpoint' }, false)).toBeUndefined();
  });
});
