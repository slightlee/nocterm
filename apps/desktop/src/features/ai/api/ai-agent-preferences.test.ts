import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

describe('AI Agent preference storage', () => {
  let storage: Map<string, string>;

  beforeEach(() => {
    vi.resetModules();
    storage = new Map();
    vi.stubGlobal('window', {
      localStorage: {
        getItem: (key: string) => storage.get(key) ?? null,
        setItem: (key: string, value: string) => storage.set(key, value),
      },
    });
  });

  afterEach(() => vi.unstubAllGlobals());

  it('migrates legacy custom models and publishes ordered preference updates', async () => {
    storage.set('nocterm.ai.custom-models.grok', JSON.stringify(['local-grok']));
    const preferences = await import('./ai-agent-preferences');
    const listener = vi.fn();
    preferences.subscribeAiAgentPreferences(listener);

    expect(preferences.getAiAgentPreferences().providers.grok.customModels).toEqual(['local-grok']);
    preferences.moveAiProvider('grok', -1);
    preferences.setAiProviderEnabled('claude-code', false);
    preferences.addAiCustomModel('codex', 'vendor/model');
    preferences.setAiModelEnabled('codex', 'vendor/model', false);

    const current = preferences.getAiAgentPreferences();
    expect(current.providerOrder).toEqual(['codex', 'grok', 'claude-code']);
    expect(current.providers['claude-code'].enabled).toBe(false);
    expect(current.providers.codex.customModels).toEqual(['vendor/model']);
    expect(current.providers.codex.disabledModels).toEqual(['vendor/model']);
    expect(listener).toHaveBeenCalledTimes(4);
    expect(JSON.parse(storage.get('nocterm.ai.agent-preferences.v1')!)).toEqual(current);
  });

  it('does not publish a snapshot when persistence fails', async () => {
    vi.stubGlobal('window', {
      localStorage: {
        getItem: () => null,
        setItem: () => {
          throw new Error('quota');
        },
      },
    });
    const preferences = await import('./ai-agent-preferences');
    const before = preferences.getAiAgentPreferences();
    expect(() => preferences.setAiProviderEnabled('grok', false)).toThrow('quota');
    expect(preferences.getAiAgentPreferences()).toBe(before);
  });

  it('does not resurrect deleted legacy models after reload', async () => {
    storage.set('nocterm.ai.custom-models.grok', JSON.stringify(['legacy-model']));
    const preferences = await import('./ai-agent-preferences');
    preferences.removeAiCustomModel('grok', 'legacy-model');
    vi.resetModules();
    const reloaded = await import('./ai-agent-preferences');
    expect(reloaded.getAiAgentPreferences().providers.grok.customModels).toEqual([]);
    expect(storage.get('nocterm.ai.custom-models.grok')).toBe('["legacy-model"]');
  });
});
