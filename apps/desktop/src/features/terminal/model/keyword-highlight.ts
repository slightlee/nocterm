/**
 * 客户端关键词高亮（对齐 MobaXterm Syntax highlighting 的核心能力）：
 * 对终端输出流中的纯文本段做正则匹配并本地着色，不依赖服务器输出颜色码。
 *
 * 设计约束：
 * - **ANSI 感知**：转义序列（CSI/OSC/单字符）原样透传，绝不对其内容做匹配，
 *   也不在序列中间注入任何字符；
 * - **不覆盖服务器颜色**：当前前景色非默认的文本段跳过匹配——服务器已经
 *   着色的内容（如彩色 `ls`）保持原样，避免双重着色；
 * - **状态可恢复**：注入高亮前记录当前 SGR 状态（粗体/前景/背景），匹配
 *   结束后恢复，避免 `\x1b[0m` 把后续文本的既有属性一并清掉；
 * - **块边界安全**：写入块末尾若存在不完整转义序列，缓存到下一次调用拼接；
 *   同时块尾可能被截断的可匹配 token（见 MAX_HOLD）也会缓存，保证 SSH 任意
 *   字节分块下着色结果与整段输入完全一致。
 */

import { resolveHighlightRoles } from './highlight-roles';
import type {
  HighlightPreset,
  HighlightRoleId,
  HighlightSlotMap,
} from '../../settings/types/settings-types';

/**
 * 角色颜色码集合：由预设 + 用户覆盖解析而来（见 highlight-roles.ts）。
 * 默认（theme 预设、无覆盖）与配置化前的常量逐字节一致。
 */
interface RoleCodes {
  error: string;
  warn: string;
  success: string;
  info: string;
  host: string;
  userhost: string;
  promptPath: string;
  perms: string;
  date: string;
  dir: string;
  link: string;
  device: string;
  exec: string;
  archive: string;
  log: string;
  config: string;
  script: string;
  media: string;
}

const ROLE_CODE_KEYS: Record<HighlightRoleId, keyof RoleCodes> = {
  kw_error: 'error',
  kw_warn: 'warn',
  kw_success: 'success',
  kw_info: 'info',
  ip: 'host',
  prompt_user_host: 'userhost',
  prompt_path: 'promptPath',
  permissions: 'perms',
  date: 'date',
  directory: 'dir',
  executable: 'exec',
  symlink: 'link',
  device: 'device',
  archive: 'archive',
  log: 'log',
  config: 'config',
  script: 'script',
  media: 'media',
};

function buildRoleCodes(preset: HighlightPreset, overrides: HighlightSlotMap): RoleCodes {
  const resolved = resolveHighlightRoles(preset, overrides);
  const entries = (Object.keys(ROLE_CODE_KEYS) as HighlightRoleId[]).map((role) => [
    ROLE_CODE_KEYS[role],
    resolved[role].code,
  ]);
  return Object.fromEntries(entries) as unknown as RoleCodes;
}

const ERROR_PATTERN = String.raw`\b(?:error|failed|failure|fatal|critical|denied|refused|invalid|unsupported|segfault|corrupt(?:ed)?|crash(?:ed)?)\b`;
const WARN_PATTERN = String.raw`\b(?:warning|warn|caution|cannot|unable|deprecated|missing|skipped|not found)\b`;
const SUCCESS_PATTERN = String.raw`\b(?:success(?:ful|fully)?|succeeded|done|ok|okay|connected|listening|started|completed|passed|active|enabled)\b`;
const INFO_PATTERN = String.raw`\b(?:info|notice|starting|loading|creating|building)\b`;
/** IPv4（可带端口）；主机名规则误报率过高，v1 只收 IP。 */
const HOST_PATTERN = String.raw`\b\d{1,3}(?:\.\d{1,3}){3}(?::\d{1,5})?\b`;

/**
 * 远程提示符与 `ls -l` 元数据的客户端着色（对齐 MobaXterm Syntax highlighting）：
 * 不向远程 Shell 注入任何命令，纯靠本地正则渲染——user@host:path 提示符、
 * 权限位块、修改日期在连接的任意阶段都能上色，且不产生终端可见输出。
 * `ls -l` 行还会按权限位首字符给文件名上类型色：目录蓝 / 可执行绿 /
 * 链接洋红 / 设备黄（对齐 MobaXterm 与经典 dircolors 语义）。
 * 各角色的具体颜色码由 highlight-roles.ts 的预设 + 用户覆盖解析而来。
 */
/** 默认状态复位（renderPlain 仅在默认前景/非粗体时调用）。 */
const CODE_RESET = '\x1b[0m';

/** user@host，可选 `:~/path` 路径段（以 ~ 或 / 开头以排除 host:port 误报）。 */
const USERHOST_PATTERN = String.raw`\b[\w.-]+@[\w.-]+(?::(?:~|/)[\w./~-]*)?`;
/** `ls -l` 权限位：类型符 + 9 个 rwx 属性字符。 */
const PERMS_PATTERN = String.raw`[-dlbcps][rwxsStT-]{9}`;
/** `ls -l` 修改日期：`Apr 22 2024` 或 `Apr 22 09:15`。 */
const DATE_PATTERN = String.raw`\b(?:Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec)\s+\d{1,2}\s+(?:\d{1,2}:\d{2}|\d{4})\b`;

/**
 * `ls -l` 长格式行：perms links owner group size date name。
 * size 用 `\S+` 兼容 GNU 设备行的 `8,` 写法（此时日期匹配失败整行退回
 * 通用规则，权限位与日期仍会着色，仅文件名不上类型色）。
 */
const LS_LONG_LINE_REGEX = new RegExp(
  String.raw`^([-dlbcps][rwxsStT-]{9})(\s+)(\S+)(\s+)(\S+)(\s+)(\S+)(\s+)(\S+)(\s+)((?:Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec)\s+\d{1,2}\s+(?:\d{1,2}:\d{2}|\d{4}))(\s+)(.*)$`
);

/**
 * 按权限位首字符映射文件类型角色；普通非可执行文件返回 null（不抢色）。
 * codes 由闭包内传入（随用户配置变化）。
 */
const lsTypeRole = (perms: string): keyof RoleCodes | null => {
  const type = perms[0];
  if (type === 'd') return 'dir';
  if (type === 'l') return 'link';
  if (type === 'b' || type === 'c') return 'device';
  if (type === '-' && /[xsStT]/.test(perms.slice(1))) return 'exec';
  return null;
};

/**
 * 文件名按扩展名着色：客户端侧补足服务器未配置 LS_COLORS 的场景，
 * 压缩包/日志/配置/脚本/媒体各自一色，与关键词规则互不重叠。
 * 用非加粗基色，与关键词的加粗亮色在视觉上分层。
 */

const FILENAME_PREFIX = String.raw`\b[\w@+.-]+`;
const ARCHIVE_PATTERN = `${FILENAME_PREFIX}\\.(?:tar\\.gz|tgz|zip|gz|bz2|xz|7z|rar|jar)`;
const LOG_PATTERN = `${FILENAME_PREFIX}\\.log`;
const CONFIG_PATTERN = `${FILENAME_PREFIX}\\.(?:conf|cfg|ini|ya?ml|json|toml|xml|properties|env)`;
const SCRIPT_PATTERN = `${FILENAME_PREFIX}\\.(?:sh|bash|zsh|py|pl|rb|js|ts|sql)`;
const MEDIA_PATTERN = `${FILENAME_PREFIX}\\.(?:jpe?g|png|gif|svg|bmp|webp|mp3|flac|wav|ogg|mp4|mkv|avi|mov|pdf)`;

const HIGHLIGHT_REGEX = new RegExp(
  `(${ARCHIVE_PATTERN})|(${LOG_PATTERN})|(${CONFIG_PATTERN})|(${SCRIPT_PATTERN})|(${MEDIA_PATTERN})|(${ERROR_PATTERN})|(${WARN_PATTERN})|(${SUCCESS_PATTERN})|(${INFO_PATTERN})|(${HOST_PATTERN})|(${USERHOST_PATTERN})|(${PERMS_PATTERN})|(${DATE_PATTERN})`,
  'gi'
);
/**
 * 正则组序（1 基）对应 RoleCodes 键：archive, log, config, script, media,
 * error, warn, success, info, host(ip), userhost, perms, date。
 * 顺序即优先级——文件类型在前，避免 error.log 之类被关键词规则拆开上色。
 */
const GROUP_ROLES: (keyof RoleCodes)[] = [
  'archive',
  'log',
  'config',
  'script',
  'media',
  'error',
  'warn',
  'success',
  'info',
  'host',
  'userhost',
  'perms',
  'date',
];

/** SGR 子集状态：仅跟踪注入恢复所需的最小信息。 */
interface SgrState {
  bold: boolean;
  /** 前景色参数串（如 `31`、`38;5;123`），null 表示默认前景。 */
  fg: string | null;
  /** 背景色参数串，null 表示默认背景。 */
  bg: string | null;
}

const DEFAULT_STATE: SgrState = { bold: false, fg: null, bg: null };

/**
 * 尾部保持（chunk 边界修复）：SSH 输出按任意字节边界分块到达，若块尾恰好
 * 截断一个可匹配 token（文件名/关键词/IP/提示符），立即冲刷会导致匹配失败、
 * 着色随机丢失。因此块尾由这些字符构成的连续段先缓存，等下一个块拼全再匹配。
 * 所有规则的 token 都不含空白，遇到空白/换行/其他符号立即冲刷，无渲染延迟。
 */
const HOLD_CHAR = /[A-Za-z0-9_.@+:~-]/;
const MAX_HOLD = 256;

function applySgrParams(params: number[], state: SgrState): void {
  let index = 0;
  while (index < params.length) {
    const value = params[index];
    if (value === 0) {
      state.bold = false;
      state.fg = null;
      state.bg = null;
    } else if (value === 1) {
      state.bold = true;
    } else if (value === 2 || value === 21 || value === 22) {
      state.bold = false;
    } else if ((value >= 30 && value <= 37) || (value >= 90 && value <= 97)) {
      state.fg = String(value);
    } else if (value === 38 || value === 48) {
      // 扩展色：38;5;N 或 38;2;r;g;b；参数缺失时终止解析，交由真实终端容错。
      const mode = params[index + 1];
      if (mode === 5) {
        const target = `${value};5;${params[index + 2] ?? ''}`;
        if (value === 38) state.fg = target;
        else state.bg = target;
        index += 3;
        continue;
      }
      if (mode === 2) {
        const target = `${value};2;${params[index + 2] ?? ''};${params[index + 3] ?? ''};${params[index + 4] ?? ''}`;
        if (value === 38) state.fg = target;
        else state.bg = target;
        index += 5;
        continue;
      }
    } else if (value === 39) {
      state.fg = null;
    } else if (value === 49) {
      state.bg = null;
    }
    index += 1;
  }
}

function restoreSequence(state: SgrState): string {
  const parts: string[] = [];
  if (state.bold) parts.push('1');
  if (state.fg) parts.push(state.fg);
  if (state.bg) parts.push(state.bg);
  if (parts.length === 0) return '\x1b[0m';
  return `\x1b[0;${parts.join(';')}m`;
}

export interface KeywordHighlighter {
  transform(data: string): string;
  /** 更新角色配色（预设 + 覆盖），立即对后续输出生效，不中断当前会话。 */
  setHighlight(preset: HighlightPreset, overrides: HighlightSlotMap): void;
  /** 释放延迟冲刷定时器，并把仍持有的尾部文本经 onDeferred 兑现。 */
  dispose(): void;
}

export interface KeywordHighlighterOptions {
  /**
   * 尾部持有文本的延迟冲刷回调。交互式输入（每个按键一个小块）依赖它：
   * 没有后续数据拼全 token 时，持有部分在 deferMs 内自动送达终端，
   * 否则用户键入的回显会被一直扣住、直到回车才显示（表现为"输入不了"）。
   */
  onDeferred?: (text: string) => void;
  /** 延迟毫秒数，默认 16（一帧内，肉眼无感）。 */
  deferMs?: number;
  /** 角色配色：预设 + 用户覆盖；缺省为经典预设且无覆盖（即出厂行为）。 */
  highlight?: { preset: HighlightPreset; overrides?: HighlightSlotMap };
}

export function createKeywordHighlighter(
  options: KeywordHighlighterOptions = {}
): KeywordHighlighter {
  const { onDeferred, deferMs = 16 } = options;
  let codes: RoleCodes = buildRoleCodes(
    options.highlight?.preset ?? 'theme',
    options.highlight?.overrides ?? {}
  );
  let carry = '';
  // carry 的类别：token 持有（可安全延迟冲刷）或未完转义序列（不可冲刷）。
  let holdIsToken = false;
  let holdTimer: ReturnType<typeof setTimeout> | null = null;
  let disposed = false;
  const state: SgrState = { ...DEFAULT_STATE };

  const setHighlight = (preset: HighlightPreset, overrides: HighlightSlotMap): void => {
    codes = buildRoleCodes(preset, overrides);
  };

  const scheduleHold = () => {
    if (holdTimer !== null || !onDeferred || disposed) return;
    holdTimer = setTimeout(() => {
      holdTimer = null;
      if (holdIsToken && carry.length > 0) {
        const held = carry;
        carry = '';
        onDeferred(held);
      }
      holdIsToken = false;
    }, deferMs);
  };

  const transform = (data: string): string => {
    holdIsToken = false;
    const input = carry + data;
    carry = '';

    let output = '';
    let plainStart = 0;
    let index = 0;

    const flushPlain = (end: number) => {
      if (end <= plainStart) return;
      output += renderPlain(input.slice(plainStart, end));
    };

    while (index < input.length) {
      const char = input[index];
      if (char !== '\x1b') {
        index += 1;
        continue;
      }
      const next = input[index + 1];
      if (next === '[') {
        // CSI：参数字节 0x30-0x3F，中间字节 0x20-0x2F，终结字节 0x40-0x7E。
        let cursor = index + 2;
        while (cursor < input.length) {
          const code = input.charCodeAt(cursor);
          if (code >= 0x40 && code <= 0x7e) break;
          cursor += 1;
        }
        if (cursor >= input.length) {
          // 序列被块边界截断：先冲刷序列前的纯文本，再缓存等待下一个块。
          flushPlain(index);
          carry = input.slice(index);
          break;
        }
        const final = input[cursor];
        if (final === 'm') {
          const raw = input.slice(index + 2, cursor);
          const params = raw
            .split(';')
            .map((part) => (part === '' ? 0 : Number.parseInt(part, 10)))
            .filter((value) => Number.isFinite(value));
          // 先用变更前的状态渲染序列之前的纯文本，再应用本次 SGR。
          flushPlain(index);
          applySgrParams(params, state);
        } else {
          flushPlain(index);
        }
        output += input.slice(index, cursor + 1);
        index = cursor + 1;
        plainStart = index;
        continue;
      }
      if (next === ']') {
        // OSC：以 BEL 或 ST（ESC \）结束。
        let cursor = index + 2;
        let terminatorLength = 1;
        let terminated = false;
        while (cursor < input.length) {
          if (input[cursor] === '\x07') {
            terminated = true;
            break;
          }
          if (input[cursor] === '\x1b' && input[cursor + 1] === '\\') {
            terminated = true;
            terminatorLength = 2;
            break;
          }
          cursor += 1;
        }
        if (!terminated) {
          flushPlain(index);
          carry = input.slice(index);
          break;
        }
        flushPlain(index);
        output += input.slice(index, cursor + terminatorLength);
        index = cursor + terminatorLength;
        plainStart = index;
        continue;
      }
      if (next !== undefined) {
        // 其他两字符转义序列（如 \x1b=、\x1b>）。
        flushPlain(index);
        output += input.slice(index, index + 2);
        index += 2;
        plainStart = index;
        continue;
      }
      // 末尾孤立的 ESC：先冲刷纯文本，再按截断处理。
      flushPlain(index);
      carry = input.slice(index);
      break;
    }

    if (carry.length === 0) {
      // 无未完序列时，把块尾可能是 token 前缀的部分留到下一块；
      // 交互输入没有"下一块"，交给延迟冲刷兜底。
      let holdStart = input.length;
      while (holdStart > plainStart && HOLD_CHAR.test(input[holdStart - 1])) holdStart -= 1;
      const holdLength = input.length - holdStart;
      if (holdLength > 0 && holdLength <= MAX_HOLD) {
        flushPlain(holdStart);
        carry = input.slice(holdStart);
        holdIsToken = true;
        scheduleHold();
      } else {
        flushPlain(input.length);
      }
    }
    return output;
  };

  const dispose = () => {
    disposed = true;
    if (holdTimer !== null) {
      clearTimeout(holdTimer);
      holdTimer = null;
    }
    if (holdIsToken && carry.length > 0 && onDeferred) {
      const held = carry;
      carry = '';
      onDeferred(held);
    }
    holdIsToken = false;
  };

  /** 对一段纯文本做关键词匹配与着色。前景色非默认时整段透传。 */
  const renderPlain = (text: string): string => {
    if (state.fg !== null || state.bold) return text;
    if (!/[A-Za-z]/.test(text)) return text;
    return text.split('\n').map(renderLine).join('\n');
  };

  /** HIGHLIGHT_REGEX 中 userhost 组的位置（1 基）：需要按 `:` 拆段配色。 */
  const USERHOST_GROUP = 11;

  /** 提示符两段配色：`user@host` 一色 + `:路径` 一色；无路径时整串一色。 */
  const renderUserHost = (match: string): string => {
    const colonIndex = match.indexOf(':');
    if (colonIndex === -1) {
      return `${codes.userhost}${match}${restoreSequence(state)}`;
    }
    return (
      `${codes.userhost}${match.slice(0, colonIndex)}${CODE_RESET}` +
      `${codes.promptPath}${match.slice(colonIndex)}${restoreSequence(state)}`
    );
  };

  /** 通用着色：关键词 / IP / 提示符 / 权限位 / 日期 / 扩展名。 */
  const renderKeywords = (text: string): string =>
    text.replace(HIGHLIGHT_REGEX, (match, ...groups) => {
      // 命中的组索引：archive=1, log=2, config=3, script=4, media=5, error=6,
      // warn=7, success=8, info=9, host=10, userhost=11。
      for (let group = 1; group <= GROUP_ROLES.length; group += 1) {
        if (groups[group - 1] !== undefined) {
          if (group === USERHOST_GROUP) return renderUserHost(match);
          return `${codes[GROUP_ROLES[group - 1]]}${match}${restoreSequence(state)}`;
        }
      }
      return match;
    });

  const renderLine = (line: string): string => {
    const match = LS_LONG_LINE_REGEX.exec(line);
    if (!match) return renderKeywords(line);
    const [, perms, ws1, links, ws2, owner, ws3, group, ws4, size, ws5, date, ws6, rest] = match;
    // 符号链接：只给链接名上类型色，` -> 目标` 保持默认色。
    const arrowIndex = rest.indexOf(' -> ');
    const name = arrowIndex === -1 ? rest : rest.slice(0, arrowIndex);
    const target = arrowIndex === -1 ? '' : rest.slice(arrowIndex);
    const typeRole = lsTypeRole(perms);
    // 类型色优先于扩展名色（目录 backup.tar.gz 也应是目录蓝）；
    // 普通文件没有类型色，扩展名规则仍生效。
    const namePart = typeRole ? `${codes[typeRole]}${name}${CODE_RESET}` : renderKeywords(name);
    return (
      `${codes.perms}${perms}${CODE_RESET}${ws1}${links}${ws2}${owner}${ws3}${group}` +
      `${ws4}${size}${ws5}${codes.date}${date}${CODE_RESET}${ws6}${namePart}${target}`
    );
  };

  return { transform, setHighlight, dispose };
}
