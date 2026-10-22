import type { TerminalSession, TerminalSessionId } from '../../terminal';
import {
  findAiRuntimeTaskByTarget,
  type AiRuntimeTasks,
  type AiTaskNavigationSource,
} from './ai-runtime-task';
import { aiConversationTargetKey } from './ai-target';

/** 空白会话只在有明确终端目标且该目标没有 AI 任务时开放。 */
export function canBeginAiConversation(targetKey: string | null, tasks: AiRuntimeTasks): boolean {
  return Boolean(targetKey) && findAiRuntimeTaskByTarget(tasks, targetKey) === null;
}

export interface AiTaskNavigationDestination {
  conversationId: string;
  targetKey: string;
  terminalSessionId: TerminalSessionId | null;
}

function terminalMatchesTask(session: TerminalSession, task: AiTaskNavigationSource): boolean {
  if (task.target.kind === 'local') {
    return session.kind === 'local' && session.id === task.target.id;
  }
  return session.kind === 'remote' && String(session.connectionId) === task.target.id;
}

/**
 * 任务入口优先返回发起 Tab；仅 SSH 可退回到同连接最近仍存在的 Tab。
 * 这里绝不创建会话，找不到时由面板停留在任务视图并提示用户。
 */
export function resolveAiTaskNavigation(
  task: AiTaskNavigationSource,
  sessions: readonly TerminalSession[],
  recentRemoteSessionIds: Readonly<Record<string, readonly string[]>>
): AiTaskNavigationDestination {
  const sourceSession = sessions.find(
    (session) => session.id === task.sourceTerminalSessionId && terminalMatchesTask(session, task)
  );
  if (sourceSession) {
    return {
      conversationId: task.conversationId,
      targetKey: aiConversationTargetKey(task.target),
      terminalSessionId: sourceSession.id,
    };
  }

  if (task.target.kind === 'ssh') {
    const recentIds = recentRemoteSessionIds[task.target.id] ?? [];
    for (const sessionId of recentIds) {
      const recentSession = sessions.find(
        (session) => session.id === sessionId && terminalMatchesTask(session, task)
      );
      if (recentSession) {
        return {
          conversationId: task.conversationId,
          targetKey: aiConversationTargetKey(task.target),
          terminalSessionId: recentSession.id,
        };
      }
    }

    // 兼容旧状态未记录最近顺序的情况，仍只选择已经存在的同连接终端。
    for (let index = sessions.length - 1; index >= 0; index -= 1) {
      const session = sessions[index];
      if (session && terminalMatchesTask(session, task)) {
        return {
          conversationId: task.conversationId,
          targetKey: aiConversationTargetKey(task.target),
          terminalSessionId: session.id,
        };
      }
    }
  }

  return {
    conversationId: task.conversationId,
    targetKey: aiConversationTargetKey(task.target),
    terminalSessionId: null,
  };
}
