import { create } from 'zustand';

/** 编辑器文件来源：本地路径与远程「连接 + 路径」同等地位，互不依赖。 */
export type FileEditorSource =
  { kind: 'local'; path: string } | { kind: 'remote'; path: string; connectionId: string };

/** 编辑器冲突基线：远端只有秒级 mtime；本地使用秒 + 纳秒全精度。 */
export interface FileEditorBaseline {
  size: number | null;
  mtimeSeconds: number | null;
  mtimeNanos: number | null;
  /** 后端签发的远程目标版本；本地文件没有连接身份。 */
  targetRevision?: string;
}

/** 单个编辑标签的完整状态：内容只驻留内存，不持久化、不进日志。 */
export interface FileEditorTab {
  /** 每次打开分配新身份，关闭后迟到的读写结果不能命中重开的同路径标签。 */
  instanceId: number;
  /** 稳定 ID：本地 `local:<身份路径>`，远程 `remote:<连接>:<路径>`，防同名碰撞。 */
  id: string;
  name: string;
  source: FileEditorSource;
  content: string;
  savedContent: string;
  baseline: FileEditorBaseline;
  status: 'loading' | 'ready' | 'saving';
  dirty: boolean;
  conflictPending: boolean;
  targetInvalidated: boolean;
}

export type FileEditorDraft = Pick<
  FileEditorTab,
  'id' | 'name' | 'source' | 'content' | 'savedContent' | 'baseline'
> &
  Partial<Pick<FileEditorTab, 'status' | 'dirty' | 'conflictPending'>>;

interface FileEditorState {
  editors: FileEditorTab[];
  /** 当前展示的编辑器；null 表示显示双栏文件管理。 */
  activeEditorId: string | null;
  closePending: boolean;
  openEditor: (draft: FileEditorDraft) => void;
  activateEditor: (id: string | null) => void;
  patchEditor: (id: string, patch: Partial<Omit<FileEditorTab, 'id' | 'source'>>) => void;
  closeEditor: (id: string) => void;
  closeEditors: (ids: readonly string[]) => void;
  setClosePending: (pending: boolean) => void;
  invalidateConnection: (connectionId: string) => void;
  completeOpen: (
    id: string,
    instanceId: number,
    source: FileEditorSource,
    patch: Pick<FileEditorTab, 'content' | 'savedContent' | 'baseline'>
  ) => void;
}

/** 由来源推导稳定编辑器 ID；规范化路径由后端返回或远端 canonicalize 保证。 */
export function fileEditorId(source: FileEditorSource): string {
  if (source.kind === 'local') return `local:${source.path}`;
  return `remote:${source.connectionId}:${source.path}`;
}

/** 取路径的最后一段作为标签名；根路径等退化情形回退到完整路径。 */
export function fileEditorName(path: string): string {
  const trimmed = path.replace(/\/+$/, '');
  const name = trimmed.split(/[\\/]/).pop();
  return name || path;
}

/** 保存期间仍允许继续输入；loading 时编辑器未展示，不能改内容。 */
export function canChangeEditorContent(status: FileEditorTab['status']): boolean {
  return status === 'ready' || status === 'saving';
}

/** 保存完成后以「当前内容 vs 实际上传快照」计算 dirty，保护保存期间的新输入。 */
export function editorDirtyAfterSave(currentContent: string, uploadedContent: string): boolean {
  return currentContent !== uploadedContent;
}

let nextEditorInstanceId = 0;

export const useFileEditorStore = create<FileEditorState>()((set) => ({
  editors: [],
  activeEditorId: null,
  closePending: false,

  /** 打开或复用：同来源只激活已有标签，绝不覆盖其未保存内容。 */
  openEditor: (draft) =>
    set((state) => {
      const exists = state.editors.some((editor) => editor.id === draft.id);
      return {
        editors: exists
          ? state.editors
          : [
              ...state.editors,
              {
                instanceId: ++nextEditorInstanceId,
                status: 'loading',
                dirty: false,
                conflictPending: false,
                targetInvalidated: false,
                ...draft,
              },
            ],
        activeEditorId: draft.id,
      };
    }),

  activateEditor: (activeEditorId) => set({ activeEditorId }),

  setClosePending: (closePending) => set({ closePending }),

  // 目标配置已改变时保留内容与 dirty，禁止该旧快照转向另一个服务器保存。
  invalidateConnection: (connectionId) =>
    set((state) => ({
      editors: state.editors.map((editor) =>
        editor.source.kind === 'remote' && editor.source.connectionId === connectionId
          ? { ...editor, targetInvalidated: true }
          : editor
      ),
    })),

  completeOpen: (id, instanceId, source, patch) =>
    set((state) => {
      const current = state.editors.find((editor) => editor.id === id);
      if (current?.instanceId !== instanceId) return state;
      const identityId = fileEditorId(source);
      const existing = state.editors.find((editor) => editor.id === identityId && editor.id !== id);
      return {
        editors: existing
          ? state.editors.filter((editor) => editor.id !== id)
          : state.editors.map((editor) =>
              editor.id === id
                ? {
                    ...editor,
                    ...patch,
                    source,
                    id: identityId,
                    name: fileEditorName(source.path),
                    status: 'ready',
                    dirty: false,
                  }
                : editor
            ),
        activeEditorId: state.activeEditorId === id ? identityId : state.activeEditorId,
      };
    }),

  /** 按 ID 精确更新：保存/加载只影响目标标签，不触碰其它在编辑文件。 */
  patchEditor: (id, patch) =>
    set((state) => ({
      editors: state.editors.map((editor) => (editor.id === id ? { ...editor, ...patch } : editor)),
    })),

  closeEditor: (id) =>
    set((state) => {
      const editors = state.editors.filter((editor) => editor.id !== id);
      return {
        editors,
        activeEditorId:
          state.activeEditorId === id
            ? (editors[editors.length - 1]?.id ?? null)
            : state.activeEditorId,
      };
    }),

  /** 批量关闭：关闭激活标签后回退到最后一个幸存编辑器，否则回到文件管理。 */
  closeEditors: (ids) =>
    set((state) => {
      const closed = new Set(ids);
      const editors = state.editors.filter((editor) => !closed.has(editor.id));
      const activeStillOpen = state.activeEditorId === null || !closed.has(state.activeEditorId);
      return {
        editors,
        activeEditorId: activeStillOpen
          ? state.activeEditorId
          : (editors[editors.length - 1]?.id ?? null),
      };
    }),
}));
