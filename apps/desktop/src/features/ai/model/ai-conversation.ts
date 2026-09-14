import type { AiMessage } from './ai-types';

const MAX_HISTORY_CHARACTERS = 12_000;

/**
 * Headless Provider 每轮都会启动新进程，因此由 Nocterm 明确携带当前对话历史。
 * 从最近消息向前截取，避免长对话无限放大 Prompt，也不会误恢复 Provider 的其他会话。
 */
export function buildConversationContext(messages: AiMessage[]): string {
  const selected: string[] = [];
  let length = 0;
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index];
    if (!message) continue;
    const entry = `${message.role === 'user' ? '用户' : '助手'}：${message.content.trim()}`;
    if (!entry.trim()) continue;
    const remaining = MAX_HISTORY_CHARACTERS - length;
    if (remaining <= 0) break;
    if (entry.length > remaining) {
      if (selected.length === 0) selected.push(entry.slice(0, remaining));
      break;
    }
    selected.push(entry);
    length += entry.length;
    if (length >= MAX_HISTORY_CHARACTERS) break;
  }
  if (selected.length === 0) return '';
  return `以下是当前 Nocterm 对话的最近上下文。请延续该对话，不要声称这是新的独立请求：\n\n${selected.reverse().join('\n\n')}\n\n`;
}
