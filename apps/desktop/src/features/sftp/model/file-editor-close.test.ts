import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  beginFileEditorClose,
  withFileEditorClose,
  withFileEditorTargetChange,
} from './file-editor-close';
import { useFileEditorStore } from './file-editor-store';

beforeEach(() => {
  useFileEditorStore.setState({ editors: [], activeEditorId: null, closePending: false });
  useFileEditorStore.getState().openEditor({
    id: 'remote:A:/file',
    name: 'file',
    source: { kind: 'remote', connectionId: 'A', path: '/file' },
    content: 'draft',
    savedContent: 'old',
    dirty: true,
    status: 'ready',
    baseline: { size: 3, mtimeSeconds: 1, mtimeNanos: null },
  });
});
describe('file editor close guard', () => {
  it('keeps content when discard is cancelled', async () => {
    const action = vi.fn();
    expect(await withFileEditorClose(['remote:A:/file'], async () => false, vi.fn(), action)).toBe(
      false
    );
    expect(action).not.toHaveBeenCalled();
    expect(useFileEditorStore.getState()).toMatchObject({
      closePending: false,
      editors: [expect.objectContaining({ content: 'draft', dirty: true })],
    });
  });
  it('rejects saving documents before showing a discard dialog', async () => {
    useFileEditorStore.getState().patchEditor('remote:A:/file', { status: 'saving' });
    const confirm = vi.fn();
    expect(await beginFileEditorClose(['remote:A:/file'], confirm, vi.fn())).toBeNull();
    expect(confirm).not.toHaveBeenCalled();
    expect(useFileEditorStore.getState().closePending).toBe(false);
  });
  it('holds the operation guard through close side effects and releases on failure', async () => {
    await expect(
      withFileEditorClose(
        ['remote:A:/file'],
        async () => true,
        vi.fn(),
        async () => {
          expect(useFileEditorStore.getState().closePending).toBe(true);
          throw new Error('disconnect failed');
        }
      )
    ).rejects.toThrow('disconnect failed');
    expect(useFileEditorStore.getState().closePending).toBe(false);
    expect(useFileEditorStore.getState().editors[0].content).toBe('draft');
  });
  it('connection changes invalidate writes while keeping unsaved content', async () => {
    await withFileEditorTargetChange('A', async () => {});
    expect(useFileEditorStore.getState().editors[0]).toMatchObject({
      content: 'draft',
      dirty: true,
      targetInvalidated: true,
    });
    expect(useFileEditorStore.getState().closePending).toBe(false);
  });
});
