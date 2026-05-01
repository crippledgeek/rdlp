//! Playlist execution: concurrent download, retry waves, and summary reporting
//!
//! These methods implement the execution phase of playlist downloads after
//! format resolution and selection are complete.

use super::{
    Duration, HashMap, HashSet, Instant, Orchestrator, PathBuf, StreamExt, archive, debug, error,
    info, warn,
};

impl Orchestrator {
    /// Download episodes concurrently with cancellation support.
    ///
    /// Returns (downloaded paths, failed episodes, whether interrupted).
    #[allow(clippy::too_many_arguments, clippy::ref_option, clippy::too_many_lines)]
    pub(super) async fn download_episodes(
        &self,
        infos: &[rdlp_types::InfoDict],
        existing_files: &HashMap<String, PathBuf>,
        archive: &Option<HashSet<String>>,
        playlist_dir: &std::path::Path,
        selected_sub_langs: &[String],
        selected_audio: &Option<String>,
        batch_resolved_at: Instant,
        total: usize,
    ) -> (Vec<PathBuf>, Vec<(usize, String, String)>, bool) {
        const DOWNLOAD_CONCURRENCY: usize = 2;
        let mut downloaded: Vec<PathBuf> = existing_files.values().cloned().collect();
        let mut failed = Vec::new();
        let mut interrupted = false;

        // Collect episodes that still need downloading
        let to_download: Vec<(usize, rdlp_types::InfoDict)> = infos
            .iter()
            .enumerate()
            .filter_map(|(i, ep)| {
                let sanitized = Self::sanitize_filename(&ep.title);
                let exists = downloaded.iter().any(|p| {
                    p.file_stem()
                        .and_then(|s| s.to_str())
                        .is_some_and(|name| name == sanitized)
                });
                if exists {
                    return None;
                }
                if let Some(archive_set) = archive
                    && archive::is_in_archive(archive_set, &ep.extractor, &ep.id)
                {
                    return None;
                }
                // Match filter check
                if !self.config.match_filters.is_empty()
                    && !super::super::check_match_filters(&self.config.match_filters, ep)
                {
                    log::info!(title = ep.title.as_str(); "Does not pass match filter, skipping");
                    return None;
                }
                Some((i, ep.clone()))
            })
            .collect();

        let download_count = to_download.len();
        if download_count > 0 {
            info!(
                count = download_count,
                concurrency = DOWNLOAD_CONCURRENCY;
                "Downloading episodes"
            );
        }

        let batch_ts = Some(batch_resolved_at);
        let futs = to_download.into_iter().map(|(index, info_owned)| {
            let position = index + 1;
            let dir = playlist_dir.to_path_buf();
            let sub_langs = selected_sub_langs.to_vec();
            let audio = selected_audio.clone();
            async move {
                debug!("{}", "\u{2500}".repeat(60));
                info!(position, total, title:? = info_owned.title; "Downloading");
                let _ = self
                    .event_tx
                    .try_send(crate::events::Event::PlaylistItemStarted {
                        id: self.download_id,
                        index: position,
                        total,
                        url: info_owned.webpage_url.clone(),
                        title: info_owned.title.clone(),
                    });
                let result = self
                    .download_from_info_to_dir(
                        &info_owned,
                        false,
                        &dir,
                        &sub_langs,
                        audio.as_deref(),
                        batch_ts,
                    )
                    .await;
                (position, info_owned, result)
            }
        });

        let mut stream = futures_util::stream::iter(futs).buffer_unordered(DOWNLOAD_CONCURRENCY);

        // Consume download stream with cancellation token support
        loop {
            tokio::select! {
                next = stream.next() => {
                    match next {
                        Some((position, info_owned, result)) => {
                            match result {
                                Ok(Some(path)) => {
                                    debug!(position, total, path:? = path.display(); "Saved");
                                    downloaded.push(path);
                                    let _ = self.event_tx.try_send(crate::events::Event::UnitCompleted {
                                        id: self.download_id,
                                        index: position,
                                    });
                                    self.record_in_archive(
                                        &info_owned.extractor,
                                        &info_owned.id,
                                    )
                                    .await;
                                }
                                Ok(None) => {
                                    debug!(position, total; "Skipped by user");
                                }
                                Err(e) => {
                                    error!(position, total; "Failed: {e}");
                                    failed.push((
                                        position,
                                        info_owned.title,
                                        e.to_string(),
                                    ));
                                }
                            }
                        }
                        None => break, // All downloads complete
                    }
                }
                () = self.cancel_token.cancelled() => {
                    info!("Playlist download interrupted by user");
                    info!("Run the same command again to resume");
                    interrupted = true;
                    break;
                }
            }
        }

        (downloaded, failed, interrupted)
    }

    /// Run retry waves for failed episodes with increasing delays.
    ///
    /// After the initial pass, retry failed episodes up to 3 more
    /// times with increasing delays. Each wave re-extracts fresh CDN
    /// URLs since old tokens are likely expired.
    #[allow(clippy::too_many_arguments, clippy::too_many_lines, clippy::ref_option)]
    pub(super) async fn run_retry_waves(
        &self,
        failed: &mut Vec<(usize, String, String)>,
        downloaded: &mut Vec<PathBuf>,
        interrupted: &mut bool,
        infos: &[rdlp_types::InfoDict],
        playlist_dir: &std::path::Path,
        selected_sub_langs: &[String],
        selected_audio: &Option<String>,
        total: usize,
    ) -> usize {
        const MAX_RETRY_WAVES: usize = 3;
        const RETRY_WAVE_DELAYS: [u64; MAX_RETRY_WAVES] = [60, 120, 180];
        // Higher concurrency for retries: failures are often transient (CDN
        // token expiry, rate limits), so retrying with more parallelism across
        // different server paths recovers faster.
        const DOWNLOAD_CONCURRENCY: usize = 4;
        let mut retried_ok = 0usize;

        if *interrupted || failed.is_empty() {
            return retried_ok;
        }

        info!("");
        info!("{} episode(s) failed — starting retry waves", failed.len());

        for (wave, &delay_secs) in RETRY_WAVE_DELAYS.iter().enumerate() {
            if failed.is_empty() || *interrupted {
                break;
            }

            let fail_count = failed.len();
            info!(
                wave = wave + 1,
                max_waves = MAX_RETRY_WAVES,
                failed = fail_count,
                delay_secs;
                "Retry wave — waiting before retry"
            );

            // Sleep with cancellation race
            let sleep = tokio::time::sleep(Duration::from_secs(delay_secs));
            tokio::pin!(sleep);
            tokio::select! {
                () = &mut sleep => {}
                () = self.cancel_token.cancelled() => {
                    info!("Retry interrupted by user");
                    *interrupted = true;
                    break;
                }
            }

            // Take current failures; will re-collect any that fail again
            let retry_batch: Vec<(usize, String, String)> = std::mem::take(failed);

            let retry_futs = retry_batch.into_iter().map(|(position, title, _err)| {
                let dir = playlist_dir.to_path_buf();
                let sub_langs = selected_sub_langs.to_vec();
                let audio = selected_audio.clone();
                let index = position - 1;
                #[allow(clippy::indexing_slicing)]
                // index = position-1; position came from infos enumeration
                let mut info_stub = infos[index].clone();
                info_stub.formats.clear();
                async move {
                    info!(
                        wave = wave + 1,
                        position,
                        total,
                        title:? = title;
                        "Retrying episode"
                    );
                    let result = self
                        .download_from_info_to_dir(
                            &info_stub,
                            false,
                            &dir,
                            &sub_langs,
                            audio.as_deref(),
                            None,
                        )
                        .await;
                    (position, title, result)
                }
            });

            let mut retry_stream =
                futures_util::stream::iter(retry_futs).buffer_unordered(DOWNLOAD_CONCURRENCY);

            loop {
                tokio::select! {
                    next = retry_stream.next() => {
                        match next {
                            Some((pos, title, result)) => {
                                match result {
                                    Ok(Some(path)) => {
                                        debug!(
                                            wave = wave + 1,
                                            pos, total,
                                            path:? = path.display();
                                            "Retry succeeded"
                                        );
                                        downloaded.push(path);
                                        let idx = pos - 1;
                                        #[allow(clippy::indexing_slicing)] // idx = pos-1; pos from infos enumeration
                                        self.record_in_archive(
                                            &infos[idx].extractor,
                                            &infos[idx].id,
                                        )
                                        .await;
                                        retried_ok += 1;
                                    }
                                    Ok(None) => {
                                        debug!(
                                            pos, total;
                                            "Skipped on retry"
                                        );
                                    }
                                    Err(e) => {
                                        error!(
                                            wave = wave + 1,
                                            pos, total;
                                            "Retry failed: {e}"
                                        );
                                        failed.push((
                                            pos,
                                            title,
                                            e.to_string(),
                                        ));
                                    }
                                }
                            }
                            None => break,
                        }
                    }
                    () = self.cancel_token.cancelled() => {
                        info!("Retry wave interrupted by user");
                        *interrupted = true;
                        break;
                    }
                }
            }

            if failed.is_empty() {
                info!("All previously-failed episodes recovered!");
                break;
            }
        }

        retried_ok
    }

    /// Print the playlist download summary report.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn print_playlist_summary(
        &self,
        playlist_dir: &std::path::Path,
        downloaded: &[PathBuf],
        failed: &[(usize, String, String)],
        missing_subs: &HashMap<String, PathBuf>,
        selected_sub_langs: &[String],
        total: usize,
        already_downloaded: usize,
        retried_ok: usize,
        sub_retried_ok: usize,
        interrupted: bool,
    ) {
        let newly_downloaded = downloaded.len() - already_downloaded;
        let dl_count = downloaded.len();

        info!("");
        info!("{}", "=".repeat(60));
        info!("Playlist Download Summary");
        info!("{}", "=".repeat(60));
        info!("Folder: {}", playlist_dir.display());
        info!("Downloaded: {dl_count}/{total}");

        if already_downloaded > 0 {
            info!("  (previously: {already_downloaded}, this session: {newly_downloaded})");
        }

        if retried_ok > 0 {
            info!("  (recovered {retried_ok} via retry waves)");
        }

        if !selected_sub_langs.is_empty() {
            info!("Subtitles: {}", selected_sub_langs.join(", "));
        }

        if !failed.is_empty() {
            error!("Failed: {}", failed.len());
            for (pos, title, err) in failed {
                error!("  [{pos}/{total}] {title}: {err}");
            }
        }

        // Report missing subtitles
        if !missing_subs.is_empty() && !selected_sub_langs.is_empty() {
            let still_missing = missing_subs.len() - sub_retried_ok;
            if sub_retried_ok > 0 {
                info!(
                    "Subtitles recovered: {sub_retried_ok}/{}",
                    missing_subs.len()
                );
            }
            if still_missing > 0 {
                warn!("Missing subtitles: {still_missing}");
                Self::log_missing_subs(missing_subs, &[], total);
                if !self.config.retry_subs {
                    info!("Hint: re-run with --retry-subs to attempt subtitle download");
                }
            }
        }

        if interrupted {
            let remaining_after = total - dl_count;
            info!("Interrupted — {remaining_after} videos remaining");
            info!("Run the same command again to resume");
        }

        info!("{}", "=".repeat(60));
    }
}
