import { create } from 'zustand';

import type { AiCommandPolicy, AiMessage, AiMessagePart, AiProviderId } from './ai-types';

export interface AiConversation {
  id: string;
  title: string;
  provider: AiProviderId;
  commandPolicy?: AiCommandPolicy;
  messages: AiMessage[];
  createdAt: number;
  updatedAt: number;
}

interface AiState {
  open: boolean;
  conversations: AiConversation[];
  activeConversationId: string;
  draft: string;
  setOpen: (open: boolean) => void;
  toggle: () => void;
  setProvider: (provider: AiProviderId) => void;
  setCommandPolicy: (policy: AiCommandPolicy) => void;
  setDraft: (draft: string) => void;
  createConversation: (provider?: AiProviderId) => void;
  deleteConversation: (id: string) => void;
  selectConversation: (id: string) => void;
  addMessage: (role: AiMessage['role'], content: string, parts?: AiMessagePart[]) => void;
  deleteMessage: (id: string) => void;
  clearMessages: () => void;
}

const createConversation = (provider: AiProviderId): AiConversation => {
  const now = Date.now();
  return {
    id: crypto.randomUUID(),
    title: '新会话',
    provider,
    commandPolicy: 'auto_safe',
    messages: [],
    createdAt: now,
    updatedAt: now,
  };
};

const initialConversation = createConversation('codex');

/** 对话只保留在当前应用进程内，终端内容和 AI 上下文不会写入磁盘。 */
export const useAiStore = create<AiState>((set) => ({
  open: false,
  conversations: [initialConversation],
  activeConversationId: initialConversation.id,
  draft: '',
  setOpen: (open) => set({ open }),
  toggle: () => set((state) => ({ open: !state.open })),
  setProvider: (provider) =>
    set((state) => ({
      conversations: state.conversations.map((conversation) =>
        conversation.id === state.activeConversationId
          ? { ...conversation, provider }
          : conversation
      ),
    })),
  setCommandPolicy: (commandPolicy) =>
    set((state) => ({
      conversations: state.conversations.map((conversation) =>
        conversation.id === state.activeConversationId
          ? { ...conversation, commandPolicy }
          : conversation
      ),
    })),
  setDraft: (draft) => set({ draft }),
  createConversation: (provider) =>
    set((state) => {
      const current = state.conversations.find((item) => item.id === state.activeConversationId);
      const conversation = createConversation(provider ?? current?.provider ?? 'codex');
      return {
        conversations: [...state.conversations, conversation],
        activeConversationId: conversation.id,
        draft: '',
      };
    }),
  deleteConversation: (id) =>
    set((state) => {
      const deletedIndex = state.conversations.findIndex((item) => item.id === id);
      if (deletedIndex === -1) return state;

      const deletedConversation = state.conversations[deletedIndex];
      const conversations = state.conversations.filter((item) => item.id !== id);
      if (conversations.length === 0) {
        const replacement = createConversation(deletedConversation.provider);
        return { conversations: [replacement], activeConversationId: replacement.id, draft: '' };
      }

      if (state.activeConversationId !== id) return { conversations };
      const nextActive = conversations[Math.min(deletedIndex, conversations.length - 1)];
      return { conversations, activeConversationId: nextActive.id, draft: '' };
    }),
  selectConversation: (id) => set({ activeConversationId: id, draft: '' }),
  addMessage: (role, content, parts) =>
    set((state) => ({
      conversations: state.conversations.map((conversation) => {
        if (conversation.id !== state.activeConversationId) return conversation;
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
  /** 供失败重试撤销本轮消息；只在活动会话内删除，消息 id 不存在时保持原状态。 */
  deleteMessage: (id) =>
    set((state) => ({
      conversations: state.conversations.map((conversation) =>
        conversation.id === state.activeConversationId
          ? {
              ...conversation,
              messages: conversation.messages.filter((message) => message.id !== id),
              updatedAt: Date.now(),
            }
          : conversation
      ),
    })),
  clearMessages: () =>
    set((state) => ({
      conversations: state.conversations.map((conversation) =>
        conversation.id === state.activeConversationId
          ? { ...conversation, messages: [], title: '新会话', updatedAt: Date.now() }
          : conversation
      ),
      draft: '',
    })),
}));
