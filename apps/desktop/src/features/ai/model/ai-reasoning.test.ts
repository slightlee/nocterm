import { describe, expect, it } from 'vitest';
import { compatibleReasoningEffort } from './ai-reasoning';
import { parseModelCatalog } from './ai-model-catalog';

describe('思考强度能力', () => {
  it('只选模型采用其默认值，手动档位优先，未知或不兼容默认值不猜测', () => {
    expect(compatibleReasoningEffort(['low', 'high'], undefined, 'low')).toBe('low');
    expect(compatibleReasoningEffort(['low', 'high'], 'high', 'low')).toBe('high');
    expect(compatibleReasoningEffort(['medium'], 'high', 'medium')).toBe('medium');
    expect(compatibleReasoningEffort(['high'], undefined)).toBeUndefined();
    expect(compatibleReasoningEffort(undefined, undefined, 'low')).toBeUndefined();
    expect(compatibleReasoningEffort(['high'], undefined, 'low')).toBeUndefined();
  });
  it('模型切换保留兼容档位，未知模型、较小能力集和默认选择不继承旧覆盖', () => {
    expect(compatibleReasoningEffort(['low', 'high'], 'high')).toBe('high');
    expect(compatibleReasoningEffort(['low'], 'high')).toBeUndefined();
    expect(compatibleReasoningEffort(undefined, 'high')).toBeUndefined();
    expect(compatibleReasoningEffort(['high'], undefined)).toBeUndefined();
  });
  it('读取并缓存能力，旧目录仍可用，损坏元数据拒绝进入菜单', () => {
    const [model] = parseModelCatalog([
      {
        id: 'test',
        name: 'Test',
        source: 'cli',
        reasoningEfforts: ['low', 'high', 'high'],
        defaultReasoningEffort: 'low',
      },
    ]);
    expect(model.reasoningEfforts).toEqual(['low', 'high']);
    expect(model.defaultReasoningEffort).toBe('low');
    expect(parseModelCatalog(['legacy'])[0].reasoningEfforts).toBeUndefined();
    for (const fields of [
      { reasoningEfforts: ['invented'] },
      { reasoningEfforts: 'high' },
      { reasoningEfforts: ['low'], defaultReasoningEffort: 'high' },
    ]) {
      expect(() =>
        parseModelCatalog([{ id: 'test', name: 'Test', source: 'cli', ...fields }])
      ).toThrow();
    }
  });
});
