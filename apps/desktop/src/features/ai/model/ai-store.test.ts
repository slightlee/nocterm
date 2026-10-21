import { beforeEach, describe, expect, it } from 'vitest';

import type { AiConversationTarget } from './ai-target';
import {
  conversationTargetKey,
  resolveActiveAiConversation,
  resolveAiComposerState,
  type AiConversation,
  useAiStore,
} from './ai-store';

const production: AiConversationTarget = { kind: 'ssh', id: '7', label: 'Production' };
const staging: AiConversationTarget = { kind: 'ssh', id: '8', label: 'Staging' };

const conversation = (
  id: string,
  target: AiConversationTarget | null = production,
  updatedAt = 1
): AiConversation => ({
  id,
  title: `会话 ${id}`,
  provider: 'codex',
  commandPolicy: 'auto_safe',
  target,
  messages: [],
  createdAt: 1,
  updatedAt,
});

describe('useAiStore target conversations', () => {
  beforeEach(() => {
    useAiStore.setState({
      conversations: [conversation('production-old'), conversation('staging', staging)],
      activeConversationIds: {},
      composers: {},
    });
  });

  it('enters a lazy blank state without creating a persisted conversation', () => {
    useAiStore.getState().beginConversation('ssh:7', 'claude-code', 'confirm_each');

    const state = useAiStore.getState();
    expect(state.conversations).toHaveLength(2);
    expect(state.activeConversationIds['ssh:7']).toBeNull();
    expect(
      resolveActiveAiConversation(state.conversations, state.activeConversationIds, 'ssh:7')
    ).toBeNull();
    expect(resolveAiComposerState(state.composers, 'ssh:7')).toMatchObject({
      draft: '',
      provider: 'claude-code',
      commandPolicy: 'confirm_each',
    });
  });

  it('creates and binds a conversation only when explicitly requested', () => {
    const id = useAiStore.getState().createConversation(production, 'codex', 'auto_safe');

    const state = useAiStore.getState();
    expect(state.conversations).toHaveLength(3);
    expect(state.activeConversationIds['ssh:7']).toBe(id);
    expect(state.conversations.at(-1)).toMatchObject({
      id,
      target: production,
      title: '新会话',
    });
  });

  it('restores the most recent conversation independently for each target', () => {
    const conversations = [
      conversation('production-old', production, 1),
      conversation('production-new', production, 9),
      conversation('staging', staging, 12),
    ];

    expect(resolveActiveAiConversation(conversations, {}, 'ssh:7')?.id).toBe('production-new');
    expect(resolveActiveAiConversation(conversations, {}, 'ssh:8')?.id).toBe('staging');
  });

  it('keeps drafts and blank selection isolated by target', () => {
    useAiStore.getState().setDraft('ssh:7', '生产环境草稿');
    useAiStore.getState().setDraft('ssh:8', '预发环境草稿');
    useAiStore.getState().beginConversation('ssh:7', 'codex', 'auto_safe');

    const state = useAiStore.getState();
    expect(resolveAiComposerState(state.composers, 'ssh:7').draft).toBe('');
    expect(resolveAiComposerState(state.composers, 'ssh:8').draft).toBe('预发环境草稿');
    expect(
      resolveActiveAiConversation(state.conversations, state.activeConversationIds, 'ssh:8')?.id
    ).toBe('staging');
  });

  it('routes a late assistant response to its original conversation', () => {
    useAiStore.getState().selectConversation('ssh:8', 'staging');
    useAiStore.getState().addMessage('production-old', 'assistant', '生产环境任务已完成');

    const state = useAiStore.getState();
    expect(
      state.conversations.find((item) => item.id === 'production-old')?.messages[0]?.content
    ).toBe('生产环境任务已完成');
    expect(state.conversations.find((item) => item.id === 'staging')?.messages).toHaveLength(0);
  });

  it('keeps command policy scoped to the selected conversation', () => {
    useAiStore.getState().setCommandPolicy('ssh:7', 'production-old', 'confirm_each');

    expect(
      useAiStore.getState().conversations.find((item) => item.id === 'production-old')
        ?.commandPolicy
    ).toBe('confirm_each');
    expect(
      useAiStore.getState().conversations.find((item) => item.id === 'staging')?.commandPolicy
    ).toBe('auto_safe');
  });

  it('selects the next conversation for the same target after deletion', () => {
    useAiStore.setState({
      conversations: [
        conversation('production-old', production, 1),
        conversation('production-new', production, 2),
        conversation('staging', staging, 3),
      ],
      activeConversationIds: { 'ssh:7': 'production-new' },
    });

    useAiStore.getState().deleteConversation('production-new');

    const state = useAiStore.getState();
    expect(state.activeConversationIds['ssh:7']).toBe('production-old');
    expect(state.conversations.map((item) => item.id)).toEqual(['production-old', 'staging']);
  });

  it('keeps legacy history unbound', () => {
    expect(conversationTargetKey(conversation('legacy', null))).toBeNull();
  });
});

describe('useAiStore hydrate', () => {
  it('keeps an empty persistence result empty instead of manufacturing a conversation', () => {
    useAiStore.setState({ conversations: [] });

    useAiStore.getState().hydrate([]);

    expect(useAiStore.getState().conversations).toEqual([]);
  });

  it('preserves a conversation created while persistence is still loading', () => {
    const local = conversation('local-send', production, 10);
    const restored = conversation('restored', staging, 5);
    useAiStore.setState({ conversations: [local] });

    useAiStore.getState().hydrate([restored]);

    expect(useAiStore.getState().conversations).toEqual([restored, local]);
  });

  it('keeps the newer in-memory snapshot when hydration contains the same conversation', () => {
    const local = {
      ...conversation('same', production, 10),
      messages: [
        {
          id: 'message-local',
          role: 'user' as const,
          content: '刚发送的问题',
          createdAt: 10,
        },
      ],
    };
    useAiStore.setState({ conversations: [local] });

    useAiStore.getState().hydrate([conversation('same', production, 5)]);

    expect(useAiStore.getState().conversations).toEqual([local]);
  });
});
