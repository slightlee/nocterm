import { describe, expect, it } from 'vitest';

import { filterModelOptions, parseModelCatalog, modelUnavailableLabel } from './ai-model-catalog';

describe('filterModelOptions', () => {
  it('sorts concrete versions numerically across families while preserving CLI order', () => {
    const catalog = parseModelCatalog([
      { id: 'sonnet', name: 'Alias', source: 'cli' },
      { id: 'opus', name: 'Alias', source: 'cli' },
      ...[
        'claude-opus-4-8',
        'claude-sonnet-5',
        'claude-fable-5-2',
        'claude-opus-5-5',
        'claude-haiku-5-5',
        'claude-opus-5-10',
      ].map((id) => ({ id, name: id, source: 'catalog' })),
    ]);
    expect(filterModelOptions(catalog, [], undefined, '').map((m) => m?.id)).toEqual([
      'sonnet',
      'opus',
      'claude-opus-5-10',
      'claude-opus-5-5',
      'claude-haiku-5-5',
      'claude-fable-5-2',
      'claude-sonnet-5',
      'claude-opus-4-8',
    ]);
  });

  it('folds legacy catalog entries but keeps search, explicit expansion and selected models reachable', () => {
    const catalog = parseModelCatalog([
      { id: 'claude-opus-5-5', name: 'Opus 5.5', source: 'catalog', isLegacy: false },
      { id: 'claude-opus-4-8', name: 'Opus 4.8', source: 'catalog', isLegacy: true },
      { id: 'claude-opus-4-5', name: 'Opus 4.5', source: 'catalog', isLegacy: true },
    ]);
    expect(filterModelOptions(catalog, [], undefined, '').map((m) => m?.id)).toEqual([
      'claude-opus-5-5',
    ]);
    expect(filterModelOptions(catalog, [], undefined, '', true)).toHaveLength(3);
    expect(filterModelOptions(catalog, [], undefined, '4.8').map((m) => m?.id)).toEqual([
      'claude-opus-4-8',
    ]);
    expect(filterModelOptions(catalog, [], 'claude-opus-4-5', '').map((m) => m?.id)).toEqual([
      'claude-opus-5-5',
      'claude-opus-4-5',
    ]);
  });

  it('preserves model IDs and order when filtering mixed sources', () => {
    expect(
      filterModelOptions(
        parseModelCatalog(['Vendor/Alpha', 'other']),
        ['Vendor/Alpha', 'vendor/Beta'],
        undefined,
        ' VENDOR/ '
      ).map((model) => model?.id)
    ).toEqual(['Vendor/Alpha', 'vendor/Beta']);
  });

  it('keeps a missing selection reachable without offering an implicit default model', () => {
    expect(filterModelOptions([], [], 'previous-model', '').map((model) => model?.id)).toEqual([
      'previous-model',
    ]);
    expect(filterModelOptions([], [], undefined, '')).toEqual([]);
    expect(filterModelOptions([], [], undefined, '默认')).toEqual([]);
    expect(filterModelOptions([], [], undefined, 'unknown')).toEqual([]);
  });

  it('searches display names and resolved models without merging distinct aliases', () => {
    const catalog = parseModelCatalog([
      { id: 'opus', name: 'Mapped Name', source: 'cli', resolvedModel: 'mapped-id' },
      { id: 'sonnet', name: 'Mapped Name', source: 'cli', resolvedModel: 'mapped-id' },
    ]);
    for (const query of ['mapped name', 'mapped-id']) {
      expect(filterModelOptions(catalog, [], undefined, query).map((m) => m?.id)).toEqual([
        'opus',
        'sonnet',
      ]);
    }
  });

  it('keeps catalog restrictions when the same ID was added as a custom model', () => {
    const catalog = parseModelCatalog([
      {
        id: 'claude-fable-5-2',
        name: 'Claude Fable 5.2',
        source: 'catalog',
        minimumCliVersion: '2.1.321',
        unavailableReason: 'cliUpdateRequired',
      },
    ]);
    const options = filterModelOptions(catalog, ['claude-fable-5-2'], undefined, 'fable');
    expect(options).toHaveLength(1);
    expect(modelUnavailableLabel(options[0])).toBe('需要 Claude Code ≥ 2.1.321');
  });
});

describe('parseModelCatalog', () => {
  it('upgrades legacy cached IDs and stores only supported metadata', () => {
    expect(parseModelCatalog(['old'])).toEqual([{ id: 'old', name: 'old', source: 'cli' }]);
    const [model] = parseModelCatalog([
      { id: 'opus', name: 'Mapped', source: 'cli', resolvedModel: 'mapped', account: 'discard-me' },
    ]);
    expect(model).not.toHaveProperty('account');
    expect(model.resolvedModel).toBe('mapped');
  });

  it('rejects corrupt caches rather than presenting invalid models as available', () => {
    for (const value of [
      null,
      ['https://example.invalid'],
      Array(1025).fill('model'),
      [{ id: 'a', name: 'A', source: 'catalog', unavailableReason: 'unexpected' }],
      [{ id: 'a', name: 'A\nB', source: 'cli' }],
      [{ id: 'a', name: 'A', source: 'catalog', minimumCliVersion: 'bogus' }],
      [{ id: 'a', name: 'A', source: 'catalog', isLegacy: 'true' }],
    ]) {
      expect(() => parseModelCatalog(value)).toThrow();
    }
  });
});
