use super::*;

#[tokio::test]
async fn test_list_categories() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use serde_json::Value;
    use tower::ServiceExt;

    println!("\n=== Testing GET /categories ===");

    // Create test downloader
    let (downloader, _temp_dir) = create_test_downloader().await;

    // Create router
    let config = downloader.get_config();
    let app = create_router(downloader.clone(), config.clone());

    // Test 1: Get categories (should be empty by default)
    println!("\nTest 1: Getting empty categories list");
    let request = Request::builder()
        .method("GET")
        .uri("/categories")
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();

    println!(
        "Categories response: {}",
        serde_json::to_string_pretty(&json).unwrap()
    );

    // Should be an empty object {}
    assert!(json.is_object());
    assert_eq!(json.as_object().unwrap().len(), 0);

    println!("✅ GET /categories test passed!");
    println!("   - Returns 200 OK");
    println!("   - Returns empty object when no categories configured");
    println!("   - Response is valid JSON object");
}

#[tokio::test]
async fn test_create_or_update_category() {
    let (downloader, _temp_dir) = create_test_downloader().await;
    let config = downloader.get_config();
    let app = create_router(downloader.clone(), config.clone());

    // Test 1: Create a new category
    let category_config = CategoryConfig {
        destination: PathBuf::from("/downloads/movies"),
        post_process: Some(PostProcess::UnpackAndCleanup),
        scripts: vec![],
    };

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/categories/movies")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&category_config).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    // Test 2: Verify the category was created by listing categories
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/categories")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let categories: std::collections::HashMap<String, CategoryConfig> =
        serde_json::from_slice(&body).unwrap();

    assert_eq!(categories.len(), 1);
    assert!(categories.contains_key("movies"));
    assert_eq!(
        categories.get("movies").unwrap().destination,
        PathBuf::from("/downloads/movies")
    );
    assert_eq!(
        categories.get("movies").unwrap().post_process,
        Some(PostProcess::UnpackAndCleanup)
    );

    // Test 3: Update the existing category
    let updated_config = CategoryConfig {
        destination: PathBuf::from("/downloads/movies-updated"),
        post_process: Some(PostProcess::Unpack),
        scripts: vec![],
    };

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/categories/movies")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&updated_config).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    // Test 4: Verify the category was updated
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/categories")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let categories: std::collections::HashMap<String, CategoryConfig> =
        serde_json::from_slice(&body).unwrap();

    assert_eq!(categories.len(), 1);
    assert!(categories.contains_key("movies"));
    assert_eq!(
        categories.get("movies").unwrap().destination,
        PathBuf::from("/downloads/movies-updated")
    );
    assert_eq!(
        categories.get("movies").unwrap().post_process,
        Some(PostProcess::Unpack)
    );

    println!("✅ PUT /categories/:name test passed!");
    println!("   - Creates new category with 204 No Content");
    println!("   - Category is retrievable via GET /categories");
    println!("   - Updates existing category with 204 No Content");
    println!("   - Updated values are reflected in GET");
}

#[tokio::test]
async fn test_delete_category() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use serde_json::Value;
    use tower::ServiceExt;

    let (downloader, _temp_dir) = create_test_downloader().await;
    let config = downloader.get_config();
    let app = create_router(downloader.clone(), config.clone());

    // Test 1: Try to delete a non-existent category (should return 404)
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/categories/nonexistent")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let error: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(error["error"]["code"], "category_not_found");
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("nonexistent")
    );

    // Test 2: Create a category first
    let category_config = CategoryConfig {
        destination: PathBuf::from("/downloads/movies"),
        post_process: Some(PostProcess::UnpackAndCleanup),
        scripts: vec![],
    };

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/categories/movies")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&category_config).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    // Test 3: Verify the category exists
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/categories")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let categories: std::collections::HashMap<String, CategoryConfig> =
        serde_json::from_slice(&body).unwrap();

    assert_eq!(categories.len(), 1);
    assert!(categories.contains_key("movies"));

    // Test 4: Delete the category (should return 204)
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/categories/movies")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    // Test 5: Verify the category is gone
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/categories")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let categories: std::collections::HashMap<String, CategoryConfig> =
        serde_json::from_slice(&body).unwrap();

    assert_eq!(categories.len(), 0);
    assert!(!categories.contains_key("movies"));

    // Test 6: Try to delete the same category again (should return 404)
    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/categories/movies")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let error: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(error["error"]["code"], "category_not_found");

    println!("✅ DELETE /categories/:name test passed!");
    println!("   - Returns 404 for non-existent category");
    println!("   - Error includes category name in message");
    println!("   - Deletes existing category with 204 No Content");
    println!("   - Category is no longer in GET /categories");
    println!("   - Second delete attempt returns 404");
}
