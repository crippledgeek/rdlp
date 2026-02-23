//! Tests for output template rendering and path generation

use super::*;

#[test]
fn test_generate_output_path_custom_template() {
    let config = Config {
        output_template: "%(id)s.%(ext)s".to_string(),
        ..Default::default()
    };
    let (tx, _rx) = mpsc::channel::<Event>(64);
    let orchestrator = Orchestrator::new(
        Arc::new(config),
        tx,
        DownloadId::next(),
        CancellationToken::new(),
        None,
    );
    let info = create_test_info_dict(vec![]);
    let format = create_test_format("720p", "720p", Some(1000000));

    let path = orchestrator.generate_output_path(&info, &format).unwrap();
    assert_eq!(path.file_name().unwrap().to_str().unwrap(), "test_id.mp4");
}

#[test]
fn test_generate_output_path_subdirectory_template() {
    let config = Config {
        output_template: "%(extractor)s/%(title)s.%(ext)s".to_string(),
        ..Default::default()
    };
    let (tx, _rx) = mpsc::channel::<Event>(64);
    let orchestrator = Orchestrator::new(
        Arc::new(config),
        tx,
        DownloadId::next(),
        CancellationToken::new(),
        None,
    );
    let info = create_test_info_dict(vec![]);
    let format = create_test_format("720p", "720p", Some(1000000));

    let path = orchestrator.generate_output_path(&info, &format).unwrap();
    // Should create a subdirectory structure
    let components: Vec<_> = path.components().collect();
    let len = components.len();
    assert!(len >= 2);
    // Last component is the filename
    assert_eq!(
        path.file_name().unwrap().to_str().unwrap(),
        "Test Video.mp4"
    );
    // Second-to-last is the extractor directory
    let parent = path.parent().unwrap();
    assert_eq!(
        parent.file_name().unwrap().to_str().unwrap(),
        "TestExtractor"
    );
}

#[test]
fn test_generate_output_path_field_with_default() {
    let config = Config {
        output_template: "%(uploader|Unknown)s - %(title)s.%(ext)s".to_string(),
        ..Default::default()
    };
    let (tx, _rx) = mpsc::channel::<Event>(64);
    let orchestrator = Orchestrator::new(
        Arc::new(config),
        tx,
        DownloadId::next(),
        CancellationToken::new(),
        None,
    );
    let info = create_test_info_dict(vec![]);
    let format = create_test_format("720p", "720p", Some(1000000));

    let path = orchestrator.generate_output_path(&info, &format).unwrap();
    // uploader is None in test_info_dict, so default "Unknown" should be used
    assert_eq!(
        path.file_name().unwrap().to_str().unwrap(),
        "Unknown - Test Video.mp4"
    );
}

#[test]
fn test_generate_output_path_sanitizes_template_field_values() {
    let config = Config {
        output_template: "%(title)s.%(ext)s".to_string(),
        ..Default::default()
    };
    let (tx, _rx) = mpsc::channel::<Event>(64);
    let orchestrator = Orchestrator::new(
        Arc::new(config),
        tx,
        DownloadId::next(),
        CancellationToken::new(),
        None,
    );
    let mut info = InfoDict::new(
        "test123",
        "Bad/Title\\With:Chars",
        "test",
        "https://example.com/test",
    );
    info.formats = vec![];
    let format = create_test_format("720p", "720p", Some(1000000));

    let path = orchestrator.generate_output_path(&info, &format).unwrap();
    let filename = path.file_name().unwrap().to_str().unwrap();
    // Slashes and colons should be replaced
    assert!(!filename.contains('/'));
    assert!(!filename.contains('\\'));
    assert!(!filename.contains(':'));
}

#[test]
fn test_generate_output_path_with_format_fields() {
    let config = Config {
        output_template: "%(title)s_%(height)s.%(ext)s".to_string(),
        ..Default::default()
    };
    let (tx, _rx) = mpsc::channel::<Event>(64);
    let orchestrator = Orchestrator::new(
        Arc::new(config),
        tx,
        DownloadId::next(),
        CancellationToken::new(),
        None,
    );
    let info = create_test_info_dict(vec![]);
    let format = create_test_format("720p", "720p", Some(1000000));

    let path = orchestrator.generate_output_path(&info, &format).unwrap();
    assert_eq!(
        path.file_name().unwrap().to_str().unwrap(),
        "Test Video_1080.mp4"
    );
}

#[test]
fn test_generate_output_path_numeric_padding() {
    let config = Config {
        output_template: "%(playlist_index)03d - %(title)s.%(ext)s".to_string(),
        ..Default::default()
    };
    let (tx, _rx) = mpsc::channel::<Event>(64);
    let orchestrator = Orchestrator::new(
        Arc::new(config),
        tx,
        DownloadId::next(),
        CancellationToken::new(),
        None,
    );
    let mut info = create_test_info_dict(vec![]);
    info.playlist_index = Some(7);
    let format = create_test_format("720p", "720p", Some(1000000));

    let path = orchestrator.generate_output_path(&info, &format).unwrap();
    assert_eq!(
        path.file_name().unwrap().to_str().unwrap(),
        "007 - Test Video.mp4"
    );
}

#[test]
fn test_generate_output_path_missing_field_defaults_to_na() {
    let config = Config {
        output_template: "%(nonexistent_field)s.%(ext)s".to_string(),
        ..Default::default()
    };
    let (tx, _rx) = mpsc::channel::<Event>(64);
    let orchestrator = Orchestrator::new(
        Arc::new(config),
        tx,
        DownloadId::next(),
        CancellationToken::new(),
        None,
    );
    let info = create_test_info_dict(vec![]);
    let format = create_test_format("720p", "720p", Some(1000000));

    // yt-dlp behavior: missing fields produce "NA" instead of an error
    let path = orchestrator.generate_output_path(&info, &format).unwrap();
    assert_eq!(path.file_name().unwrap().to_str().unwrap(), "NA.mp4");
}

#[test]
fn test_generate_output_path_with_output_directory() {
    let config = Config {
        output_template: "%(title)s.%(ext)s".to_string(),
        output_directory: PathBuf::from("/tmp/downloads"),
        ..Default::default()
    };
    let (tx, _rx) = mpsc::channel::<Event>(64);
    let orchestrator = Orchestrator::new(
        Arc::new(config),
        tx,
        DownloadId::next(),
        CancellationToken::new(),
        None,
    );
    let info = create_test_info_dict(vec![]);
    let format = create_test_format("720p", "720p", Some(1000000));

    let path = orchestrator.generate_output_path(&info, &format).unwrap();
    assert!(path.starts_with("/tmp/downloads") || path.starts_with("\\tmp\\downloads"));
    assert_eq!(
        path.file_name().unwrap().to_str().unwrap(),
        "Test Video.mp4"
    );
}
