import { aiConversationTargetKey, type AiConversationTarget } from './ai-target';
import type { AiMessagePart, AiProviderId, AiToolApprovalEvent } from './ai-types';

/** 运行任务和完成提醒共享的导航身份；不包含任何终端凭据或命令输出。 */
export interface AiTaskNavigationSource {
  sessionId: string;
  conversationId: string;
  target: AiConversationTarget;
  /** 用户启动任务时所在的终端 Tab；SSH 目标本身仍按 connectionId 共享。 */
  sourceTerminalSessionId: string;
  provider: AiProviderId;
}

/** 单个 Provider 任务的可展示快照；进程句柄和计时器仍由运行时 Hook 私有管理。 */
export interface AiRuntimeTask extends AiTaskNavigationSource {
  startedAt: number;
  streamText: string;
  runParts: AiMessagePart[];
  pendingApproval: AiToolApprovalEvent | null;
  approvalSubmitting: boolean;
  notice: string | null;
}

export type AiRuntimeTasks = Record<string, AiRuntimeTask>;

export type AiTaskOutcome = 'completed' | 'failed' | 'stopped' | 'timed_out';

/** 已结束任务的未读提醒；完整回答和错误详情仍只存在于对应 AI 会话中。 */
export interface AiCompletedTaskResult extends AiTaskNavigationSource {
  completedAt: number;
  outcome: AiTaskOutcome;
}

export type AiCompletedTaskResults = Record<string, AiCompletedTaskResult>;

export function completeAiRuntimeTask(
  task: AiRuntimeTask,
  outcome: AiTaskOutcome,
  completedAt: number
): AiCompletedTaskResult {
  return {
    sessionId: task.sessionId,
    conversationId: task.conversationId,
    target: task.target,
    sourceTerminalSessionId: task.sourceTerminalSessionId,
    provider: task.provider,
    completedAt,
    outcome,
  };
}

/** 同一会话只保留最新一条待查看结果，避免连续任务让入口无限增长。 */
export function recordCompletedAiTaskResult(
  results: AiCompletedTaskResults,
  result: AiCompletedTaskResult
): AiCompletedTaskResults {
  return { ...results, [result.conversationId]: result };
}

export function removeCompletedAiTaskResult(
  results: AiCompletedTaskResults,
  conversationId: string
): AiCompletedTaskResults {
  if (!results[conversationId]) return results;
  const next = { ...results };
  delete next[conversationId];
  return next;
}

export function listCompletedAiTaskResults(
  results: AiCompletedTaskResults
): AiCompletedTaskResult[] {
  return Object.values(results).sort((first, second) => second.completedAt - first.completedAt);
}

/** 同一目标只允许一个任务，避免两个 Agent 并发操作同一终端。 */
export function canStartAiRuntimeTask(
  tasks: AiRuntimeTasks,
  conversationId: string,
  target: AiConversationTarget
): boolean {
  const targetKey = aiConversationTargetKey(target);
  return !Object.values(tasks).some(
    (task) =>
      task.conversationId === conversationId || aiConversationTargetKey(task.target) === targetKey
  );
}

/** 所有 Provider 事件都必须通过 sessionId 定位，不能依赖当前可见 Tab。 */
export function updateAiRuntimeTask(
  tasks: AiRuntimeTasks,
  sessionId: string,
  patch: Partial<AiRuntimeTask>
): AiRuntimeTasks {
  const task = tasks[sessionId];
  if (!task) return tasks;
  return { ...tasks, [sessionId]: { ...task, ...patch, sessionId: task.sessionId } };
}

export function removeAiRuntimeTask(tasks: AiRuntimeTasks, sessionId: string): AiRuntimeTasks {
  if (!tasks[sessionId]) return tasks;
  const next = { ...tasks };
  delete next[sessionId];
  return next;
}

export function findAiRuntimeTaskByConversation(
  tasks: AiRuntimeTasks,
  conversationId: string | null
): AiRuntimeTask | null {
  if (!conversationId) return null;
  return Object.values(tasks).find((task) => task.conversationId === conversationId) ?? null;
}

export function findAiRuntimeTaskByTarget(
  tasks: AiRuntimeTasks,
  targetKey: string | null
): AiRuntimeTask | null {
  if (!targetKey) return null;
  return (
    Object.values(tasks).find((task) => aiConversationTargetKey(task.target) === targetKey) ?? null
  );
}

/** 标题栏入口只报告其他终端任务；当前目标的进度直接在对话区展示。 */
export function listBackgroundAiRuntimeTasks(
  tasks: AiRuntimeTasks,
  currentTargetKey: string | null,
  currentConversationId: string | null = null
): AiRuntimeTask[] {
  return Object.values(tasks)
    .filter((task) => {
      if (currentConversationId) return task.conversationId !== currentConversationId;
      return aiConversationTargetKey(task.target) !== currentTargetKey;
    })
    .sort((first, second) => first.startedAt - second.startedAt);
}
