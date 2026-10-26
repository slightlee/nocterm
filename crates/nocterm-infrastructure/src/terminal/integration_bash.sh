# Nocterm 本地终端 shell integration（bash，macOS / 其它 Unix）。
#
# 职责：包装 PROMPT_COMMAND，在每轮提示符渲染前把上一条命令的真实退出码以
# OSC 633;D;<code> 上报，再以 OSC 633;A 标记提示符边界。与 zsh 版本共享 633 协议。
#
# 约束：
# - 经 --rcfile 包装注入：包装脚本先 source 用户原始 .bashrc，再注册钩子；
# - 幂等：重复注入不会叠加包装层；
# - bash 不区分 precmd/preexec，靠 PROMPT_COMMAND 与历史/退出码捕获；
# - 首个提示符会发射一次 D（尚无命令时的 $?），后端以计数快照忽略启动期信号。
if [[ -n "$NOCTERM_SHELL_INTEGRATION" ]]; then
  return 0 2>/dev/null || true
fi
NOCTERM_SHELL_INTEGRATION=1

__nocterm_prompt_command() {
  builtin printf '\033]633;D;%s\007' "$?"
  builtin printf '\033]633;A\007'
}
PROMPT_COMMAND=__nocterm_prompt_command
