//! Native transport distinction for the shared supervisor. A PTY is a combined
//! terminal stream, not stdout data with stderr silently discarded.
use super::{OutputChunk, PipeProcess, ProcessError, ProcessExit, PtyProcess, TerminalSize};

pub(super) enum Transport {
    Pipe(PipeProcess),
    Terminal(PtyProcess),
}
pub(super) enum TransportOutput {
    Pipe(OutputChunk),
    Terminal(Vec<u8>),
}
impl Transport {
    pub fn interrupt(&self) -> Result<(), ProcessError> {
        match self {
            Self::Pipe(process) => process.interrupt(),
            Self::Terminal(process) => process.interrupt(),
        }
    }
    pub fn resize(&self, size: TerminalSize) -> Result<(), ProcessError> {
        match self {
            Self::Pipe(_) => Err(ProcessError::Unsupported),
            Self::Terminal(process) => process.resize(size),
        }
    }
    pub fn terminate(&mut self) -> Result<(), ProcessError> {
        match self {
            Self::Pipe(process) => process.terminate(),
            Self::Terminal(process) => process.terminate(),
        }
    }
    pub fn exit(&self) -> Option<ProcessExit> {
        match self {
            Self::Pipe(process) => process.exit(),
            Self::Terminal(process) => process.exit(),
        }
    }
    pub async fn next_output(&mut self) -> Result<Option<TransportOutput>, ProcessError> {
        match self {
            Self::Pipe(process) => Ok(process.next_output().await?.map(TransportOutput::Pipe)),
            Self::Terminal(process) => {
                Ok(process.next_output().await?.map(TransportOutput::Terminal))
            }
        }
    }
}
