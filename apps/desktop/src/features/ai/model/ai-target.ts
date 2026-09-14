type AiBindableSession =
  | { id: number; kind: 'remote'; name: string; username: string; host: string }
  | { id: string; kind: 'local'; name: string };

export interface AiTerminalTarget {
  context: string;
  connectionId?: number;
  targetSessionId?: string;
}

/** 仅绑定后端已确认 connected 的目标，标签存在本身不代表 SSH 或 PTY 已就绪。 */
export function resolveAiTerminalTarget(
  session: AiBindableSession | null,
  status: string
): AiTerminalTarget {
  if (!session || status !== 'connected') return { context: '' };
  if (session.kind === 'remote') {
    return {
      connectionId: session.id,
      context: `当前 SSH 连接：${session.name}（${session.username}@${session.host}）。查询主机、进程、端口、服务、日志、磁盘、内存或 Docker 时优先使用 Nocterm 提供的结构化检查能力，其他命令使用 Nocterm 的远程终端能力，由 Nocterm 按当前权限策略决定是否确认。远程执行使用独立 Shell，不要假设它继承可见终端中的 cd、export 或虚拟环境；需要目录上下文时先查询并在后续命令中显式 cd。不要在本机 Shell 中代替执行；如果 Nocterm 工具不可用，直接报告错误，不要尝试浏览器、GUI 或其他 Shell。面向用户说明操作时使用自然语言，不要提及 MCP、函数名或 Nocterm 内部工具名。\n\n`,
    };
  }
  return {
    targetSessionId: session.id,
    context: `当前 Nocterm 本地终端：${session.name}。涉及主机、目录、文件、进程、端口、服务、日志、安装或部署时，必须先实际调用 Nocterm 提供的当前终端能力，由 Nocterm 按当前权限策略决定是否确认。命令和输出需要留在当前终端中可见，不要使用 Provider 自带 Shell、其他本机工具、浏览器或 GUI 代替；只有 Nocterm 工具调用真实返回错误后才能报告不可用。面向用户说明操作时使用自然语言，不要提及 MCP、函数名或 Nocterm 内部工具名。\n\n`,
  };
}
