import { describe, expect, it } from 'vitest';

import type { AiMessage } from './ai-types';
import { buildConversationContext } from './ai-conversation';

const message = (id: string, role: AiMessage['role'], content: string): AiMessage => ({
  id,
  role,
  content,
  createdAt: 1,
});

describe('buildConversationContext', () => {
  it('keeps the current conversation order and role boundaries', () => {
    expect(
      buildConversationContext([
        message('one', 'user', '检查当前目录'),
        message('two', 'assistant', '当前目录是 /tmp'),
      ])
    ).toContain('用户：检查当前目录\n\n助手：当前目录是 /tmp');
  });

  it('keeps recent messages within the bounded prompt budget', () => {
    const context = buildConversationContext([
      message('old', 'user', '旧'.repeat(12_000)),
      message('recent', 'assistant', '最近的回答'),
    ]);

    expect(context).toContain('最近的回答');
    expect(context).not.toContain('旧'.repeat(100));
    expect(context.length).toBeLessThan(12_200);
  });
});
