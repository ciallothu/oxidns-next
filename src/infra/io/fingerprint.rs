// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

//! Sequential reader fingerprinting shared by replayable input formats.

use std::io::{self, Read};

use sha2::{Digest, Sha256};

/// Hash bytes in the order they are read.
///
/// This wrapper intentionally does not implement [`std::io::Seek`]: skipping
/// or rereading bytes would make the digest cease to represent the underlying
/// stream contents. Callers must consume the complete stream before calling
/// [`finish`](Self::finish).
pub(crate) struct FingerprintReader<R> {
    inner: R,
    hasher: Sha256,
}

impl<R> FingerprintReader<R> {
    pub(crate) fn new(inner: R) -> Self {
        Self {
            inner,
            hasher: Sha256::new(),
        }
    }

    pub(crate) fn finish(self) -> [u8; 32] {
        self.hasher.finalize().into()
    }
}

impl<R: Read> Read for FingerprintReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.read(buffer)?;
        self.hasher.update(&buffer[..read]);
        Ok(read)
    }
}
