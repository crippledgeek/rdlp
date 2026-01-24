//! FFmpeg and FFprobe integration.
//!
//! This module provides utilities for:
//! - Detecting FFmpeg/FFprobe executables
//! - Running FFmpeg commands with proper argument handling
//! - Probing media files for codec and format information
//!
//! # Example
//!
//! ```no_run
//! use rdlp_postprocess::ffmpeg::FFmpegRunner;
//!
//! # async fn example() -> rdlp_postprocess::error::Result<()> {
//! let ffmpeg = FFmpegRunner::new()?;
//!
//! // Probe a media file
//! let info = ffmpeg.probe("video.mp4").await?;
//! println!("Duration: {:?}", info.duration);
//! println!("Video codec: {:?}", info.video_codec);
//!
//! // Run FFmpeg command
//! ffmpeg.run(&["-i", "input.mp4", "-c:v", "copy", "output.mkv"]).await?;
//! # Ok(())
//! # }
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Output;

use regex::Regex;
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use log::{debug, trace};

use crate::error::{PostProcessError, Result};

/// Audio codec configuration for extraction/conversion.
#[derive(Debug, Clone)]
pub struct AudioCodecConfig {
    /// FFmpeg encoder name (e.g., "libmp3lame", "aac")
    pub encoder: Option<&'static str>,
    /// Output file extension
    pub extension: &'static str,
    /// Quality scale range (worst, best) for -q:a
    pub quality_scale: Option<(u8, u8)>,
    /// Bitrate range in kbps (min, max) for -b:a
    pub bitrate_range: Option<(u32, u32)>,
}

/// Supported audio codecs and their configurations.
pub static AUDIO_CODECS: &[(&str, AudioCodecConfig)] = &[
    (
        "mp3",
        AudioCodecConfig {
            encoder: Some("libmp3lame"),
            extension: "mp3",
            quality_scale: Some((9, 0)), // VBR quality (0=best, 9=worst)
            bitrate_range: Some((32, 320)),
        },
    ),
    (
        "aac",
        AudioCodecConfig {
            encoder: Some("aac"),
            extension: "m4a",
            quality_scale: None,
            bitrate_range: Some((32, 512)),
        },
    ),
    (
        "m4a",
        AudioCodecConfig {
            encoder: Some("aac"),
            extension: "m4a",
            quality_scale: None,
            bitrate_range: Some((32, 512)),
        },
    ),
    (
        "opus",
        AudioCodecConfig {
            encoder: Some("libopus"),
            extension: "opus",
            quality_scale: None,
            bitrate_range: Some((6, 510)),
        },
    ),
    (
        "vorbis",
        AudioCodecConfig {
            encoder: Some("libvorbis"),
            extension: "ogg",
            quality_scale: Some((0, 10)), // Quality (0=worst, 10=best)
            bitrate_range: Some((32, 500)),
        },
    ),
    (
        "flac",
        AudioCodecConfig {
            encoder: None, // Native FLAC encoder
            extension: "flac",
            quality_scale: None,
            bitrate_range: None, // Lossless
        },
    ),
    (
        "alac",
        AudioCodecConfig {
            encoder: Some("alac"),
            extension: "m4a",
            quality_scale: None,
            bitrate_range: None, // Lossless
        },
    ),
    (
        "wav",
        AudioCodecConfig {
            encoder: None, // PCM
            extension: "wav",
            quality_scale: None,
            bitrate_range: None,
        },
    ),
];

/// Get audio codec configuration by name.
pub fn get_audio_codec(name: &str) -> Option<&'static AudioCodecConfig> {
    AUDIO_CODECS
        .iter()
        .find(|(n, _)| *n == name.to_lowercase())
        .map(|(_, config)| config)
}

/// Media file information from FFprobe.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MediaInfo {
    /// File path
    pub path: PathBuf,
    /// Duration in seconds
    pub duration: Option<f64>,
    /// Container format (e.g., "mp4", "mkv")
    pub format: Option<String>,
    /// Video codec (e.g., "h264", "vp9")
    pub video_codec: Option<String>,
    /// Audio codec (e.g., "aac", "mp3")
    pub audio_codec: Option<String>,
    /// Video width in pixels
    pub width: Option<u32>,
    /// Video height in pixels
    pub height: Option<u32>,
    /// Video frame rate
    pub fps: Option<f64>,
    /// Video bitrate in kbps
    pub video_bitrate: Option<u32>,
    /// Audio bitrate in kbps
    pub audio_bitrate: Option<u32>,
    /// Audio sample rate in Hz
    pub sample_rate: Option<u32>,
    /// Number of audio channels
    pub channels: Option<u8>,
    /// Total file bitrate in kbps
    pub bitrate: Option<u32>,
    /// File size in bytes
    pub filesize: Option<u64>,
    /// Number of streams
    pub stream_count: usize,
    /// Whether file has video stream
    pub has_video: bool,
    /// Whether file has audio stream
    pub has_audio: bool,
    /// Raw stream information
    pub streams: Vec<StreamInfo>,
    /// Metadata tags
    pub metadata: HashMap<String, String>,
}

/// Information about a single stream.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StreamInfo {
    /// Stream index
    pub index: usize,
    /// Codec type ("video", "audio", "subtitle", "data")
    pub codec_type: String,
    /// Codec name
    pub codec_name: Option<String>,
    /// Stream-specific metadata
    pub metadata: HashMap<String, String>,
}

/// FFmpeg/FFprobe runner.
#[derive(Debug, Clone)]
pub struct FFmpegRunner {
    /// Path to FFmpeg executable
    ffmpeg_path: PathBuf,
    /// Path to FFprobe executable
    ffprobe_path: PathBuf,
    /// FFmpeg version string
    version: Option<String>,
}

impl FFmpegRunner {
    /// Create a new FFmpeg runner, auto-detecting executables from PATH.
    pub fn new() -> Result<Self> {
        Self::with_location(None)
    }

    /// Create a new FFmpeg runner with a custom location.
    ///
    /// If `location` is `Some`, it should be either:
    /// - A path to a directory containing ffmpeg and ffprobe
    /// - A path to the ffmpeg executable (ffprobe will be in the same directory)
    pub fn with_location(location: Option<&Path>) -> Result<Self> {
        let (ffmpeg_path, ffprobe_path) = Self::find_executables(location)?;

        Ok(Self {
            ffmpeg_path,
            ffprobe_path,
            version: None,
        })
    }

    /// Find FFmpeg and FFprobe executables.
    fn find_executables(location: Option<&Path>) -> Result<(PathBuf, PathBuf)> {
        let ffmpeg_names = if cfg!(windows) {
            vec!["ffmpeg.exe", "ffmpeg"]
        } else {
            vec!["ffmpeg"]
        };

        let ffprobe_names = if cfg!(windows) {
            vec!["ffprobe.exe", "ffprobe"]
        } else {
            vec!["ffprobe"]
        };

        let ffmpeg_path = if let Some(loc) = location {
            Self::find_in_location(loc, &ffmpeg_names)?
        } else {
            Self::find_in_path(&ffmpeg_names).ok_or(PostProcessError::FFmpegNotFound)?
        };

        let ffprobe_path = if let Some(loc) = location {
            Self::find_in_location(loc, &ffprobe_names)?
        } else {
            // Try to find ffprobe in same directory as ffmpeg first
            let ffmpeg_dir = ffmpeg_path.parent();
            let in_ffmpeg_dir = ffmpeg_dir.and_then(|dir| {
                ffprobe_names
                    .iter()
                    .map(|name| dir.join(name))
                    .find(|p| p.exists())
            });

            in_ffmpeg_dir
                .or_else(|| Self::find_in_path(&ffprobe_names))
                .ok_or(PostProcessError::FFprobeNotFound)?
        };

        debug!("Found FFmpeg at: {}", ffmpeg_path.display());
        debug!("Found FFprobe at: {}", ffprobe_path.display());

        Ok((ffmpeg_path, ffprobe_path))
    }

    /// Find executable in a specific location.
    fn find_in_location(location: &Path, names: &[&str]) -> Result<PathBuf> {
        // If location is a file, check if it's one of the executables
        if location.is_file() {
            if let Some(name) = location.file_name().and_then(|n| n.to_str()) {
                if names.iter().any(|n| name.contains(n.trim_end_matches(".exe"))) {
                    return Ok(location.to_path_buf());
                }
            }
            // If it's a file but not the right one, check its directory
            if let Some(dir) = location.parent() {
                for name in names {
                    let path = dir.join(name);
                    if path.exists() {
                        return Ok(path);
                    }
                }
            }
        }

        // If location is a directory, search in it
        if location.is_dir() {
            for name in names {
                let path = location.join(name);
                if path.exists() {
                    return Ok(path);
                }
            }
        }

        Err(PostProcessError::FFmpegNotFound)
    }

    /// Find executable in PATH.
    fn find_in_path(names: &[&str]) -> Option<PathBuf> {
        for name in names {
            if let Ok(path) = which::which(name) {
                return Some(path);
            }
        }
        None
    }

    /// Check if FFmpeg is available.
    pub fn available(&self) -> bool {
        self.ffmpeg_path.exists() && self.ffprobe_path.exists()
    }

    /// Get the FFmpeg version.
    pub async fn version(&mut self) -> Result<&str> {
        if self.version.is_none() {
            let output = Command::new(&self.ffmpeg_path)
                .arg("-version")
                .output()
                .await
                .map_err(|e| PostProcessError::ffmpeg_failed_with_source("Failed to get version", e))?;

            let stdout = String::from_utf8_lossy(&output.stdout);
            let version = stdout
                .lines()
                .next()
                .and_then(|line| {
                    // Parse "ffmpeg version N.N.N ..." or "ffmpeg version N.N ..."
                    let re = Regex::new(r"ffmpeg version (\S+)").ok()?;
                    re.captures(line).map(|c| c[1].to_string())
                })
                .unwrap_or_else(|| "unknown".to_string());

            self.version = Some(version);
        }

        Ok(self.version.as_ref().unwrap())
    }

    /// Get the path to the FFmpeg executable.
    pub fn ffmpeg_path(&self) -> &Path {
        &self.ffmpeg_path
    }

    /// Get the path to the FFprobe executable.
    pub fn ffprobe_path(&self) -> &Path {
        &self.ffprobe_path
    }

    /// Probe a media file and return its information.
    pub async fn probe(&self, path: impl AsRef<Path>) -> Result<MediaInfo> {
        let path = path.as_ref();

        if !path.exists() {
            return Err(PostProcessError::InputNotFound {
                path: path.to_path_buf(),
            });
        }

        let output = Command::new(&self.ffprobe_path)
            .args([
                "-v",
                "quiet",
                "-print_format",
                "json",
                "-show_format",
                "-show_streams",
            ])
            .arg(path)
            .output()
            .await
            .map_err(|e| PostProcessError::ffmpeg_failed_with_source("Failed to run FFprobe", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(PostProcessError::FFmpegExitCode {
                code: output.status.code().unwrap_or(-1),
                stderr: stderr.to_string(),
            });
        }

        let json_str = String::from_utf8_lossy(&output.stdout);
        self.parse_probe_output(path, &json_str)
    }

    /// Parse FFprobe JSON output into MediaInfo.
    fn parse_probe_output(&self, path: &Path, json: &str) -> Result<MediaInfo> {
        let data: serde_json::Value = serde_json::from_str(json).map_err(|e| {
            PostProcessError::ParseError {
                message: format!("Invalid JSON from FFprobe: {e}"),
            }
        })?;

        let mut info = MediaInfo {
            path: path.to_path_buf(),
            ..Default::default()
        };

        // Parse format information
        if let Some(format) = data.get("format") {
            info.format = format
                .get("format_name")
                .and_then(|v| v.as_str())
                .map(|s| s.split(',').next().unwrap_or(s).to_string());

            info.duration = format
                .get("duration")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse().ok());

            info.bitrate = format
                .get("bit_rate")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<u64>().ok())
                .map(|b| (b / 1000) as u32);

            info.filesize = format
                .get("size")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse().ok());

            // Parse format metadata
            if let Some(tags) = format.get("tags").and_then(|t| t.as_object()) {
                for (key, value) in tags {
                    if let Some(v) = value.as_str() {
                        info.metadata.insert(key.to_lowercase(), v.to_string());
                    }
                }
            }
        }

        // Parse streams
        if let Some(streams) = data.get("streams").and_then(|s| s.as_array()) {
            info.stream_count = streams.len();

            for (i, stream) in streams.iter().enumerate() {
                let codec_type = stream
                    .get("codec_type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");

                let codec_name = stream
                    .get("codec_name")
                    .and_then(|v| v.as_str())
                    .map(String::from);

                let mut stream_info = StreamInfo {
                    index: i,
                    codec_type: codec_type.to_string(),
                    codec_name: codec_name.clone(),
                    metadata: HashMap::new(),
                };

                // Parse stream metadata
                if let Some(tags) = stream.get("tags").and_then(|t| t.as_object()) {
                    for (key, value) in tags {
                        if let Some(v) = value.as_str() {
                            stream_info.metadata.insert(key.to_lowercase(), v.to_string());
                        }
                    }
                }

                match codec_type {
                    "video" => {
                        info.has_video = true;
                        info.video_codec = codec_name;
                        info.width = stream.get("width").and_then(|v| v.as_u64()).map(|v| v as u32);
                        info.height = stream.get("height").and_then(|v| v.as_u64()).map(|v| v as u32);

                        // Parse frame rate (could be "30/1" or "29.97")
                        if let Some(fps_str) = stream.get("r_frame_rate").and_then(|v| v.as_str()) {
                            info.fps = Self::parse_frame_rate(fps_str);
                        }

                        info.video_bitrate = stream
                            .get("bit_rate")
                            .and_then(|v| v.as_str())
                            .and_then(|s| s.parse::<u64>().ok())
                            .map(|b| (b / 1000) as u32);
                    }
                    "audio" => {
                        info.has_audio = true;
                        if info.audio_codec.is_none() {
                            info.audio_codec = codec_name;
                        }

                        info.sample_rate = stream
                            .get("sample_rate")
                            .and_then(|v| v.as_str())
                            .and_then(|s| s.parse().ok());

                        info.channels = stream
                            .get("channels")
                            .and_then(|v| v.as_u64())
                            .map(|c| c as u8);

                        if info.audio_bitrate.is_none() {
                            info.audio_bitrate = stream
                                .get("bit_rate")
                                .and_then(|v| v.as_str())
                                .and_then(|s| s.parse::<u64>().ok())
                                .map(|b| (b / 1000) as u32);
                        }
                    }
                    _ => {}
                }

                info.streams.push(stream_info);
            }
        }

        Ok(info)
    }

    /// Parse frame rate string like "30/1" or "29.97".
    fn parse_frame_rate(fps_str: &str) -> Option<f64> {
        if fps_str.contains('/') {
            let parts: Vec<&str> = fps_str.split('/').collect();
            if parts.len() == 2 {
                let num: f64 = parts[0].parse().ok()?;
                let den: f64 = parts[1].parse().ok()?;
                if den > 0.0 {
                    return Some(num / den);
                }
            }
        }
        fps_str.parse().ok()
    }

    /// Run FFmpeg with the given arguments.
    ///
    /// This method automatically adds `-y` (overwrite) and sets an appropriate loglevel.
    pub async fn run(&self, args: &[&str]) -> Result<Output> {
        self.run_with_options(args, true, "error").await
    }

    /// Run FFmpeg with custom options.
    pub async fn run_with_options(
        &self,
        args: &[&str],
        overwrite: bool,
        loglevel: &str,
    ) -> Result<Output> {
        let mut cmd = Command::new(&self.ffmpeg_path);

        if overwrite {
            cmd.arg("-y");
        }

        cmd.args(["-loglevel", loglevel]);
        cmd.args(args);

        trace!(
            "Running FFmpeg: {} {}",
            self.ffmpeg_path.display(),
            args.join(" ")
        );

        let output = cmd
            .output()
            .await
            .map_err(|e| PostProcessError::ffmpeg_failed_with_source("Failed to run FFmpeg", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(PostProcessError::FFmpegExitCode {
                code: output.status.code().unwrap_or(-1),
                stderr: stderr.to_string(),
            });
        }

        Ok(output)
    }

    /// Run FFmpeg with input and output file paths.
    ///
    /// Handles special characters in filenames by using the `file:` protocol.
    pub async fn run_with_files(
        &self,
        inputs: &[&Path],
        output: &Path,
        opts: &[&str],
    ) -> Result<Output> {
        let mut args = Vec::new();

        // Add input files
        for input in inputs {
            args.push("-i".to_string());
            args.push(Self::filename_arg(input));
        }

        // Add options
        args.extend(opts.iter().map(|s| s.to_string()));

        // Add output
        args.push(Self::filename_arg(output));

        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        self.run(&args_refs).await
    }

    /// Create a filename argument with proper escaping.
    fn filename_arg(path: &Path) -> String {
        let path_str = path.to_string_lossy();

        // Use file: protocol for paths that might contain special characters
        if path_str.starts_with('-') || path_str.contains(':') && !path_str.starts_with("file:") {
            format!("file:{path_str}")
        } else {
            path_str.to_string()
        }
    }

    /// Get the audio codec from a file.
    pub async fn get_audio_codec(&self, path: impl AsRef<Path>) -> Result<Option<String>> {
        let info = self.probe(path).await?;
        Ok(info.audio_codec)
    }

    /// Check if a file has an audio stream.
    pub async fn has_audio(&self, path: impl AsRef<Path>) -> Result<bool> {
        let info = self.probe(path).await?;
        Ok(info.has_audio)
    }

    /// Check if a file has a video stream.
    pub async fn has_video(&self, path: impl AsRef<Path>) -> Result<bool> {
        let info = self.probe(path).await?;
        Ok(info.has_video)
    }
}

impl MediaInfo {
    /// Get a resolution string (e.g., "1920x1080").
    pub fn resolution_string(&self) -> Option<String> {
        match (self.width, self.height) {
            (Some(w), Some(h)) => Some(format!("{w}x{h}")),
            _ => None,
        }
    }

    /// Get the video stream info.
    pub fn video_stream(&self) -> Option<&StreamInfo> {
        self.streams.iter().find(|s| s.codec_type == "video")
    }

    /// Get the audio stream info.
    pub fn audio_stream(&self) -> Option<&StreamInfo> {
        self.streams.iter().find(|s| s.codec_type == "audio")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_audio_codec() {
        assert!(get_audio_codec("mp3").is_some());
        assert!(get_audio_codec("MP3").is_some()); // Case insensitive
        assert!(get_audio_codec("aac").is_some());
        assert!(get_audio_codec("unknown_codec").is_none());
    }

    #[test]
    fn test_audio_codec_config() {
        let mp3 = get_audio_codec("mp3").unwrap();
        assert_eq!(mp3.encoder, Some("libmp3lame"));
        assert_eq!(mp3.extension, "mp3");
        assert_eq!(mp3.quality_scale, Some((9, 0)));

        let flac = get_audio_codec("flac").unwrap();
        assert!(flac.encoder.is_none()); // Native codec
        assert!(flac.bitrate_range.is_none()); // Lossless
    }

    #[test]
    fn test_parse_frame_rate() {
        assert_eq!(FFmpegRunner::parse_frame_rate("30/1"), Some(30.0));
        assert_eq!(FFmpegRunner::parse_frame_rate("30000/1001"), Some(29.97002997002997));
        assert_eq!(FFmpegRunner::parse_frame_rate("24"), Some(24.0));
        assert!(FFmpegRunner::parse_frame_rate("invalid").is_none());
    }

    #[test]
    fn test_filename_arg() {
        // Normal path
        assert_eq!(FFmpegRunner::filename_arg(Path::new("video.mp4")), "video.mp4");

        // Path starting with dash
        assert_eq!(FFmpegRunner::filename_arg(Path::new("-output.mp4")), "file:-output.mp4");
    }

    #[test]
    fn test_media_info_resolution() {
        let mut info = MediaInfo::default();
        assert!(info.resolution_string().is_none());

        info.width = Some(1920);
        info.height = Some(1080);
        assert_eq!(info.resolution_string(), Some("1920x1080".to_string()));
    }
}
