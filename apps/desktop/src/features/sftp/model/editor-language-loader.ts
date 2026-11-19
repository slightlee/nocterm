import { StreamLanguage } from '@codemirror/language';
import type { Extension } from '@codemirror/state';
import type { EditorLanguage } from './editor-language';

// 每种语言保留独立 import 边界；只加载当前文件需要的解析器，不引入完整语言集合。
// 仅取 language，避免把各语言附带的代码补全或诊断一并变成编辑器新行为。
const LOADERS: Record<EditorLanguage, () => Promise<Extension>> = {
  json: () => import('@codemirror/lang-json').then((module) => module.json().language),
  yaml: () => import('@codemirror/lang-yaml').then((module) => module.yaml().language),
  python: () => import('@codemirror/lang-python').then((module) => module.python().language),
  javascript: () =>
    import('@codemirror/lang-javascript').then((module) => module.javascript().language),
  jsx: () =>
    import('@codemirror/lang-javascript').then(
      (module) => module.javascript({ jsx: true }).language
    ),
  typescript: () =>
    import('@codemirror/lang-javascript').then(
      (module) => module.javascript({ typescript: true }).language
    ),
  tsx: () =>
    import('@codemirror/lang-javascript').then(
      (module) => module.javascript({ typescript: true, jsx: true }).language
    ),
  shell: () =>
    import('@codemirror/legacy-modes/mode/shell').then((module) =>
      StreamLanguage.define(module.shell)
    ),
  toml: () =>
    import('@codemirror/legacy-modes/mode/toml').then((module) =>
      StreamLanguage.define(module.toml)
    ),
  ini: () =>
    import('@codemirror/legacy-modes/mode/properties').then((module) =>
      StreamLanguage.define(module.properties)
    ),
  dockerfile: () =>
    import('@codemirror/legacy-modes/mode/dockerfile').then((module) =>
      StreamLanguage.define(module.dockerFile)
    ),
};

const pendingLanguages = new Map<EditorLanguage, Promise<Extension>>();

/** 多个文件共享解析器加载；失败不缓存，重新打开时仍可重试，调用方负责展示失败。 */
export function loadEditorLanguage(language: EditorLanguage): Promise<Extension> {
  const existing = pendingLanguages.get(language);
  if (existing) return existing;
  const pending = LOADERS[language]().catch((error: unknown) => {
    pendingLanguages.delete(language);
    throw error;
  });
  pendingLanguages.set(language, pending);
  return pending;
}
