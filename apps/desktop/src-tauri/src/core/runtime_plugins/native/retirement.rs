//! Fence a service if its existing host cleanup is cancelled before settlement.
use super::NativeHostError;
use std::sync::atomic::{AtomicBool, Ordering};

pub(crate) struct CleanupGuard<'a> {
    cleanup_failed: &'a AtomicBool,
    completed: bool,
}

impl<'a> CleanupGuard<'a> {
    pub(crate) fn new(cleanup_failed: &'a AtomicBool) -> Self {
        Self {
            cleanup_failed,
            completed: false,
        }
    }

    pub(crate) fn finish(mut self, error: Option<&NativeHostError>) {
        if error.is_some_and(|error| error.cleanup_error.is_some()) {
            self.cleanup_failed.store(true, Ordering::Release);
        }
        self.completed = true;
    }
}

impl Drop for CleanupGuard<'_> {
    fn drop(&mut self) {
        if !self.completed {
            self.cleanup_failed.store(true, Ordering::Release);
        }
    }
}

#[cfg(test)]
mod tests;
