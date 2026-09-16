/** Provider 输出是 JSONL 事件流；只提取面向用户的 Agent 文本，忽略生命周期事件。 */
export function extractAiText(line: string): string | null {
  const trimmed = line.trim();
  if (!trimmed) return null;
  let event: unknown;
  try {
    event = JSON.parse(trimmed);
  } catch {
    return line;
  }
  if (!isRecord(event)) return null;
  const acpUpdate = getAcpUpdate(event);
  if (
    acpUpdate?.sessionUpdate === 'agent_message_chunk' &&
    isRecord(acpUpdate.content) &&
    typeof acpUpdate.content.text === 'string'
  ) {
    return acpUpdate.content.text;
  }
  if (
    event.method === 'item/completed' &&
    isRecord(event.params) &&
    isRecord(event.params.item) &&
    event.params.item.type === 'agentMessage' &&
    typeof event.params.item.text === 'string'
  ) {
    return event.params.item.text;
  }
  if (typeof event.text === 'string') return event.text;
  // Grok streaming-json 使用 {type:"text",data:"..."}，不能只读取 Claude/Codex 的 text 字段。
  if (event.type === 'text' && typeof event.data === 'string') return event.data;
  if (event.type === 'error' && typeof event.message === 'string') return event.message;
  if (event.type === 'thought') return null;
  // Claude 的 result 事件会重复整段已通过 assistant 事件流出的全文，跳过以免答案出现两遍。
  if (event.type === 'result') return null;
  if (typeof event.result === 'string' && event.type === 'final') {
    return event.result;
  }
  const item = isRecord(event.item) ? event.item : event;
  if (item.type === 'agent_message' && typeof item.text === 'string') return item.text;
  if (item.type === 'assistant' && typeof item.message === 'string') return item.message;
  if (item.type === 'assistant' && isRecord(item.message))
    return textFromContent(item.message.content);
  if (event.type === 'assistant') return textFromContent(event.message);
  return null;
}

export interface AiActivity {
  kind: 'thinking' | 'tool';
  text: string;
}

export interface AiStreamDelta {
  answer?: string;
  thinking?: string;
}

/**
 * 提取增量流片段（--include-partial-messages 的 stream_event 事件）；
 * 非 stream_event 行返回 null，由整段事件的处理路径接手。
 */
export function extractAiStreamDelta(line: string): AiStreamDelta | null {
  const trimmed = line.trim();
  if (!trimmed) return null;
  let event: unknown;
  try {
    event = JSON.parse(trimmed);
  } catch {
    return null;
  }
  if (!isRecord(event)) return null;
  const acpUpdate = getAcpUpdate(event);
  if (
    (acpUpdate?.sessionUpdate === 'agent_message_chunk' ||
      acpUpdate?.sessionUpdate === 'agent_thought_chunk') &&
    isRecord(acpUpdate.content) &&
    typeof acpUpdate.content.text === 'string' &&
    acpUpdate.content.text
  ) {
    return acpUpdate.sessionUpdate === 'agent_message_chunk'
      ? { answer: acpUpdate.content.text }
      : { thinking: acpUpdate.content.text };
  }
  if (
    (event.method === 'item/agentMessage/delta' ||
      event.method === 'item/reasoning/summaryTextDelta') &&
    isRecord(event.params) &&
    typeof event.params.delta === 'string' &&
    event.params.delta
  ) {
    return event.method === 'item/agentMessage/delta'
      ? { answer: event.params.delta }
      : { thinking: event.params.delta };
  }
  // Grok 的 headless streaming-json 直接把增量放在 data 字段。
  if (event.type === 'text' && typeof event.data === 'string') {
    return event.data ? { answer: event.data } : null;
  }
  if (event.type === 'thought' && typeof event.data === 'string') {
    return event.data ? { thinking: event.data } : null;
  }
  if (event.type !== 'stream_event' || !isRecord(event.event)) return null;
  if (event.event.type !== 'content_block_delta' || !isRecord(event.event.delta)) return null;
  const delta = event.event.delta;
  if (delta.type === 'text_delta' && typeof delta.text === 'string') {
    return delta.text ? { answer: delta.text } : null;
  }
  if (delta.type === 'thinking_delta' && typeof delta.thinking === 'string') {
    return delta.thinking ? { thinking: delta.thinking } : null;
  }
  return null;
}

/**
 * 判断是否为携带整段文本的事件（assistant 消息 / result 结果）。
 * 增量已经流出全文时，这类事件会造成重复，由调用方跳过。
 */
export function isAiFullTextEvent(line: string): boolean {
  const trimmed = line.trim();
  if (!trimmed) return false;
  let event: unknown;
  try {
    event = JSON.parse(trimmed);
  } catch {
    return false;
  }
  return (
    isRecord(event) &&
    (event.type === 'assistant' ||
      event.type === 'result' ||
      (event.method === 'item/completed' &&
        isRecord(event.params) &&
        isRecord(event.params.item) &&
        event.params.item.type === 'agentMessage'))
  );
}

const AI_ACTIVITY_LIMIT = 160;

/** 过程信息只做摘要展示，超长思考或命令截断，避免等待区被刷屏。 */
function truncateActivity(text: string): string {
  const compact = text.replace(/\s+/g, ' ').trim();
  return compact.length > AI_ACTIVITY_LIMIT ? `${compact.slice(0, AI_ACTIVITY_LIMIT)}…` : compact;
}

function summarizeToolUse(part: Record<string, unknown>): string {
  const name = typeof part.name === 'string' && part.name.trim() ? part.name : 'tool';
  const input = isRecord(part.input) ? part.input : {};
  const noctermTool = name.startsWith('mcp__nocterm__')
    ? name.slice('mcp__nocterm__'.length)
    : null;
  if (noctermTool) return summarizeNoctermTool(noctermTool, input);
  const detail = [
    input.command,
    input.file_path,
    input.path,
    input.pattern,
    input.description,
  ].find((value): value is string => typeof value === 'string' && value.trim().length > 0);
  return truncateActivity(detail ? `${name} ${detail}` : name);
}

/** Nocterm 工具过程使用用户可理解的操作名；只展示已知、受约束的参数。 */
function summarizeNoctermTool(tool: string, input: Record<string, unknown>): string {
  const service = typeof input.service === 'string' ? input.service : '';
  const container = typeof input.container === 'string' ? input.container : '';
  const command = typeof input.command === 'string' ? input.command : '';
  const labels: Record<string, string> = {
    session_context: '确认当前终端目标',
    get_system_info: '读取服务器系统信息',
    list_processes: '查看服务器进程',
    list_listening_ports: '查看服务器监听端口',
    get_service_status: service ? `检查服务状态：${service}` : '检查服务状态',
    read_service_logs: service ? `读取服务日志：${service}` : '读取服务日志',
    get_disk_usage: '查看服务器磁盘使用量',
    get_memory_usage: '查看服务器内存使用量',
    list_docker_containers: '查看 Docker 容器',
    get_docker_info: '读取 Docker 配置',
    get_docker_container_status: container ? `检查容器状态：${container}` : '检查容器状态',
    read_docker_logs: container ? `读取容器日志：${container}` : '读取容器日志',
    ssh_exec: command ? `执行远程命令：${command}` : '执行远程命令',
    local_terminal_exec: command ? `执行本地命令：${command}` : '执行本地命令',
  };
  return truncateActivity(labels[tool] ?? `使用 Nocterm 工具：${tool}`);
}

/** Grok 只暴露两个聚合工具；优先从 use_tool 输入还原实际 Nocterm 操作。 */
function summarizeAcpToolCall(update: Record<string, unknown>): string {
  const title = typeof update.title === 'string' ? update.title.trim() : '';
  const normalizedTitle = title.toLowerCase();
  const rawInput = isRecord(update.rawInput)
    ? update.rawInput
    : isRecord(update.raw_input)
      ? update.raw_input
      : {};
  if (normalizedTitle.startsWith('search_tool')) return '查找可用终端能力';
  if (normalizedTitle.startsWith('use_tool')) {
    const tool = [rawInput.tool_name, rawInput.toolName, rawInput.name].find(
      (value): value is string => typeof value === 'string' && value.trim().length > 0
    );
    const input = [rawInput.arguments, rawInput.input].find(isRecord) ?? {};
    return tool ? summarizeNoctermTool(tool, input) : '执行终端操作';
  }
  return truncateActivity(title || '执行终端操作');
}

/** 提取等待期的过程信息（思考摘要、工具调用）；纯文本行或生命周期事件返回空数组。 */
export function extractAiActivities(line: string): AiActivity[] {
  const trimmed = line.trim();
  if (!trimmed) return [];
  let event: unknown;
  try {
    event = JSON.parse(trimmed);
  } catch {
    return [];
  }
  if (!isRecord(event)) return [];
  const activities: AiActivity[] = [];

  // Grok ACP：文本和思考走增量路径，工具开始事件固化为一条可读活动记录。
  const acpUpdate = getAcpUpdate(event);
  if (acpUpdate?.sessionUpdate === 'tool_call') {
    activities.push({ kind: 'tool', text: summarizeAcpToolCall(acpUpdate) });
    return activities;
  }

  // Codex app-server：事件包在 JSON-RPC params 中，item 类型使用 camelCase。
  if (event.method === 'item/started' && isRecord(event.params) && isRecord(event.params.item)) {
    const item = event.params.item;
    if (
      item.type === 'commandExecution' &&
      typeof item.command === 'string' &&
      item.command.trim()
    ) {
      activities.push({ kind: 'tool', text: truncateActivity(item.command) });
    }
    if (item.type === 'mcpToolCall') {
      const server = typeof item.server === 'string' ? item.server : 'MCP';
      const tool = typeof item.tool === 'string' ? item.tool : 'tool';
      const input = isRecord(item.arguments) ? item.arguments : {};
      activities.push({
        kind: 'tool',
        text:
          server === 'nocterm'
            ? summarizeNoctermTool(tool, input)
            : truncateActivity(`${server}/${tool}`),
      });
    }
    return activities;
  }

  // Claude stream-json：assistant 消息里的 thinking 与 tool_use 块；思考块固化展示，增量部分只负责实时滚动。
  if (
    event.type === 'assistant' &&
    isRecord(event.message) &&
    Array.isArray(event.message.content)
  ) {
    for (const part of event.message.content) {
      if (!isRecord(part)) continue;
      if (part.type === 'thinking' && typeof part.thinking === 'string' && part.thinking.trim()) {
        activities.push({ kind: 'thinking', text: truncateActivity(part.thinking) });
      }
      if (part.type === 'tool_use') {
        activities.push({ kind: 'tool', text: summarizeToolUse(part) });
      }
    }
    return activities;
  }

  // Codex JSONL：命令在 item.started 时即展示，推理条目完成后展示摘要。
  if (isRecord(event.item)) {
    const item = event.item;
    if (
      event.type === 'item.started' &&
      item.type === 'command_execution' &&
      typeof item.command === 'string' &&
      item.command.trim()
    ) {
      activities.push({ kind: 'tool', text: truncateActivity(item.command) });
    }
    if (
      event.type === 'item.completed' &&
      item.type === 'reasoning' &&
      typeof item.text === 'string' &&
      item.text.trim()
    ) {
      activities.push({ kind: 'thinking', text: truncateActivity(item.text) });
    }
    if (event.type === 'item.started' && item.type === 'mcp_tool_call') {
      const server = typeof item.server === 'string' ? item.server : 'MCP';
      const tool = typeof item.tool === 'string' ? item.tool : 'tool';
      const input = isRecord(item.arguments) ? item.arguments : {};
      activities.push({
        kind: 'tool',
        text:
          server === 'nocterm'
            ? summarizeNoctermTool(tool, input)
            : truncateActivity(`${server}/${tool}`),
      });
    }
  }
  return activities;
}

function textFromContent(content: unknown): string | null {
  if (typeof content === 'string') return content;
  if (!Array.isArray(content)) return null;
  const text = content
    .filter(isRecord)
    .filter((part) => part.type === 'text' && typeof part.text === 'string')
    .map((part) => part.text as string)
    .join('');
  return text || null;
}

function getAcpUpdate(event: Record<string, unknown>): Record<string, unknown> | null {
  if (event.method !== 'session/update' || !isRecord(event.params)) return null;
  return isRecord(event.params.update) ? event.params.update : null;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}
