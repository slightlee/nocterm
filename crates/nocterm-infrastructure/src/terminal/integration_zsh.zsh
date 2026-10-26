# Nocterm 本地终端 shell integration（zsh，macOS / 其它 Unix）。
#
# 职责：包装 precmd 钩子，在每轮提示符渲染前把上一条命令的真实退出码以
# OSC 633;D;<code> 上报，再以 OSC 633;A 标记提示符边界。Nocterm 后端解析这些
# 序列判断 AI 命令完成；序列属于控制平面，不参与终端折行，窄窗口下依旧可靠。
#
# 约束：
# - 经临时 ZDOTDIR 注入：包装脚本先加载用户原始配置（.zshrc / .zprofile 等），
#   再注册钩子，用户配置与环境不被破坏（VS Code / Ghostty 同款模式）；
# - 幂等：重复注入不会叠加包装层；
# - 首个提示符会发射一次 D（尚无命令时的 $?），后端以计数快照忽略启动期信号；
# - 真实输出、提示符样式完全交还用户配置，本脚本只追加序列发射。
if [[ "$NOCTERM_SHELL_INTEGRATION" == "1" ]]; then
  return 0 2>/dev/null || true
fi
NOCTERM_SHELL_INTEGRATION=1

# 用户配置已在包装脚本中先于本文件加载；此处只挂 precmd 钩子。
typeset -gi __NOCTERM_LAST_STATUS=0
precmd() {
  builtin printf '\033]633;D;%d\007' "${__NOCTERM_LAST_STATUS}"
  builtin printf '\033]633;A\007'
}
# 命令即将执行：捕获上一轮遗留状态码（此时 $? 仍是上一条命令的值）。
preexec() {
  __NOCTERM_LAST_STATUS=$?
}
