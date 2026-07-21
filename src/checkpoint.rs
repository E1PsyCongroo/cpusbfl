use std::{fs, io::Write, path::Path};

use libafl::prelude::{Corpus, CorpusId, HasCorpus};
use md5::Digest;
use serde::{Deserialize, Serialize};

use crate::fuzzer::{FuzzSession, FuzzerState};

const MAGIC: &[u8] = b"SBFLCORPUS\0";
const FORMAT_VERSION: u32 = 1;
const DIGEST_LEN: usize = size_of::<Digest>();

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

pub(crate) fn save(
    path: impl AsRef<Path>,
    config: &CheckpointConfig,
    session: &FuzzSession,
) -> Result<(), Box<dyn std::error::Error>> {
    let path = path.as_ref();
    let payload = serde_json::to_vec(&CorpusCheckpointRefV1 {
        version: FORMAT_VERSION,
        config,
        initial_corpus_id: session.initial_corpus_id,
        completed_iters: session.completed_iters,
        state: &session.state,
    })?;
    let digest = md5::compute(&payload);

    let mut temporary = temporary_file(path)?;
    temporary.write_all(MAGIC)?;
    temporary.write_all(&digest.0)?;
    temporary.write_all(&payload)?;
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;

    log::info!(
        "Saved SBFL corpus checkpoint: path={}, corpus_cases={}, completed_iters={}",
        path.display(),
        session.state.corpus().count(),
        session.completed_iters,
    );
    Ok(())
}

pub(crate) fn load(
    path: impl AsRef<Path>,
) -> Result<(CheckpointConfig, FuzzSession), Box<dyn std::error::Error>> {
    let path = path.as_ref();
    let bytes = fs::read(path)?;
    let header_len = MAGIC.len() + DIGEST_LEN;
    if bytes.len() < header_len || &bytes[..MAGIC.len()] != MAGIC {
        return Err(format!("{} is not an SBFL corpus checkpoint", path.display()).into());
    }

    let (stored_digest, payload) = bytes[MAGIC.len()..].split_at(DIGEST_LEN);
    let actual_digest = md5::compute(payload);
    if stored_digest != actual_digest.0 {
        return Err(format!(
            "SBFL corpus checkpoint checksum mismatch: {}",
            path.display()
        )
        .into());
    }

    let checkpoint: CorpusCheckpointV1 = serde_json::from_slice(payload)?;
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
        "Loaded SBFL corpus checkpoint: path={}, corpus_cases={}, completed_iters={}",
        path.display(),
        session.state.corpus().count(),
        session.completed_iters,
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
