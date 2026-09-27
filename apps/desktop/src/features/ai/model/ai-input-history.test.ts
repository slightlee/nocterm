import { describe, expect, it } from 'vitest';

import type { AiConversation } from './ai-store';
import { buildInputHistory, navigateInputHistory } from './ai-input-history';

const conversation = (id: string, userMessages: Array<[string, number]>): AiConversation => ({
  id,
  title: `会话 ${id}`,
  provider: 'codex',
  createdAt: 1,
  updatedAt: 1,
  messages: userMessages.map(([content, createdAt]) => ({
    id: `${id}-${createdAt}`,
    role: 'user' as const,
    content,
    createdAt,
  })),
});

describe('buildInputHistory', () => {
  it('collects user inputs across conversations from oldest to newest', () => {
    const history = buildInputHistory([
      conversation('a', [['查看日志', 10]]),
      conversation('b', [
        ['检查磁盘', 30],
        ['重启服务', 20],
      ]),
    ]);

    // 数组按时间正序（旧→新），最近输入在末尾，供导航状态机从尾部向上翻。
    expect(history).toEqual(['查看日志', '重启服务', '检查磁盘']);
  });

  it('ignores assistant messages and strips attachment marks', () => {
    const withAssistant = conversation('a', [['查看磁盘\n\n附件：df.txt', 10]]);
    withAssistant.messages.push({
      id: 'a-reply',
      role: 'assistant',
      content: '分析结果',
      createdAt: 11,
    });

    const history = buildInputHistory([withAssistant]);

    expect(history).toEqual(['查看磁盘']);
  });

  it('deduplicates by content keeping the most recent occurrence', () => {
    const history = buildInputHistory([
      conversation('a', [['查看日志', 10]]),
      conversation('b', [['查看日志', 20]]),
    ]);

    expect(history).toEqual(['查看日志']);
  });

  it('drops entries that become empty after stripping attachment marks', () => {
    const history = buildInputHistory([conversation('a', [['\n\n附件：dump.bin', 10]])]);

    expect(history).toEqual([]);
  });

  it('caps the history at the most recent 100 entries', () => {
    const messages = Array.from(
      { length: 130 },
      (_, index) => [`问题 ${index}`, index] as [string, number]
    );
    const history = buildInputHistory([conversation('a', messages)]);

    expect(history).toHaveLength(100);
    // 正序排列下：末尾是最近输入，开头是保留下限内的最早输入。
    expect(history.at(-1)).toBe('问题 129');
    expect(history[0]).toBe('问题 30');
  });
});

describe('navigateInputHistory', () => {
  const entries = ['第一条', '第二条', '第三条'];

  it('enters browsing from the draft and walks backwards through entries', () => {
    let step = navigateInputHistory(null, entries, '正在写的内容', 'up');
    expect(step).toEqual({ browser: { savedDraft: '正在写的内容', index: 2 }, text: '第三条' });

    step = navigateInputHistory(step.browser, entries, '', 'up');
    expect(step).toEqual({ browser: { savedDraft: '正在写的内容', index: 1 }, text: '第二条' });

    step = navigateInputHistory(step.browser, entries, '', 'up');
    expect(step.text).toBe('第一条');
  });

  it('stays on the oldest entry when pressing up beyond it', () => {
    let step = navigateInputHistory(null, entries, 'draft', 'up');
    step = navigateInputHistory(step.browser, entries, '', 'up');
    step = navigateInputHistory(step.browser, entries, '', 'up');
    const oldest = navigateInputHistory(step.browser, entries, '', 'up');

    expect(oldest.text).toBe('第一条');
    expect(oldest.browser?.index).toBe(0);
  });

  it('walks forward and restores the saved draft when moving past the newest entry', () => {
    let step = navigateInputHistory(null, entries, '正在写的内容', 'up');
    step = navigateInputHistory(step.browser, entries, '', 'up');

    step = navigateInputHistory(step.browser, entries, '', 'down');
    expect(step.text).toBe('第三条');
    expect(step.browser).not.toBeNull();

    step = navigateInputHistory(step.browser, entries, '', 'down');
    expect(step).toEqual({ browser: null, text: '正在写的内容' });
  });

  it('is a no-op when pressing down outside of browsing', () => {
    const step = navigateInputHistory(null, entries, '当前草稿', 'down');

    expect(step).toEqual({ browser: null, text: '当前草稿' });
  });
});
