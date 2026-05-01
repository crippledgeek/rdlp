use rdlp_types::Progress;
use std::sync::Arc;

/// Callback for reporting post-processing progress.
///
/// Implementations receive [`Progress`] updates during `FFmpeg` post-processing
/// stages. The fraction is always in `[0.0, 1.0]`; call [`Progress::percent`]
/// at display sites to scale to `0.0..=100.0`.
///
/// # Example
///
/// ```rust
/// use rdlp_core::PostProcessCallback;
/// use rdlp_types::Progress;
///
/// struct PrintProgress;
/// impl PostProcessCallback for PrintProgress {
///     fn on_progress(&self, progress: Progress) {
///         println!("Progress: {:.0}%", progress.percent());
///     }
/// }
/// ```
pub trait PostProcessCallback: Send + Sync {
    /// Report progress as a clamped fraction.
    fn on_progress(&self, progress: Progress);

    /// Forward a log message from the post-processing stage.
    fn on_log(&self, _message: &str) {}
}

/// Factory for creating per-stage post-processing callbacks.
pub type PostProcessCallbackFactory =
    Arc<dyn Fn(&str) -> Arc<dyn PostProcessCallback> + Send + Sync>;
