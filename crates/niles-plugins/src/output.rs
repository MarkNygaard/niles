//! A plugin's stdout and stderr, as lines in its log.
//!
//! `println!`, `console.log`, `print()` and a panic's message all land
//! here: stdout as info, stderr as warn. WASI hands over bytes, not
//! lines, so they are gathered until a newline, and an over-long line
//! is let out at the log's line limit rather than kept forever.

use crate::bindings::niles::integration::log::Level;
use crate::runtime::Plugin;
use bytes::Bytes;
use std::io;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use wasmtime_wasi::cli::{IsTerminal, StdoutStream};
use wasmtime_wasi::p2::{OutputStream, Pollable, StreamResult};

const LINE_LIMIT: usize = 2048;

#[derive(Clone)]
pub(crate) struct LogStream {
    plugin: Arc<Plugin>,
    level: Level,
    pending: Arc<Mutex<Vec<u8>>>,
}

impl LogStream {
    pub fn new(plugin: Arc<Plugin>, level: Level) -> Self {
        Self {
            plugin,
            level,
            pending: Arc::default(),
        }
    }

    fn take(&self, bytes: &[u8]) {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        pending.extend_from_slice(bytes);
        while let Some(end) = pending.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = pending.drain(..=end).collect();
            self.emit(&line);
        }
        if pending.len() >= LINE_LIMIT {
            let line: Vec<u8> = std::mem::take(&mut *pending);
            self.emit(&line);
        }
    }

    fn emit(&self, line: &[u8]) {
        let line = String::from_utf8_lossy(line);
        let line = line.trim_end();
        if !line.is_empty() {
            self.plugin.log(self.level, line);
        }
    }
}

impl IsTerminal for LogStream {
    fn is_terminal(&self) -> bool {
        false
    }
}

impl StdoutStream for LogStream {
    fn p2_stream(&self) -> Box<dyn OutputStream> {
        Box::new(self.clone())
    }

    fn async_stream(&self) -> Box<dyn tokio::io::AsyncWrite + Send + Sync> {
        Box::new(self.clone())
    }
}

#[async_trait::async_trait]
impl Pollable for LogStream {
    async fn ready(&mut self) {}
}

impl OutputStream for LogStream {
    fn write(&mut self, bytes: Bytes) -> StreamResult<()> {
        self.take(&bytes);
        Ok(())
    }

    fn flush(&mut self) -> StreamResult<()> {
        Ok(())
    }

    fn check_write(&mut self) -> StreamResult<usize> {
        Ok(64 * 1024)
    }
}

impl tokio::io::AsyncWrite for LogStream {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.take(buf);
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}
