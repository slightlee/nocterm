# Nocterm 本地终端 shell integration（PowerShell，pwsh 7 与 Windows PowerShell 5.1 通用）。
#
# 职责：包装 prompt 函数，在每轮提示符渲染前把上一条命令的真实退出码以
# OSC 633;D;<code> 上报，再以 OSC 633;A 标记提示符边界。Nocterm 后端解析这些
# 序列判断 AI 命令完成；序列属于控制平面，不参与终端折行，窄窗口下依旧可靠。
#
# 约束：
# - 经 -EncodedCommand 注入，用户 Profile 先于本脚本加载，原 prompt 被完整保留；
# - 兼容 PS 5.1：不使用 `e 转义等 pwsh 7 专属语法，统一用 [char]27 构造 ESC；
# - 首个提示符会发射一次 D;0（尚无命令），后端以计数快照忽略启动期信号；
# - 脚本幂等：重复注入不会叠加包装层。
if ($global:__NOCTERM_SHELL_INTEGRATION -eq $true) { return }
$global:__NOCTERM_SHELL_INTEGRATION = $true

$global:__NOCTERM_ORIGINAL_PROMPT = $function:prompt
function global:prompt {
    $succeeded = $?
    $code = if ($succeeded) { 0 } elseif ($null -ne $LASTEXITCODE) { $LASTEXITCODE } else { 1 }
    $esc = [char]27
    $bel = [char]7
    [Console]::Write("$esc]633;D;$code$bel")
    $original = if ($null -ne $global:__NOCTERM_ORIGINAL_PROMPT) {
        & $global:__NOCTERM_ORIGINAL_PROMPT
    } else {
        "PS $($executionContext.SessionState.Path.CurrentLocation)> "
    }
    [Console]::Write("$esc]633;A$bel")
    $original
}
