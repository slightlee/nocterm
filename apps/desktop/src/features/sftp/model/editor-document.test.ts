import { EditorState } from '@codemirror/state';
import { describe, expect, it } from 'vitest';
import {
  documentExtensions,
  externalDocumentTransaction,
  externalDocumentUpdate,
} from './editor-document';

describe('editor document format', () => {
  it.each(['\n', '\r\n', '\r'])('preserves %j through open, edit and undo content', (eol) => {
    const original = `first${eol}second${eol}`;
    const state = EditorState.create({ doc: original, extensions: [documentExtensions(original)] });
    expect(state.sliceDoc()).toBe(original);
    const edited = state.update({ changes: { from: 0, insert: '!' } }).state;
    expect(edited.sliceDoc()).toBe(`!${original}`);
    expect(edited.update({ changes: { from: 0, to: 1 } }).state.sliceDoc()).toBe(original);
  });

  it('marks external reloads and switches the serialization format', () => {
    const state = EditorState.create({ doc: 'old\n', extensions: [documentExtensions('old\n')] });
    const update = state.update(externalDocumentTransaction(state, 'new\r\n'));
    expect(update.annotation(externalDocumentUpdate)).toBe(true);
    expect(update.state.sliceDoc()).toBe('new\r\n');
  });
});
