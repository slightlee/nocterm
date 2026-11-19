import { useFileEditorStore, type FileEditorTab } from '../../model/file-editor-store';
import { CodeMirrorEditor } from './CodeMirrorEditor';
import styles from './RemoteFileEditor.module.css';

interface FileEditorViewProps {
  file: FileEditorTab;
  onChange: (value: string) => void;
  onSave: () => void;
}

/**
 * 文件编辑器主视图：激活编辑标签时占据主内容区（等价原双栏位置），本地与远程
 * 文件共用同一布局。头部只承载身份信息（文件名 + 来源·路径），不放动作按钮：
 * - 未保存状态由顶部标签的橙色 ● 承载（全局可见，含切换到其它标签时）；
 * - 保存统一走 Ctrl/Cmd+S（编辑器键位与窗口监听均已生效），与 VS Code 一致
 *   不在头部放置保存按钮；
 * - 关闭动作只有标签上的 ×，dirty 关闭有确认弹窗兜底。
 */
export function FileEditorView({ file, onChange, onSave }: FileEditorViewProps) {
  const loading = file.status === 'loading';
  const isRemote = file.source.kind === 'remote';
  const closePending = useFileEditorStore((state) => state.closePending);

  return (
    <section
      className={styles.editorPanel}
      aria-label={isRemote ? '远程文件编辑器' : '本地文件编辑器'}
    >
      <header className={styles.editorHeader}>
        <div className={styles.editorTitles}>
          <span className={styles.editorName}>{file.name}</span>
          <span className={styles.editorPath} title={file.source.path}>
            {isRemote ? '远程' : '本地'} · {file.source.path}
          </span>
        </div>
      </header>
      {file.targetInvalidated && (
        <div className={styles.editorNotice}>
          连接已变更，此文件已停止远程保存。请复制未保存内容后关闭并重新打开。
        </div>
      )}
      {loading ? (
        <div className={styles.editorMessage}>加载文件中…</div>
      ) : (
        <>
          {/* 按文件 ID 重建 CodeMirror，隔离文档与撤销历史；否则切换标签后
              Ctrl+Z 可能撤回到另一个文件的内容。V1 接受切换时不保留 undo 栈。 */}
          <CodeMirrorEditor
            key={file.id}
            value={file.content}
            onChange={onChange}
            onSave={onSave}
            readOnly={closePending}
          />
        </>
      )}
    </section>
  );
}
