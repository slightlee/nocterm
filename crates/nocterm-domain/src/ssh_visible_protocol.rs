//! SSH 可见执行的 bash/zsh Marker 数据面协议。
//!
//! 本模块是纯函数协议契约：生成不可预测标识的格式化、构造 Shell 包装器和
//! 解析原始 PTY 输出。随机字节来自基础设施（OS CSPRNG，见
//! `nocterm-infrastructure` 的 marker 生成），Prompt Ready、execution owner、
//! 输入门控等业务状态仍由上层控制面决定。Host 状态层（UI 过滤）与执行用例
//! （结果取回）共享同一份解析，保证两条路径对协议的理解不会漂移。

use std::time::Duration;

/// Marker 观察后释放 owner gate 前的等待窗口：给优先级更高的 nonce-bound OSC
/// 留出到达时间。Marker 只证明包装命令结束，不能证明慢提示符已经完成渲染，
/// 因此 grace 结束也绝不伪造 Prompt Ready。
pub const MARKER_RECOVERY_GRACE: Duration = Duration::from_millis(250);

/// 由 16 字节随机数构造完成标识；格式与 [`completion`] 的解析互为镜像。
/// 随机性由基础设施提供，本函数只做纯格式化，便于无环境测试格式契约。
pub fn marker_from_random(random: &[u8; 16]) -> String {
    let random = random
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("__NOCTERM_SSH_AI_DONE_{random}__")
}

/// bash/zsh 在当前 Shell 顶层执行 `eval`，保留 cd/export/alias 以及顶层语义；
/// 随机 cleanup 函数只清理状态变量，并以原退出码结束供 OSC hook 读取。
pub fn wrapped_command(command: &str, marker: &str) -> String {
    let function_suffix = marker
        .strip_prefix("__NOCTERM_SSH_AI_DONE_")
        .and_then(|value| value.strip_suffix("__"))
        .unwrap_or("fallback");
    let status_name = format!("__nocterm_status_{function_suffix}");
    let cleanup_name = format!("__nocterm_cleanup_{function_suffix}");
    let quoted_command = command.replace('\'', "'\\''");
    format!(
        " eval '{quoted_command}'; {status_name}=$?; printf '{marker}%s__\\n' \"${status_name}\"; {cleanup_name}() {{ local __nocterm_code=\"${status_name}\"; unset {status_name}; unset -f {cleanup_name}; return \"$__nocterm_code\"; }}; {cleanup_name}\n"
    )
}

/// 识别 `<marker><exitCode>__`，返回完整协议片段边界与退出码。
pub fn completion(data: &str, marker: &str) -> Option<(usize, usize, i64)> {
    let mut offset = 0;
    while let Some(relative) = data[offset..].find(marker) {
        let start = offset + relative;
        let code_start = start + marker.len();
        let code_end_relative = data[code_start..].find("__")?;
        let code_end = code_start + code_end_relative;
        if let Ok(code) = data[code_start..code_end].parse::<i64>() {
            let mut end = code_end + 2;
            if data[end..].starts_with("\r\n") {
                end += 2;
            } else if data[end..].starts_with('\n') {
                end += 1;
            }
            return Some((start, end, code));
        }
        offset = code_start;
    }
    None
}

/// 包装器回显锚点：`wrapped_command` 的固定前缀（前导空格配合
/// HISTCONTROL=ignorespace）。可见执行期间输入被门控，用户不可能敲出同样的
/// 行；命令自身输出若在回显窗口内出现该片段会被一并隐藏，与既有启发式
/// 同一取舍级别。
pub const WRAPPER_ECHO_ANCHOR: &str = " eval '";

/// 定位完整包装器回显跨度：从 ` eval '` 锚点所在行到 Marker 所在行（含）。
///
/// 多行命令的包装器回显横跨多行（shell 续行符由 readline 渲染为输出），
/// 只剥 Marker 所在行会把前面的 ` eval '…` 行泄漏到终端；单行命令锚点与
/// Marker 重合，行为不变。返回范围只覆盖包装器回显，前面的异步后台输出
/// 和后面的命令输出都必须保留。
pub fn wrapper_echo_range(data: &str, marker: &str) -> Option<std::ops::Range<usize>> {
    let marker_at = data.find(marker)?;
    let start = wrapper_echo_start(data, marker_at)?;
    let terminator_at = marker_at + data[marker_at..].find(['\r', '\n'])?;
    Some(start..line_end_after(data, terminator_at))
}

/// 终局剥离专用的包装器回显跨度：允许 Marker 所在行尚未终结（缓冲就此结束）。
///
/// AI 结果过滤与 OSC 清理只在缓冲终了时调用，此时“行未终结”意味着完成
/// Marker 先于回显行换行到达的极端分块；流式路径不能使用本函数，否则会把
/// 仍在到达途中的回显行截断。
pub fn wrapper_echo_range_at_end(data: &str, marker: &str) -> Option<std::ops::Range<usize>> {
    let marker_at = data.find(marker)?;
    let start = wrapper_echo_start(data, marker_at)?;
    let end = match data[marker_at..].find(['\r', '\n']) {
        Some(relative) => line_end_after(data, marker_at + relative),
        None => data.len(),
    };
    Some(start..end)
}

/// 包装器跨度的起点：锚点所在行的行首。锚点必须先于 Marker 出现（单行命令
/// 两者同行）；找不到锚点时退回 Marker 所在行——等价旧的保守行为。
fn wrapper_echo_start(data: &str, marker_at: usize) -> Option<usize> {
    match data.find(WRAPPER_ECHO_ANCHOR) {
        Some(anchor_at) if anchor_at <= marker_at => Some(line_start_before(data, anchor_at)),
        _ => Some(line_start_before(data, marker_at)),
    }
}

/// Provider 结果只包含业务输出：去掉包装器回显与内部 Marker。
pub fn visible_result(output: &str, marker: Option<&str>) -> String {
    let Some(marker) = marker else {
        return output.trim().to_string();
    };
    let Some((marker_start, _, _)) = completion(output, marker) else {
        return output.trim().to_string();
    };
    let before_completion = &output[..marker_start];
    // 有 PTY echo 时只删除包含 Marker 字面量的包装器行（终局剥离，允许行尚未
    // 终结）。订阅建立后、包装器回显前到达的后台输出不能被当成内部协议丢弃；
    // 无 echo 时 before_completion 不含 Marker，原样保留全部业务输出。
    let visible = match wrapper_echo_range_at_end(before_completion, marker) {
        Some(range) => format!(
            "{}{}",
            &before_completion[..range.start],
            &before_completion[range.end..]
        ),
        None => before_completion.to_string(),
    };
    visible.trim().to_string()
}

fn line_start_before(data: &str, marker_at: usize) -> usize {
    data[..marker_at]
        .rfind(['\r', '\n'])
        .map_or(0, |index| index + 1)
}

/// 返回换行终止符之后的行尾下标，`\r\n` 视为一个终止符。
fn line_end_after(data: &str, terminator_at: usize) -> usize {
    let mut end = terminator_at + 1;
    if data.as_bytes().get(terminator_at) == Some(&b'\r')
        && data.as_bytes().get(end) == Some(&b'\n')
    {
        end += 1;
    }
    end
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapper_preserves_status_and_result_filter_supports_disabled_pty_echo() {
        let marker = marker_from_random(b"deadbeefdeadbeef");
        let wrapped = wrapped_command("printf 'ok'", &marker);
        assert!(wrapped.contains("eval 'printf '\\''ok'\\'''"));
        assert!(wrapped.contains(&format!("printf '{marker}%s__\\n'")));
        assert!(wrapped.contains("return \"$__nocterm_code\""));

        let echoed = format!("{wrapped}\r\nok\r\n{marker}17__\r\nprompt$ ");
        assert_eq!(visible_result(&echoed, Some(&marker)), "ok");
        let no_echo = format!("first\r\nsecond\r\n{marker}17__\r\nprompt$ ");
        assert_eq!(visible_result(&no_echo, Some(&marker)), "first\r\nsecond");
    }

    #[test]
    fn result_filter_removes_only_the_wrapper_line_after_asynchronous_output() {
        let marker = marker_from_random(b"deadbeefdeadbeef");
        let wrapper = wrapped_command("printf 'ok'", &marker);
        let output = format!("background notice\r\n{wrapper}ok\r\n{marker}0__\r\nprompt$ ");

        assert_eq!(
            visible_result(&output, Some(&marker)),
            "background notice\r\nok"
        );
        let range = wrapper_echo_range(&output, &marker).expect("wrapper echo line");
        assert_eq!(&output[..range.start], "background notice\r\n");
        assert!(output[range.clone()].contains(&marker));
    }

    #[test]
    fn result_filter_strips_unterminated_wrapper_line_before_completion() {
        let marker = marker_from_random(b"deadbeefdeadbeef");
        let wrapper = wrapped_command("printf 'ok'", &marker);
        // 远端行编辑器可能省略回显行的收尾换行。极端分块下完成 Marker 直接
        // 跟在未终结的回显之后，终局剥离必须与 OSC 清理路径一致，把已到达的
        // 回显整段去掉，而不是当作业务输出交给 Provider。
        let echo = wrapper.trim_end();
        let output = format!("{echo}{marker}17__\r\nprompt$ ");
        assert_eq!(visible_result(&output, Some(&marker)), "");

        let range = wrapper_echo_range_at_end(echo, &marker).expect("unterminated echo");
        assert_eq!(range, 0..echo.len());
        // 流式路径必须继续等待换行，不能在行中途截断。
        assert!(wrapper_echo_range(echo, &marker).is_none());
    }

    #[test]
    fn wrapper_echo_span_covers_multiline_command_echo_with_continuation_prompts() {
        let marker = marker_from_random(b"deadbeefdeadbeef");
        // 多行命令：包装器回显横跨两行，readline 在续行前渲染 `> ` 提示符，
        // Marker 位于最后一个源行（printf 片段）。跨度必须从 ` eval '` 行
        // 开始，否则第一行会连同内部协议一起泄漏到终端。
        let echo = format!(
            "user@host:~$  eval 'docker info --format 'A\r\n> B'\\''; \
             __nocterm_status_x=$?; printf '{marker}%s__\\n' \"${{status_name}}\"; \
             __nocterm_cleanup_x() {{ return 3; }}; __nocterm_cleanup_x\r\n"
        );
        let range = wrapper_echo_range(&echo, &marker).expect("multiline wrapper echo");
        assert!(echo[range.clone()].starts_with("user@host:~$  eval '"));
        assert!(!echo[..range.start].contains("eval"));
        assert!(range.end >= echo.len() - 2);

        // 不含锚点的异步输出必须保留（含锚点的行按文档化取舍会被跨度一并
        // 隐藏——执行窗口内该取舍优先于回显整洁）。
        let with_background = format!(
            "background notice\r\n eval 'A\r\n> B'\\''; printf '{marker}%s__\\n' \"x\"; \r\n"
        );
        let range = wrapper_echo_range(&with_background, &marker).expect("wrapper after output");
        assert_eq!(&with_background[..range.start], "background notice\r\n");
        assert!(with_background[range.clone()].starts_with(" eval 'A"));
    }

    #[test]
    fn marker_format_round_trips_through_completion_parser() {
        let marker = marker_from_random(&[0_u8; 16]);
        assert_eq!(
            marker,
            "__NOCTERM_SSH_AI_DONE_00000000000000000000000000000000__"
        );
        // `<marker>` + 一位退出码 + `__`；返回的 end 覆盖到协议片段末尾。
        assert_eq!(
            completion(&format!("out\r\n{marker}3__"), &marker),
            Some((5, 5 + marker.len() + 3, 3))
        );
    }
}
