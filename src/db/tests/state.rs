use crate::db::*;
use tempfile::NamedTempFile;

#[tokio::test]
async fn test_shutdown_state_initial() {
    // Test initial shutdown state after migration
    let temp_file = NamedTempFile::new().unwrap();
    let db = Database::new(temp_file.path()).await.unwrap();

    // After migration, shutdown state should be "false" (unclean)
    let was_unclean = db.was_unclean_shutdown().await.unwrap();
    assert!(
        was_unclean,
        "Initial state should indicate unclean shutdown"
    );

    db.close().await;
}

#[tokio::test]
async fn test_shutdown_state_clean_lifecycle() {
    // Test clean start and shutdown sequence
    let temp_file = NamedTempFile::new().unwrap();
    let db = Database::new(temp_file.path()).await.unwrap();

    // Mark clean start (application started)
    db.set_clean_start().await.unwrap();
    let was_unclean = db.was_unclean_shutdown().await.unwrap();
    assert!(
        was_unclean,
        "After clean start, should still indicate unclean (not yet shut down)"
    );

    // Mark clean shutdown (application shutting down gracefully)
    db.set_clean_shutdown().await.unwrap();
    let was_unclean = db.was_unclean_shutdown().await.unwrap();
    assert!(!was_unclean, "After clean shutdown, should indicate clean");

    db.close().await;
}

#[tokio::test]
async fn test_shutdown_state_unclean_detection() {
    // Test unclean shutdown detection (crash scenario)
    let temp_file = NamedTempFile::new().unwrap();

    // First session: start but don't shut down cleanly (simulating crash)
    {
        let db = Database::new(temp_file.path()).await.unwrap();
        db.set_clean_start().await.unwrap();
        // Intentionally NOT calling set_clean_shutdown() - simulates crash
        db.close().await;
    }

    // Second session: detect unclean shutdown
    {
        let db = Database::new(temp_file.path()).await.unwrap();
        let was_unclean = db.was_unclean_shutdown().await.unwrap();
        assert!(
            was_unclean,
            "Should detect unclean shutdown from previous session"
        );

        // Now do a clean shutdown
        db.set_clean_start().await.unwrap();
        db.set_clean_shutdown().await.unwrap();
        db.close().await;
    }

    // Third session: should be clean now
    {
        let db = Database::new(temp_file.path()).await.unwrap();
        let was_unclean = db.was_unclean_shutdown().await.unwrap();
        assert!(
            !was_unclean,
            "Should detect clean shutdown from previous session"
        );
        db.close().await;
    }
}
