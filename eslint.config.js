import path from 'node:path';

import eslint from '@eslint/js';
import reactHooks from 'eslint-plugin-react-hooks';
import reactRefresh from 'eslint-plugin-react-refresh';
import tseslint from 'typescript-eslint';

/**
 * 架构边界规则（AGENTS.md「架构与代码边界」的 lint 化）：
 *
 * 1. app → features → shared 单向依赖：features 和 shared 不得反向引用 app；
 * 2. shared 独立：不得依赖任何 Feature；
 * 3. Feature 之间只能通过对方的 index.ts 引用，不得深入内部文件。
 *
 * 自定义规则而非 no-restricted-imports：跨 Feature 违规通常写作相对路径
 * （`../../terminal/model/...`），字面模式无法稳定匹配，必须解析出真实
 * 目标路径后按目录归属判断。规则只作用于 `apps/desktop/src` 下的源码。
 */
function featureBoundaryRule() {
  const SRC_MARKER = `${path.sep}src${path.sep}`;

  /** 归一化相对 src 根的路径：去掉扩展名，返回 features/shared/app/root 区域描述。 */
  function describe(relative) {
    const normalized = relative.replace(/\\/g, '/').replace(/\.(ts|tsx|js|jsx|mjs|css)$/, '');
    const feature = normalized.match(/^features\/([^/]+)(?:\/(.*))?$/);
    if (feature) return { area: 'features', feature: feature[1], rest: feature[2] ?? '' };
    if (normalized === 'shared' || normalized.startsWith('shared/')) return { area: 'shared' };
    if (normalized === 'app' || normalized.startsWith('app/')) return { area: 'app' };
    // src 根上的入口（main.tsx、vite-env.d.ts）与 src 之外的资源不在边界管辖内。
    return null;
  }

  return {
    meta: {
      type: 'problem',
      docs: {
        description: '强制 app → features → shared 单向依赖；Feature 之间只能引用对方 index.ts',
      },
      schema: [],
      messages: {
        crossFeature:
          '不得引用其他 Feature 的内部文件 "{{imported}}"：跨 Feature 能力只能通过对方的 index.ts 暴露；若该契约被多个 Feature 共用，应下沉到 shared。',
        sharedToFeature:
          'shared 不得依赖 Feature "{{imported}}"：shared 是最底层，公共契约应放在 shared 内部。',
        upwardToApp:
          '依赖方向必须保持 app → features → shared：{{from}} 不得引用 app 层 "{{imported}}"。',
      },
    },
    create(context) {
      const filename = context.filename;
      const markerIndex = filename.lastIndexOf(SRC_MARKER);
      // 规则按 files 约束只挂在 apps/desktop/src/**；找不到 src 段时不执行判断。
      if (markerIndex < 0) return {};
      const srcRoot = filename.slice(0, markerIndex + SRC_MARKER.length - 1);
      // 导入方自身可能位于 src 根（main.tsx），不在任何区域内，无需校验。
      const from = describe(path.relative(srcRoot, filename));
      if (!from) return {};

      // 静态导入、动态导入和 re-export 都可能跨边界，统一校验。
      function checkSource(node) {
        const request = node.source?.value;
        if (typeof request !== 'string' || !request.startsWith('.')) return;
        const resolved = path.resolve(path.dirname(filename), request);
        const target = describe(path.relative(srcRoot, resolved));
        if (!target) return;
        if (from.area === 'shared' && target.area === 'features') {
          context.report({ node, messageId: 'sharedToFeature', data: { imported: request } });
          return;
        }
        if ((from.area === 'features' || from.area === 'shared') && target.area === 'app') {
          context.report({
            node,
            messageId: 'upwardToApp',
            data: { from: from.area, imported: request },
          });
          return;
        }
        if (
          from.area === 'features' &&
          target.area === 'features' &&
          from.feature !== target.feature &&
          // 目录导入（features/x）与显式 index 导入都是合规的公共入口。
          target.rest !== '' &&
          target.rest !== 'index'
        ) {
          context.report({ node, messageId: 'crossFeature', data: { imported: request } });
        }
      }

      return {
        ImportDeclaration(node) {
          checkSource(node);
        },
        ImportExpression(node) {
          checkSource(node);
        },
        ExportAllDeclaration(node) {
          checkSource(node);
        },
        ExportNamedDeclaration(node) {
          if (node.source) checkSource(node);
        },
      };
    },
  };
}

export default tseslint.config(
  {
    ignores: ['**/dist/**', '**/node_modules/**', '**/target/**', '**/coverage/**'],
  },
  eslint.configs.recommended,
  ...tseslint.configs.recommended,
  {
    files: ['apps/desktop/src/**/*.{ts,tsx}'],
    plugins: {
      'react-hooks': reactHooks,
      'react-refresh': reactRefresh,
      boundaries: { rules: { 'feature-boundaries': featureBoundaryRule() } },
    },
    rules: {
      ...reactHooks.configs.flat.recommended.rules,
      'react-refresh/only-export-components': ['warn', { allowConstantExport: true }],
      'boundaries/feature-boundaries': 'error',
    },
  }
);
