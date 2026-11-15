//! Bounded SSE framing. UTF-8 decoding waits for whole lines, including when a
//! network chunk splits a scalar or CRLF. Reconnection is intentionally absent.
use research_agent_core::model::ModelError;

pub(crate) struct SseDecoder {
    line: Vec<u8>,
    data: String,
    first_line: bool,
    after_cr: bool,
    frame_bytes: usize,
    max_frame_bytes: usize,
}

impl SseDecoder {
    pub fn new(max_frame_bytes: usize) -> Self {
        Self {
            line: Vec::new(),
            data: String::new(),
            first_line: true,
            after_cr: false,
            frame_bytes: 0,
            max_frame_bytes,
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Result<Vec<String>, ModelError> {
        let (frames, failure) = self.feed_partial(bytes);
        failure.map_or(Ok(frames), Err)
    }

    /// Complete frames precede a later framing failure even in one HTTP chunk.
    pub fn feed_partial(&mut self, bytes: &[u8]) -> (Vec<String>, Option<ModelError>) {
        let mut frames = Vec::new();
        let result = (|| {
            for &byte in bytes {
                if self.after_cr {
                    self.after_cr = false;
                    if byte == b'\n' {
                        continue;
                    }
                }
                self.frame_bytes = self
                    .frame_bytes
                    .checked_add(1)
                    .ok_or(ModelError::BudgetExceeded)?;
                if self.frame_bytes > self.max_frame_bytes {
                    return Err(ModelError::BudgetExceeded);
                }
                if byte == b'\r' || byte == b'\n' {
                    self.after_cr = byte == b'\r';
                    self.end_line(&mut frames)?;
                } else {
                    self.line.push(byte);
                }
            }
            Ok::<(), ModelError>(())
        })();
        (frames, result.err())
    }

    fn end_line(&mut self, frames: &mut Vec<String>) -> Result<(), ModelError> {
        let bytes = std::mem::take(&mut self.line);
        let line = std::str::from_utf8(&bytes).map_err(|_| ModelError::InvalidStream)?;
        let line = if std::mem::replace(&mut self.first_line, false) {
            line.strip_prefix('\u{feff}').unwrap_or(line)
        } else {
            line
        };
        if line.is_empty() {
            if !self.data.is_empty() {
                self.data.pop(); // SSE joins data fields with LF, dropping only the final LF.
                frames.push(std::mem::take(&mut self.data));
            }
            self.frame_bytes = 0;
        } else if !line.starts_with(':') {
            let (field, value) = line.split_once(':').unwrap_or((line, ""));
            if field == "data" {
                self.data.push_str(value.strip_prefix(' ').unwrap_or(value));
                self.data.push('\n');
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_network_split_preserves_unicode_bom_crlf_and_multiline_data() {
        let input = "\u{feff}: ping\r\ndata: {\r\ndata: \"text\":\"中文\"}\r\n\r\ndata: [DONE]\n\n"
            .as_bytes();
        for split in 0..=input.len() {
            let mut decoder = SseDecoder::new(512);
            let mut actual = decoder.feed(&input[..split]).unwrap();
            actual.extend(decoder.feed(&input[split..]).unwrap());
            assert_eq!(actual, ["{\n\"text\":\"中文\"}", "[DONE]"]);
        }
    }
    #[test]
    fn bare_cr_comments_and_empty_data_obey_sse_dispatch_rules() {
        let mut decoder = SseDecoder::new(512);
        assert_eq!(
            decoder
                .feed(b": hi\r\rid: 1\rdata\r\rdata:  x \r\r")
                .unwrap(),
            ["", " x "]
        );
        assert!(decoder.feed(b"data: incomplete").unwrap().is_empty());
    }
    #[test]
    fn invalid_utf8_and_unterminated_frame_are_bounded() {
        assert!(matches!(
            SseDecoder::new(20).feed(b"data: \xff\n"),
            Err(ModelError::InvalidStream)
        ));
        let mut decoder = SseDecoder::new(10);
        decoder.feed(b":01234567").unwrap();
        assert!(matches!(
            decoder.feed(b"89"),
            Err(ModelError::BudgetExceeded)
        ));
    }
    #[test]
    fn complete_frames_are_retained_before_a_later_framing_error() {
        let (frames, error) = SseDecoder::new(64).feed_partial(b"data: accepted\n\ndata: \xff\n");
        assert_eq!(frames, ["accepted"]);
        assert_eq!(error, Some(ModelError::InvalidStream));
    }
}
