/** 高亮仅决定展示方式，不参与可编辑性、文件编码或保存内容的判断。 */
export type EditorLanguage =
  | 'json'
  | 'yaml'
  | 'toml'
  | 'ini'
  | 'shell'
  | 'dockerfile'
  | 'javascript'
  | 'jsx'
  | 'typescript'
  | 'tsx'
  | 'python';

const EXTENSIONS: Readonly<Record<string, EditorLanguage>> = {
  json: 'json',
  yaml: 'yaml',
  yml: 'yaml',
  toml: 'toml',
  ini: 'ini',
  cfg: 'ini',
  properties: 'ini',
  sh: 'shell',
  bash: 'shell',
  zsh: 'shell',
  ksh: 'shell',
  js: 'javascript',
  mjs: 'javascript',
  cjs: 'javascript',
  jsx: 'jsx',
  ts: 'typescript',
  mts: 'typescript',
  cts: 'typescript',
  tsx: 'tsx',
  py: 'python',
  pyw: 'python',
};

const SHELL_NAMES = new Set([
  '.profile',
  '.bashrc',
  '.bash_profile',
  '.bash_login',
  '.bash_logout',
  '.zshrc',
  '.zprofile',
  '.zshenv',
  '.zlogin',
  '.zlogout',
  '.kshrc',
]);

/** 文件名优先；仅对未识别名称参考首行 shebang，不猜测通用 .conf 的真实语法。 */
export function detectEditorLanguage(fileName: string, content: string): EditorLanguage | null {
  const name = fileName.split(/[\\/]/).pop()?.toLowerCase() ?? '';
  if (/^(dockerfile|containerfile)(\..+)?$/.test(name)) return 'dockerfile';
  if (SHELL_NAMES.has(name) || /^\.env(?:\.|$)/.test(name) || name.endsWith('.env')) {
    return 'shell';
  }
  if (name === '.gitconfig' || name === '.npmrc') return 'ini';
  const dot = name.lastIndexOf('.');
  const extension = dot < 0 ? '' : name.slice(dot + 1);
  if (Object.hasOwn(EXTENSIONS, extension)) return EXTENSIONS[extension];

  // 只检查已读取内容的短首行；不能为语言识别新增本地或远程请求。
  // env 形式只接受明确的解释器位置，不把参数或字符串里的 python/sh 当作命中。
  const firstLine = content
    .slice(0, 256)
    .split(/[\r\n]/, 1)[0]
    .replace(/^\uFEFF/, '');
  const command = /^#!\s*(?:\S*\/)?(\w[\w.-]*)(?:[\t ]+(.*))?$/.exec(firstLine);
  if (!command) return null;
  const interpreter =
    command[1] === 'env' ? /^(?:-S\s+)?([\w.-]+)(?:\s|$)/.exec(command[2] ?? '')?.[1] : command[1];
  if (interpreter && /^(?:ba|z|k)?sh$/.test(interpreter)) return 'shell';
  if (interpreter && /^python(?:[23](?:\.\d+)*)?$/.test(interpreter)) return 'python';
  if (interpreter === 'node' || interpreter === 'nodejs') return 'javascript';
  return null;
}
