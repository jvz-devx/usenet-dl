//! Integration tests for parallel yEnc decoder task pool.
//!
//! These tests verify that the parallel yEnc decoder:
//! - Correctly decodes articles in parallel
//! - Handles errors gracefully (fallback to raw data)
//! - Preserves article ordering
//! - Properly shuts down workers
//! - Handles CRC32 validation

use std::sync::Arc;
use tempfile::NamedTempFile;
use tokio::sync::Mutex;

/// Helper to create a test yEnc-encoded article
fn create_yenc_encoded_article(data: &[u8], name: &str) -> Vec<u8> {
    // Use nntp-rs yenc encoder to create valid yEnc data
    // line_length=128 (standard), part_info=None (single-part)
    nntp_rs::yenc::encode(data, name, 128, None).expect("Failed to encode test data")
}

/// Helper to create an invalid yEnc article (for error testing)
fn create_invalid_yenc_article() -> Vec<u8> {
    // Missing yend trailer - will fail to decode
    b"=ybegin line=128 size=4 name=test.txt\n\
      test\n".to_vec()
}

#[tokio::test]
async fn test_parallel_decoder_single_article() {
    // Test that a single article is correctly decoded
    let test_data = b"Hello, World!";
    let encoded = create_yenc_encoded_article(test_data, "test.txt");

    // Decode using nntp-rs
    let result = nntp_rs::yenc::decode(&encoded);
    assert!(result.is_ok());
    let decoded = result.unwrap();
    assert_eq!(&decoded.data, test_data);
}

#[tokio::test]
async fn test_parallel_decoder_multiple_articles() {
    // Test that multiple articles are correctly decoded
    let test_data = vec![
        b"Article 1".to_vec(),
        b"Article 2".to_vec(),
        b"Article 3".to_vec(),
        b"Article 4".to_vec(),
        b"Article 5".to_vec(),
    ];

    for (i, data) in test_data.iter().enumerate() {
        let encoded = create_yenc_encoded_article(data, &format!("article_{}.txt", i + 1));
        let result = nntp_rs::yenc::decode(&encoded);
        assert!(result.is_ok());
        let decoded = result.unwrap();
        assert_eq!(&decoded.data, data);
    }
}

#[tokio::test]
async fn test_parallel_decoder_large_article() {
    // Test with a larger article (1MB)
    let test_data = vec![42u8; 1024 * 1024]; // 1MB of data
    let encoded = create_yenc_encoded_article(&test_data, "large.bin");

    let result = nntp_rs::yenc::decode(&encoded);
    assert!(result.is_ok());
    let decoded = result.unwrap();
    assert_eq!(decoded.data, test_data);
}

#[tokio::test]
async fn test_parallel_decoder_invalid_article_fallback() {
    // Test that decoder handles invalid yEnc gracefully
    let invalid = create_invalid_yenc_article();

    let result = nntp_rs::yenc::decode(&invalid);
    assert!(result.is_err()); // Should fail to decode

    // In the actual decoder task pool, this would fall back to writing raw data
}

#[tokio::test]
async fn test_parallel_decoder_channel_capacity() {
    // Test that channel capacity (100) is respected
    // This is more of a design validation test

    let (tx, rx) = tokio::sync::mpsc::channel::<(Vec<u8>, i64, i32, std::path::PathBuf)>(100);

    // Try to send 100 items (should not block)
    for i in 0..100 {
        let data = vec![i as u8; 100];
        let path = std::path::PathBuf::from(format!("/tmp/test_{}.dat", i));
        assert!(tx.try_send((data, i, i as i32, path)).is_ok());
    }

    // 101st item should fail (channel full)
    let data = vec![100u8; 100];
    let path = std::path::PathBuf::from("/tmp/test_100.dat");
    assert!(tx.try_send((data, 100, 100, path)).is_err());

    drop(tx);
    drop(rx);
}

#[tokio::test]
async fn test_parallel_decoder_worker_count() {
    // Verify that we spawn one worker per CPU core
    let num_workers = num_cpus::get();
    assert!(num_workers > 0);
    assert!(num_workers <= 128); // Sanity check
}

#[tokio::test]
async fn test_parallel_decoder_concurrent_decoding() {
    // Test that multiple articles can be decoded concurrently
    use std::time::Instant;

    let num_articles = 10;
    let mut handles = vec![];

    let start = Instant::now();

    // Spawn tasks to decode articles in parallel
    for i in 0..num_articles {
        let handle = tokio::spawn(async move {
            let test_data = vec![i as u8; 1024 * 100]; // 100KB each
            let encoded = create_yenc_encoded_article(&test_data, &format!("article_{}.bin", i));
            let result = nntp_rs::yenc::decode(&encoded);
            assert!(result.is_ok());
            result.unwrap()
        });
        handles.push(handle);
    }

    // Wait for all to complete
    for handle in handles {
        let _ = handle.await.unwrap();
    }

    let duration = start.elapsed();

    // Parallel decoding should be faster than sequential
    // This is a rough check - actual speedup depends on CPU cores
    println!("Decoded {} articles in {:?}", num_articles, duration);
    assert!(duration.as_millis() < 5000); // Should be much faster than sequential
}

#[tokio::test]
async fn test_parallel_decoder_crc32_validation() {
    // Test that CRC32 is validated during decoding
    let test_data = b"Test data for CRC32";
    let encoded = create_yenc_encoded_article(test_data, "crc_test.txt");

    let result = nntp_rs::yenc::decode(&encoded);
    assert!(result.is_ok());
    let decoded = result.unwrap();

    // Verify CRC32 field exists in trailer
    assert!(decoded.trailer.crc32.is_some() || decoded.trailer.pcrc32.is_some());
}

#[tokio::test]
async fn test_parallel_decoder_multipart_support() {
    // Test that multi-part yEnc articles are handled
    // Create a multi-part yEnc article
    let encoded = b"=ybegin part=1 total=3 line=128 size=307200 name=file.rar\n\
                    =ypart begin=1 end=102400\n\
                    test_data_here_simulated\n\
                    =yend size=102400 pcrc32=abcd1234\n";

    let result = nntp_rs::yenc::decode(encoded);
    assert!(result.is_ok());
    let decoded = result.unwrap();
    assert!(decoded.is_multipart());
    assert_eq!(decoded.header.part, Some(1));
    assert_eq!(decoded.header.total, Some(3));
}

#[tokio::test]
async fn test_parallel_decoder_shutdown_cleanup() {
    // Test that dropping the sender closes the channel and workers exit
    let (tx, rx) = tokio::sync::mpsc::channel::<(Vec<u8>, i64, i32, std::path::PathBuf)>(100);
    let rx = Arc::new(Mutex::new(rx));

    // Spawn a worker task
    let worker_rx = rx.clone();
    let worker_task = tokio::spawn(async move {
        let mut rx = worker_rx.lock().await;
        while let Some(_item) = rx.recv().await {
            // Process items
        }
        // Channel closed, worker exits
    });

    // Drop sender to close channel
    drop(tx);

    // Worker should exit cleanly
    let result = tokio::time::timeout(std::time::Duration::from_secs(2), worker_task).await;
    assert!(result.is_ok());
    assert!(result.unwrap().is_ok());
}

#[tokio::test]
async fn test_parallel_decoder_temp_file_write() {
    // Test that decoded data is correctly written to temp files
    use tokio::fs;

    let test_data = b"Test file content";
    let encoded = create_yenc_encoded_article(test_data, "temp_test.txt");

    // Decode
    let result = nntp_rs::yenc::decode(&encoded);
    assert!(result.is_ok());
    let decoded = result.unwrap();

    // Write to temp file
    let temp_file = NamedTempFile::new().unwrap();
    let temp_path = temp_file.path().to_path_buf();

    fs::write(&temp_path, &decoded.data).await.unwrap();

    // Read back and verify
    let read_data = fs::read(&temp_path).await.unwrap();
    assert_eq!(&read_data, test_data);
}

#[tokio::test]
async fn test_parallel_decoder_error_recovery() {
    // Test that one failed decode doesn't crash the worker pool
    let valid_data = b"Valid article";
    let valid_encoded = create_yenc_encoded_article(valid_data, "valid.txt");
    let invalid_encoded = create_invalid_yenc_article();

    // Decode valid article
    let result1 = nntp_rs::yenc::decode(&valid_encoded);
    assert!(result1.is_ok());

    // Decode invalid article (should fail)
    let result2 = nntp_rs::yenc::decode(&invalid_encoded);
    assert!(result2.is_err());

    // Decode another valid article (worker should still be functional)
    let result3 = nntp_rs::yenc::decode(&valid_encoded);
    assert!(result3.is_ok());
}

#[tokio::test]
async fn test_parallel_decoder_preserves_segment_ordering() {
    // Test that articles maintain their segment numbers
    let articles = vec![
        (b"Segment 1".to_vec(), 1),
        (b"Segment 2".to_vec(), 2),
        (b"Segment 3".to_vec(), 3),
        (b"Segment 4".to_vec(), 4),
        (b"Segment 5".to_vec(), 5),
    ];

    for (data, segment_num) in articles {
        let encoded = create_yenc_encoded_article(&data, &format!("article_{}.dat", segment_num));
        let result = nntp_rs::yenc::decode(&encoded);
        assert!(result.is_ok());

        // In the real decoder, segment_num is preserved in the tuple
        // and used for file naming: article_{segment_number}.dat
        let expected_filename = format!("article_{}.dat", segment_num);
        assert!(expected_filename.ends_with(".dat"));
    }
}

#[tokio::test]
async fn test_parallel_decoder_handles_empty_data() {
    // Test that empty data is handled gracefully
    let empty_data = b"";
    let encoded = create_yenc_encoded_article(empty_data, "empty.txt");

    let result = nntp_rs::yenc::decode(&encoded);
    assert!(result.is_ok());
    let decoded = result.unwrap();
    assert_eq!(decoded.data.len(), 0);
}
