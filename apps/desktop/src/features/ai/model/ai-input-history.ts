import type { AiConversation } from './ai-store';
import type { AiMessage } from './ai-types';

/** 历史输入条数上限：与用户消息持久化解耦，超出后只保留最近输入。 */
const INPUT_HISTORY_LIMIT = 100;

/**
 * 从全部会话的用户消息构建全局历史输入，按使用时间正序排列（旧→新）、内容去重，
 * 与 Shell 历史指针模型一致：导航从数组末尾（最近输入）开始向上翻。
 * 数据直接派生自 store（消息已随 #42 持久化），不新增任何存储结构。
 */
export function buildInputHistory(conversations: AiConversation[]): string[] {
  const messages = conversations
    .flatMap((conversation) => conversation.messages)
    .filter((message): message is AiMessage & { role: 'user' } => message.role === 'user')
    .sort((a, b) => b.createdAt - a.createdAt);

  const seen = new Set<string>();
  const recentFirst: string[] = [];
  for (const message of messages) {
    const content = stripAttachmentSuffix(message.content).trim();
    // 纯附件提问剥离标记后没有可复用的文本，跳过；重复内容只保留最近一次。
    if (!content || seen.has(content)) continue;
    seen.add(content);
    recentFirst.push(content);
    if (recentFirst.length >= INPUT_HISTORY_LIMIT) break;
  }
  // 去重和截断都在“最近优先”方向完成后统一反转，保证最近输入位于数组末尾。
  return recentFirst.reverse();
}

/** 发送时附加的“附件：名称”标记属于展示文案，历史输入只回收用户原话。 */
function stripAttachmentSuffix(content: string): string {
  return content.replace(/\n\n附件：[^\n]*$/, '');
}

/** 浏览状态：null 表示不在历史浏览中；index 指向 entries 下标，等于长度时回到草稿。 */
export interface AiHistoryBrowseState {
  /** 进入浏览时未发送的草稿；向下翻出界时原样还原，保证用户不丢内容。 */
  savedDraft: string;
  index: number;
}

export interface InputHistoryStep {
  /** 新浏览状态；null 表示已退出浏览（回到草稿）。 */
  browser: AiHistoryBrowseState | null;
  /** 本次应回填到输入框的文本。 */
  text: string;
}

/**
 * 历史输入导航状态机：
 * - 上翻：从草稿进入浏览（记住草稿），逐条走向更早输入，到达最早一条后停留；
 * - 下翻：逐条走回最近输入，越过最近一条时退出浏览并还原草稿；
 * - 入口由调用方保证（entries 非空），这里不处理空历史。
 */
export function navigateInputHistory(
  browser: AiHistoryBrowseState | null,
  entries: string[],
  currentDraft: string,
  direction: 'up' | 'down'
): InputHistoryStep {
  if (direction === 'up') {
    const next = browser ?? { savedDraft: currentDraft, index: entries.length };
    const index = Math.max(0, next.index - 1);
    return { browser: { ...next, index }, text: entries[index] ?? next.savedDraft };
  }

  if (!browser) return { browser: null, text: currentDraft };
  const index = browser.index + 1;
  if (index >= entries.length) {
    return { browser: null, text: browser.savedDraft };
  }
  return { browser: { ...browser, index }, text: entries[index] };
}
