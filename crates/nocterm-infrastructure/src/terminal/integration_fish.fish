# Nocterm 本地终端 shell integration（fish，macOS / 其它 Unix）。
#
# 职责：注册 fish 的 _nocterm_prompt_preexec/precmd 钩子，在每轮提示符渲染前把
# 上一条命令的真实退出码以 OSC 633;D;<code> 上报，再以 OSC 633;A 标记提示符边界。
#
# 约束：
# - 经 conf.d 注入风格：脚本在 fish 启动时被自动 source，幂等保护防叠加；
# - fish 的 $status 是上一条命令的退出码，precmd 钩子读取时正是目标值；
# - 首个提示符会发射一次 D（尚无命令时的 status），后端以计数快照忽略启动期信号。
if test "$NOCTERM_SHELL_INTEGRATION" = 1
    exit 0 2>/dev/null; or return 0
end
set -gx NOCTERM_SHELL_INTEGRATION 1

function __nocterm_fish_precmd --on-variable PWD
    printf '\033]633;D;%s\007' $status
    printf '\033]633;A\007'
end

function __nocterm_fish_postexec --on-event fish_postexec
    # fish 在命令结束后、下一次 prompt 渲染前触发；状态码由 precmd 读取。
    true
end
