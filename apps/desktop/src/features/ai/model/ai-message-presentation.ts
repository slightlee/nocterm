import type { AiExecutionPresentation, AiMessagePart } from './ai-types';

export interface AiProcessEntry {
  kind: 'progress' | 'tool' | 'execution';
  content: string;
  execution?: AiExecutionPresentation;
}

export interface AiMessagePresentation {
  processEntries: AiProcessEntry[];
  answer: string;
  activityCount: number;
}

/**
 * 工具调用之前和工具调用之间的普通文本属于执行叙述，最后一次活动之后的文本才是答案。
 * 该边界仅依赖事件顺序，避免按 Provider、语言或固定措辞维护展示规则。
 */
export function presentAiMessageParts(
  content: string,
  parts?: AiMessagePart[]
): AiMessagePresentation {
  if (!parts) {
    return { processEntries: [], answer: content, activityCount: 0 };
  }

  let lastActivityIndex = -1;
  for (let index = parts.length - 1; index >= 0; index -= 1) {
    if (parts[index]?.type === 'activity' || parts[index]?.type === 'execution') {
      lastActivityIndex = index;
      break;
    }
  }
  if (lastActivityIndex < 0) {
    return {
      processEntries: [],
      answer: parts
        .filter((part): part is Extract<AiMessagePart, { type: 'text' }> => part.type === 'text')
        .map((part) => part.content)
        .join(''),
      activityCount: 0,
    };
  }

  const processEntries: AiProcessEntry[] = parts
    .slice(0, lastActivityIndex + 1)
    .flatMap((part): AiProcessEntry[] => {
      if (part.type === 'execution') {
        return [
          {
            kind: 'execution',
            content: '',
            execution: part.execution,
          },
        ];
      }
      const normalized = part.content.trim();
      if (!normalized) return [];
      return [
        {
          kind: part.type === 'text' ? 'progress' : part.kind,
          content: normalized,
        },
      ];
    });
  const answer = parts
    .slice(lastActivityIndex + 1)
    .filter((part): part is Extract<AiMessagePart, { type: 'text' }> => part.type === 'text')
    .map((part) => part.content)
    .join('');

  return {
    processEntries,
    answer,
    activityCount: parts.filter((part) => part.type === 'activity' || part.type === 'execution')
      .length,
  };
}
