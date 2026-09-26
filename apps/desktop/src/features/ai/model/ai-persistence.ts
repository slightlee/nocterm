import type { AiConversationSnapshot } from '../api/ai-client';

import type { AiConversation } from './ai-store';
import type { AiMessage, AiMessagePart } from './ai-types';

/** 把内存会话转换为 IPC 快照；parts 内联在消息上，序列化成 JSON 字符串存放。 */
export function conversationToSnapshot(conversation: AiConversation): AiConversationSnapshot {
  return {
    id: conversation.id,
    title: conversation.title,
    provider: conversation.provider,
    commandPolicy: conversation.commandPolicy ?? null,
    createdAtMs: conversation.createdAt,
    updatedAtMs: conversation.updatedAt,
    messages: conversation.messages.map((message) => ({
      id: message.id,
      role: message.role,
      content: message.content,
      partsJson: message.parts ? JSON.stringify(message.parts) : null,
      createdAtMs: message.createdAt,
    })),
  };
}

/** 持久化快照还原为内存会话；parts JSON 损坏时丢弃装饰性活动，保留正文不丢历史。 */
export function snapshotToConversation(snapshot: AiConversationSnapshot): AiConversation {
  return {
    id: snapshot.id,
    title: snapshot.title,
    provider: snapshot.provider,
    commandPolicy: (snapshot.commandPolicy ?? undefined) as AiConversation['commandPolicy'],
    createdAt: snapshot.createdAtMs,
    updatedAt: snapshot.updatedAtMs,
    messages: snapshot.messages.map((message) => {
      let parts: AiMessagePart[] | undefined;
      if (message.partsJson) {
        try {
          parts = JSON.parse(message.partsJson) as AiMessagePart[];
        } catch {
          parts = undefined;
        }
      }
      return {
        id: message.id,
        role: message.role,
        content: message.content,
        parts,
        createdAt: message.createdAtMs,
      } satisfies AiMessage;
    }),
  };
}

export interface ConversationDiff {
  /** 新增或内容发生变化的会话，按快照整体覆盖写入。 */
  changed: AiConversation[];
  /** 已从内存删除、需要同步从持久化层移除的会话 ID。 */
  deletedIds: string[];
}

/**
 * 对比两次会话列表，只提交真正变化的会话。
 * 比较基于完整内容的序列化值：标题、消息、Provider、策略任一变化都会被捕获。
 */
export function diffConversations(
  prev: AiConversation[],
  next: AiConversation[]
): ConversationDiff {
  const prevById = new Map(prev.map((conversation) => [conversation.id, conversation]));
  const nextIds = new Set(next.map((conversation) => conversation.id));

  const changed = next.filter((conversation) => {
    const previous = prevById.get(conversation.id);
    return !previous || JSON.stringify(previous) !== JSON.stringify(conversation);
  });
  const deletedIds = prev.map((conversation) => conversation.id).filter((id) => !nextIds.has(id));

  return { changed, deletedIds };
}
