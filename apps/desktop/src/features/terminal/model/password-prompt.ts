/**
 * 终端内口令提示符的纯状态机。
 *
 * 进程内 russh 必须在建连前拿到完整口令，所以口令由前端在终端里收集，
 * 再随 `ssh_terminal_open` 传给后端。输入**完全不回显**——连长度都不暴露，
 * 与 OpenSSH、PuTTY 的口令提示保持一致，因此这里只维护缓冲，不产出任何可渲染内容。
 *
 * 抽成纯函数是为了让按键语义（退格、Ctrl+C、Ctrl+U、方向键）可被单元测试覆盖，
 * 而不必在测试里驱动一个真实的 xterm 实例。
 */

export type PasswordPromptStatus = 'editing' | 'submitted' | 'cancelled';

export interface PasswordPromptState {
  /** 已累积的口令字符；`cancelled` 时清空，避免放弃后仍在内存里留下明文。 */
  value: string;
  status: PasswordPromptStatus;
}

export const EMPTY_PASSWORD_PROMPT: PasswordPromptState = { value: '', status: 'editing' };

/** 上限防止误粘贴大段文本（例如整个私钥文件）把缓冲撑爆。 */
const MAX_PASSWORD_LENGTH = 1024;

/** 把一段 xterm 输入折叠进提示状态；已结束的状态保持不变，便于调用方幂等处理。 */
export function reducePasswordPrompt(
  state: PasswordPromptState,
  data: string
): PasswordPromptState {
  if (state.status !== 'editing') return state;

  let value = state.value;
  let index = 0;
  while (index < data.length) {
    const char = data[index];
    index += 1;

    // 方向键、功能键等以 ESC 开头的序列必须整段跳过，
    // 否则 "\x1b[A" 里的 "[" 和 "A" 会被当成口令字符混进缓冲。
    if (char === '\x1b') {
      index = skipEscapeSequence(data, index);
      continue;
    }
    if (char === '\r' || char === '\n') return { value, status: 'submitted' };
    // Ctrl+C / Ctrl+D 放弃输入，与命令行下的习惯一致。
    if (char === '\x03' || char === '\x04') return { value: '', status: 'cancelled' };
    if (char === '\x7f' || char === '\b') {
      value = value.slice(0, -1);
      continue;
    }
    // Ctrl+U 清空整行，方便输错后重来。
    if (char === '\x15') {
      value = '';
      continue;
    }
    // 其余控制字符（Tab、未映射的 Ctrl 组合键）不参与口令。
    if (char < ' ') continue;
    if (value.length < MAX_PASSWORD_LENGTH) value += char;
  }
  return { value, status: 'editing' };
}

/** 跳过 ESC 之后的整段序列：CSI/SS3 以 @–~ 收尾，其余按两字符序列处理。 */
function skipEscapeSequence(data: string, start: number): number {
  const introducer = data[start];
  if (introducer !== '[' && introducer !== 'O') return start + 1;
  let index = start + 1;
  while (index < data.length) {
    const char = data[index];
    index += 1;
    if (char >= '@' && char <= '~') break;
  }
  return index;
}
