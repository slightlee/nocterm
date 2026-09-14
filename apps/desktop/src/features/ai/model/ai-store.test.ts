import { beforeEach, describe, expect, it } from 'vitest';

import type { AiConversation } from './ai-store';
import { useAiStore } from './ai-store';

const conversation = (id: string, provider: AiConversation['provider'] = 'codex') => ({
  id,
  title: `会话 ${id}`,
  provider,
  messages: [],
  createdAt: 1,
  updatedAt: 1,
});

describe('useAiStore conversations', () => {
  beforeEach(() => {
    useAiStore.setState({
      conversations: [conversation('first'), conversation('second')],
      activeConversationId: 'first',
      draft: '未发送内容',
    });
  });

  it('creates and activates a blank conversation with the current provider', () => {
    useAiStore.getState().createConversation();

    const state = useAiStore.getState();
    expect(state.conversations).toHaveLength(3);
    expect(state.activeConversationId).toBe(state.conversations[2]?.id);
    expect(state.conversations[2]).toMatchObject({
      title: '新会话',
      provider: 'codex',
      commandPolicy: 'auto_safe',
    });
    expect(state.draft).toBe('');
  });

  it('keeps command execution policy scoped to the active conversation', () => {
    useAiStore.getState().setCommandPolicy('confirm_each');

    expect(useAiStore.getState().conversations[0]?.commandPolicy).toBe('confirm_each');
    expect(useAiStore.getState().conversations[1]?.commandPolicy).toBeUndefined();
  });

  it('deletes the active conversation and selects the adjacent conversation', () => {
    useAiStore.getState().deleteConversation('first');

    expect(useAiStore.getState()).toMatchObject({
      conversations: [conversation('second')],
      activeConversationId: 'second',
      draft: '',
    });
  });

  it('replaces the final deleted conversation and preserves its provider', () => {
    useAiStore.setState({
      conversations: [conversation('only', 'claude-code')],
      activeConversationId: 'only',
    });

    useAiStore.getState().deleteConversation('only');

    const state = useAiStore.getState();
    expect(state.conversations).toHaveLength(1);
    expect(state.activeConversationId).toBe(state.conversations[0]?.id);
    expect(state.conversations[0]).toMatchObject({ title: '新会话', provider: 'claude-code' });
  });

  it('deletes a single message only from the active conversation', () => {
    useAiStore.setState({
      conversations: [
        {
          ...conversation('first'),
          messages: [
            { id: 'm1', role: 'user', content: '检查部署状态', createdAt: 1 },
            { id: 'm2', role: 'assistant', content: '执行失败残留', createdAt: 2 },
          ],
        },
        {
          ...conversation('second'),
          messages: [{ id: 'm3', role: 'user', content: '其他会话', createdAt: 3 }],
        },
      ],
      activeConversationId: 'first',
    });

    useAiStore.getState().deleteMessage('m2');

    const state = useAiStore.getState();
    expect(state.conversations[0]?.messages.map((message) => message.id)).toEqual(['m1']);
    expect(state.conversations[1]?.messages).toHaveLength(1);
  });

  it('keeps conversations unchanged when deleting an unknown message id', () => {
    useAiStore.setState({
      conversations: [
        {
          ...conversation('first'),
          messages: [{ id: 'm1', role: 'user', content: '问题', createdAt: 1 }],
        },
      ],
      activeConversationId: 'first',
    });

    useAiStore.getState().deleteMessage('missing');

    expect(useAiStore.getState().conversations[0]?.messages).toHaveLength(1);
  });

  it('keeps assistant text and tool activity in their original order', () => {
    const parts = [
      { type: 'text' as const, content: '先检查客户端。' },
      { type: 'activity' as const, kind: 'tool' as const, content: 'nocterm/which docker' },
      { type: 'text' as const, content: '客户端已安装。' },
    ];

    useAiStore.getState().addMessage('assistant', '先检查客户端。客户端已安装。', parts);

    expect(useAiStore.getState().conversations[0]?.messages[0]?.parts).toEqual(parts);
  });
});
