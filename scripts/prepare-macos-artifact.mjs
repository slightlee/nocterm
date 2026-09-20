/* global console, process */
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  rmSync,
  statSync,
  writeFileSync,
} from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

import { parseReleaseVersion } from './check-release-version.mjs';

const REPOSITORY_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
// 发布矩阵覆盖 Apple Silicon 与 Intel 两个原生架构；Node 运行时命名与 Tauri、发布命名各不相同，必须显式映射。
const MACOS_ARCHITECTURE_MAP = {
  arm64: { source: 'aarch64', release: 'aarch64' },
  x64: { source: 'x64', release: 'x86_64' },
};
// 与 tauri.conf.json 的 macOS 最低运行版本和 CI 环境保持一致，避免本机产物链接到更高系统版本的符号。
const MACOS_DEPLOYMENT_TARGET = '14.0';

/** 发布名称使用稳定平台标识；运行时的 arm64 命名不直接进入发布产物。 */
export function buildMacosArtifactName(version, architecture) {
  if (!parseReleaseVersion(version)) throw new Error(`无效的产品版本：${version}`);
  if (!MACOS_ARCHITECTURE_MAP[architecture]) {
    throw new Error(`当前只支持 macOS aarch64 与 x86_64 发布产物，实际架构为 ${architecture}`);
  }
  const { release } = MACOS_ARCHITECTURE_MAP[architecture];
  return `Nocterm_${version}_macos_${release}.dmg`;
}

/** Tauri 源产物使用其原生架构缩写；映射关系集中维护，避免调用方各自猜测。 */
function resolveMacosSourceArchitecture(architecture) {
  const mapped = MACOS_ARCHITECTURE_MAP[architecture];
  if (!mapped) throw new Error(`不支持的 macOS 构建架构：${architecture}`);
  return mapped.source;
}

/** 保留 Tauri 原始输出，并把可分发副本与校验和统一收口到 artifacts 目录。 */
export function prepareMacosArtifact({ sourcePath, outputDirectory, version, architecture }) {
  const artifactName = buildMacosArtifactName(version, architecture);
  const artifactPath = resolve(outputDirectory, artifactName);
  mkdirSync(outputDirectory, { recursive: true });
  copyFileSync(sourcePath, artifactPath);

  // 校验和必须基于实际分发副本，避免记录到 Tauri 临时文件的哈希。
  const checksum = createHash('sha256').update(readFileSync(artifactPath)).digest('hex');
  const checksumPath = `${artifactPath}.sha256`;
  writeFileSync(checksumPath, `${checksum}  ${artifactName}\n`);

  return { artifactName, artifactPath, checksum, checksumPath };
}

/** 清理精确的当前版本输出，确保构建失败时不会留下可被误发的旧包。 */
export function buildMacosReleaseArtifact({ repositoryRoot, version, architecture, runBuild }) {
  const artifactName = buildMacosArtifactName(version, architecture);
  const sourceArchitecture = resolveMacosSourceArchitecture(architecture);
  const sourcePath = join(
    repositoryRoot,
    'target',
    'release',
    'bundle',
    'dmg',
    `Nocterm_${version}_${sourceArchitecture}.dmg`
  );
  const outputDirectory = join(repositoryRoot, 'target', 'release', 'artifacts');
  const artifactPath = join(outputDirectory, artifactName);
  const checksumPath = `${artifactPath}.sha256`;

  // 只删除当前版本的三个确定文件，不清空 target 或其他版本的审计产物。
  for (const path of [sourcePath, artifactPath, checksumPath]) rmSync(path, { force: true });

  try {
    runBuild({ sourcePath });
  } catch (error) {
    // Tauri 失败时也可能留下部分文件；必须移除，不给后续步骤提供模糊输入。
    rmSync(sourcePath, { force: true });
    throw error;
  }
  if (!existsSync(sourcePath)) throw new Error(`Tauri 构建完成后未找到 DMG 产物：${sourcePath}`);

  const sourceStats = statSync(sourcePath);
  if (!sourceStats.isFile() || sourceStats.size === 0) {
    rmSync(sourcePath, { force: true });
    throw new Error(`Tauri DMG 产物不是有效非空文件：${sourcePath}`);
  }

  return {
    ...prepareMacosArtifact({ sourcePath, outputDirectory, version, architecture }),
    sourcePath,
    sourceSize: sourceStats.size,
  };
}

export function main() {
  // 产物必须在目标系统原生构建，本脚本不允许用其他平台伪装 macOS 结果。
  if (process.platform !== 'darwin') {
    throw new Error(`macOS 产物整理脚本只能在 macOS 执行，当前平台为 ${process.platform}`);
  }

  const { version } = JSON.parse(readFileSync(join(REPOSITORY_ROOT, 'package.json'), 'utf8'));
  const pnpmCli = process.env.npm_execpath;
  if (!pnpmCli) throw new Error('无法定位 pnpm CLI，请通过 corepack pnpm 执行本命令');

  const result = buildMacosReleaseArtifact({
    repositoryRoot: REPOSITORY_ROOT,
    version,
    architecture: process.arch,
    runBuild: () => {
      // 前端构建与 Tauri 都通过当前 pnpm CLI（npm_execpath）直接调用：
      // 任何经 PATH 解析的嵌套 pnpm 都可能命中其他版本（如全局 pnpm 12），
      // 在 Corepack 环境下会因版本不一致直接报 ERR_PNPM_BAD_PM_VERSION。
      execFileSync(process.execPath, [pnpmCli, '--filter', '@nocterm/desktop', 'build'], {
        cwd: REPOSITORY_ROOT,
        stdio: 'inherit',
      });
      // 前端产物已由上一步生成；显式跳过 beforeBuildCommand，
      // 避免 Tauri 内部再经 PATH 调用 pnpm 触发同样的版本冲突。
      execFileSync(
        process.execPath,
        [
          pnpmCli,
          '--filter',
          '@nocterm/desktop',
          'exec',
          'tauri',
          'build',
          '--config',
          '{"bundle":{"active":true},"build":{"beforeBuildCommand":""}}',
          '--bundles',
          'dmg',
          '--ci',
        ],
        {
          cwd: REPOSITORY_ROOT,
          stdio: 'inherit',
          env: {
            ...process.env,
            MACOSX_DEPLOYMENT_TARGET:
              process.env.MACOSX_DEPLOYMENT_TARGET ?? MACOS_DEPLOYMENT_TARGET,
          },
        }
      );
    },
  });

  console.log(`Tauri source artifact: ${result.sourcePath} (${result.sourceSize} bytes)`);
  console.log(`macOS release artifact: ${result.artifactPath}`);
  console.log(`SHA-256: ${result.checksum}`);
  console.log(`Checksum file: ${result.checksumPath}`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    main();
  } catch (error) {
    console.error(`Build macOS artifact failed: ${error instanceof Error ? error.message : error}`);
    process.exitCode = 1;
  }
}
