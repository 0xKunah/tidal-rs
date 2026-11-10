//! A byte buffer filled by a download while the decoder reads it.
//! Reads block until the bytes exist, so decoding can start before the download ends.

use std::io::{self, Read, Seek, SeekFrom};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

#[derive(Default)]
struct State {
	data: Vec<u8>,
	len: Option<u64>,
	finished: bool,
	error: Option<String>,
}

#[derive(Default)]
struct Shared {
	state: Mutex<State>,
	changed: Condvar,
}

impl Shared {
	fn lock(&self) -> MutexGuard<'_, State> {
		self.state
			.lock()
			.unwrap_or_else(|poisoned| poisoned.into_inner())
	}

	fn update(&self, change: impl FnOnce(&mut State)) {
		change(&mut self.lock());
		self.changed.notify_all();
	}
}

pub fn stream_buffer(total_len: Option<u64>) -> (BufferWriter, StreamReader) {
	let shared = Arc::new(Shared::default());
	shared.lock().len = total_len;
	(
		BufferWriter(shared.clone()),
		StreamReader { shared, pos: 0 },
	)
}

/// Download side. Dropping it without `finish` fails the reader.
pub struct BufferWriter(Arc<Shared>);

impl BufferWriter {
	pub fn push(&self, bytes: &[u8]) {
		self.0.update(|state| state.data.extend_from_slice(bytes));
	}

	/// Total size, from `Content-Length`.
	pub fn set_len(&self, len: u64) {
		self.0.update(|state| state.len = Some(len));
	}

	pub fn finish(&self) {
		self.0.update(|state| {
			state.finished = true;
			state.len = Some(state.data.len() as u64);
		});
	}

	pub fn fail(&self, error: String) {
		self.0.update(|state| state.error = Some(error));
	}
}

impl Drop for BufferWriter {
	fn drop(&mut self) {
		self.0.update(|state| {
			if !state.finished && state.error.is_none() {
				state.error = Some("download stopped".into());
			}
		});
	}
}

/// Decoder side.
pub struct StreamReader {
	shared: Arc<Shared>,
	pos: u64,
}

impl StreamReader {
	/// A new reader at position 0 over the same bytes (to rebuild a decoder).
	pub fn reopen(&self) -> StreamReader {
		StreamReader {
			shared: self.shared.clone(),
			pos: 0,
		}
	}

	/// Total size once the download has finished.
	pub fn finished_len(&self) -> Option<u64> {
		let state = self.shared.lock();
		state.finished.then_some(state.data.len() as u64)
	}

	/// Blocks until the total size is known; `None` if the download failed first.
	pub fn wait_len(&self) -> Option<u64> {
		let mut state = self.shared.lock();
		loop {
			if let Some(len) = state.len {
				return Some(len);
			}
			if state.error.is_some() || state.finished {
				return None;
			}
			state = self
				.shared
				.changed
				.wait(state)
				.unwrap_or_else(|poisoned| poisoned.into_inner());
		}
	}
}

impl Read for StreamReader {
	fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
		if buf.is_empty() {
			return Ok(0);
		}
		let mut state = self.shared.lock();
		loop {
			let available = state.data.len() as u64;
			if self.pos < available {
				let start = self.pos as usize;
				let count = buf.len().min(state.data.len() - start);
				buf[..count].copy_from_slice(&state.data[start..start + count]);
				self.pos += count as u64;
				return Ok(count);
			}
			if let Some(error) = &state.error {
				return Err(io::Error::other(error.clone()));
			}
			if state.finished || state.len.is_some_and(|len| self.pos >= len) {
				return Ok(0);
			}
			state = self
				.shared
				.changed
				.wait(state)
				.unwrap_or_else(|poisoned| poisoned.into_inner());
		}
	}
}

impl Seek for StreamReader {
	fn seek(&mut self, target: SeekFrom) -> io::Result<u64> {
		let position = match target {
			SeekFrom::Start(offset) => Some(offset),
			SeekFrom::Current(delta) => self.pos.checked_add_signed(delta),
			SeekFrom::End(delta) => {
				let len = self
					.wait_len()
					.ok_or_else(|| io::Error::other("stream length unknown"))?;
				len.checked_add_signed(delta)
			}
		};
		let position = position
			.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "seek before start"))?;
		self.pos = position;
		Ok(position)
	}
}

#[cfg(test)]
mod tests {
	use std::io::{Read, Seek, SeekFrom};
	use std::time::Duration;

	use super::*;

	#[test]
	fn reader_blocks_until_bytes_arrive() {
		let (writer, mut reader) = stream_buffer(Some(4));
		let feeder = std::thread::spawn(move || {
			std::thread::sleep(Duration::from_millis(50));
			writer.push(&[1, 2, 3, 4]);
			writer.finish();
		});
		let mut bytes = [0u8; 4];
		reader.read_exact(&mut bytes).unwrap();
		assert_eq!(bytes, [1, 2, 3, 4]);
		feeder.join().unwrap();
	}

	#[test]
	fn reader_seeks_inside_written_data() {
		let (writer, mut reader) = stream_buffer(Some(6));
		writer.push(&[1, 2, 3, 4, 5, 6]);
		writer.finish();
		reader.seek(SeekFrom::Start(4)).unwrap();
		let mut bytes = [0u8; 2];
		reader.read_exact(&mut bytes).unwrap();
		assert_eq!(bytes, [5, 6]);
		assert_eq!(reader.seek(SeekFrom::End(-1)).unwrap(), 5);
		assert_eq!(reader.seek(SeekFrom::Current(-2)).unwrap(), 3);
	}

	#[test]
	fn reader_eof_after_finish() {
		let (writer, mut reader) = stream_buffer(None);
		writer.push(&[9]);
		writer.finish();
		let mut all = vec![];
		reader.read_to_end(&mut all).unwrap();
		assert_eq!(all, vec![9]);
	}

	#[test]
	fn reader_errors_after_fail() {
		let (writer, mut reader) = stream_buffer(None);
		writer.fail("boom".into());
		let mut bytes = [0u8; 1];
		assert!(reader.read(&mut bytes).is_err());
	}

	#[test]
	fn reopened_reader_starts_at_zero_and_sees_everything() {
		let (writer, mut reader) = stream_buffer(None);
		writer.push(&[1, 2, 3]);
		let mut first = [0u8; 2];
		reader.read_exact(&mut first).unwrap();
		writer.finish();
		let mut again = reader.reopen();
		let mut all = vec![];
		again.read_to_end(&mut all).unwrap();
		assert_eq!(all, vec![1, 2, 3]);
	}

	#[test]
	fn finished_len_is_known_only_after_finish() {
		let (writer, reader) = stream_buffer(None);
		writer.push(&[1, 2]);
		assert_eq!(reader.finished_len(), None);
		writer.finish();
		assert_eq!(reader.finished_len(), Some(2));
	}

	#[test]
	fn length_becomes_known_when_set() {
		let (writer, reader) = stream_buffer(None);
		writer.set_len(10);
		assert_eq!(reader.wait_len(), Some(10));
	}
}
