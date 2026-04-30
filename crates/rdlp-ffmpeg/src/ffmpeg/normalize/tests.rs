//! Tests for audio normalization.

use super::super::{AudioNormMode, LoudnormMeasurements, NormalizeOptions};
use super::helpers::*;

#[test]
fn test_select_audio_encoder_for_container() {
    // AAC-compatible containers return the preferred AAC encoder
    // (libfdk_aac if available, built-in aac otherwise)
    let aac = crate::ffmpeg::audio_codecs::preferred_aac_encoder();
    assert_eq!(select_audio_encoder_for_container("mp4"), aac);
    assert_eq!(select_audio_encoder_for_container("m4a"), aac);
    assert_eq!(select_audio_encoder_for_container("mov"), aac);
    assert_eq!(select_audio_encoder_for_container("webm"), "libopus");
    // MKV supports everything; registry prefers Opus for quality/size efficiency
    assert_eq!(select_audio_encoder_for_container("mkv"), "libopus");
    assert_eq!(select_audio_encoder_for_container("avi"), "libmp3lame");
    assert_eq!(select_audio_encoder_for_container("mp3"), "libmp3lame");
    assert_eq!(select_audio_encoder_for_container("flac"), "flac");
    assert_eq!(select_audio_encoder_for_container("wav"), "pcm_s16le");
    assert_eq!(select_audio_encoder_for_container("ts"), aac);
    assert_eq!(select_audio_encoder_for_container("ogg"), "libopus");
    assert_eq!(select_audio_encoder_for_container("flv"), aac);
    assert_eq!(select_audio_encoder_for_container("xyz"), aac);
}

#[test]
fn test_default_bitrate_for_encoder() {
    assert_eq!(default_bitrate_for_encoder("aac"), 128_000);
    assert_eq!(default_bitrate_for_encoder("libfdk_aac"), 128_000);
    assert_eq!(default_bitrate_for_encoder("libmp3lame"), 192_000);
    assert_eq!(default_bitrate_for_encoder("libopus"), 128_000);
    assert_eq!(default_bitrate_for_encoder("flac"), 0);
    assert_eq!(default_bitrate_for_encoder("pcm_s16le"), 0);
}

#[test]
fn test_parse_loudnorm_json() {
    let lines = vec![
        "[Parsed_loudnorm_0 @ 0x...] ".to_string(),
        "{\n".to_string(),
        "    \"input_i\" : \"-24.50\",\n".to_string(),
        "    \"input_tp\" : \"-3.20\",\n".to_string(),
        "    \"input_lra\" : \"8.30\",\n".to_string(),
        "    \"input_thresh\" : \"-35.10\",\n".to_string(),
        "    \"output_i\" : \"-16.00\",\n".to_string(),
        "    \"output_tp\" : \"-1.50\",\n".to_string(),
        "    \"output_lra\" : \"7.20\",\n".to_string(),
        "    \"output_thresh\" : \"-26.60\",\n".to_string(),
        "    \"normalization_type\" : \"dynamic\",\n".to_string(),
        "    \"target_offset\" : \"0.50\"\n".to_string(),
        "}\n".to_string(),
    ];

    let m = parse_loudnorm_json(&lines).unwrap();
    assert!((m.input_i - (-24.5)).abs() < 0.01);
    assert!((m.input_tp - (-3.2)).abs() < 0.01);
    assert!((m.input_lra - 8.3).abs() < 0.01);
    assert!((m.input_thresh - (-35.1)).abs() < 0.01);
    assert!((m.target_offset - 0.5).abs() < 0.01);
}

#[test]
fn test_parse_loudnorm_json_missing_field() {
    let lines = vec!["{ \"input_i\" : \"-24.50\", \"input_tp\" : \"-3.20\" }".to_string()];

    let result = parse_loudnorm_json(&lines);
    assert!(result.is_err());
}

#[test]
fn test_extract_json_value() {
    let text = r#""input_i" : "-24.50""#;
    assert!((extract_json_value(text, "input_i").unwrap() - (-24.5)).abs() < 0.01);

    let text = r#""target_offset" : "0.50""#;
    assert!((extract_json_value(text, "target_offset").unwrap() - 0.5).abs() < 0.01);

    assert!(extract_json_value(text, "nonexistent").is_none());
}

#[test]
fn test_build_alimiter_spec_headroom() {
    // target_tp=-1.0 → ceiling = 10^((-1.0 - 1.5) / 20) = 10^(-0.125)
    let spec = build_alimiter_spec(-1.0);
    assert!(spec.starts_with("alimiter=limit="));
    assert!(spec.contains("attack=5"));
    assert!(spec.contains("release=50"));

    // Verify headroom: ceiling should be lower than 10^(-1/20) ≈ 0.891
    // With 1.5 dB headroom: 10^(-2.5/20) ≈ 0.750
    let limit_str = spec
        .strip_prefix("alimiter=limit=")
        .unwrap()
        .split(':')
        .next()
        .unwrap();
    let limit: f64 = limit_str.parse().unwrap();
    let expected = 10f64.powf((-1.0 - ALIMITER_TP_HEADROOM_DB) / 20.0);
    assert!(
        (limit - expected).abs() < 0.001,
        "limit={limit}, expected={expected}"
    );
}

#[test]
fn test_build_loudnorm_pass2_filter_linear_no_shortfall() {
    let opts = NormalizeOptions {
        mode: AudioNormMode::Loudnorm,
        target_i: -14.0,
        target_tp: -1.0,
        target_lra: 11.0,
        ..Default::default()
    };
    let m = LoudnormMeasurements {
        input_i: -20.0,
        input_tp: -7.0,
        input_lra: 8.0,
        input_thresh: -30.0,
        target_offset: 0.0,
    };
    let filter = build_loudnorm_pass2_filter(&opts, &m);
    assert!(filter.contains("linear=true"));
    assert!(!filter.contains("alimiter="));
    assert!(!filter.contains("volume="));
    assert!(!filter.contains("acompressor="));
}

#[test]
fn test_build_loudnorm_pass2_filter_moderate_shortfall() {
    let opts = NormalizeOptions {
        mode: AudioNormMode::Loudnorm,
        target_i: -14.0,
        target_tp: -1.0,
        target_lra: 11.0,
        ..Default::default()
    };
    let m = LoudnormMeasurements {
        input_i: -17.0,
        input_tp: -1.5,
        input_lra: 8.0,
        input_thresh: -27.0,
        target_offset: 0.0,
    };
    let filter = build_loudnorm_pass2_filter(&opts, &m);
    assert!(filter.contains("linear=true"));
    assert!(!filter.contains("alimiter="));
    assert!(!filter.contains("volume="));
}

#[test]
fn test_build_loudnorm_pass2_filter_large_shortfall() {
    let opts = NormalizeOptions {
        mode: AudioNormMode::Loudnorm,
        target_i: -14.0,
        target_tp: -1.0,
        target_lra: 11.0,
        ..Default::default()
    };
    let m = LoudnormMeasurements {
        input_i: -30.0,
        input_tp: -1.0,
        input_lra: 12.0,
        input_thresh: -40.0,
        target_offset: 0.0,
    };
    let filter = build_loudnorm_pass2_filter(&opts, &m);
    assert!(filter.contains("linear=true"));
    assert!(!filter.contains("alimiter="));
    assert!(!filter.contains("linear=false"));
}

#[test]
fn test_build_loudnorm_pass2_filter_force_dynamic() {
    let opts = NormalizeOptions {
        mode: AudioNormMode::Loudnorm,
        target_i: -14.0,
        target_tp: -1.0,
        target_lra: 11.0,
        force_dynamic: true,
        ..Default::default()
    };
    let m = LoudnormMeasurements {
        input_i: -20.0,
        input_tp: -7.0,
        input_lra: 8.0,
        input_thresh: -30.0,
        target_offset: 0.0,
    };
    let filter = build_loudnorm_pass2_filter(&opts, &m);
    assert!(filter.contains("linear=false"));
    assert!(!filter.contains("alimiter="));
    assert!(!filter.contains("linear=true"));
}

#[test]
fn test_build_loudnorm_pass2_filter_precompress() {
    let opts = NormalizeOptions {
        mode: AudioNormMode::Loudnorm,
        target_i: -14.0,
        target_tp: -1.0,
        target_lra: 11.0,
        precompress: true,
        ..Default::default()
    };
    let m = LoudnormMeasurements {
        input_i: -20.0,
        input_tp: -7.0,
        input_lra: 8.0,
        input_thresh: -30.0,
        target_offset: 0.0,
    };
    let filter = build_loudnorm_pass2_filter(&opts, &m);
    assert!(filter.contains("acompressor="));
    assert!(filter.contains("linear=true"));
    assert!(!filter.contains("alimiter="));
    let comp_pos = filter.find("acompressor=").unwrap();
    let loud_pos = filter.find("loudnorm=").unwrap();
    assert!(comp_pos < loud_pos, "acompressor must precede loudnorm");
}

#[test]
fn test_audio_only_extension_for() {
    // MOV-based formats now use MKA to avoid ENOMEM
    assert_eq!(audio_only_extension_for("mp4"), "mka");
    assert_eq!(audio_only_extension_for("m4a"), "mka");
    assert_eq!(audio_only_extension_for("mov"), "mka");
    assert_eq!(audio_only_extension_for("f4v"), "mka");
    assert_eq!(audio_only_extension_for("3gp"), "mka");
    assert_eq!(audio_only_extension_for("ts"), "mka");
    assert_eq!(audio_only_extension_for("mpg"), "mka");
    assert_eq!(audio_only_extension_for("flv"), "mka");

    // Matroska-based formats
    assert_eq!(audio_only_extension_for("mkv"), "mka");
    assert_eq!(audio_only_extension_for("mka"), "mka");
    assert_eq!(audio_only_extension_for("webm"), "mka");

    // Other formats
    assert_eq!(audio_only_extension_for("avi"), "mp3");
    assert_eq!(audio_only_extension_for("mp3"), "mp3");
    assert_eq!(audio_only_extension_for("ogg"), "opus");
    assert_eq!(audio_only_extension_for("opus"), "opus");
    assert_eq!(audio_only_extension_for("flac"), "flac");
    assert_eq!(audio_only_extension_for("wav"), "wav");

    // Default
    assert_eq!(audio_only_extension_for("xyz"), "mka");
}

#[test]
fn test_limiter_boost_synthetic_analysis() {
    let opts = NormalizeOptions {
        target_tp: -1.0,
        boost_enabled: true,
        boost_gain_db: 12.0,
        ..Default::default()
    };
    let ceiling_db = opts.target_tp - ALIMITER_TP_HEADROOM_DB;
    assert!((ceiling_db - (-2.5)).abs() < f64::EPSILON);

    let synthetic_peak = ceiling_db - opts.boost_gain_db;
    assert!((synthetic_peak - (-14.5)).abs() < f64::EPSILON);

    let computed_gain = ceiling_db - synthetic_peak;
    assert!((computed_gain - 12.0).abs() < f64::EPSILON);

    let linear_limit = 10f64.powf(ceiling_db / 20.0);
    let expected_limit = 10f64.powf(-2.5 / 20.0);
    assert!((linear_limit - expected_limit).abs() < 0.001);
}

#[test]
fn test_limiter_boost_threshold_below() {
    let m = LoudnormMeasurements {
        input_i: -20.0,
        input_tp: -7.0,
        input_lra: 8.0,
        input_thresh: -30.0,
        target_offset: 0.0,
    };
    let shortfall = m.linear_shortfall(-14.0, -1.0);
    assert!(shortfall <= super::helpers::LIMITER_BOOST_SHORTFALL_THRESHOLD);
}

#[test]
fn test_limiter_boost_limit_linear() {
    let ceiling = -1.0 - ALIMITER_TP_HEADROOM_DB;
    let limit = 10f64.powf(ceiling / 20.0);
    assert!(
        (limit - 0.749_894).abs() < 0.001,
        "default limit={limit}, expected ~0.749894"
    );

    let ceiling2 = -2.0 - ALIMITER_TP_HEADROOM_DB;
    let limit2 = 10f64.powf(ceiling2 / 20.0);
    assert!(
        limit2 < limit,
        "lower TP should produce lower limit: {limit2} vs {limit}"
    );
}

#[test]
fn test_limiter_boost_threshold_above() {
    let m = LoudnormMeasurements {
        input_i: -24.0,
        input_tp: -3.0,
        input_lra: 8.0,
        input_thresh: -34.0,
        target_offset: 0.0,
    };
    let shortfall = m.linear_shortfall(-14.0, -1.0);
    assert!((shortfall - 8.0).abs() < f64::EPSILON);
    assert!(shortfall > super::helpers::LIMITER_BOOST_SHORTFALL_THRESHOLD);
}

#[test]
fn test_peak_filter_high_gain_uses_oversample() {
    let gain_db: f64 = 14.0;
    let linear_limit: f64 = 10f64.powf(-1.0 / 20.0);
    let enc_rate: u32 = 44100;
    let oversample_prefix = if gain_db >= TRUE_PEAK_OVERSAMPLE_GAIN_THRESHOLD {
        let rate_4x = enc_rate * 4;
        format!("aresample={rate_4x},")
    } else {
        String::new()
    };
    let spec = format!(
        "volume={gain_db:.6}dB,{oversample_prefix}aresample,\
         alimiter=limit={linear_limit:.6}:attack=5:release=50,\
         aformat=sample_fmts=flt:sample_rates={enc_rate}:channel_layouts=stereo",
    );
    assert!(
        spec.contains("aresample=176400,"),
        "high gain should insert 4x oversample: {spec}"
    );
    assert!(
        spec.contains("aresample=176400,aresample,alimiter="),
        "chain order: upsample → aresample → alimiter: {spec}"
    );
}

#[test]
fn test_peak_filter_low_gain_no_oversample() {
    let gain_db: f64 = 3.0;
    let linear_limit: f64 = 10f64.powf(-1.0 / 20.0);
    let enc_rate: u32 = 48000;
    let oversample_prefix = if gain_db >= TRUE_PEAK_OVERSAMPLE_GAIN_THRESHOLD {
        let rate_4x = enc_rate * 4;
        format!("aresample={rate_4x},")
    } else {
        String::new()
    };
    let spec = format!(
        "volume={gain_db:.6}dB,{oversample_prefix}aresample,\
         alimiter=limit={linear_limit:.6}:attack=5:release=50,\
         aformat=sample_fmts=flt:sample_rates={enc_rate}:channel_layouts=stereo",
    );
    assert!(
        !spec.contains("aresample=192000"),
        "low gain should not oversample: {spec}"
    );
    assert!(
        spec.contains("aresample,alimiter="),
        "aresample before alimiter even at low gain: {spec}"
    );
}

#[test]
fn test_peak_filter_at_threshold_uses_oversample() {
    let gain_db: f64 = 6.0;
    assert!(gain_db >= TRUE_PEAK_OVERSAMPLE_GAIN_THRESHOLD);
    let enc_rate: u32 = 48000;
    let rate_4x = enc_rate * 4;
    assert_eq!(rate_4x, 192_000);
}
