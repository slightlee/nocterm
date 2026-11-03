# Nocterm 本地终端 shell integration（PowerShell，pwsh 7 与 Windows PowerShell 5.1 通用）。
#
# 职责：包装 prompt 函数，在每轮提示符渲染前把上一条命令的真实退出码以
# OSC 633;D;<code> 上报，再以 OSC 633;A 标记提示符边界。Nocterm 后端解析这些
# 序列判断 AI 命令完成；序列属于控制平面，不参与终端折行，窄窗口下依旧可靠。
#
# 约束：
# - 本地终端经 -EncodedCommand、SSH 经内存 ScriptBlock 注入；两者都在用户 Profile
#   加载后执行，原 prompt 被完整保留；
# - 兼容 PS 5.1：不使用 `e 转义等 pwsh 7 专属语法，统一用 [char]27 构造 ESC；
# - nonce 关联：NOCTERM_OSC_NONCE 存在时追加为最后一个字段；本地终端未设置时
#   保持既有两字段协议；
# - 首个提示符会发射一次 D;0（尚无命令），后端以计数快照忽略启动期信号；
# - 脚本幂等：重复注入不会叠加包装层。
if ($global:__NOCTERM_SHELL_INTEGRATION -eq $true) { return }
$global:__NOCTERM_SHELL_INTEGRATION = $true

$global:__NOCTERM_ORIGINAL_PROMPT = $function:prompt
$global:__NOCTERM_LAST_ERROR_RECORD = if ($global:Error.Count -gt 0) { $global:Error[0] } else { $null }
function global:__nocterm_emit {
    param([string]$EventName, [AllowNull()][string]$Body)
    $fields = $EventName
    if (-not [string]::IsNullOrEmpty($Body)) { $fields = "$fields;$Body" }
    if (-not [string]::IsNullOrEmpty($env:NOCTERM_OSC_NONCE)) {
        $fields = "$fields;$($env:NOCTERM_OSC_NONCE)"
    }
    $esc = [char]27
    $bel = [char]7
    [Console]::Write("$esc]633;$fields$bel")
}

function global:prompt {
    $succeeded = $?
    # `$LASTEXITCODE` 只在 native 程序运行时更新，cmdlet 失败后可能仍残留更早的值。
    # 新 ErrorRecord 表明本轮由 PowerShell 命令失败，统一映射为 1；没有新记录的
    # native 失败才读取 `$LASTEXITCODE`，从而保留 curl/cmd 等程序的真实退出码。
    # Windows PowerShell 5.1 在函数作用域中可能暴露局部 `$Error` 视图，必须显式
    # 读取 global Error 列表，才能观察到交互命令刚产生的 ErrorRecord。
    $currentError = if ($global:Error.Count -gt 0) { $global:Error[0] } else { $null }
    # 保持单行布尔表达式：Windows PowerShell 5.1 对运算符后的隐式续行行为
    # 与 PowerShell 7 不一致，拆行会使最后一个条件脱离赋值表达式。
    $hasNewPowerShellError = -not $succeeded -and $null -ne $currentError -and -not [object]::ReferenceEquals($currentError, $global:__NOCTERM_LAST_ERROR_RECORD)
    $code = if ($succeeded) {
        0
    } elseif ($hasNewPowerShellError) {
        1
    } elseif ($null -ne $LASTEXITCODE) {
        $LASTEXITCODE
    } else {
        1
    }
    __nocterm_emit "D" "$code"
    $original = if ($null -ne $global:__NOCTERM_ORIGINAL_PROMPT) {
        & $global:__NOCTERM_ORIGINAL_PROMPT
    } else {
        "PS $($executionContext.SessionState.Path.CurrentLocation)> "
    }
    __nocterm_emit "A" $null
    # 原 prompt 也可能产生 ErrorRecord；在渲染完成后更新基线，不能污染下一条命令。
    $global:__NOCTERM_LAST_ERROR_RECORD = if ($global:Error.Count -gt 0) { $global:Error[0] } else { $null }
    $original
}
