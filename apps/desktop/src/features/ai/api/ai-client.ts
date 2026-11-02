import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { openUrl } from '@tauri-apps/plugin-opener';

import { isDesktopRuntime } from '../../../shared/lib/tauri-runtime';
import type {
  AiExitEvent,
  AiCommandPolicy,
  AiMessage,
  AiOutputEvent,
  AiProviderId,
  AiToolApprovalClosedEvent,
  AiToolApprovalEvent,
  AiToolExecutionEvent,
} from '../model/ai-types';

export interface AiProviderStatus {
  id: AiProviderId;
  command: string;
  available: boolean;
}

export function getAiProviderStatus() {
  return invoke<AiProviderStatus[]>('ai_provider_status');
}

/** 消息内链接交给系统浏览器打开，避免桌面端 WebView 被导航走；浏览器预览退化为新标签页。 */
export async function openAiExternalLink(url: string): Promise<void> {
  if (isDesktopRuntime()) {
    await openUrl(url);
    return;
  }
  window.open(url, '_blank', 'noopener,noreferrer');
}

export function startAiSession(
  clientSessionId: string,
  conversationId: string,
  provider: AiProviderId,
  prompt: string,
  continuationPrompt: string,
  workingDirectory?: string,
  connectionId?: number,
  targetSessionId?: string,
  commandPolicy: AiCommandPolicy = 'auto_safe'
) {
  return invoke<{ sessionId: string }>('ai_session_start', {
    request: {
      clientSessionId,
      conversationId,
      provider,
      prompt,
      continuationPrompt,
      workingDirectory,
      connectionId,
      targetSessionId,
      commandPolicy,
    },
  });
}

export function stopAiSession(sessionId: string) {
  return invoke<void>('ai_session_stop', { sessionId });
}

export function resetAiConversation(conversationId: string) {
  return invoke<void>('ai_conversation_reset', { conversationId });
}

/** 持久化层的单条消息快照；parts 由调用方序列化为 JSON 字符串。 */
export interface AiMessageSnapshot {
  id: string;
  role: AiMessage['role'];
  content: string;
  partsJson: string | null;
  createdAtMs: number;
}

/** 一次 AI 会话的完整快照，保存请求与查询响应共用同一结构。 */
export interface AiConversationSnapshot {
  id: string;
  title: string;
  provider: AiProviderId;
  commandPolicy: string | null;
  targetKind: 'local' | 'ssh' | null;
  targetId: string | null;
  targetLabel: string | null;
  createdAtMs: number;
  updatedAtMs: number;
  messages: AiMessageSnapshot[];
}

/** 读取全部已持久化会话；失败由调用方决定是否降级为纯内存模式。 */
export function listAiConversations() {
  return invoke<AiConversationSnapshot[]>('ai_conversation_list');
}

export function saveAiConversation(conversation: AiConversationSnapshot) {
  return invoke<void>('ai_conversation_save', { conversation });
}

export function deleteSavedAiConversation(conversationId: string) {
  return invoke<void>('ai_conversation_delete', { conversationId });
}

export function onAiOutput(handler: (event: AiOutputEvent) => void): Promise<UnlistenFn> {
  return listen<AiOutputEvent>('nocterm://ai-output', (event) => handler(event.payload));
}

export function onAiExit(handler: (event: AiExitEvent) => void): Promise<UnlistenFn> {
  return listen<AiExitEvent>('nocterm://ai-exit', (event) => handler(event.payload));
}

export function onAiToolApproval(
  handler: (event: AiToolApprovalEvent) => void
): Promise<UnlistenFn> {
  return listen<AiToolApprovalEvent>('nocterm://ai-tool-approval', (event) =>
    handler(event.payload)
  );
}

export function onAiToolApprovalClosed(
  handler: (event: AiToolApprovalClosedEvent) => void
): Promise<UnlistenFn> {
  return listen<AiToolApprovalClosedEvent>('nocterm://ai-tool-approval-closed', (event) =>
    handler(event.payload)
  );
}

export function resolveAiToolApproval(approvalId: string, sessionId: string, approved: boolean) {
  return invoke<void>('ai_tool_approval_resolve', { approvalId, sessionId, approved });
}

/** 当前 SSH 连接是否启用终端可见执行；默认开启，可按连接切换为隔离执行。 */
export function getAiSshVisibleEnabled(connectionId: number): Promise<boolean> {
  return invoke<boolean>('ai_ssh_visible_get', { connectionId });
}

export function setAiSshVisibleEnabled(connectionId: number, enabled: boolean): Promise<void> {
  return invoke<void>('ai_ssh_visible_set', { connectionId, enabled });
}

/** 监听后端权威执行生命周期事件；事件可丢失，快照查询是恢复时的权威来源。 */
export function onAiToolExecution(
  handler: (event: AiToolExecutionEvent) => void
): Promise<UnlistenFn> {
  return listen<AiToolExecutionEvent>('nocterm://ai-tool-execution', (event) =>
    handler(event.payload)
  );
}

/** 事件丢失或面板重展开时，按对话或 Provider 会话查询后端权威执行快照。 */
export function listAiToolExecutions(
  conversationId?: string,
  providerSessionId?: string
): Promise<AiToolExecutionEvent[]> {
  return invoke<AiToolExecutionEvent[]>('ai_tool_execution_list', {
    conversationId,
    providerSessionId,
  });
}
