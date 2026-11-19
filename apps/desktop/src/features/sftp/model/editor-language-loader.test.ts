import { Compartment, EditorState, type Extension } from '@codemirror/state';
import { ensureSyntaxTree, syntaxTree } from '@codemirror/language';
import { highlightTree } from '@lezer/highlight';
import { describe, expect, it } from 'vitest';
import { documentExtensions } from './editor-document';
import { loadEditorLanguage } from './editor-language-loader';
import { editorHighlightStyle } from './editor-highlight';
import type { EditorLanguage } from './editor-language';

function highlightedTokens(state: EditorState) {
  const tokens: string[] = [];
  ensureSyntaxTree(state, state.doc.length, 1000);
  highlightTree(syntaxTree(state), editorHighlightStyle, (from, to) => {
    tokens.push(state.doc.sliceString(from, to));
  });
  return tokens;
}

describe('editor syntax languages', () => {
  it.each<[EditorLanguage, string, string]>([
    ['json', '{"enabled": true, "port": 22}', 'true'],
    ['yaml', 'enabled: true\n# note\nname: "nocterm"', '# note'],
    ['toml', '[server]\nenabled = true\nport = 22', '22'],
    ['ini', '[server]\nname=nocterm\n# note', '# note'],
    ['shell', '# note\nif true; then echo "hello"; fi', 'if'],
    ['dockerfile', '# note\nFROM alpine\nRUN echo "hello"', 'FROM'],
    ['javascript', 'const name = "nocterm"; // note', 'const'],
    ['jsx', 'const node = <span title="hello" />;', 'title'],
    ['typescript', 'const port: number = 22;', 'number'],
    ['tsx', 'const node: JSX.Element = <span title="hello" />;', 'title'],
    ['python', '# note\ndef hello():\n    return "hello"', 'def'],
  ])('loads %s and produces syntax tokens', async (language, content, expectedToken) => {
    const extension = await loadEditorLanguage(language);
    const state = EditorState.create({ doc: content, extensions: [extension] });
    expect(highlightedTokens(state).join(' ')).toContain(expectedToken);
    expect(state.sliceDoc()).toBe(content);
  });

  it('shares an in-flight parser between files', async () => {
    const first = loadEditorLanguage('json');
    const second = loadEditorLanguage('json');
    expect(second).toBe(first);
    expect(await second).toBe(await first);
  });

  it.each(['\n', '\r\n', '\r'])(
    'reconfigures syntax without changing %j content or selection',
    async (eol) => {
      const original = `\uFEFF{"enabled": true}${eol}`;
      const compartment = new Compartment();
      let state = EditorState.create({
        doc: original,
        selection: { anchor: 4 },
        extensions: [documentExtensions(original), compartment.of([])],
      });
      // 在解析器下载期间输入的内容必须保留，不能重新装载初始快照。
      state = state.update({ changes: { from: 2, insert: 'x' } }).state;
      const edited = state.sliceDoc();
      const selection = state.selection.toJSON();
      const extension = await loadEditorLanguage('json');
      for (const language of [extension, [] as Extension]) {
        const transaction = state.update({ effects: compartment.reconfigure(language) });
        expect(transaction.docChanged).toBe(false);
        state = transaction.state;
        expect(state.sliceDoc()).toBe(edited);
        expect(state.selection.toJSON()).toEqual(selection);
      }
    }
  );
});
