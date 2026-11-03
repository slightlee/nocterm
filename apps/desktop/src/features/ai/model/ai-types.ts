export type AiProviderId = 'codex' | 'claude-code' | 'grok';

/** 通用终端命令的会话级授权策略。未知值由后端拒绝，不提供全局永久放行。 */
export type AiCommandPolicy = 'deny_all' | 'confirm_each' | 'auto_safe' | 'full_access';

export interface AiProvider {
  id: AiProviderId;
  name: string;
  command: string;
  description: string;
}

export interface AiMessage {
  id: string;
  role: 'user' | 'assistant';
  content: string;
  /** 助手单轮内按到达顺序保存正文与工具活动；content 仍用于对话上下文。 */
  parts?: AiMessagePart[];
  createdAt: number;
}

export type AiMessagePart =
  | { type: 'text'; content: string }
  | { type: 'activity'; kind: 'tool'; content: string }
  | { type: 'execution'; execution: AiExecutionPresentation };

/**
 * 后端权威执行生命周期事件的投影。
 * 完整命令、参数和输出不属于该模型；前端只消费后端事件和快照查询。
 */
export interface AiToolExecutionEvent {
  executionId: string;
  sequence: number;
  providerSessionId: string;
  conversationId: string;
  targetKind: string;
  workspaceSessionId: string | null;
  terminalId: string | null;
  terminalGeneration: number | null;
  connectionId: number | null;
  tool: string;
  phase: AiExecutionPhase;
  approvalState: AiExecutionApprovalState;
  executionMode: AiExecutionMode;
  fallbackReason: string | null;
  resultState: AiExecutionResultState;
  dispatchReceipt: AiDispatchReceipt;
  exitCode: number | null;
  durationMs: number | null;
  errorCode: string | null;
  updatedAtMs: number;
}

export type AiExecutionPhase =
  | 'requested'
  | 'awaiting_approval'
  | 'authorized'
  | 'terminal_reserved'
  | 'dispatching'
  | 'dispatched'
  | 'completed'
  | 'failed'
  | 'denied'
  | 'cancelled'
  | 'timed_out'
  | 'result_unknown';

export type AiExecutionApprovalState =
  'not_requested' | 'not_required' | 'pending' | 'approved' | 'rejected' | 'timed_out' | 'revoked';

export type AiExecutionMode =
  | 'unselected'
  | 'local_visible_osc'
  | 'local_visible_marker'
  | 'ssh_visible_osc'
  | 'ssh_visible_marker'
  | 'ssh_independent_exec';

export type AiExecutionResultState =
  'pending' | 'succeeded' | 'failed' | 'denied' | 'cancelled' | 'timed_out' | 'unknown';

export type AiDispatchReceipt = 'not_started' | 'intent_persisted' | 'confirmed';

/**
 * 执行卡展示模型：从后端事件投影而来，前端不自行创建权威状态。
 */
export interface AiExecutionPresentation {
  executionId: string;
  sequence: number;
  phase: AiExecutionPhase;
  approvalState: AiExecutionApprovalState;
  executionMode: AiExecutionMode;
  fallbackReason: string | null;
  resultState: AiExecutionResultState;
  dispatchReceipt: AiDispatchReceipt;
  exitCode: number | null;
  durationMs: number | null;
  errorCode: string | null;
  tool: string;
  targetKind: string;
  updatedAtMs: number;
}

export interface AiOutputEvent {
  sessionId: string;
  connectionId: number | null;
  stream: 'stdout' | 'stderr';
  data: string;
}

export interface AiExitEvent {
  sessionId: string;
  connectionId: number | null;
  code: number | null;
  cancelled: boolean;
}

export interface AiToolApprovalEvent {
  approvalId: string;
  sessionId: string;
  targetKind: 'local' | 'ssh';
  targetLabel: string;
  command: string;
  expiresAtUnixMs: number;
}

export interface AiToolApprovalClosedEvent {
  approvalId: string;
  sessionId: string;
  resolution: 'approved' | 'rejected' | 'revoked' | 'timed_out';
}

export const AI_PROVIDERS: readonly AiProvider[] = [
  {
    id: 'codex',
    name: 'Codex',
    command: 'codex',
    description: '本机 Codex Agent',
  },
  {
    id: 'claude-code',
    name: 'Claude Code',
    command: 'claude',
    description: '本机 Claude Code Agent',
  },
  {
    id: 'grok',
    name: 'Grok',
    command: 'grok',
    description: '本机 Grok Agent',
  },
];
