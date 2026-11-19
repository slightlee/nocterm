import { useEffect, useRef } from 'react';
import { EditorView, basicSetup } from 'codemirror';
import { Compartment, EditorState, Prec } from '@codemirror/state';
import { keymap } from '@codemirror/view';
import {
  documentExtensions,
  externalDocumentTransaction,
  externalDocumentUpdate,
} from '../../model/editor-document';
import styles from './RemoteFileEditor.module.css';

/**
 * 编辑器内部配色：走 CodeMirror 官方 theme API 而非 CSS Modules 的 :global
 * 选择器（仓库 stylelint 未启用 CSS Modules 伪类支持）。所有取值引用语义
 * token，深浅主题由 html[data-theme] 切换变量自然生效，组件无需感知主题。
 */
const editorTheme = EditorView.theme({
  '&': {
    height: '100%',
    fontSize: '12px',
    backgroundColor: 'var(--bg-primary)',
    color: 'var(--text-primary)',
  },
  '&.cm-focused': { outline: 'none' },
  '.cm-scroller': { fontFamily: 'var(--font-mono)', lineHeight: '1.5' },
  '.cm-gutters': {
    backgroundColor: 'var(--bg-secondary)',
    color: 'var(--text-tertiary)',
    borderRight: '1px solid var(--border-secondary)',
  },
  '.cm-activeLine': { backgroundColor: 'var(--bg-tertiary)' },
  '.cm-activeLineGutter': {
    backgroundColor: 'var(--bg-tertiary)',
    color: 'var(--text-secondary)',
  },
  // 查找替换面板跟随面板配色，避免默认白底在深色主题下突兀。
  '.cm-panels': {
    backgroundColor: 'var(--bg-secondary)',
    color: 'var(--text-primary)',
    borderColor: 'var(--border-secondary)',
  },
  '.cm-panel.cm-search input': {
    backgroundColor: 'var(--bg-input)',
    color: 'var(--text-primary)',
    border: '1px solid var(--border-primary)',
  },
  '.cm-panel.cm-search label': { color: 'var(--text-secondary)' },
  '.cm-panel.cm-search button[name="close"]': { color: 'var(--text-tertiary)' },
  '.cm-searchMatch': { backgroundColor: 'var(--accent-orange-soft)' },
  '.cm-searchMatch-selected': { backgroundColor: 'var(--accent-blue-soft)' },
  // 光标与选区引用 token，避免默认黑色光标在深色背景下不可见。
  '.cm-content': { caretColor: 'var(--accent-blue)' },
  '.cm-cursor': { borderLeftColor: 'var(--accent-blue)' },
  '.cm-selectionBackground': { backgroundColor: 'var(--bg-selected) !important' },
});

interface CodeMirrorEditorProps {
  value: string;
  /** 用户输入引起的内容变化；外部值更新（切换文件/重载远端版本）不触发。 */
  onChange: (value: string) => void;
  /** 编辑器聚焦时的 Ctrl/Cmd+S 保存入口。 */
  onSave: () => void;
  readOnly?: boolean;
}

/**
 * CodeMirror 6 的薄壳组件：只负责挂载/销毁、Ctrl/Cmd+S 键位与外部值同步；
 * 行号、撤销、查找替换、括号匹配等编辑能力全部来自 basicSetup，不在此叠加。
 */
export function CodeMirrorEditor({
  value,
  onChange,
  onSave,
  readOnly = false,
}: CodeMirrorEditorProps) {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const viewRef = useRef<EditorView | null>(null);
  // 挂载时的初始文档：useRef 初值只读不写，渲染期间不再回写 ref。
  const initialDocRef = useRef(value);
  const onChangeRef = useRef(onChange);
  const onSaveRef = useRef(onSave);
  const readOnlyConfigRef = useRef(new Compartment());

  // 提交后同步最新回调，编辑器实例始终通过 ref 调用，避免闭包捕获过期 props。
  useEffect(() => {
    onChangeRef.current = onChange;
    onSaveRef.current = onSave;
  });

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    const view = new EditorView({
      doc: initialDocRef.current,
      extensions: [
        basicSetup,
        editorTheme,
        documentExtensions(initialDocRef.current),
        readOnlyConfigRef.current.of([]),
        // Mod-S 拦截放在最高优先级：编辑器聚焦时保存回传优先于一切默认键位。
        Prec.highest(
          keymap.of([
            {
              key: 'Mod-s',
              preventDefault: true,
              run: () => {
                onSaveRef.current();
                return true;
              },
            },
          ])
        ),
        EditorView.updateListener.of((update) => {
          if (
            update.docChanged &&
            !update.transactions.some((transaction) =>
              transaction.annotation(externalDocumentUpdate)
            )
          ) {
            onChangeRef.current(update.state.sliceDoc());
          }
        }),
      ],
      parent: host,
    });
    viewRef.current = view;
    return () => {
      view.destroy();
      viewRef.current = null;
    };
  }, []);

  // 外部值变化（切换文件、重载远端版本）整体替换文档；用户输入回流时文档已一致，跳过。
  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    if (view.state.sliceDoc() !== value) {
      view.dispatch(externalDocumentTransaction(view.state, value));
    }
  }, [value]);

  useEffect(() => {
    viewRef.current?.dispatch({
      effects: readOnlyConfigRef.current.reconfigure([
        EditorState.readOnly.of(readOnly),
        EditorView.editable.of(!readOnly),
      ]),
    });
  }, [readOnly]);

  return <div ref={hostRef} className={styles.editorHost} />;
}
