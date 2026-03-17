//! Content-Length framed message reading for JSON-RPC over stdio.

use tokio::io::{AsyncBufReadExt, AsyncReadExt};

use crate::error::{McpError, McpResult};

/// Read a single Content-Length framed message from a buffered reader.
pub(super) async fn read_message<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
) -> McpResult<Vec<u8>> {
    let mut content_length: Option<usize> = None;

    // Read headers until we hit the empty line.
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            return Err(McpError::Transport("EOF while reading headers".to_string()));
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            // End of headers.
            break;
        }

        if let Some(value) = trimmed.strip_prefix("Content-Length:") {
            content_length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .map_err(|e| McpError::Protocol(format!("Invalid Content-Length: {}", e)))?,
            );
        }
        // Ignore other headers (e.g., Content-Type).
    }

    let length = content_length
        .ok_or_else(|| McpError::Protocol("Missing Content-Length header".to_string()))?;

    let mut body = vec![0u8; length];
    reader.read_exact(&mut body).await?;

    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_read_message_basic() {
        let input = b"Content-Length: 14\r\n\r\n{\"hello\":true}";
        let mut reader = tokio::io::BufReader::new(&input[..]);
        let body = read_message(&mut reader).await.unwrap();
        assert_eq!(body, b"{\"hello\":true}");
    }

    #[tokio::test]
    async fn test_read_message_with_extra_header() {
        let input = b"Content-Length: 2\r\nContent-Type: application/json\r\n\r\n{}";
        let mut reader = tokio::io::BufReader::new(&input[..]);
        let body = read_message(&mut reader).await.unwrap();
        assert_eq!(body, b"{}");
    }

    #[tokio::test]
    async fn test_read_message_missing_content_length() {
        let input = b"X-Custom: foo\r\n\r\n{}";
        let mut reader = tokio::io::BufReader::new(&input[..]);
        let result = read_message(&mut reader).await;
        assert!(result.is_err());
    }
}
