import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { parseModelCatalog } from '../model/ai-model-catalog';

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));

describe('Provider 模型目录缓存', () => {
  beforeEach(() => {
    vi.resetModules();
    mocks.invoke.mockReset();
    vi.useFakeTimers();
    const storage = new Map<string, string>();
    vi.stubGlobal('window', {
      localStorage: {
        getItem: (key: string) => storage.get(key) ?? null,
        setItem: (key: string, value: string) => storage.set(key, value),
        removeItem: (key: string) => storage.delete(key),
      },
    });
  });
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  it('同一 Agent 的视图同步收到成功及失败快照，缓存命中和取消订阅不重复通知', async () => {
    const { subscribeModelCatalog, refreshModelCatalog, getModelCatalogSnapshot } =
      await import('./ai-model-catalog');
    const snapshots: unknown[] = [];
    const toolbar = vi.fn(() => snapshots.push(getModelCatalogSnapshot('grok')));
    const menu = vi.fn();
    const otherProvider = vi.fn();
    const unsubscribe = subscribeModelCatalog('grok', menu);
    subscribeModelCatalog('grok', toolbar);
    subscribeModelCatalog('codex', otherProvider);
    mocks.invoke.mockResolvedValueOnce(['first']);
    const first = await refreshModelCatalog('grok');
    expect(toolbar).toHaveBeenCalledTimes(1);
    expect(menu).toHaveBeenCalledTimes(1);
    expect(snapshots[0]).toBe(first);
    await refreshModelCatalog('grok');
    expect(toolbar).toHaveBeenCalledTimes(1);
    unsubscribe();
    vi.advanceTimersByTime(5 * 60_000);
    mocks.invoke.mockResolvedValueOnce([{ id: 'first', name: 'Renamed', source: 'cli' }]);
    const renamed = await refreshModelCatalog('grok');
    expect(snapshots.at(-1)).toBe(renamed);
    expect(renamed.models?.[0].name).toBe('Renamed');
    vi.advanceTimersByTime(5 * 60_000);
    mocks.invoke.mockRejectedValueOnce('AI_MODEL_CATALOG_TIMEOUT');
    const failed = await refreshModelCatalog('grok');
    expect(snapshots.at(-1)).toBe(failed);
    expect(failed.models).toBe(renamed.models);
    vi.advanceTimersByTime(30_000);
    mocks.invoke.mockRejectedValueOnce('AI_PROVIDER_NOT_INSTALLED');
    expect(await refreshModelCatalog('grok')).toEqual({ error: 'not-installed' });
    expect(toolbar).toHaveBeenCalledTimes(4);
    expect(menu).toHaveBeenCalledTimes(1);
    expect(otherProvider).not.toHaveBeenCalled();
  });

  it.each(['codex', 'claude-code', 'grok'] as const)(
    '%s 合并预取和点击，缓存期间不额外探测版本',
    async (provider) => {
      const { refreshModelCatalog, getModelCatalogSnapshot } = await import('./ai-model-catalog');
      mocks.invoke.mockResolvedValueOnce(['a']);
      const pending = refreshModelCatalog(provider);
      expect(refreshModelCatalog(provider)).toBe(pending);
      await pending;
      await refreshModelCatalog(provider);
      vi.advanceTimersByTime(5 * 60_000 - 1);
      await refreshModelCatalog(provider);
      expect(mocks.invoke.mock.calls).toEqual([['ai_provider_models', { provider }]]);
      expect(getModelCatalogSnapshot(provider).models).toEqual(parseModelCatalog(['a']));
    }
  );

  it('到期刷新时保持旧模型可选，成功后替换整个列表', async () => {
    const { refreshModelCatalog, getModelCatalogSnapshot } = await import('./ai-model-catalog');
    mocks.invoke.mockResolvedValueOnce(['a']).mockResolvedValueOnce(['b']);
    await refreshModelCatalog('codex');
    vi.advanceTimersByTime(5 * 60_000);
    const pending = refreshModelCatalog('codex');
    expect(getModelCatalogSnapshot('codex').models).toEqual(parseModelCatalog(['a']));
    expect(await pending).toEqual({ models: parseModelCatalog(['b']) });
  });

  it('重启时先回显，失败保留目录并延迟重试，卸载只清理发现缓存', async () => {
    const first = await import('./ai-model-catalog');
    mocks.invoke.mockResolvedValueOnce(['vendor/model']);
    await first.refreshModelCatalog('claude-code');
    vi.resetModules();
    const next = await import('./ai-model-catalog');
    expect(next.getModelCatalogSnapshot('claude-code').models).toEqual(
      parseModelCatalog(['vendor/model'])
    );
    mocks.invoke.mockRejectedValueOnce('AI_MODEL_CATALOG_TIMEOUT');
    await next.refreshModelCatalog('claude-code');
    expect(await next.refreshModelCatalog('claude-code')).toEqual({
      models: parseModelCatalog(['vendor/model']),
      error: 'unavailable',
    });
    expect(mocks.invoke).toHaveBeenCalledTimes(2);
    vi.advanceTimersByTime(30_000);
    mocks.invoke.mockRejectedValueOnce('AI_PROVIDER_NOT_INSTALLED');
    expect(await next.refreshModelCatalog('claude-code')).toEqual({ error: 'not-installed' });
    vi.resetModules();
    expect(
      (await import('./ai-model-catalog')).getModelCatalogSnapshot('claude-code').models
    ).toBeUndefined();
  });

  it('升级旧磁盘缓存并保留新目录的显示名和兼容性，损坏响应不覆盖成功结果', async () => {
    window.localStorage.setItem(
      'nocterm.ai.discovered-models.claude-code',
      JSON.stringify(['opus'])
    );
    const { refreshModelCatalog, getModelCatalogSnapshot } = await import('./ai-model-catalog');
    expect(getModelCatalogSnapshot('claude-code').models).toEqual(parseModelCatalog(['opus']));
    const models = parseModelCatalog([
      {
        id: 'claude-fable-5-2',
        name: 'Claude Fable 5.2',
        source: 'catalog',
        minimumCliVersion: '2.1.321',
        unavailableReason: 'cliUpdateRequired',
      },
    ]);
    mocks.invoke.mockResolvedValueOnce(models);
    expect(await refreshModelCatalog('claude-code')).toEqual({
      models,
    });
    vi.advanceTimersByTime(5 * 60_000);
    mocks.invoke.mockResolvedValueOnce([{ id: 'invalid id' }]);
    expect(await refreshModelCatalog('claude-code')).toEqual({
      models,
      error: 'unavailable',
    });
    expect(
      JSON.parse(window.localStorage.getItem('nocterm.ai.discovered-models.claude-code')!)
    ).toEqual({ models });
  });

  it('Claude Code 缓存到期后通过一次目录请求更新版本兼容性', async () => {
    const { refreshModelCatalog, getModelCatalogSnapshot } = await import('./ai-model-catalog');
    mocks.invoke.mockResolvedValueOnce([
      {
        id: 'claude-fable-5-2',
        name: 'Claude Fable 5.2',
        source: 'catalog',
        unavailableReason: 'cliUpdateRequired',
        minimumCliVersion: '2.1.321',
      },
    ]);
    await refreshModelCatalog('claude-code');
    vi.advanceTimersByTime(5 * 60_000);
    mocks.invoke.mockResolvedValueOnce([
      {
        id: 'claude-fable-5-2',
        name: 'Claude Fable 5.2',
        source: 'catalog',
      },
    ]);
    await refreshModelCatalog('claude-code');
    expect(getModelCatalogSnapshot('claude-code').models?.[0].unavailableReason).toBeUndefined();
    expect(mocks.invoke.mock.calls).toEqual([
      ['ai_provider_models', { provider: 'claude-code' }],
      ['ai_provider_models', { provider: 'claude-code' }],
    ]);
  });

  it.each(['codex', 'claude-code', 'grok'] as const)(
    '%s 首次探测失败和未安装均等待 30 秒再重试',
    async (provider) => {
      const { refreshModelCatalog } = await import('./ai-model-catalog');
      for (const reason of ['AI_MODEL_CATALOG_TIMEOUT', 'AI_PROVIDER_NOT_INSTALLED']) {
        mocks.invoke.mockRejectedValueOnce(reason);
        const snapshot = await refreshModelCatalog(provider);
        const count = mocks.invoke.mock.calls.length;
        expect(snapshot.error).toBe(
          reason === 'AI_PROVIDER_NOT_INSTALLED' ? 'not-installed' : 'unavailable'
        );
        expect(await refreshModelCatalog(provider)).toBe(snapshot);
        vi.advanceTimersByTime(29_999);
        expect(await refreshModelCatalog(provider)).toBe(snapshot);
        expect(mocks.invoke).toHaveBeenCalledTimes(count);
        vi.advanceTimersByTime(1);
      }
      mocks.invoke.mockResolvedValueOnce([]);
      expect(await refreshModelCatalog(provider)).toEqual({ models: [] });
      expect(mocks.invoke).toHaveBeenCalledTimes(3);
    }
  );

  it('版本未知的有效目录同样缓存，旧版本字段不触发额外请求', async () => {
    window.localStorage.setItem(
      'nocterm.ai.discovered-models.claude-code',
      JSON.stringify({
        models: ['opus'],
        providerVersion: '2.1.280',
      })
    );
    const { refreshModelCatalog, getModelCatalogSnapshot } = await import('./ai-model-catalog');
    expect(getModelCatalogSnapshot('claude-code').models).toEqual(parseModelCatalog(['opus']));
    const models = parseModelCatalog([
      {
        id: 'claude-test',
        name: 'Test',
        source: 'catalog',
        unavailableReason: 'cliVersionUnknown',
      },
    ]);
    mocks.invoke.mockResolvedValueOnce(models);
    await refreshModelCatalog('claude-code');
    await refreshModelCatalog('claude-code');
    expect(mocks.invoke).toHaveBeenCalledTimes(1);
    expect(getModelCatalogSnapshot('claude-code')).toEqual({ models });
  });

  it('冷却从慢请求完成时开始，等待中的请求持续合并', async () => {
    const { refreshModelCatalog } = await import('./ai-model-catalog');
    let reject!: (reason: string) => void;
    mocks.invoke.mockReturnValueOnce(
      new Promise((_, fail) => {
        reject = fail;
      })
    );
    const pending = refreshModelCatalog('claude-code');
    vi.advanceTimersByTime(60_000);
    expect(refreshModelCatalog('claude-code')).toBe(pending);
    reject('AI_MODEL_CATALOG_TIMEOUT');
    await pending;
    vi.advanceTimersByTime(29_999);
    await refreshModelCatalog('claude-code');
    expect(mocks.invoke).toHaveBeenCalledTimes(1);
    vi.advanceTimersByTime(1);
    mocks.invoke.mockResolvedValueOnce([]);
    expect(await refreshModelCatalog('claude-code')).toEqual({ models: [] });
    expect(mocks.invoke).toHaveBeenCalledTimes(2);
  });
});
