/* global console, process */
import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';

export const VERSION_FILES = ['package.json'];

const DESKTOP_PACKAGE_FILE = 'apps/desktop/package.json';
const TAURI_CONFIG_FILE = 'apps/desktop/src-tauri/tauri.conf.json';
const TAURI_PRODUCT_VERSION_SOURCE = '../../../package.json';

const VERSION_PATTERN =
  /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-(alpha|beta|rc)\.([1-9]\d*))?$/;
const CHANNEL_ORDER = { alpha: 0, beta: 1, rc: 2, stable: 3 };

/** 仅接受仓库定义的 SemVer 子集，避免各平台对 build metadata 产生不同排序。 */
export function parseReleaseVersion(version) {
  const match = VERSION_PATTERN.exec(version);
  if (!match) return null;
  return {
    raw: version,
    major: Number(match[1]),
    minor: Number(match[2]),
    patch: Number(match[3]),
    channel: match[4] ?? 'stable',
    sequence: match[5] ? Number(match[5]) : null,
  };
}

/** 返回负数、零或正数，语义与 Array.sort 比较器一致。 */
export function compareReleaseVersions(left, right) {
  const a = typeof left === 'string' ? parseReleaseVersion(left) : left;
  const b = typeof right === 'string' ? parseReleaseVersion(right) : right;
  if (!a || !b) throw new Error('无法比较不符合发布规范的版本号');

  for (const key of ['major', 'minor', 'patch']) {
    if (a[key] !== b[key]) return a[key] - b[key];
  }
  if (a.channel !== b.channel) return CHANNEL_ORDER[a.channel] - CHANNEL_ORDER[b.channel];
  return (a.sequence ?? 0) - (b.sequence ?? 0);
}

/** Tag 是已发布版本的唯一依据；无有效 Tag 时允许建立首个预发布版本。 */
export function findLatestReleaseVersion(tags) {
  return tags
    .map((tag) => (tag.startsWith('v') ? tag.slice(1) : null))
    .filter((version) => version && parseReleaseVersion(version))
    .sort(compareReleaseVersions)
    .at(-1);
}

export function validateReleaseProgression(version, tags) {
  const latestVersion = findLatestReleaseVersion(tags);
  if (!latestVersion || !parseReleaseVersion(version)) return [];
  if (compareReleaseVersions(version, latestVersion) > 0) return [];
  return [`新版本 ${version} 必须高于最新已发布版本 ${latestVersion}`];
}

export function validateVersionEntries(entries) {
  const errors = [];
  for (const [file, version] of entries) {
    if (!parseReleaseVersion(version)) {
      errors.push(`${file} 的版本“${version}”不符合 Nocterm 发布格式`);
    }
  }
  return errors;
}

/** 防止重新引入多个产品版本源；内部 npm/Cargo 包版本不参与桌面产品发布。 */
export function validateVersionSourceConfiguration(readContent) {
  const errors = [];
  const desktopPackage = JSON.parse(readContent(DESKTOP_PACKAGE_FILE));
  const tauriConfig = JSON.parse(readContent(TAURI_CONFIG_FILE));

  if (Object.hasOwn(desktopPackage, 'version')) {
    errors.push(`${DESKTOP_PACKAGE_FILE} 不应声明独立产品版本`);
  }
  if (tauriConfig.version !== TAURI_PRODUCT_VERSION_SOURCE) {
    errors.push(`${TAURI_CONFIG_FILE} 的 version 必须引用 ${TAURI_PRODUCT_VERSION_SOURCE}`);
  }
  return errors;
}

export function findChangedVersionFiles(beforeEntries, afterEntries) {
  const before = new Map(beforeEntries);
  const after = new Map(afterEntries);
  return VERSION_FILES.filter((file) => before.get(file) !== after.get(file));
}

export function validateReleaseCommit(message, changedFiles, version) {
  const subject = message.split(/\r?\n/, 1)[0];
  const expected = `chore: prepare v${version}`;
  const touchesVersion = changedFiles.some((file) => VERSION_FILES.includes(file));
  if (touchesVersion && subject !== expected) {
    return [`版本变更提交信息必须为“${expected}”，当前为“${subject}”`];
  }
  if (!touchesVersion && subject.startsWith('chore: prepare v')) {
    return ['发布提交必须实际修改产品版本'];
  }
  return [];
}

function readVersionFromContent(file, content) {
  const version = JSON.parse(content).version;
  if (typeof version !== 'string') throw new Error(`${file} 缺少字符串 version`);
  return version;
}

function runGit(args) {
  return execFileSync('git', args, { encoding: 'utf8' }).trim();
}

function readEntries(readContent) {
  return VERSION_FILES.map((file) => [file, readVersionFromContent(file, readContent(file))]);
}

function readWorkingTreeEntries() {
  return readEntries((file) => readFileSync(file, 'utf8'));
}

function readGitEntries(ref) {
  return readEntries((file) => runGit(['show', `${ref}:${file}`]));
}

function readReachableReleaseTags(ref) {
  const output = runGit(['tag', '--merged', ref, '--list', 'v*']);
  return output ? output.split('\n') : [];
}

function readAllReleaseTags() {
  const output = runGit(['tag', '--list', 'v*']);
  return output ? output.split('\n') : [];
}

function hasCompleteGitHistory() {
  return runGit(['rev-parse', '--is-shallow-repository']) === 'false';
}

function assertValidEntries(entries, errors) {
  errors.push(...validateVersionEntries(entries));
  return entries[0]?.[1] ?? '';
}

function validateStagedCommit(messageFile, errors) {
  const stagedEntries = readGitEntries('');
  const headEntries = readGitEntries('HEAD');
  const version = assertValidEntries(stagedEntries, errors);
  const changedVersionFiles = findChangedVersionFiles(headEntries, stagedEntries);
  errors.push(
    ...validateReleaseCommit(readFileSync(messageFile, 'utf8'), changedVersionFiles, version)
  );
}

function validateReleaseHistory(baseRef, headRef, version, errors) {
  // PR CI 可能检出平台生成的临时合并提交；显式 headRef 可避免把它误算为第二次版本变更。
  const commitsOutput = runGit(['log', '--format=%H', '--reverse', `${baseRef}..${headRef}`]);
  const commits = commitsOutput ? commitsOutput.split('\n') : [];
  const versionCommits = commits
    .map((commit) => ({
      commit,
      changedVersionFiles: findChangedVersionFiles(
        readGitEntries(`${commit}^`),
        readGitEntries(commit)
      ),
    }))
    .filter(({ changedVersionFiles }) => changedVersionFiles.length > 0);

  if (versionCommits.length !== 1) {
    errors.push(
      `一次发布范围内的版本号必须由一个提交统一修改，当前涉及 ${versionCommits.length} 个提交`
    );
    return;
  }

  const { commit, changedVersionFiles } = versionCommits[0];
  const subject = runGit(['show', '-s', '--format=%s', commit]);
  errors.push(...validateReleaseCommit(subject, changedVersionFiles, version));
}

function validateAgainstBase(baseRef, headRef, errors) {
  if (!baseRef || /^0+$/.test(baseRef)) return;
  const baseEntries = readGitEntries(baseRef);
  const currentEntries = readGitEntries(headRef);
  const currentVersion = assertValidEntries(currentEntries, errors);
  const changedVersionFiles = findChangedVersionFiles(baseEntries, currentEntries);
  if (changedVersionFiles.length === 0) return;

  assertValidEntries(baseEntries, errors);
  if (hasCompleteGitHistory()) {
    errors.push(...validateReleaseProgression(currentVersion, readReachableReleaseTags(baseRef)));
  } else {
    errors.push('发布版本校验需要完整 Git 历史与 Tag，请取消浅克隆后重试');
  }
  validateReleaseHistory(baseRef, headRef, currentVersion, errors);
}

function validateTag(tag, errors) {
  const version = assertValidEntries(readGitEntries('HEAD'), errors);
  const expectedTag = `v${version}`;
  if (tag !== expectedTag) errors.push(`Git Tag 必须为 ${expectedTag}，当前为 ${tag}`);

  if (hasCompleteGitHistory()) {
    const previousTags = readAllReleaseTags().filter((candidate) => candidate !== tag);
    errors.push(...validateReleaseProgression(version, previousTags));
  } else {
    errors.push('发布 Tag 校验需要完整 Git 历史与 Tag，请取消浅克隆后重试');
  }

  try {
    const tagType = runGit(['cat-file', '-t', `refs/tags/${tag}`]);
    if (tagType !== 'tag')
      errors.push(`Git Tag ${tag} 必须是 annotated tag，当前类型为 ${tagType}`);
  } catch {
    errors.push(`无法读取 Git Tag ${tag}`);
  }

  const subject = runGit(['show', '-s', '--format=%s', 'HEAD']);
  const expectedSubject = `chore: prepare ${expectedTag}`;
  if (subject !== expectedSubject) {
    errors.push(`Tag 必须指向发布提交“${expectedSubject}”，当前提交为“${subject}”`);
  }
}

function optionValue(args, option) {
  const index = args.indexOf(option);
  if (index < 0) return null;
  const value = args[index + 1];
  if (!value || value.startsWith('--')) throw new Error(`${option} 缺少参数`);
  return value;
}

export function main(args = process.argv.slice(2)) {
  const errors = [];
  const commitMessageFile = optionValue(args, '--commit-msg');
  const baseRef = optionValue(args, '--base');
  const headRef = optionValue(args, '--head') ?? 'HEAD';
  const tag = optionValue(args, '--tag');

  const readContent = commitMessageFile
    ? (file) => runGit(['show', `:${file}`])
    : baseRef
      ? (file) => runGit(['show', `${headRef}:${file}`])
      : (file) => readFileSync(file, 'utf8');
  errors.push(...validateVersionSourceConfiguration(readContent));
  const entries = commitMessageFile
    ? readGitEntries('')
    : baseRef
      ? readGitEntries(headRef)
      : readWorkingTreeEntries();
  const currentVersion = assertValidEntries(entries, errors);
  if (commitMessageFile) validateStagedCommit(commitMessageFile, errors);
  if (baseRef) validateAgainstBase(baseRef, headRef, errors);
  if (tag) validateTag(tag, errors);

  if (errors.length > 0) throw new Error(errors.map((error) => `- ${error}`).join('\n'));
  console.log(`Release version check passed: ${currentVersion}`);
}

// pathToFileURL 同时处理 POSIX 与 Windows 路径，避免 Windows Hook 只导入而不执行检查。
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    main();
  } catch (error) {
    console.error(
      `Release version check failed:\n${error instanceof Error ? error.message : error}`
    );
    process.exitCode = 1;
  }
}
