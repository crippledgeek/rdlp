//! Resume detection and chunk merging functionality

use super::{Orchestrator, errors::Result};
use anyhow::Context;
use log::{debug, warn};
use rdlp_downloader::{ChunkKind, ChunkManifest, ChunkSet, intact_len};
use std::path::{Path, PathBuf};
use tracing::instrument;

/// Upper bound on how many distinct download attempts (`download_id`s) a
/// scan probes for new-style chunks. Not a grammar constraint — `download_id`
/// is a monotonic per-process counter starting at 0, so any in-progress or
/// recently-abandoned download is well within this window.
const MAX_DOWNLOAD_ID_SCAN: u64 = 100;

/// Ceiling on how many sequential chunk ids [`cleanup_old_chunks`] and
/// [`log_orphaned_resume_chunks`] probe within one chunk set before
/// concluding the set is exhausted.
///
/// This is a sanity ceiling against a pathological or corrupted directory,
/// not a live limit on any grammar: new-style power-of-two chunking (and the
/// resume grammar, which chunks the same way) can legitimately produce
/// thousands of small chunks, and — per #559's investigation — the legacy
/// grammar's historical `concurrent_fragments` cap of 10 was never itself
/// enforced as a scan/cleanup bound. A hardcoded `0..10` cleanup bound would
/// silently strand any legacy chunk set that ever did exceed 10 (see
/// `test_cleanup_legacy_chunks_beyond_old_ten_chunk_bound`); using the same
/// generous ceiling everywhere removes that trap.
///
/// Both of those functions probe the FILESYSTEM directly (one `stat`/
/// `remove_file` per candidate id) rather than trusting a manifest, so a
/// fixed ceiling is safe there: real chunk sets never come close to it, and
/// an attacker can at most make the probe run to completion (no I/O is
/// skipped, it always terminates). [`collect_contiguous_chunks`] does NOT
/// use this — an id-span ceiling on a manifest's own (attacker-writable)
/// keys rejected legitimate large-file manifests from the adaptive
/// download path (its chunk size floors at 256 KiB indefinitely rather
/// than shrinking further), so that scan is bounded by the manifest's
/// entry COUNT instead (see that function's doc).
const CHUNK_SCAN_CEILING: u64 = 10_000;

/// The first chunk id that broke a manifest-verified prefix, and both
/// lengths involved — the manifest's recorded length and whatever length
/// (if any) is actually on disk. Carries enough for a caller to report a
/// truncation precisely (id + both lengths) rather than just its existence,
/// and for a test to assert on that report directly instead of on a log
/// line (#675 spec review finding 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BadChunk {
    /// The chunk id where the verified prefix stops.
    pub id: u64,
    /// The length actually on disk at `id`, or `None` if no file is there.
    pub on_disk: Option<u64>,
    /// The length the manifest recorded for `id`, or `None` if the manifest
    /// never recorded this id as complete (a real gap, not corruption).
    pub recorded: Option<u64>,
}

/// Information about detected chunk files
#[derive(Debug, Clone)]
pub struct ChunkInfo {
    /// Download ID (None for old-style chunks without ID)
    pub(crate) download_id: Option<u64>,
    /// Chunk file paths in order
    pub(crate) chunk_paths: Vec<PathBuf>,
    /// Recorded length for each entry in `chunk_paths`, same order. Always
    /// the manifest-verified length (#675) — a chunk set never reaches this
    /// field unless every entry already passed [`intact_len`] against a
    /// trusted record, so `merge_chunk_files` has something to re-check the
    /// actual copy against.
    pub(crate) chunk_lengths: Vec<u64>,
    /// Total size of all chunks (sum of `chunk_lengths`)
    pub(crate) total_size: u64,
    /// The chunk id (and both lengths) that broke the verified prefix, if
    /// any. `Some` whenever the manifest recorded more than what could be
    /// verified — a gap or a corrupt/truncated chunk — so a caller (or a
    /// test) can assert the truncation was reported, not just that the
    /// returned offset was smaller than expected (#675).
    pub(crate) first_bad: Option<BadChunk>,
    /// Chunks the manifest still records as complete beyond `first_bad` —
    /// never merged, but counted so the truncation's extent is reported
    /// rather than silent (#675).
    pub(crate) stranded_beyond_gap: usize,
}

/// Outcome of probing one manifest-backed chunk set for a verified,
/// contiguous prefix starting at chunk id 0. `pub` (like [`ChunkInfo`]) so
/// tests can assert on `first_bad`/`stranded_beyond_gap` directly even when
/// the outer [`detect_chunk_files`] would return `None` (chunk 0 itself
/// unreachable leaves `chunk_paths` empty, which `detect_chunk_files`
/// doesn't surface as a `ChunkInfo` at all).
#[derive(Debug, Clone, Default)]
pub struct ContiguousChunks {
    pub chunk_paths: Vec<PathBuf>,
    pub chunk_lengths: Vec<u64>,
    pub total_size: u64,
    pub first_bad: Option<BadChunk>,
    pub stranded_beyond_gap: usize,
}

/// Probe `set` for a manifest-verified, contiguous run of chunk ids starting
/// at 0, accepting only chunks whose on-disk length exactly matches what
/// `manifest` recorded ([`intact_len`]) — never a bare non-empty-file check
/// (#675). Read-only: builds each candidate path via [`ChunkSet::path_in`]
/// and checks its metadata, never enumerating the directory's contents.
///
/// Walks `manifest.completed` itself (a `BTreeMap`, so iteration is already
/// in ascending key order) rather than an `0..=max_id` id-span loop: cost
/// and iteration count are both O(the manifest's own entry count), never a
/// function of the id VALUES it contains. An earlier version of this
/// function scanned by id span, capped at a fixed ceiling to stop an
/// attacker-controlled `{"<huge id>": len}` entry from making the scan
/// effectively unbounded — but the ceiling was itself wrong for real
/// inputs: the adaptive download path's chunk size floors at 256 KiB
/// indefinitely (`CHUNK_LEVELS[MIN_CHUNK_LEVEL]`, never shrinking further
/// the way the static `Auto` strategy does), so any download past ~2.44
/// GiB can legitimately produce more chunks than any reasonable fixed
/// ceiling. Walking the map instead removes the need for a ceiling at all —
/// a huge id key costs one comparison, not an iteration up to its value —
/// and the resource that DOES need bounding (the sidecar file's byte size)
/// is bounded at the read, in `atomic::read_json_sidecar`'s
/// `MAX_SIDECAR_BYTES` cap, not here.
///
/// At position `index` (0-based) in that ordered walk, the verified prefix
/// holds exactly while the entry's key equals `index`: a key greater than
/// `index` means id `index` itself was never recorded (a real gap), and a
/// matching key whose on-disk length doesn't verify means that entry itself
/// is corrupt/truncated. Either way scanning stops there; every entry
/// strictly beyond the break is "stranded" — still recorded complete, but
/// unreachable past it — and reported via [`ContiguousChunks::stranded_beyond_gap`]
/// rather than silently dropped, per #675's acceptance criteria.
pub async fn collect_contiguous_chunks(
    set: &ChunkSet,
    parent_dir: &Path,
    manifest: &ChunkManifest,
) -> ContiguousChunks {
    let mut out = ContiguousChunks::default();
    let total_entries = manifest.completed.len();

    for (index, (&chunk_id, &recorded_len)) in manifest.completed.iter().enumerate() {
        let Ok(expected_id) = u64::try_from(index) else {
            // A manifest with more than u64::MAX entries cannot exist in
            // practice (it would exceed any real filesystem's capacity many
            // times over); treated as a break rather than panicking.
            out.first_bad = Some(BadChunk {
                id: chunk_id,
                on_disk: None,
                recorded: Some(recorded_len),
            });
            break;
        };

        if chunk_id != expected_id {
            // A real gap: id `expected_id` was never recorded as complete.
            // No file is expected here either, so `on_disk` is left `None`
            // rather than spending a probe on an id nothing claims. This
            // entry (`chunk_id`) and everything after it are stranded.
            out.first_bad = Some(BadChunk {
                id: expected_id,
                on_disk: None,
                recorded: None,
            });
            out.stranded_beyond_gap = total_entries - index;
            break;
        }

        let chunk_path = set.path_in(parent_dir, chunk_id);
        let on_disk_len = tokio::fs::metadata(&chunk_path).await.ok().map(|m| m.len());
        if let Some(verified) = on_disk_len.and_then(|len| intact_len(len, recorded_len)) {
            out.total_size += verified;
            out.chunk_lengths.push(verified);
            out.chunk_paths.push(chunk_path);
        } else {
            // Missing file, or present but truncated/grown since it was
            // recorded — either way this id itself cannot be trusted. It
            // IS the break, not something stranded "beyond" it, so it is
            // excluded from the stranded count.
            out.first_bad = Some(BadChunk {
                id: chunk_id,
                on_disk: on_disk_len,
                recorded: Some(recorded_len),
            });
            out.stranded_beyond_gap = total_entries - index - 1;
            break;
        }
    }

    out
}

/// One chunk set discovered on disk that could not be verified for merging,
/// recorded so the scan can decide — only once every candidate has been
/// probed — how loudly to report it.
struct UnverifiableClaim {
    sentinel: PathBuf,
    reason: &'static str,
}

/// Probe `set` for an unverifiable claim: chunk 0's presence (mirrors the
/// sentinel [`cleanup_old_chunks`] and [`log_orphaned_resume_chunks`]
/// already use) so the overwhelmingly common "nothing here" case costs one
/// metadata call, not a claim per unused `download_id`.
async fn probe_unverifiable_claim(
    set: &ChunkSet,
    parent_dir: &Path,
    reason: &'static str,
) -> Option<UnverifiableClaim> {
    let sentinel = set.path_in(parent_dir, 0);
    tokio::fs::metadata(&sentinel)
        .await
        .is_ok()
        .then_some(UnverifiableClaim { sentinel, reason })
}

/// Build the operator-facing message for one unverifiable-chunk-set claim,
/// worded according to whether anything ELSE from this same scan is going
/// to be merged.
///
/// Code-quality review finding: the previous fixed wording ("this download
/// will start fresh. The files are left in place.") was emitted for every
/// unverifiable set unconditionally — including a legacy set sitting
/// alongside a verified new-style set that DOES merge (nothing here starts
/// fresh), and including a legacy set whose files `cleanup_old_chunks`
/// unconditionally deletes moments later (not "left in place"). The
/// severity and wording are only correct once the scan knows whether
/// `all_chunks` ended up empty.
fn unverifiable_claim_message(claim: &UnverifiableClaim, anything_merged: bool) -> String {
    if anything_merged {
        format!(
            "Found chunk files next to '{}' that cannot be verified ({}); \
             not merging them — another verified chunk set from this scan is used instead.",
            claim.sentinel.display(),
            claim.reason
        )
    } else {
        format!(
            "Found chunk files next to '{}' that cannot be verified ({}); \
             not merging them — this download will start fresh. The files are \
             left in place.",
            claim.sentinel.display(),
            claim.reason
        )
    }
}

/// Emit every collected claim at the severity `anything_merged` implies:
/// `warn!` only when NOTHING from this scan merges (a genuine fresh start
/// the operator should know about); a quiet `debug!` otherwise (a sibling
/// set already covers this download, so the claim is informational).
fn emit_unverifiable_claims(claims: &[UnverifiableClaim], anything_merged: bool) {
    for claim in claims {
        let message = unverifiable_claim_message(claim, anything_merged);
        if anything_merged {
            debug!("{message}");
        } else {
            warn!("{message}");
        }
    }
}

/// Detect chunk files for a given output path
///
/// Supports both:
/// - Old-style: `{filename}.part{i}` (Phase 2 coarse-grained chunking)
/// - New-style: `{filename}.{downloadid}.part{i}` (Phase 2.5+ power-of-two chunking)
///
/// A chunk set — of either grammar — is only ever merged when its chunk
/// lengths were verified against an on-disk manifest (#675): any non-empty
/// file used to be trusted at its truncated length, and a set with no
/// manifest is unverifiable rather than assumed intact. The legacy grammar
/// predates the manifest and has no writer that could ever produce one, so
/// legacy chunk sets are now permanently unverifiable and are never merged
/// — a deliberate policy decision (#675), not an oversight.
///
/// Returns the most recent verified chunk set (highest download ID)
pub async fn detect_chunk_files(output_path: &Path) -> Option<ChunkInfo> {
    // Validates the precondition every `ChunkSet` constructor below shares.
    output_path.file_name()?;
    let parent_dir = output_path.parent().unwrap_or_else(|| Path::new("."));

    let mut all_chunks: Vec<ChunkInfo> = Vec::new();
    let mut unverifiable_claims: Vec<UnverifiableClaim> = Vec::new();

    // 1. Check for new-style chunks: {filename}.{downloadid}.part{i}
    for download_id in 0..MAX_DOWNLOAD_ID_SCAN {
        // Precondition already validated above; a failure here can only mean
        // `output_path` changed underneath us mid-scan, so skip rather than panic.
        let Ok(set) = ChunkSet::for_attempt(output_path, download_id, ChunkKind::Fresh) else {
            break;
        };
        // `for_attempt` with a real filename always has a manifest path.
        let Some(manifest_path) = set.manifest_path_in(parent_dir) else {
            continue;
        };
        if let Some(manifest) =
            ChunkManifest::load_matching(&manifest_path, download_id, ChunkKind::Fresh).await
        {
            let scanned = collect_contiguous_chunks(&set, parent_dir, &manifest).await;
            // Gated on `first_bad`, not `stranded_beyond_gap > 0`: a
            // truncated LAST chunk (nothing recorded beyond it) leaves
            // `stranded_beyond_gap` at 0 but must still be reported — the
            // whole point of #675 is that this truncation is never silent.
            if let Some(bad) = scanned.first_bad {
                warn!(
                    download_id,
                    bad_chunk_id = bad.id,
                    on_disk = bad.on_disk,
                    recorded = bad.recorded,
                    stranded = scanned.stranded_beyond_gap;
                    "Chunk set for download_id {download_id} breaks at chunk {} \
                     (on-disk: {:?} bytes, recorded: {:?} bytes); {} chunk(s) beyond it \
                     are still recorded complete but cannot be merged past the break and \
                     are left in place",
                    bad.id,
                    bad.on_disk,
                    bad.recorded,
                    scanned.stranded_beyond_gap,
                );
            }
            if !scanned.chunk_paths.is_empty() {
                all_chunks.push(ChunkInfo {
                    download_id: Some(download_id),
                    chunk_paths: scanned.chunk_paths,
                    chunk_lengths: scanned.chunk_lengths,
                    total_size: scanned.total_size,
                    first_bad: scanned.first_bad,
                    stranded_beyond_gap: scanned.stranded_beyond_gap,
                });
            }
        } else {
            // "No usable manifest" covers three distinct causes — a missing
            // file, a parse failure, and a schema/download_id/kind
            // mismatch. Distinguish the middle two (a file existed but was
            // rejected) from the common "nothing here" case.
            if tokio::fs::metadata(&manifest_path).await.is_ok() {
                debug!(
                    download_id;
                    "Chunk manifest at {} exists but was rejected (parse failure or a \
                     schema/download_id/kind mismatch) — treating as absent",
                    manifest_path.display()
                );
            }
            if let Some(claim) =
                probe_unverifiable_claim(&set, parent_dir, "no usable chunk manifest").await
            {
                unverifiable_claims.push(claim);
            }
        }
    }

    // 2. Check for old-style (legacy) chunks: {filename}.part{i}
    // Precondition already validated above, so `Err` can only mean `output_path`
    // changed underneath us mid-scan: skip the legacy grammar and report whatever
    // the new-style scan already found, rather than duplicating the reduction.
    match ChunkSet::legacy(output_path) {
        Ok(legacy_set) => {
            if let Some(claim) = probe_unverifiable_claim(
                &legacy_set,
                parent_dir,
                "the legacy chunk grammar predates chunk manifests and can never have one",
            )
            .await
            {
                unverifiable_claims.push(claim);
            }
        }
        Err(_) => debug!("Skipping legacy chunk scan: output path lost its filename mid-scan"),
    }

    // 3. Return the most recent verified chunk set (highest download ID).
    // Claims are only emitted once the outcome is known: `warn!` when
    // NOTHING merges (a genuine fresh start), `debug!` when a sibling set
    // covers this download instead — see `unverifiable_claim_message`.
    let result = all_chunks
        .into_iter()
        .max_by_key(|info| info.download_id.unwrap_or(0));
    emit_unverifiable_claims(&unverifiable_claims, result.is_some());
    result
}

/// Merge chunk files into the output file
///
/// Supports both old-style and new-style chunk patterns
pub async fn merge_chunk_files(output_path: &Path, chunk_info: &ChunkInfo) -> anyhow::Result<u64> {
    use tokio::fs::File;
    use tokio::io::{AsyncWriteExt, BufWriter};

    let chunk_count = chunk_info.chunk_paths.len();
    let chunk_type = chunk_info.download_id.map_or_else(
        || "old-style".to_owned(),
        |id| format!("new-style (ID: {id})"),
    );

    debug!(chunk_count, chunk_type:?; "Merging chunks");

    anyhow::ensure!(
        chunk_info.chunk_lengths.len() == chunk_count,
        "chunk_lengths ({}) and chunk_paths ({chunk_count}) must be the same length",
        chunk_info.chunk_lengths.len()
    );

    // Create output file
    let file = File::create(output_path)
        .await
        .with_context(|| format!("failed to create output file {}", output_path.display()))?;
    let mut writer = BufWriter::with_capacity(2 * 1024 * 1024, file); // 2 MB buffer

    let mut total_size = 0u64;

    // Merge each chunk in order
    for (idx, chunk_path) in chunk_info.chunk_paths.iter().enumerate() {
        if !chunk_path.exists() {
            anyhow::bail!("missing chunk file: {}", chunk_path.display());
        }

        let mut chunk_file = File::open(chunk_path)
            .await
            .with_context(|| format!("failed to open chunk file {}", chunk_path.display()))?;

        let bytes_copied = tokio::io::copy(&mut chunk_file, &mut writer)
            .await
            .with_context(|| format!("failed to copy chunk {} to output", chunk_path.display()))?;

        // #675: the length a chunk was verified at during detection must
        // still hold at merge time — closes the (narrow) race between the
        // scan and this copy, the same way `verify_merged_size` closes it
        // for the assembled whole.
        let expected = *chunk_info
            .chunk_lengths
            .get(idx)
            .context("chunk_lengths shorter than chunk_paths despite the length check above")?;
        anyhow::ensure!(
            bytes_copied == expected,
            "chunk {} changed size between detection and merge: expected {expected} bytes, copied {bytes_copied}",
            chunk_path.display()
        );

        total_size += bytes_copied;

        // Progress update every 100 chunks
        if (idx + 1) % 100 == 0 || idx == chunk_count - 1 {
            debug!(merged = idx + 1, total = chunk_count; "   Merge progress");
        }

        // Delete chunk file after successful merge
        tokio::fs::remove_file(chunk_path)
            .await
            .with_context(|| format!("failed to remove chunk file {}", chunk_path.display()))?;
    }

    anyhow::ensure!(
        total_size == chunk_info.total_size,
        "merged total {total_size} bytes does not match the recorded sum of {} bytes",
        chunk_info.total_size
    );

    writer
        .flush()
        .await
        .context("failed to flush merged output file")?;

    debug!(chunk_count; "Cleaned up chunk files");

    // The manifest is deleted HERE, not from a probed path elsewhere (the
    // #573 pattern this rule exists to avoid: never delete a path this
    // function didn't itself just finish consuming) — but ONLY when the
    // manifest is fully consumed: `first_bad.is_some()` means chunks beyond
    // the gap/corruption are still recorded complete but were NOT part of
    // this merge (`chunk_paths` stopped at the break). Deleting the
    // manifest in that case would be the exact silence #675 exists to
    // prevent: those stranded chunks would lose their only record and
    // never be reported again by a later scan. Not deleted on any failure
    // path above (`bail!`/`ensure!` return before reaching here) for the
    // same reason — a partially-merged or failed attempt's manifest must
    // survive to be re-scanned.
    if chunk_info.first_bad.is_none()
        && let Some(id) = chunk_info.download_id
        && let Ok(set) = ChunkSet::for_attempt(output_path, id, ChunkKind::Fresh)
    {
        let parent_dir = output_path.parent().unwrap_or_else(|| Path::new("."));
        if let Some(manifest_path) = set.manifest_path_in(parent_dir) {
            let _ = tokio::fs::remove_file(manifest_path).await;
        }
    }

    Ok(total_size)
}

/// Clean up legacy-grammar chunks (`{filename}.part{i}`) when they are no
/// longer going to be used for resume (a complete/oversized/simple-partial
/// file was found, or new-style chunks were used instead).
///
/// Deletes only exact paths computed via [`ChunkSet::path_in`] — never a
/// directory sweep (`scripts/check-no-dir-sweep-delete.sh`, #558).
///
/// # Sentinel gate
///
/// This runs on essentially every `detect_resume_point` call (up to 3x per
/// call — see the branches below), so its cost has to be bounded for the
/// overwhelmingly common case where there is no legacy chunk set at all.
/// Chunk id 0 is checked first: a legacy download always writes id 0 first,
/// so its absence means there is nothing here to clean up, and the rest of
/// [`CHUNK_SCAN_CEILING`] is skipped entirely rather than attempting up to
/// 10,000 `remove_file` calls that all resolve to `NotFound`. This mirrors
/// the sentinel [`log_orphaned_resume_chunks`] already uses for the same
/// reason.
///
/// When chunk 0 IS present, the full ceiling is scanned without breaking on
/// a hole: an interrupted adaptive/resume download completes chunks out of
/// order, so a holed set (e.g. `part0`, `part2`, `part5`) is the normal case,
/// and breaking on the first gap would leak the remainder (#568 C1).
///
/// Narrow limitation, shared with `log_orphaned_resume_chunks`: a set that
/// has lost chunk 0 specifically (to some other partial cleanup) is not
/// cleaned by this pass. That trade is acceptable — it bounds the cost of
/// the common "nothing here" case to a single syscall, and a legacy chunk
/// set missing its first chunk is itself an unusual, already-degraded state.
async fn cleanup_old_chunks(output_path: &Path) {
    let Ok(set) = ChunkSet::legacy(output_path) else {
        return;
    };
    let parent_dir = output_path.parent().unwrap_or_else(|| Path::new("."));

    let mut deleted = 0;
    let sentinel = set.path_in(parent_dir, 0);
    match tokio::fs::remove_file(&sentinel).await {
        Ok(()) => deleted += 1,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
        Err(e) => debug!("Failed to remove legacy chunk {}: {e}", sentinel.display()),
    }

    for chunk_id in 1..CHUNK_SCAN_CEILING {
        let chunk_path = set.path_in(parent_dir, chunk_id);
        // Attempt the delete directly instead of probing with a synchronous
        // `exists()` first: `NotFound` tells us exactly what the probe
        // would have, in one syscall instead of two, without blocking the
        // async executor thread (#568 C4). Never break on a miss: a legacy
        // set can be holed same as any other grammar, and the scan bound is
        // [`CHUNK_SCAN_CEILING`], not "first gap".
        match tokio::fs::remove_file(&chunk_path).await {
            Ok(()) => deleted += 1,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => debug!(
                "Failed to remove legacy chunk {}: {e}",
                chunk_path.display()
            ),
        }
    }

    if deleted > 0 {
        debug!(deleted; "Cleaned up legacy-style chunk files");
    }
}

/// Discover orphaned resume-kind chunk sets (`{filename}.{download_id}.resume{i}`)
/// left behind by an abandoned resume attempt, and log them for the operator
/// to remove manually (refs #568, #559, #571 HIGH). **Never deletes.**
///
/// A resume chunk set encodes only a chunk index *relative* to the resume
/// attempt's byte offset — the offset itself is never persisted in the file
/// name or anywhere on disk (see `Attempt::Resume` in
/// `rdlp_downloader::http::parallel`), so a resume chunk set discovered here
/// can never be safely merged: there is no way to recover which byte offset
/// its `chunk_id 0` continues from, and the base file it was meant to extend
/// may since have been deleted (an oversized-file restart) or already
/// resumed independently at a different offset.
///
/// # Why this only logs, and never deletes
///
/// `download_id` is a monotonic counter **per process**, starting at 0. Two
/// rdlp processes downloading to the same output path concurrently are
/// therefore using the same low ids *at the same time* — an existence probe
/// here cannot tell "abandoned by a past attempt" apart from "being written
/// right now by a live peer" (the exact defect class #558 was about, one
/// crate over). `TempRegistry::cleanup_stale` in `rdlp-postprocess`
/// (`pipeline::registry`) earns the right to delete by requiring an fs4
/// exclusive advisory lock on a sidecar that its own writer
/// (`FileTracker::register`) takes at creation time. The downloader's chunk
/// writer takes no equivalent lock on chunk files, so reproducing that proof
/// here would require a cross-crate change to `rdlp-downloader`'s write
/// path — a real option, but out of proportion to the benefit: an orphaned
/// resume set can never be merged anyway (see above), so all automatic
/// cleanup would reclaim is a few megabytes of disk, and #568's acceptance
/// criterion 4 explicitly allows "documented as requiring manual cleanup" as
/// an alternative to automatic reclamation. This function takes that
/// alternative: discover, log with the exact paths implied by
/// `download_id`, and leave removal to the operator.
///
/// # Hole tolerance
///
/// Never breaks a per-`download_id` scan on the first missing chunk id:
/// resume chunks complete out of order on the adaptive path (`try_buffer_unordered`),
/// so a holed set (e.g. `resume0`, `resume2`, `resume4`) is the NORMAL case,
/// not evidence the set ends there (#568 C1). Chunk id 0 is checked first as
/// a cheap sentinel — a resume attempt always schedules id 0 immediately, so
/// its absence means this `download_id` was never used, and the rest of the
/// ceiling is skipped for it. This bounds the cost of the overwhelmingly
/// common "nothing orphaned" case to one probe per `download_id`. Narrow
/// limitation: a set that has lost chunk 0 specifically (e.g. to some other
/// partial cleanup) is not discovered by this pass.
///
/// Returns every discovered chunk path (across all `download_id`s), so
/// callers/tests can assert on discovery independently of the (absent)
/// deletion side effect.
pub async fn log_orphaned_resume_chunks(output_path: &Path) -> Vec<PathBuf> {
    let parent_dir = output_path.parent().unwrap_or_else(|| Path::new("."));
    let mut discovered = Vec::new();

    for download_id in 0..MAX_DOWNLOAD_ID_SCAN {
        let Ok(set) = ChunkSet::for_attempt(output_path, download_id, ChunkKind::Resume) else {
            break;
        };

        let sentinel = set.path_in(parent_dir, 0);
        if tokio::fs::metadata(&sentinel).await.is_err() {
            continue;
        }

        let mut found_for_id = vec![sentinel];
        for chunk_id in 1..CHUNK_SCAN_CEILING {
            let chunk_path = set.path_in(parent_dir, chunk_id);
            if tokio::fs::metadata(&chunk_path).await.is_ok() {
                found_for_id.push(chunk_path);
            }
        }

        warn!(
            download_id,
            chunk_count = found_for_id.len();
            "Found an orphaned resume chunk set (download_id {download_id}) next to {}: \
             not deleting automatically — a concurrent rdlp process may still own this \
             download_id, and safely merging would require the original byte offset, \
             which chunk file names don't encode. Remove \
             '{}.{download_id}.resume*' manually once you've confirmed no other rdlp \
             process is using this output path.",
            output_path.display(),
            output_path.display(),
        );
        discovered.extend(found_for_id);
    }

    discovered
}

impl Orchestrator {
    /// Detect the resume point for a download
    ///
    /// Checks for:
    /// 1. Existing complete or partial download file
    /// 2. Interrupted parallel download chunks:
    ///    - New-style (Phase 2.5+): `{filename}.{downloadid}.part{i}`
    ///    - Old-style (Phase 2): `{filename}.part{i}`
    ///
    /// Prioritizes new-style chunks (highest download ID) over old-style.
    /// Automatically merges and cleans up chunk files.
    ///
    /// Returns the byte offset to resume from (0 for fresh download)
    #[instrument(skip(self), fields(path = %output_path.display()))]
    pub(super) async fn detect_resume_point(
        &self,
        output_path: &Path,
        expected_size: Option<u64>,
    ) -> Result<u64> {
        // 0. Discover (and log-only) any orphaned resume-kind chunk sets.
        // Unconditional and first, so every branch below sees it run — not
        // just the branches that also happen to run legacy cleanup (#568 C3:
        // the old call was nested inside `cleanup_old_chunks`/one `else`
        // arm, so the legacy-chunk-present branch never reached it).
        log_orphaned_resume_chunks(output_path).await;

        // 1. Check for existing complete or partial download file
        if output_path.exists()
            && let Ok(metadata) = tokio::fs::metadata(output_path).await
        {
            let size = metadata.len();
            if size > 0 {
                // Check if file is already complete
                if let Some(expected) = expected_size {
                    if size == expected {
                        #[allow(clippy::cast_precision_loss)] // display-only MB value
                        let mb = size as f64 / (1024.0 * 1024.0);
                        debug!("File already downloaded ({mb:.1} MB), skipping...");
                        // Clean up any orphaned chunks
                        cleanup_old_chunks(output_path).await;
                        return Ok(size);
                    } else if size > expected {
                        #[allow(clippy::cast_precision_loss)] // display-only MB values
                        let (size_mb, exp_mb) = (
                            size as f64 / (1024.0 * 1024.0),
                            expected as f64 / (1024.0 * 1024.0),
                        );
                        warn!(
                            "Partial file is larger than expected ({size_mb:.1} MB > {exp_mb:.1} MB), starting fresh..."
                        );
                        tokio::fs::remove_file(output_path).await.ok();
                        cleanup_old_chunks(output_path).await;
                        return Ok(0);
                    }
                }
                #[allow(clippy::cast_precision_loss)] // display-only MB value
                let size_mb = size as f64 / (1024.0 * 1024.0);
                debug!("Found partial download ({size_mb:.1} MB), resuming...");
                // Clean up any orphaned chunks from failed parallel attempts
                cleanup_old_chunks(output_path).await;
                return Ok(size);
            }
        }

        // 2. Check for interrupted parallel download chunks
        if let Some(chunk_info) = detect_chunk_files(output_path).await {
            let chunk_type = chunk_info.download_id.map_or_else(
                || "old-style".to_owned(),
                |id| format!("new-style (download ID: {id})"),
            );

            #[allow(clippy::cast_precision_loss)] // display-only MB value
            let total_mb = chunk_info.total_size as f64 / (1024.0 * 1024.0);
            debug!(
                "Found {} interrupted {} chunk files ({total_mb:.1} MB, break at {:?}, {} \
                 stranded beyond it and left in place), merging and resuming...",
                chunk_info.chunk_paths.len(),
                chunk_type,
                chunk_info.first_bad,
                chunk_info.stranded_beyond_gap,
            );

            // If using new-style chunks, clean up any old-style chunks first
            if chunk_info.download_id.is_some() {
                cleanup_old_chunks(output_path).await;
            }

            // Merge chunks into the main file. `merge_chunk_files` deletes
            // this attempt's manifest itself once its chunks are consumed
            // by a successful merge (#675) — nothing to do here on that
            // front for either outcome.
            match merge_chunk_files(output_path, &chunk_info).await {
                Ok(size) => {
                    #[allow(clippy::cast_precision_loss)] // display-only MB value
                    let mb = size as f64 / (1024.0 * 1024.0);
                    debug!(
                        chunks = chunk_info.chunk_paths.len(),
                        mb:?;
                        "Merged chunks into main file"
                    );
                    Ok(size)
                }
                Err(e) => {
                    warn!("Failed to merge chunks: {e}. Starting fresh.");
                    // Clean up partial chunks
                    for chunk_path in &chunk_info.chunk_paths {
                        let _ = tokio::fs::remove_file(chunk_path).await;
                    }
                    Ok(0)
                }
            }
        } else {
            // No main file, and no legacy/new-style Fresh chunk set found
            // either. Orphaned resume chunks (if any) were already
            // discovered and logged by step 0 above.
            Ok(0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claim() -> UnverifiableClaim {
        UnverifiableClaim {
            sentinel: PathBuf::from("video.mp4.0.part0"),
            reason: "no usable chunk manifest",
        }
    }

    /// Spec review finding 1: when nothing else from the scan merges, the
    /// message must say so plainly — this really is a fresh start and the
    /// files really are left untouched.
    #[test]
    fn message_claims_fresh_start_when_nothing_merged() {
        let message = unverifiable_claim_message(&claim(), false);
        assert!(message.contains("this download will start fresh"));
        assert!(message.contains("left in place"));
    }

    /// The other branch: a sibling verified set IS merging, so neither
    /// claim was true before this fix — assert the false claims are gone,
    /// not merely that some other text is present.
    #[test]
    fn message_does_not_claim_fresh_start_when_something_merged() {
        let message = unverifiable_claim_message(&claim(), true);
        assert!(!message.contains("start fresh"));
        assert!(!message.contains("left in place"));
        assert!(message.contains("another verified chunk set"));
    }
}
