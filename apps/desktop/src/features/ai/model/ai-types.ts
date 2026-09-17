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
  { type: 'text'; content: string } | { type: 'activity'; kind: 'tool'; content: string };

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
