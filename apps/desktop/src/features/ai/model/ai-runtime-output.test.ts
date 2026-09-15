import { describe, expect, it } from 'vitest';

import { reduceAiRuntimeOutput, type AiRuntimeOutputSnapshot } from './ai-runtime-output';

const empty = (): AiRuntimeOutputSnapshot => ({ text: '', parts: [], answerStreamed: false });

describe('reduceAiRuntimeOutput', () => {
  it('merges answer deltas and ignores the later full-text duplicate', () => {
    const first = reduceAiRuntimeOutput(
      '{"method":"item/agentMessage/delta","params":{"delta":"检查"}}',
      empty()
    );
    const second = reduceAiRuntimeOutput(
      '{"method":"item/agentMessage/delta","params":{"delta":"完成"}}',
      first
    );
    const completed = reduceAiRuntimeOutput(
      '{"method":"item/completed","params":{"item":{"type":"agentMessage","text":"检查完成"}}}',
      second
    );

    expect(completed.text).toBe('检查完成');
    expect(completed.parts).toEqual([{ type: 'text', content: '检查完成' }]);
    expect(completed.changed).toBe(false);
    expect(completed.clearThinking).toBe(true);
  });

  it('keeps activity and text in their arrival order', () => {
    const reduced = reduceAiRuntimeOutput(
      '{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"先检查状态"},{"type":"text","text":"服务正常"}]}}',
      empty()
    );

    expect(reduced.text).toBe('服务正常\n');
    expect(reduced.parts).toEqual([
      { type: 'activity', kind: 'thinking', content: '先检查状态' },
      { type: 'text', content: '服务正常\n' },
    ]);
  });

  it('returns thinking deltas without changing the answer snapshot', () => {
    const reduced = reduceAiRuntimeOutput('{"type":"thought","data":"读取容器状态"}', empty());

    expect(reduced.thinkingDelta).toBe('读取容器状态');
    expect(reduced.changed).toBe(false);
    expect(reduced.text).toBe('');
  });
});
