/**
 * 一键彩色：向远程 Shell 发送一条会话级命令，立即开启彩色提示符与
 * `ls` / `grep` 颜色输出。
 *
 * 远程会话全灰的根因是服务器端 Shell 未输出 ANSI 颜色码（`ls` 别名未开、
 * PS1 无色），客户端调色板无从渲染。本命令只改变**当前会话**的 PS1 与
 * 别名，不写入远端任何文件——刷新即失效，安全可逆；永久生效的提示由命
 * 令自身回显在终端里。
 *
 * 兼容性：bash 与 zsh 都支持 `\[\e[..m\]` 提示符转义；POSIX 别名两边的
 * Shell 也都支持。发送内容保持单行，避免对远端行编辑器的多行粘贴语义。
 */
export const REMOTE_COLORIZE_COMMAND = [
  "export PS1='\\[\\e[1;32m\\]\\u@\\h\\[\\e[0m\\]:\\[\\e[1;34m\\]\\w\\[\\e[0m\\]\\$ '",
  "alias ls='ls --color=auto'",
  "alias grep='grep --color=auto'",
  "alias egrep='egrep --color=auto'",
  "alias fgrep='fgrep --color=auto'",
  "echo '[Nocterm] 已为当前会话开启彩色提示符与 ls/grep 颜色；追加到 ~/.bashrc 可永久生效'",
].join(' ; ');
