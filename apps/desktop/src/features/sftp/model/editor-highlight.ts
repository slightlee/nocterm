import { HighlightStyle, syntaxHighlighting } from '@codemirror/language';
import { tags } from '@lezer/highlight';

/** 语法角色独立于 UI 强调色；切换 CSS 主题即可换色，无需重建文档或编辑器。 */
export const editorHighlightStyle = HighlightStyle.define([
  { tag: [tags.comment, tags.meta], color: 'var(--editor-comment)' },
  { tag: [tags.keyword, tags.modifier], color: 'var(--editor-keyword)' },
  { tag: [tags.string, tags.regexp, tags.quote], color: 'var(--editor-string)' },
  { tag: [tags.number, tags.bool, tags.null, tags.atom], color: 'var(--editor-number)' },
  {
    tag: [tags.propertyName, tags.attributeName, tags.definition(tags.variableName)],
    color: 'var(--editor-property)',
  },
  {
    tag: [
      tags.function(tags.variableName),
      tags.function(tags.propertyName),
      tags.standard(tags.variableName),
    ],
    color: 'var(--editor-function)',
  },
  { tag: [tags.typeName, tags.className, tags.tagName, tags.heading], color: 'var(--editor-type)' },
  // 普通变量和标点保留正文颜色；未知语言不会被统一套上某种语法颜色。
  { tag: [tags.variableName, tags.operator, tags.punctuation], color: 'var(--text-primary)' },
]);

export const editorSyntaxHighlighting = syntaxHighlighting(editorHighlightStyle);
