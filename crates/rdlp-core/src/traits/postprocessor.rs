use std::sync::Arc;

/// Callback for reporting post-processing progress.
///
/// Implementations receive progress updates (0.0–1.0) during FFmpeg
/// post-processing stages.
///
/// # Example
///
/// ```rust
/// use rdlp_core::PostProcessCallback;
/// use std::sync::Arc;
///
/// struct PrintProgress;
/// impl PostProcessCallback for PrintProgress {
///     fn on_progress(&self, progress: f64) {
///         println!("Progress: {:.0}%", progress * 100.0);
///     }
/// }
/// ```
pub trait PostProcessCallback: Send + Sync {
    /// Report progress as a fraction in \[0.0, 1.0\].
    fn on_progress(&self, progress: f64);

    /// Forward a log message from the post-processing stage.
    fn on_log(&self, _message: &str) {}
}

/// Factory for creating per-stage post-processing callbacks.
pub type PostProcessCallbackFactory =
    Arc<dyn Fn(&str) -> Arc<dyn PostProcessCallback> + Send + Sync>;
