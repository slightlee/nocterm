# Nocterm 本地终端 shell integration（zsh，macOS / 其它 Unix）。
#
# 职责：在每轮提示符渲染前把上一条命令的真实退出码以 OSC 633;D;<code> 上报，
# 再以 OSC 633;A 标记提示符边界。Nocterm 后端解析这些序列判断 AI 命令完成；
# 序列属于控制平面，不参与终端折行，窄窗口下依旧可靠。
#
# 约束：
# - 经临时 ZDOTDIR 注入：包装脚本先加载用户原始配置（.zshrc / .zprofile 等），
#   再注册钩子，用户配置与环境不被破坏（VS Code / Ghostty 同款模式）；
# - 幂等：重复注入不会叠加包装层；
# - 钩子追加而非覆盖：用户已有的 precmd_functions / preexec_functions 原样保留；
# - nonce 关联：NOCTERM_OSC_NONCE 存在时输出 `;nonce` 尾巴（关联标识，非认证）；
# - 协议字段：A 事件体为空（`633;A` / `633;A;<nonce>`），D 事件携带退出码——
#   禁止输出 `633;A;;<nonce>` 这类空位序列（后端按畸形丢弃）；
# - builtin 前缀只用于内建命令（printf）；自定义函数直接调用；
# - 首个提示符会发射一次 D（尚无命令时的 $?），后端以计数快照忽略启动期信号；
# - 真实输出、提示符样式完全交还用户配置，本脚本只追加序列发射。
if [[ "$NOCTERM_SHELL_INTEGRATION" == "1" ]]; then
  return 0 2>/dev/null || true
fi
NOCTERM_SHELL_INTEGRATION=1

typeset -gi __NOCTERM_LAST_STATUS=0
# 组合式输出：$1 = 事件名，$2 = 事件体（A 为空、D 为退出码）；
# nonce 存在时追加为最后一个分号字段。
__nocterm_emit() {
  local fields="$1"
  [[ -n "$2" ]] && fields="$1;$2"
  [[ -n "${NOCTERM_OSC_NONCE:-}" ]] && fields="$fields;$NOCTERM_OSC_NONCE"
  builtin printf '\033]633;%s\007' "$fields"
}
# 追加到 zsh 官方钩子数组：用户已注册的 precmd/preexec 不会丢失。
precmd_functions+=(__nocterm_precmd_hook)
preexec_functions+=(__nocterm_preexec_hook)
__nocterm_precmd_hook() {
  __nocterm_emit "D" "${__NOCTERM_LAST_STATUS}"
  __nocterm_emit "A" ""
}
# 命令即将执行：捕获上一轮遗留状态码（此时 $? 仍是上一条命令的值）。
__nocterm_preexec_hook() {
  __NOCTERM_LAST_STATUS=$?
}
