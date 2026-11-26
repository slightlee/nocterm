import { beforeEach, describe, expect, it } from 'vitest';

import type { AiConversationTarget } from './ai-target';
import {
  conversationTargetKey,
  resolveActiveAiConversation,
  resolveActiveAiConversationId,
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

  it('keeps selected models scoped to their conversation and provider', () => {
    const store = useAiStore.getState();
    store.setProvider('ssh:7', 'production-old', 'codex', 'vendor/model');
    expect(useAiStore.getState().conversations[0]?.model).toBe('vendor/model');
    expect(useAiStore.getState().conversations[1]?.model).toBeUndefined();
    store.setProvider('ssh:7', 'production-old', 'claude-code');
    expect(useAiStore.getState().conversations[0]?.model).toBeUndefined();
  });

  it('思考强度随会话和草稿隔离，新会话与默认选择清除显式覆盖', () => {
    const store = useAiStore.getState();
    store.setProvider('ssh:7', 'production-old', 'codex', 'test-model', 'high', true, true);
    expect(useAiStore.getState().conversations[0].reasoningEffort).toBe('high');
    expect(useAiStore.getState().conversations[0].fastMode).toBe(true);
    expect(useAiStore.getState().conversations[0].supportsFastMode).toBe(true);
    expect(useAiStore.getState().conversations[1].reasoningEffort).toBeUndefined();
    expect(resolveAiComposerState(useAiStore.getState().composers, 'ssh:7').reasoningEffort).toBe(
      'high'
    );
    store.setProvider('ssh:7', 'production-old', 'codex', 'test-model');
    expect(useAiStore.getState().conversations[0].reasoningEffort).toBeUndefined();
    const id = store.createConversation(
      production,
      'codex',
      'auto_safe',
      'test-model',
      'low',
      false,
      true
    );
    expect(useAiStore.getState().conversations.find((c) => c.id === id)?.reasoningEffort).toBe(
      'low'
    );
    expect(useAiStore.getState().conversations.find((c) => c.id === id)?.fastMode).toBe(false);
    expect(useAiStore.getState().conversations.find((c) => c.id === id)?.supportsFastMode).toBe(
      true
    );
    store.beginConversation('ssh:7', 'codex', 'auto_safe');
    expect(
      resolveAiComposerState(useAiStore.getState().composers, 'ssh:7').reasoningEffort
    ).toBeUndefined();
    expect(
      resolveAiComposerState(useAiStore.getState().composers, 'ssh:7').fastMode
    ).toBeUndefined();
    expect(
      resolveAiComposerState(useAiStore.getState().composers, 'ssh:7').supportsFastMode
    ).toBeUndefined();
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

  it('resolves the fallback conversation id used by provider change validation', () => {
    const conversations = [
      conversation('production-old', production, 1),
      conversation('production-new', production, 9),
    ];

    expect(resolveActiveAiConversationId(conversations, {}, 'ssh:7')).toBe('production-new');
    expect(resolveActiveAiConversationId(conversations, { 'ssh:7': null }, 'ssh:7')).toBeNull();
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
