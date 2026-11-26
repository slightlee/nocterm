/** 菜单与工具栏统一显示英文档位；发送参数仍使用 CLI 原生值，不从文案反推。 */
export const REASONING_LABELS = {
  none: 'none',
  minimal: 'minimal',
  low: 'low',
  medium: 'medium',
  high: 'high',
  xhigh: 'xhigh',
  max: 'max',
  ultra: 'ultra',
} as const;

export type AiReasoningEffort = keyof typeof REASONING_LABELS;

export function isReasoningEffort(value: unknown): value is AiReasoningEffort {
  return typeof value === 'string' && Object.hasOwn(REASONING_LABELS, value);
}

/** 只采用能力列表中的档位；调用方可传模型默认值，未知能力不猜测。 */
export function compatibleReasoningEffort(
  supported: readonly AiReasoningEffort[] | undefined,
  effort: AiReasoningEffort | undefined,
  defaultEffort?: AiReasoningEffort
): AiReasoningEffort | undefined {
  if (effort && supported?.includes(effort)) return effort;
  return defaultEffort && supported?.includes(defaultEffort) ? defaultEffort : undefined;
}
