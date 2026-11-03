# Nocterm 本地终端 shell integration（bash，macOS / 其它 Unix）。
#
# 职责：在每轮提示符渲染前把上一条命令的真实退出码以 OSC 633;D;<code> 上报，
# 再以 OSC 633;A 标记提示符边界。与 zsh 版本共享 633 协议。
#
# 约束：
# - 经 --rcfile 包装注入：包装脚本先 source 用户原始 .bashrc，再注册钩子；
# - 幂等：重复注入不会叠加包装层；
# - 钩子组合而非覆盖：用户已有的 PROMPT_COMMAND 保留并在本钩子之后执行；
#   （bash 5.1+ 数组形式 PROMPT_COMMAND 会被拼接为字符串，首元素语义保留）
# - nonce 关联：NOCTERM_OSC_NONCE 存在时输出 `;nonce` 尾巴，供后端把事件
#   关联到当前终端 generation；它是关联标识，不是密码学认证；
# - 协议字段：A 事件体为空（`633;A` / `633;A;<nonce>`），D 事件携带退出码
#   （`633;D;<code>` / `633;D;<code>;<nonce>`）——事件体为空时 nonce 直接
#   顶上第二字段，禁止输出 `633;A;;<nonce>` 这类空位序列（后端按畸形丢弃）；
# - builtin 前缀只用于内建命令（printf）；自定义函数直接调用；
# - 首个提示符会发射一次 D（尚无命令时的 $?），后端以计数快照忽略启动期信号。
if [[ -n "$NOCTERM_SHELL_INTEGRATION" ]]; then
  return 0 2>/dev/null || true
fi
NOCTERM_SHELL_INTEGRATION=1

# 组合式输出：$1 = 事件名，$2 = 事件体（A 为空、D 为退出码）；
# nonce 存在时追加为最后一个分号字段。
__nocterm_emit() {
  local fields="$1"
  [[ -n "$2" ]] && fields="$1;$2"
  [[ -n "${NOCTERM_OSC_NONCE:-}" ]] && fields="$fields;$NOCTERM_OSC_NONCE"
  builtin printf '\033]633;%s\007' "$fields"
}

__nocterm_prompt_command() {
  __nocterm_emit "D" "$?"
  __nocterm_emit "A" ""
}
# 组合而非覆盖：本钩子先执行（D 事件尽早到达），用户既有钩子原样保留。
PROMPT_COMMAND="__nocterm_prompt_command${PROMPT_COMMAND:+;$PROMPT_COMMAND}"
