//! SSH 可见执行完成标识的生成。
//!
//! 标识的前后缀格式与解析契约在 [`nocterm_domain::ssh_visible_protocol`]；
//! 随机性来自操作系统 CSPRNG，属于基础设施能力，因此生成在此收口，domain
//! 保持纯函数、无环境依赖。

use nocterm_domain::ssh_visible_protocol::marker_from_random;

/// 为每次可见执行生成独立的 128 位 Marker，避免迟到输出完成后续命令。
pub fn create_completion_marker() -> Result<String, getrandom::Error> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes)?;
    Ok(marker_from_random(&bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_markers_have_independent_128_bit_payloads() {
        let first = create_completion_marker().expect("first marker");
        let second = create_completion_marker().expect("second marker");
        assert_ne!(first, second);
        assert_eq!(first.len(), "__NOCTERM_SSH_AI_DONE___".len() + 32);
    }
}
