use std::{
    fs::{self, File},
    io::{BufReader, Read, Seek, SeekFrom, Write},
    path::Path,
};

use libafl::prelude::{Corpus, CorpusId, HasCorpus};
use md5::{Context, Digest};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::fuzzer::{FuzzSession, FuzzerState};

const MAGIC: &[u8] = b"SBFLCORPUS\0";
const FORMAT_VERSION: u32 = 1;
const DIGEST_LEN: usize = size_of::<Digest>();
const HEADER_LEN: usize = MAGIC.len() + DIGEST_LEN;
const ZSTD_COMPRESSION_LEVEL: i32 = 3;
const DESERIALIZE_BUFFER_SIZE: usize = 64 * 1024;

struct DigestWriter<W> {
    inner: W,
    context: Context,
    bytes_written: u64,
}

impl<W> DigestWriter<W> {
    fn new(inner: W) -> Self {
        Self {
            inner,
            context: Context::new(),
            bytes_written: 0,
        }
    }

    fn finish(self) -> (W, Digest, u64) {
        (self.inner, self.context.compute(), self.bytes_written)
    }
}

impl<W> Write for DigestWriter<W>
where
    W: Write,
{
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let written = self.inner.write(buf)?;
        self.context.consume(&buf[..written]);
        self.bytes_written += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

struct DigestReader<R> {
    inner: R,
    context: Context,
    bytes_read: u64,
}

impl<R> DigestReader<R> {
    fn new(inner: R) -> Self {
        Self {
            inner,
            context: Context::new(),
            bytes_read: 0,
        }
    }

    fn finish(self) -> (R, Digest, u64) {
        (self.inner, self.context.compute(), self.bytes_read)
    }
}

impl<R> Read for DigestReader<R>
where
    R: Read,
{
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(buf)?;
        self.context.consume(&buf[..read]);
        self.bytes_read += read as u64;
        Ok(read)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct CheckpointConfig {
    pub(crate) coverage: String,
    pub(crate) state: String,
    pub(crate) tracker_window_size: usize,
}

#[derive(Deserialize)]
struct CorpusCheckpointV1 {
    version: u32,
    config: CheckpointConfig,
    initial_corpus_id: CorpusId,
    completed_iters: u64,
    state: FuzzerState,
}

#[derive(Serialize)]
struct CorpusCheckpointRefV1<'a> {
    version: u32,
    config: &'a CheckpointConfig,
    initial_corpus_id: CorpusId,
    completed_iters: u64,
    state: &'a FuzzerState,
}

fn parent_dir(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn temporary_file(path: &Path) -> Result<tempfile::NamedTempFile, Box<dyn std::error::Error>> {
    fs::create_dir_all(parent_dir(path))?;
    Ok(tempfile::Builder::new()
        .prefix(".sbfl-corpus-")
        .tempfile_in(parent_dir(path))?)
}

fn encode_payload<T>(
    writer: impl Write,
    value: &T,
) -> Result<(Digest, u64), Box<dyn std::error::Error>>
where
    T: Serialize + ?Sized,
{
    let encoder = zstd::stream::write::Encoder::new(writer, ZSTD_COMPRESSION_LEVEL)?;
    let digest_writer = DigestWriter::new(encoder);
    let digest_writer = postcard::to_io(value, digest_writer)?;
    let (encoder, digest, uncompressed_bytes) = digest_writer.finish();
    encoder.finish()?;
    Ok((digest, uncompressed_bytes))
}

fn decode_payload<T>(reader: impl Read) -> Result<(T, Digest, u64), Box<dyn std::error::Error>>
where
    T: DeserializeOwned,
{
    let decoder = zstd::stream::read::Decoder::new(reader)?;
    let digest_reader = DigestReader::new(decoder);
    let mut buffer = vec![0; DESERIALIZE_BUFFER_SIZE];
    let (value, (mut digest_reader, _)) =
        postcard::from_io((digest_reader, buffer.as_mut_slice()))?;

    let mut trailing = [0_u8; 1];
    if digest_reader.read(&mut trailing)? != 0 {
        return Err("SBFL corpus checkpoint has trailing payload data".into());
    }

    let (_, digest, uncompressed_bytes) = digest_reader.finish();
    Ok((value, digest, uncompressed_bytes))
}

pub(crate) fn save(
    path: impl AsRef<Path>,
    config: &CheckpointConfig,
    session: &FuzzSession,
) -> Result<(), Box<dyn std::error::Error>> {
    let path = path.as_ref();
    let checkpoint = CorpusCheckpointRefV1 {
        version: FORMAT_VERSION,
        config,
        initial_corpus_id: session.initial_corpus_id,
        completed_iters: session.completed_iters,
        state: &session.state,
    };

    let mut temporary = temporary_file(path)?;
    temporary.write_all(MAGIC)?;
    temporary.write_all(&[0; DIGEST_LEN])?;
    let (digest, uncompressed_bytes) = encode_payload(&mut temporary, &checkpoint)?;

    temporary
        .as_file_mut()
        .seek(SeekFrom::Start(MAGIC.len() as u64))?;
    temporary.as_file_mut().write_all(&digest.0)?;
    temporary.as_file_mut().seek(SeekFrom::End(0))?;
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    let checkpoint_bytes = temporary.as_file().metadata()?.len();
    temporary.persist(path).map_err(|error| error.error)?;

    log::info!(
        "Saved SBFL corpus checkpoint: path={}, corpus_cases={}, completed_iters={}, encoding=postcard, compression=zstd, uncompressed_bytes={}, checkpoint_bytes={}",
        path.display(),
        session.state.corpus().count(),
        session.completed_iters,
        uncompressed_bytes,
        checkpoint_bytes,
    );
    Ok(())
}

pub(crate) fn load(
    path: impl AsRef<Path>,
) -> Result<(CheckpointConfig, FuzzSession), Box<dyn std::error::Error>> {
    let path = path.as_ref();
    let file = File::open(path)?;
    let checkpoint_bytes = file.metadata()?.len();
    if checkpoint_bytes < HEADER_LEN as u64 {
        return Err(format!("{} is not an SBFL corpus checkpoint", path.display()).into());
    }

    let mut reader = BufReader::new(file);
    let mut magic = [0; MAGIC.len()];
    reader.read_exact(&mut magic)?;
    if magic != MAGIC {
        return Err(format!("{} is not an SBFL corpus checkpoint", path.display()).into());
    }

    let mut stored_digest = [0; DIGEST_LEN];
    reader.read_exact(&mut stored_digest)?;
    let (checkpoint, actual_digest, uncompressed_bytes): (CorpusCheckpointV1, _, _) =
        decode_payload(reader)?;
    if stored_digest != actual_digest.0 {
        return Err(format!(
            "SBFL corpus checkpoint checksum mismatch: {}",
            path.display()
        )
        .into());
    }

    if checkpoint.version != FORMAT_VERSION {
        return Err(format!(
            "Unsupported SBFL corpus checkpoint version {} (expected {})",
            checkpoint.version, FORMAT_VERSION
        )
        .into());
    }
    let session = FuzzSession::from_restored_state(
        checkpoint.state,
        checkpoint.initial_corpus_id,
        checkpoint.completed_iters,
    )?;
    if session.state.corpus().count() == 0 {
        return Err("SBFL corpus checkpoint has an empty main corpus".into());
    }

    log::info!(
        "Loaded SBFL corpus checkpoint: path={}, corpus_cases={}, completed_iters={}, encoding=postcard, compression=zstd, uncompressed_bytes={}, checkpoint_bytes={}",
        path.display(),
        session.state.corpus().count(),
        session.completed_iters,
        uncompressed_bytes,
        checkpoint_bytes,
    );
    Ok((checkpoint.config, session))
}

pub(crate) fn validate_config(
    saved: &CheckpointConfig,
    requested: &CheckpointConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    if saved != requested {
        return Err(format!(
            "SBFL corpus checkpoint configuration mismatch: saved coverage={:?}, state={:?}, tracker_window_size={}; requested coverage={:?}, state={:?}, tracker_window_size={}",
            saved.coverage,
            saved.state,
            saved.tracker_window_size,
            requested.coverage,
            requested.state,
            requested.tracker_window_size,
        )
        .into());
    }
    Ok(())
}
