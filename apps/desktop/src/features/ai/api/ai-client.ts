import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { openUrl } from '@tauri-apps/plugin-opener';

import { isDesktopRuntime } from '../../../shared/lib/tauri-runtime';
import type {
  AiExitEvent,
  AiCommandPolicy,
  AiOutputEvent,
  AiProviderId,
  AiToolApprovalClosedEvent,
  AiToolApprovalEvent,
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
