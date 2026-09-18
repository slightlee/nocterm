import {
  extractAiActivities,
  extractAiStreamDelta,
  extractAiText,
  isAiFullTextEvent,
} from './ai-output';
import type { AiMessagePart } from './ai-types';

/** Provider 输出归并前的不可变快照，不包含任何 React 或 IPC 状态。 */
export interface AiRuntimeOutputSnapshot {
  text: string;
  parts: AiMessagePart[];
  answerStreamed: boolean;
}

/**
 * 单个事件的归并结果。
 * `changed` 表示持久展示内容发生变化；Provider 私有推理不会进入展示快照。
 */
export interface AiRuntimeOutputReduction extends AiRuntimeOutputSnapshot {
  changed: boolean;
}

/** Tauri command 可能拒绝字符串或 Error；面板必须保留后端可操作的失败说明。 */
export function aiRuntimeErrorMessage(error: unknown, fallback: string): string {
  if (error instanceof Error && error.message.trim()) return error.message.trim();
  if (typeof error === 'string' && error.trim()) return error.trim();
  return fallback;
}

function appendTextPart(parts: AiMessagePart[], text: string): AiMessagePart[] {
  const last = parts.at(-1);
  if (last?.type === 'text') {
    return [...parts.slice(0, -1), { ...last, content: last.content + text }];
  }
  return [...parts, { type: 'text', content: text }];
}

/** 把一个 Provider 事件归并为不可变快照，React Hook 只负责提交状态和副作用。 */
export function reduceAiRuntimeOutput(
  data: string,
  current: AiRuntimeOutputSnapshot
): AiRuntimeOutputReduction {
  const delta = extractAiStreamDelta(data);
  if (delta?.answer) {
    return {
      text: current.text + delta.answer,
      parts: appendTextPart(current.parts, delta.answer),
      answerStreamed: true,
      changed: true,
    };
  }
  if (delta) return { ...current, changed: false };

  // 非增量事件可能同时包含工具活动和最终文本，必须按事件内顺序归并。
  const activities = extractAiActivities(data);
  let parts = [
    ...current.parts,
    ...activities.map((activity): AiMessagePart => ({
      type: 'activity',
      kind: activity.kind,
      content: activity.text,
    })),
  ];
  let text = current.text;
  const fullTextEvent = isAiFullTextEvent(data);
  const extracted = extractAiText(data);
  // 已流式展示回答时，Provider 的最终全文事件只负责收尾，不能重复追加正文。
  if (extracted && !(current.answerStreamed && fullTextEvent)) {
    text += `${extracted}\n`;
    parts = appendTextPart(parts, `${extracted}\n`);
  }
  return {
    text,
    parts,
    answerStreamed: current.answerStreamed,
    changed: text !== current.text || parts.length !== current.parts.length,
  };
}
