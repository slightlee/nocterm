# Nocterm 本地终端 shell integration（fish，macOS / 其它 Unix）。
#
# 职责：注册 fish 事件钩子，在每条命令结束后把真实退出码以 OSC 633;D;<code>
# 上报，再以 OSC 633;A 标记提示符边界。
#
# 约束：
# - 经 conf.d 注入风格：脚本在 fish 启动时被自动 source；幂等守卫用 if 包裹
#   主体而不是 exit——conf.d 被 source 进交互 shell，`exit` 会杀掉整个会话
#   （嵌套 fish 继承守卫变量时的典型事故）；
# - 完成信号挂在 fish_postexec 事件：它在每条命令结束后、下一次 prompt 渲染前
#   触发，比按 PWD 变化监听可靠（cd 回同一目录时不会漏报）；
# - nonce 关联：NOCTERM_OSC_NONCE 存在时输出 `;nonce` 尾巴（关联标识，非认证）；
# - 协议字段：A 事件体为空（`633;A` / `633;A;<nonce>`），D 事件携带退出码——
#   禁止输出 `633;A;;<nonce>` 这类空位序列（后端按畸形丢弃）；
# - 首个提示符会发射一次 D（尚无命令时的 status），后端以计数快照忽略启动期信号。
if not test "$NOCTERM_SHELL_INTEGRATION" = 1
    set -gx NOCTERM_SHELL_INTEGRATION 1

    # 组合式输出：$argv[1] = 事件名，$argv[2] = 事件体（A 为空、D 为退出码）；
    # nonce 存在时追加为最后一个分号字段。
    function __nocterm_emit
        set -l fields $argv[1]
        test -n "$argv[2]"; and set fields "$fields;$argv[2]"
        set -q NOCTERM_OSC_NONCE[1]; and set fields "$fields;$NOCTERM_OSC_NONCE"
        printf '\033]633;%s\007' $fields
    end

    function __nocterm_fish_postexec --on-event fish_postexec
        # $status 此刻正是刚结束命令的退出码（iTerm2 同款时序）。
        __nocterm_emit "D" $status
        __nocterm_emit "A" ""
    end
end
