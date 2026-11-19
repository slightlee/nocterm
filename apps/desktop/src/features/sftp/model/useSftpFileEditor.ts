import { useEffect, useMemo } from 'react';
import { createFileEditorController } from './file-editor-controller';
import { useFileEditorStore, type FileEditorSource } from './file-editor-store';

interface EditorOptions {
  onFileSaved: (source: FileEditorSource) => void;
  showToast: (message: string, tone?: 'error' | 'info' | 'success') => void;
}

/** React 层只选择展示状态和注册快捷键，异步业务可脱离组件单独验证。 */
export function useSftpFileEditor({ onFileSaved, showToast }: EditorOptions) {
  const controller = useMemo(
    () => createFileEditorController({ onFileSaved, showToast }),
    [onFileSaved, showToast]
  );
  const editors = useFileEditorStore((state) => state.editors);
  const activeEditorId = useFileEditorStore((state) => state.activeEditorId);
  const activateEditor = useFileEditorStore((state) => state.activateEditor);
  const activeEditor = editors.find((editor) => editor.id === activeEditorId) ?? null;
  const editorActive = activeEditor !== null;

  useEffect(() => {
    // 快捷键读取即时激活 ID；后台保存完成不会改变当前编辑标签。
    // 控制器检查 closePending，确认关闭期间不能被快捷键启动新的保存。
    if (!editorActive) return;
    const handler = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 's') {
        event.preventDefault();
        const id = useFileEditorStore.getState().activeEditorId;
        if (id) void controller.saveFile(id);
      }
    };
    window.addEventListener('keydown', handler);
    // 文件工作区常驻，但键盘监听只在编辑条目激活时存在。
    return () => window.removeEventListener('keydown', handler);
  }, [editorActive, controller]);

  return {
    ...controller,
    editors,
    activeEditorId,
    activeEditor,
    activateEditor,
    conflictPendingEditor: activeEditor?.conflictPending ? activeEditor : null,
  };
}
