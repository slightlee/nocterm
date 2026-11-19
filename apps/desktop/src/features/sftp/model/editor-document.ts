import { Annotation, Compartment, EditorState, Text, Transaction } from '@codemirror/state';

/** 换行属于文件格式；序列化必须使用 state.sliceDoc，不能使用固定 LF 的 Text.toString。 */
export function documentLineSeparator(value: string): string {
  return value.match(/\r\n|\r|\n/)?.[0] ?? '\n';
}

/** 外部同步只更新文档，不作为用户输入回流，也不进入撤销历史。 */
export const externalDocumentUpdate = Annotation.define<boolean>();
export const documentLineSeparatorConfig = new Compartment();

export function documentExtensions(value: string) {
  return documentLineSeparatorConfig.of(EditorState.lineSeparator.of(documentLineSeparator(value)));
}

export function externalDocumentTransaction(state: EditorState, value: string) {
  return {
    changes: {
      from: 0,
      to: state.doc.length,
      insert: Text.of(value.split(documentLineSeparator(value))),
    },
    effects: documentLineSeparatorConfig.reconfigure(
      EditorState.lineSeparator.of(documentLineSeparator(value))
    ),
    annotations: [externalDocumentUpdate.of(true), Transaction.addToHistory.of(false)],
  };
}
