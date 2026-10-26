//! AI 面板视图模型：从控制器编排逻辑中提取的纯函数，覆盖上下文文案、
//! 目标不可用原因与通知优先级。全部为无副作用映射，便于在 node 环境直测。

import type { AiConversation } from './ai-store';
import type { AiWorkspaceView } from './ai-target';
import type { TerminalSession } from '../../terminal';

/** 上下文栏左侧状态点的语义分组；渲染层映射为 CSS 类。 */
export type AiContextStatus =
  'mismatch' | 'sftp' | 'connected' | 'connecting' | 'error' | 'inactive';

/** 上下文栏的完整展示输入：会话、状态与当前查看的会话。 */
export interface AiContextBarInput {
  activeSession: TerminalSession | null;
  activeSessionStatus: string;
  activeView: AiWorkspaceView;
  activeConversation: AiConversation | null;
  viewingCurrentTarget: boolean;
}

/** 上下文栏展示：状态分组与面向用户的连接说明文案。 */
export function resolveAiContextBar(input: AiContextBarInput): {
  status: AiContextStatus;
  text: string;
} {
  const {
    activeSession,
    activeSessionStatus,
    activeView,
    activeConversation,
    viewingCurrentTarget,
  } = input;
  const status: AiContextStatus =
    activeConversation && !viewingCurrentTarget
      ? 'mismatch'
      : activeView === 'sftp'
        ? 'sftp'
        : activeSessionStatus === 'connected'
          ? 'connected'
          : activeSessionStatus === 'connecting'
            ? 'connecting'
            : activeSessionStatus === 'error'
              ? 'error'
              : 'inactive';
  const text = (() => {
    if (activeConversation && !viewingCurrentTarget) {
      return activeConversation.target
        ? `正在查看 ${activeConversation.target.label} 的会话 · 当前目标不匹配`
        : '正在查看未绑定历史 · 仅可查看';
    }
    if (activeView === 'sftp') return '文件会话 · 暂未接入 AI 执行上下文';
    if (activeSession && activeSessionStatus === 'connected') {
      return `已连接 ${activeSession.name}，可分析命令、日志与报错`;
    }
    if (activeSessionStatus === 'connecting') return '终端连接中，连接完成后可发送请求';
    if (activeSessionStatus === 'error') return '终端连接异常，请先恢复连接';
    return '连接终端后，可分析命令、日志与报错';
  })();
  return { status, text };
}

/** 目标不可用原因：composer 的状态提示与提交守卫共用同一文案。 */
export function resolveTargetUnavailableMessage(input: {
  activeConversation: AiConversation | null;
  activeSessionStatus: string;
  activeView: AiWorkspaceView;
  hasBinding: boolean;
  viewingCurrentTarget: boolean;
}): string {
  const { activeConversation, activeSessionStatus, activeView, hasBinding, viewingCurrentTarget } =
    input;
  if (activeConversation && !viewingCurrentTarget) {
    return activeConversation.target
      ? `该会话绑定 ${activeConversation.target.label}，请切回对应终端`
      : '旧版历史没有终端绑定，只可查看';
  }
  if (activeView === 'sftp') return '文件会话暂未接入 AI 执行上下文';
  if (!hasBinding) return '请先连接本地终端或远程服务器';
  if (activeSessionStatus === 'connecting') return '终端连接完成后才能发送请求';
  if (activeSessionStatus === 'error') return '请先恢复终端连接';
  return '当前终端不可用';
}

/** 面板通知的优先级链：运行任务 > 当前展示 > 任务导航 > 全局 > 面板级。 */
export function resolveVisibleNotice(input: {
  activeConversationId: string | null;
  activeTaskNotice: string | null | undefined;
  activePresentationNotice: string | null | undefined;
  globalNotice: string | null;
  panelNotice: string | null;
  taskNavigationNotice: { conversationId: string; message: string } | null;
}): string | null {
  const {
    activeConversationId,
    activeTaskNotice,
    activePresentationNotice,
    globalNotice,
    panelNotice,
    taskNavigationNotice,
  } = input;
  return (
    activeTaskNotice ??
    activePresentationNotice ??
    (taskNavigationNotice?.conversationId === activeConversationId
      ? taskNavigationNotice.message
      : null) ??
    globalNotice ??
    panelNotice
  );
}

/** 无草稿时的提问兜底文案：按附件组合选择最贴切的指令。 */
export function resolveFallbackQuestion(input: {
  hasTerminalSelection: boolean;
  fileName: string | null;
}): string {
  const { hasTerminalSelection, fileName } = input;
  if (hasTerminalSelection && fileName) return '请结合文件和终端选区分析问题。';
  if (hasTerminalSelection) return '请分析这段终端输出。';
  return `请分析附件：${fileName ?? '未命名文件'}`;
}
