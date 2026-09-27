import { describe, expect, it } from 'vitest';

import type { AiConversationSnapshot } from '../api/ai-client';
import type { AiConversation } from './ai-store';
import {
  conversationToSnapshot,
  diffConversations,
  snapshotToConversation,
} from './ai-persistence';

const conversation = (id: string, overrides: Partial<AiConversation> = {}): AiConversation => ({
  id,
  title: `会话 ${id}`,
  provider: 'codex',
  commandPolicy: 'auto_safe',
  createdAt: 1,
  updatedAt: 2,
  messages: [
    {
      id: `${id}-m1`,
      role: 'user',
      content: '查看磁盘占用',
      parts: [{ type: 'activity', kind: 'tool', content: 'nocterm/df -h' }],
      createdAt: 3,
    },
  ],
  ...overrides,
});

describe('conversation snapshots', () => {
  it('round trips through the persistence snapshot without losing data', () => {
    const original = conversation('conv-1');

    const snapshot = conversationToSnapshot(original);
    const restored = snapshotToConversation(snapshot);

    expect(restored).toEqual(original);
    expect(snapshot.messages[0]?.partsJson).toBe(
      JSON.stringify([{ type: 'activity', kind: 'tool', content: 'nocterm/df -h' }])
    );
  });

  it('keeps messages when the stored parts json is corrupted', () => {
    const snapshot = conversationToSnapshot(conversation('conv-1'));
    snapshot.messages[0]!.partsJson = '{not-json';

    const restored = snapshotToConversation(snapshot);

    // parts 是装饰性活动记录：损坏时丢弃，正文必须完整保留。
    expect(restored.messages[0]?.content).toBe('查看磁盘占用');
    expect(restored.messages[0]?.parts).toBeUndefined();
  });

  it('maps an absent command policy to an undefined value', () => {
    const snapshot: AiConversationSnapshot = {
      ...conversationToSnapshot(conversation('conv-1')),
      commandPolicy: null,
    };

    expect(snapshotToConversation(snapshot).commandPolicy).toBeUndefined();
  });
});

describe('diffConversations', () => {
  it('reports only genuinely changed conversations', () => {
    const prev = [conversation('a'), conversation('b')];
    const next = [conversation('a'), { ...conversation('b'), title: '新标题' }, conversation('c')];

    const diff = diffConversations(prev, next);

    expect(diff.changed.map((item) => item.id)).toEqual(['b', 'c']);
    expect(diff.deletedIds).toEqual([]);
  });

  it('reports deletions so the persisted rows can be removed', () => {
    const prev = [conversation('a'), conversation('b')];
    const next = [conversation('a')];

    const diff = diffConversations(prev, next);

    expect(diff.changed).toEqual([]);
    expect(diff.deletedIds).toEqual(['b']);
  });

  it('treats message level changes as a conversation change', () => {
    const conversationWithExtraMessage = conversation('a');
    conversationWithExtraMessage.messages = [
      ...conversationWithExtraMessage.messages,
      { id: 'a-m2', role: 'assistant', content: '占用最高的是日志目录', createdAt: 4 },
    ];

    const diff = diffConversations([conversation('a')], [conversationWithExtraMessage]);

    expect(diff.changed.map((item) => item.id)).toEqual(['a']);
    expect(diff.deletedIds).toEqual([]);
  });
});
