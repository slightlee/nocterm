import { create } from 'zustand';

import { aiConversationTargetKey, type AiConversationTarget } from './ai-target';
import type { AiCommandPolicy, AiMessage, AiMessagePart, AiProviderId } from './ai-types';

export interface AiConversation {
  id: string;
  title: string;
  provider: AiProviderId;
  commandPolicy?: AiCommandPolicy;
  /** 旧版历史没有目标，只允许查看，不能静默绑定到当前终端。 */
  target: AiConversationTarget | null;
  messages: AiMessage[];
  createdAt: number;
  updatedAt: number;
}

interface AiTargetComposerState {
  draft: string;
  provider: AiProviderId;
  commandPolicy: AiCommandPolicy;
}

interface AiState {
  open: boolean;
  conversations: AiConversation[];
  /** null 表示用户显式进入了尚未发送的新会话；缺少键时回落到该目标最近会话。 */
  activeConversationIds: Record<string, string | null>;
  composers: Record<string, AiTargetComposerState>;
  setOpen: (open: boolean) => void;
  toggle: () => void;
  setProvider: (targetKey: string, conversationId: string | null, provider: AiProviderId) => void;
  setCommandPolicy: (
    targetKey: string,
    conversationId: string | null,
    policy: AiCommandPolicy
  ) => void;
  setDraft: (targetKey: string, draft: string) => void;
  beginConversation: (
    targetKey: string,
    provider: AiProviderId,
    commandPolicy: AiCommandPolicy
  ) => void;
  createConversation: (
    target: AiConversationTarget,
    provider: AiProviderId,
    commandPolicy: AiCommandPolicy
  ) => string;
  deleteConversation: (id: string) => void;
  selectConversation: (targetKey: string, id: string) => void;
  /** 用持久化层读出的会话整体替换内存历史；空列表保持真正空态。 */
  hydrate: (conversations: AiConversation[]) => void;
  addMessage: (
    conversationId: string,
    role: AiMessage['role'],
    content: string,
    parts?: AiMessagePart[]
  ) => void;
  deleteMessage: (conversationId: string, messageId: string) => void;
  clearMessages: (conversationId: string) => void;
}

const DEFAULT_COMPOSER: AiTargetComposerState = {
  draft: '',
  provider: 'codex',
  commandPolicy: 'auto_safe',
};

export function conversationTargetKey(conversation: AiConversation): string | null {
  return conversation.target ? aiConversationTargetKey(conversation.target) : null;
}

/** 未显式选择时恢复该目标最近更新的会话；显式新建态不自动回跳旧历史。 */
export function resolveActiveAiConversation(
  conversations: AiConversation[],
  activeConversationIds: Record<string, string | null>,
  targetKey: string
): AiConversation | null {
  if (Object.prototype.hasOwnProperty.call(activeConversationIds, targetKey)) {
    const selectedId = activeConversationIds[targetKey];
    if (selectedId === null) return null;
    const selected = conversations.find(
      (conversation) =>
        conversation.id === selectedId && conversationTargetKey(conversation) === targetKey
    );
    if (selected) return selected;
  }

  return (
    conversations
      .filter((conversation) => conversationTargetKey(conversation) === targetKey)
      .sort((first, second) => second.updatedAt - first.updatedAt)[0] ?? null
  );
}

export function resolveAiComposerState(
  composers: Record<string, AiTargetComposerState>,
  targetKey: string
): AiTargetComposerState {
  return composers[targetKey] ?? DEFAULT_COMPOSER;
}

const newConversation = (
  target: AiConversationTarget,
  provider: AiProviderId,
  commandPolicy: AiCommandPolicy
): AiConversation => {
  const now = Date.now();
  return {
    id: crypto.randomUUID(),
    title: '新会话',
    provider,
    commandPolicy,
    target,
    messages: [],
    createdAt: now,
    updatedAt: now,
  };
};

function updateComposer(state: AiState, targetKey: string, patch: Partial<AiTargetComposerState>) {
  return {
    ...state.composers,
    [targetKey]: { ...DEFAULT_COMPOSER, ...state.composers[targetKey], ...patch },
  };
}

/** 目标选择和会话数据分离：切换终端只做查询，不创建或持久化空会话。 */
export const useAiStore = create<AiState>((set) => ({
  open: false,
  conversations: [],
  activeConversationIds: {},
  composers: {},
  setOpen: (open) => set({ open }),
  toggle: () => set((state) => ({ open: !state.open })),
  setProvider: (targetKey, conversationId, provider) =>
    set((state) => ({
      conversations: conversationId
        ? state.conversations.map((conversation) =>
            conversation.id === conversationId ? { ...conversation, provider } : conversation
          )
        : state.conversations,
      composers: updateComposer(state, targetKey, { provider }),
    })),
  setCommandPolicy: (targetKey, conversationId, commandPolicy) =>
    set((state) => ({
      conversations: conversationId
        ? state.conversations.map((conversation) =>
            conversation.id === conversationId ? { ...conversation, commandPolicy } : conversation
          )
        : state.conversations,
      composers: updateComposer(state, targetKey, { commandPolicy }),
    })),
  setDraft: (targetKey, draft) =>
    set((state) => ({ composers: updateComposer(state, targetKey, { draft }) })),
  beginConversation: (targetKey, provider, commandPolicy) =>
    set((state) => ({
      activeConversationIds: { ...state.activeConversationIds, [targetKey]: null },
      composers: updateComposer(state, targetKey, { draft: '', provider, commandPolicy }),
    })),
  createConversation: (target, provider, commandPolicy) => {
    const conversation = newConversation(target, provider, commandPolicy);
    const targetKey = aiConversationTargetKey(target);
    set((state) => ({
      conversations: [...state.conversations, conversation],
      activeConversationIds: {
        ...state.activeConversationIds,
        [targetKey]: conversation.id,
      },
      composers: updateComposer(state, targetKey, {
        draft: '',
        provider,
        commandPolicy,
      }),
    }));
    return conversation.id;
  },
  deleteConversation: (id) =>
    set((state) => {
      const deleted = state.conversations.find((conversation) => conversation.id === id);
      if (!deleted) return state;
      const conversations = state.conversations.filter((conversation) => conversation.id !== id);
      const targetKey = conversationTargetKey(deleted);
      if (!targetKey) return { conversations };

      const replacement = conversations
        .filter((conversation) => conversationTargetKey(conversation) === targetKey)
        .sort((first, second) => second.updatedAt - first.updatedAt)[0];
      return {
        conversations,
        activeConversationIds: {
          ...state.activeConversationIds,
          [targetKey]: replacement?.id ?? null,
        },
      };
    }),
  selectConversation: (targetKey, id) =>
    set((state) => ({
      activeConversationIds: { ...state.activeConversationIds, [targetKey]: id },
      composers: updateComposer(state, targetKey, { draft: '' }),
    })),
  hydrate: (conversations) =>
    set((state) => {
      if (state.conversations.length === 0) return { conversations };
      // 首次发送可能早于 SQLite 回读完成；内存快照优先，避免 hydrate 丢掉刚启动的任务。
      const localById = new Map(
        state.conversations.map((conversation) => [conversation.id, conversation])
      );
      const restoredIds = new Set(conversations.map((conversation) => conversation.id));
      return {
        conversations: [
          ...conversations.map((conversation) => localById.get(conversation.id) ?? conversation),
          ...state.conversations.filter((conversation) => !restoredIds.has(conversation.id)),
        ],
      };
    }),
  addMessage: (conversationId, role, content, parts) =>
    set((state) => ({
      conversations: state.conversations.map((conversation) => {
        if (conversation.id !== conversationId) return conversation;
        const messages = [
          ...conversation.messages,
          { id: crypto.randomUUID(), role, content, parts, createdAt: Date.now() },
        ];
        return {
          ...conversation,
          title:
            conversation.title === '新会话' && role === 'user'
              ? content.slice(0, 24) || '新会话'
              : conversation.title,
          messages,
          updatedAt: Date.now(),
        };
      }),
    })),
  deleteMessage: (conversationId, messageId) =>
    set((state) => ({
      conversations: state.conversations.map((conversation) =>
        conversation.id === conversationId
          ? {
              ...conversation,
              messages: conversation.messages.filter((message) => message.id !== messageId),
              updatedAt: Date.now(),
            }
          : conversation
      ),
    })),
  clearMessages: (conversationId) =>
    set((state) => ({
      conversations: state.conversations.map((conversation) =>
        conversation.id === conversationId
          ? { ...conversation, messages: [], title: '新会话', updatedAt: Date.now() }
          : conversation
      ),
    })),
}));
