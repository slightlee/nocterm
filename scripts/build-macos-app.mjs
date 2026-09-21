/* global console, process */
import { execFileSync } from 'node:child_process';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const REPOSITORY_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const DESKTOP_PACKAGE = '@nocterm/desktop';
// 与发布脚本和 tauri.conf.json 保持一致的最低运行版本，避免本机产物链接到更高系统版本的符号。
const MACOS_DEPLOYMENT_TARGET = '14.0';

/**
 * Tauri 的 `--bundles dmg` 只产出 DMG，并在构建结束后自动清理 .app（上游设计行为）。
 * 本地需要可双击运行的 .app 时，改用 app-only 构建，产物保留在 bundle/macos 下。
 */
export function buildAppTauriArguments() {
  return [
    'exec',
    'tauri',
    'build',
    // 显式跳过 beforeBuildCommand：前端由本脚本先行构建，避免 Tauri 经 PATH 嵌套调用 pnpm 触发版本冲突。
    '--config',
    '{"bundle":{"active":true},"build":{"beforeBuildCommand":""}}',
    '--bundles',
    'app',
    '--ci',
  ];
}

/** 宿主守卫与发布脚本对称：.app 属于 macOS 原生产物，不允许在其他平台伪装构建。 */
export function assertMacosHost(platform) {
  if (platform !== 'darwin') {
    throw new Error(`build:app 只能在 macOS 执行，当前平台为 ${platform}`);
  }
}

/** 依次构建前端与 .app；执行步骤经 runStep 注入，便于单测断言调用序列而不触发真实构建。 */
export function buildMacosApp({ pnpmCli, runStep }) {
  if (!pnpmCli) throw new Error('无法定位 pnpm CLI，请通过 corepack pnpm 执行本命令');
  // 所有子命令都通过当前 pnpm CLI（npm_execpath）调用：任何经 PATH 解析的嵌套 pnpm
  // 都可能命中其他版本（如全局 pnpm 12），在 Corepack 环境下会报 ERR_PNPM_BAD_PM_VERSION。
  runStep([pnpmCli, '--filter', DESKTOP_PACKAGE, 'build']);
  runStep([pnpmCli, '--filter', DESKTOP_PACKAGE, ...buildAppTauriArguments()]);
}

export function main() {
  assertMacosHost(process.platform);
  const pnpmCli = process.env.npm_execpath;
  buildMacosApp({
    pnpmCli,
    runStep: (args) => {
      execFileSync(process.execPath, args, {
        cwd: REPOSITORY_ROOT,
        stdio: 'inherit',
        env: {
          ...process.env,
          MACOSX_DEPLOYMENT_TARGET: process.env.MACOSX_DEPLOYMENT_TARGET ?? MACOS_DEPLOYMENT_TARGET,
        },
      });
    },
  });
  console.log(
    `macOS app bundle: ${join(REPOSITORY_ROOT, 'target', 'release', 'bundle', 'macos', 'Nocterm.app')}`
  );
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    main();
  } catch (error) {
    console.error(`Build macOS app failed: ${error instanceof Error ? error.message : error}`);
    process.exitCode = 1;
  }
}
