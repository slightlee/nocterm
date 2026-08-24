export type TabCloseAction = 'current' | 'others' | 'all';

/** 根据右键目标和操作类型计算待关闭标签，保持原始标签顺序。 */
export function resolveTabIdsToClose<T>(
  ids: readonly T[],
  targetId: T,
  action: TabCloseAction
): T[] {
  if (action === 'all') return [...ids];
  if (action === 'others') return ids.filter((id) => id !== targetId);
  return ids.includes(targetId) ? [targetId] : [];
}
