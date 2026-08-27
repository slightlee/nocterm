import { resolve } from 'node:path';
import process from 'node:process';
import { pathToFileURL } from 'node:url';

import { isEnglishAsciiText } from '../commitlint.config.mjs';

export const validatePullRequestTitleLanguage = (title) => {
  if (!title?.trim()) {
    return 'Pull request title is required.';
  }

  // Squash merge 会把 PR 标题写入 main 的永久历史，因此复用 Commitlint 的字符规则。
  return isEnglishAsciiText(title)
    ? null
    : 'Pull request title must use English ASCII text; Issue and PR body may use Chinese or English.';
};

const isMainModule =
  process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href;

if (isMainModule) {
  const error = validatePullRequestTitleLanguage(process.env.PR_TITLE);

  if (error) {
    process.stderr.write(`${error}\n`);
    process.exitCode = 1;
  }
}
