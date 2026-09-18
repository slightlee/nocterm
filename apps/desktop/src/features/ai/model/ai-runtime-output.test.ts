import { describe, expect, it } from 'vitest';

import {
  aiRuntimeErrorMessage,
  reduceAiRuntimeOutput,
  type AiRuntimeOutputSnapshot,
} from './ai-runtime-output';

const empty = (): AiRuntimeOutputSnapshot => ({ text: '', parts: [], answerStreamed: false });

describe('reduceAiRuntimeOutput', () => {
  it('streams Grok ACP output and records tool activity in arrival order', () => {
    const tool = reduceAiRuntimeOutput(
      '{"method":"session/update","params":{"update":{"sessionUpdate":"tool_call","title":"读取服务器系统信息"}}}',
      empty()
    );
    const answer = reduceAiRuntimeOutput(
      '{"method":"session/update","params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"主机正常"}}}}',
      tool
    );

    expect(answer.text).toBe('主机正常');
    expect(answer.parts).toEqual([
      { type: 'activity', kind: 'tool', content: '读取服务器系统信息' },
      { type: 'text', content: '主机正常' },
    ]);
  });

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
  });

  it('hides private thinking while keeping answer text', () => {
    const reduced = reduceAiRuntimeOutput(
      '{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"先检查状态"},{"type":"text","text":"服务正常"}]}}',
      empty()
    );

    expect(reduced.text).toBe('服务正常\n');
    expect(reduced.parts).toEqual([{ type: 'text', content: '服务正常\n' }]);
  });

  it('drops thinking deltas without changing the answer snapshot', () => {
    const reduced = reduceAiRuntimeOutput('{"type":"thought","data":"读取容器状态"}', empty());

    expect(reduced.changed).toBe(false);
    expect(reduced.text).toBe('');
  });
});

describe('aiRuntimeErrorMessage', () => {
  it('preserves actionable Tauri string errors', () => {
    expect(
      aiRuntimeErrorMessage(
        '当前 Grok 版本 1.0.33 不受支持，请升级到 Grok 1.0.34 或更高版本后重试',
        '启动失败'
      )
    ).toContain('Grok 1.0.34');
  });

  it('uses Error messages and falls back for unknown values', () => {
    expect(aiRuntimeErrorMessage(new Error('连接失败'), '启动失败')).toBe('连接失败');
    expect(aiRuntimeErrorMessage({ code: 'unknown' }, '启动失败')).toBe('启动失败');
  });
});
