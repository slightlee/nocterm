import { describe, expect, it } from 'vitest';

import {
  applyModelPreferences,
  createAiAgentPreferences,
  enabledProviders,
  moveId,
  moveVisibleId,
  parseAiAgentPreferences,
} from './ai-agent-preferences';

describe('AI Agent preferences', () => {
  it('repairs partial data and migrates legacy custom models', () => {
    const result = parseAiAgentPreferences(
      {
        providerOrder: ['grok', 'unknown', 'grok'],
        providers: {
          grok: { enabled: false, modelOrder: ['grok-3'], disabledModels: ['bad model'] },
        },
      },
      { codex: ['gpt-5.6', 'https://invalid.example'] }
    );

    expect(result.providerOrder).toEqual(['grok', 'codex', 'claude-code']);
    expect(result.providers.grok.enabled).toBe(false);
    expect(result.providers.grok.disabledModels).toEqual([]);
    expect(result.providers.codex.customModels).toEqual(['gpt-5.6']);
  });

  it('returns only enabled providers in the saved order', () => {
    const preferences = createAiAgentPreferences();
    preferences.providerOrder = ['claude-code', 'grok', 'codex'];
    preferences.providers.grok.enabled = false;
    expect(enabledProviders(preferences).map(({ id }) => id)).toEqual(['claude-code', 'codex']);
  });

  it('retains accumulated preferences beyond the single catalog response limit', () => {
    const ids = Array.from({ length: 1025 }, (_, index) => `model-${index}`);
    const preferences = createAiAgentPreferences({ codex: ids });
    expect(parseAiAgentPreferences(preferences).providers.codex.customModels).toEqual(ids);
  });

  it('applies model visibility and stable ordering', () => {
    const preferences = createAiAgentPreferences().providers.codex;
    preferences.modelOrder = ['third', 'first'];
    preferences.disabledModels = ['second'];
    const models = ['first', 'second', 'third', 'new'].map((id) => ({
      id,
      name: id,
      source: 'cli' as const,
    }));

    expect(applyModelPreferences(models, preferences).map(({ id }) => id)).toEqual([
      'third',
      'first',
      'new',
    ]);
    expect(applyModelPreferences(models, preferences, true).map(({ id }) => id)).toEqual([
      'third',
      'first',
      'second',
      'new',
    ]);
  });

  it('moves an ID without mutating the source list', () => {
    const source = ['a', 'b', 'c'];
    expect(moveId(source, 'b', -1)).toEqual(['b', 'a', 'c']);
    expect(moveId(source, 'a', -1)).toEqual(source);
    expect(source).toEqual(['a', 'b', 'c']);
  });

  it('keeps temporarily missing model positions while reordering visible models', () => {
    expect(
      moveVisibleId(['first', 'missing', 'second'], ['first', 'second', 'new'], 'second', -1)
    ).toEqual(['second', 'missing', 'first', 'new']);
  });
});
