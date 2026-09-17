import { describe, expect, it } from 'vitest';

import { presentAiMessageParts } from './ai-message-presentation';

describe('presentAiMessageParts', () => {
  it('moves text before the last activity into the process and keeps later text as the answer', () => {
    expect(
      presentAiMessageParts('完整上下文', [
        { type: 'text', content: '我先确认目标。' },
        { type: 'activity', kind: 'tool', content: '确认当前终端目标' },
        { type: 'text', content: '当前连接正常。' },
      ])
    ).toEqual({
      processEntries: [
        { kind: 'progress', content: '我先确认目标。' },
        { kind: 'tool', content: '确认当前终端目标' },
      ],
      answer: '当前连接正常。',
      activityCount: 1,
    });
  });

  it('keeps text after a leading activity as the answer', () => {
    expect(
      presentAiMessageParts('', [
        { type: 'activity', kind: 'tool', content: '读取服务器系统信息' },
        { type: 'text', content: '主机名：server-01' },
      ]).answer
    ).toBe('主机名：server-01');
  });

  it('keeps a text-only message unchanged', () => {
    expect(
      presentAiMessageParts('旧内容', [{ type: 'text', content: '无需调用工具的回答' }])
    ).toEqual({
      processEntries: [],
      answer: '无需调用工具的回答',
      activityCount: 0,
    });
  });

  it('preserves interleaved narration and activities in the process order', () => {
    expect(
      presentAiMessageParts('', [
        { type: 'text', content: '先检查环境。' },
        { type: 'activity', kind: 'tool', content: '确认当前终端目标' },
        { type: 'text', content: '再读取系统信息。' },
        { type: 'activity', kind: 'tool', content: '读取服务器系统信息' },
        { type: 'text', content: '检查完成。' },
      ]).processEntries
    ).toEqual([
      { kind: 'progress', content: '先检查环境。' },
      { kind: 'tool', content: '确认当前终端目标' },
      { kind: 'progress', content: '再读取系统信息。' },
      { kind: 'tool', content: '读取服务器系统信息' },
    ]);
  });

  it('does not fall back to full content when parts contain no final answer', () => {
    expect(
      presentAiMessageParts('不应显示的完整内容', [
        { type: 'activity', kind: 'tool', content: '执行远程命令' },
      ])
    ).toEqual({
      processEntries: [{ kind: 'tool', content: '执行远程命令' }],
      answer: '',
      activityCount: 1,
    });
  });

  it('uses legacy content only when structured parts are absent', () => {
    expect(presentAiMessageParts('历史回答')).toEqual({
      processEntries: [],
      answer: '历史回答',
      activityCount: 0,
    });
  });
});
