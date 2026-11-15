//! Adapted from OpenAI Codex core/src/unified_exec/head_tail_buffer.rs at
//! e363b08c9175ac1cbe5893615dd2cb9ddf95043b. Copyright 2025 OpenAI. Apache-2.0;
//! see packages/agent-core/third-party/codex/{LICENSE,NOTICE}.
//! Changes: byte slices, explicit structured omissions, no text conversion.
use std::collections::VecDeque;

/// Raw bytes remain separate across the omitted middle. Hosts must render an
/// omission marker when omitted_bytes is nonzero, never concatenate silently.
#[derive(Debug, PartialEq, Eq)]
pub struct RetainedOutput {
    pub head: Vec<u8>,
    pub tail: Vec<u8>,
    pub omitted_bytes: u64,
}
pub(super) struct HeadTailBuffer {
    head_budget: usize,
    tail_budget: usize,
    head: Vec<u8>,
    tail: VecDeque<u8>,
    omitted: u64,
}
impl HeadTailBuffer {
    pub fn new(max_bytes: usize) -> Self {
        Self {
            head_budget: max_bytes / 2,
            tail_budget: max_bytes - max_bytes / 2,
            head: vec![],
            tail: VecDeque::new(),
            omitted: 0,
        }
    }
    pub fn push(&mut self, bytes: &[u8]) {
        let head_len = (self.head_budget - self.head.len()).min(bytes.len());
        self.head.extend_from_slice(&bytes[..head_len]);
        let bytes = &bytes[head_len..];
        if bytes.len() >= self.tail_budget {
            let dropped = bytes.len() - self.tail_budget;
            self.omitted = self
                .omitted
                .saturating_add(self.tail.len() as u64)
                .saturating_add(dropped as u64);
            self.tail.clear();
            self.tail.extend(&bytes[dropped..]);
        } else {
            self.tail.extend(bytes);
            let excess = self.tail.len().saturating_sub(self.tail_budget);
            self.tail.drain(..excess);
            self.omitted = self.omitted.saturating_add(excess as u64);
        }
    }
    pub fn snapshot(&self) -> RetainedOutput {
        RetainedOutput {
            head: self.head.clone(),
            tail: self.tail.iter().copied().collect(),
            omitted_bytes: self.omitted,
        }
    }
    pub fn finish(self) -> RetainedOutput {
        RetainedOutput {
            head: self.head,
            tail: self.tail.into_iter().collect(),
            omitted_bytes: self.omitted,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_prefix_suffix_and_exact_omissions_across_chunk_boundaries() {
        let bytes: Vec<_> = (0..=255).cycle().take(1025).collect();
        for capacity in [0, 1, 2, 3, 16, 1024, 1025, 2048] {
            for chunk_size in [1, 3, 32, 1025] {
                let mut buffer = HeadTailBuffer::new(capacity);
                buffer.push(&[]);
                for chunk in bytes.chunks(chunk_size) {
                    buffer.push(chunk);
                }
                let output = buffer.finish();
                assert_eq!(output.head, bytes[..(capacity / 2).min(bytes.len())]);
                assert_eq!(
                    output.head.len() + output.tail.len(),
                    capacity.min(bytes.len())
                );
                assert_eq!(output.tail, bytes[bytes.len() - output.tail.len()..]);
                assert_eq!(
                    output.omitted_bytes,
                    bytes.len().saturating_sub(capacity) as u64
                );
            }
        }
    }
}
