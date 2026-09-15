//! Bridge 两端共用的有界换行 JSON 传输。

use std::{
    io::{self, BufRead, Read, Write},
    net::TcpStream,
};

use serde_json::Value;

pub(super) fn write_json(stream: &mut TcpStream, value: &Value) -> io::Result<()> {
    let payload = serde_json::to_vec(value).map_err(io::Error::other)?;
    stream.write_all(&payload)?;
    stream.write_all(b"\n")
}

/// MCP stdio 传输使用换行分隔 JSON-RPC 消息，单条消息设上限避免异常输入占满内存。
pub(super) fn read_bounded_line(
    reader: &mut impl BufRead,
    limit: usize,
) -> io::Result<Option<Vec<u8>>> {
    let mut line = Vec::new();
    // 额外读取 CRLF 两个分隔字节，限制只计算 JSON 负载本身。
    let mut bounded = reader.take(limit.saturating_add(2) as u64);
    if bounded.read_until(b'\n', &mut line)? == 0 {
        return Ok(None);
    }
    while matches!(line.last(), Some(b'\r' | b'\n')) {
        line.pop();
    }
    if line.len() > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "message exceeds size limit",
        ));
    }
    Ok((!line.is_empty()).then_some(line))
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::read_bounded_line;
    use crate::commands::ai_bridge::MAX_MESSAGE_BYTES;

    #[test]
    fn reads_newline_delimited_json() {
        let mut reader = std::io::BufReader::new(Cursor::new(
            br#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}
"#,
        ));
        let line = read_bounded_line(&mut reader, MAX_MESSAGE_BYTES)
            .expect("read message")
            .expect("mcp message");
        let value: serde_json::Value = serde_json::from_slice(&line).expect("json message");
        assert_eq!(value["method"], "tools/list");
    }

    #[test]
    fn rejects_a_message_larger_than_the_limit() {
        let input = vec![b'a'; MAX_MESSAGE_BYTES + 1];
        let mut reader = std::io::BufReader::new(Cursor::new(input));

        let error = read_bounded_line(&mut reader, MAX_MESSAGE_BYTES)
            .expect_err("oversized message must fail");

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn accepts_a_message_at_the_exact_limit_with_crlf() {
        let mut input = vec![b'a'; MAX_MESSAGE_BYTES];
        input.extend_from_slice(b"\r\n");
        let mut reader = std::io::BufReader::new(Cursor::new(input));

        let line = read_bounded_line(&mut reader, MAX_MESSAGE_BYTES)
            .expect("read bounded message")
            .expect("message");

        assert_eq!(line.len(), MAX_MESSAGE_BYTES);
    }
}
