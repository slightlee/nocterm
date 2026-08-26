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
// 发布矩阵当前只声明 Windows x64；不在未验证架构上猜测 Tauri 命名。
const TAURI_WINDOWS_ARCHITECTURE = 'x64';
const RELEASE_WINDOWS_ARCHITECTURE = 'x86_64';

/** 发布名称使用稳定平台标识，不暴露 Tauri 的平台专用缩写。 */
export function buildWindowsArtifactName(version, architecture) {
  if (!parseReleaseVersion(version)) throw new Error(`无效的产品版本：${version}`);
  if (architecture !== TAURI_WINDOWS_ARCHITECTURE) {
    throw new Error(`当前只支持 Windows x64 发布产物，实际架构为 ${architecture}`);
  }
  return `Nocterm_${version}_windows_${RELEASE_WINDOWS_ARCHITECTURE}-setup.exe`;
}

/** 保留 Tauri 原始输出，并把可分发副本与校验和统一收口到 artifacts 目录。 */
export function prepareWindowsArtifact({ sourcePath, outputDirectory, version, architecture }) {
  const artifactName = buildWindowsArtifactName(version, architecture);
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
export function buildWindowsReleaseArtifact({ repositoryRoot, version, architecture, runBuild }) {
  const artifactName = buildWindowsArtifactName(version, architecture);
  const sourcePath = join(
    repositoryRoot,
    'target',
    'release',
    'bundle',
    'nsis',
    `Nocterm_${version}_${TAURI_WINDOWS_ARCHITECTURE}-setup.exe`
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
  if (!existsSync(sourcePath)) throw new Error(`Tauri 构建完成后未找到 NSIS 产物：${sourcePath}`);

  const sourceStats = statSync(sourcePath);
  if (!sourceStats.isFile() || sourceStats.size === 0) {
    rmSync(sourcePath, { force: true });
    throw new Error(`Tauri NSIS 产物不是有效非空文件：${sourcePath}`);
  }

  return {
    ...prepareWindowsArtifact({ sourcePath, outputDirectory, version, architecture }),
    sourcePath,
    sourceSize: sourceStats.size,
  };
}

export function main() {
  // 产物必须在目标系统原生构建，本脚本不允许用其他平台伪装 Windows 结果。
  if (process.platform !== 'win32') {
    throw new Error(`Windows 产物整理脚本只能在 Windows 执行，当前平台为 ${process.platform}`);
  }

  const { version } = JSON.parse(readFileSync(join(REPOSITORY_ROOT, 'package.json'), 'utf8'));
  const pnpmCli = process.env.npm_execpath;
  if (!pnpmCli) throw new Error('无法定位 pnpm CLI，请通过 corepack pnpm 执行本命令');

  const result = buildWindowsReleaseArtifact({
    repositoryRoot: REPOSITORY_ROOT,
    version,
    architecture: process.arch,
    runBuild: () => {
      // 使用当前 pnpm 进程的精确 CLI，避免 Windows PATHEXT 对 .cmd 解析产生差异。
      execFileSync(
        process.execPath,
        [
          pnpmCli,
          'tauri',
          'build',
          '--config',
          '{"bundle":{"active":true}}',
          '--bundles',
          'nsis',
          '--ci',
        ],
        { cwd: REPOSITORY_ROOT, stdio: 'inherit' }
      );
    },
  });

  console.log(`Tauri source artifact: ${result.sourcePath} (${result.sourceSize} bytes)`);
  console.log(`Windows release artifact: ${result.artifactPath}`);
  console.log(`SHA-256: ${result.checksum}`);
  console.log(`Checksum file: ${result.checksumPath}`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    main();
  } catch (error) {
    console.error(
      `Build Windows artifact failed: ${error instanceof Error ? error.message : error}`
    );
    process.exitCode = 1;
  }
}
