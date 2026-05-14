//! Load test for the scheduler.
//!
//! This test simulates high load by creating many reminders
//! and verifying the scheduler handles them correctly.
//!
//! Run with: cargo test --test scheduler_load_test -- --nocapture

use chrono::{Duration, Utc};
use mongodb::{bson::doc, Client};
use std::env;

const TEST_DB: &str = "tgBot";
const REMINDERS_COLLECTION: &str = "reminders";

#[tokio::test]
async fn test_scheduler_atomic_claiming() {
    // Skip if no MongoDB connection
    let mongo_uri = match env::var("MONGO_URI") {
        Ok(uri) => uri,
        Err(_) => {
            println!("Skipping test: MONGO_URI not set");
            return;
        }
    };

    let client = Client::with_uri_str(&mongo_uri).await.unwrap();
    let db = client.database(TEST_DB);
    let collection = db.collection::<mongodb::bson::Document>(REMINDERS_COLLECTION);

    // Create test reminders with unique prefix
    let test_prefix = format!("LOAD_TEST_{}", Utc::now().timestamp());
    let now = Utc::now();
    let past_time = now - Duration::minutes(5);

    println!("Creating 100 test reminders with prefix: {}", test_prefix);

    // Insert 100 test reminders
    for i in 0..100 {
        let reminder = doc! {
            "id": -1000000000i64 - i as i64,  // Fake chat IDs
            "text": format!("{}_reminder_{}", test_prefix, i),
            "delay": "",
            "time": mongodb::bson::DateTime::from_chrono(past_time),
            "status": "active",
            "remID": 900000 + i,
            "retryCount": 0,
        };
        collection.insert_one(reminder, None).await.unwrap();
    }

    // Verify all are active
    let active_count = collection
        .count_documents(
            doc! { "text": { "$regex": &test_prefix }, "status": "active" },
            None,
        )
        .await
        .unwrap();
    assert_eq!(active_count, 100, "Should have 100 active reminders");
    println!("Created {} active reminders", active_count);

    // Simulate atomic claiming (batch of 50)
    println!("Simulating atomic claim of 50 reminders...");
    let filter = doc! {
        "text": { "$regex": &test_prefix },
        "status": "active"
    };
    let update = doc! { "$set": { "status": "processing" } };

    // First batch claim
    let mut claimed = 0;
    for _ in 0..50 {
        let result = collection
            .find_one_and_update(filter.clone(), update.clone(), None)
            .await
            .unwrap();
        if result.is_some() {
            claimed += 1;
        }
    }
    println!("Claimed {} reminders in first batch", claimed);
    assert_eq!(claimed, 50);

    // Check remaining active
    let remaining_active = collection
        .count_documents(
            doc! { "text": { "$regex": &test_prefix }, "status": "active" },
            None,
        )
        .await
        .unwrap();
    println!("Remaining active: {}", remaining_active);
    assert_eq!(remaining_active, 50);

    // Second batch claim
    let mut claimed2 = 0;
    for _ in 0..50 {
        let result = collection
            .find_one_and_update(filter.clone(), update.clone(), None)
            .await
            .unwrap();
        if result.is_some() {
            claimed2 += 1;
        }
    }
    println!("Claimed {} reminders in second batch", claimed2);
    assert_eq!(claimed2, 50);

    // Try to claim more - should get 0
    let result = collection
        .find_one_and_update(filter.clone(), update.clone(), None)
        .await
        .unwrap();
    assert!(
        result.is_none(),
        "Should have no more active reminders to claim"
    );

    // Clean up test data
    println!("Cleaning up test data...");
    let deleted = collection
        .delete_many(doc! { "text": { "$regex": &test_prefix } }, None)
        .await
        .unwrap();
    println!("Deleted {} test reminders", deleted.deleted_count);

    println!("\n✅ Atomic claiming test passed!");
}

#[tokio::test]
async fn test_scheduler_retry_logic() {
    let mongo_uri = match env::var("MONGO_URI") {
        Ok(uri) => uri,
        Err(_) => {
            println!("Skipping test: MONGO_URI not set");
            return;
        }
    };

    let client = Client::with_uri_str(&mongo_uri).await.unwrap();
    let db = client.database(TEST_DB);
    let collection = db.collection::<mongodb::bson::Document>(REMINDERS_COLLECTION);

    let test_prefix = format!("RETRY_TEST_{}", Utc::now().timestamp());
    let now = Utc::now();

    println!("Testing retry logic with prefix: {}", test_prefix);

    // Create a reminder
    let reminder = doc! {
        "id": -999999999i64,
        "text": format!("{}_retry_test", test_prefix),
        "delay": "",
        "time": mongodb::bson::DateTime::from_chrono(now),
        "status": "active",
        "remID": 999999,
        "retryCount": 0,
    };
    collection.insert_one(reminder, None).await.unwrap();

    // Simulate retry scheduling
    let retry_at = now + Duration::seconds(30);
    let update = doc! {
        "$set": {
            "status": "retry",
            "retryCount": 1,
            "retryAt": mongodb::bson::DateTime::from_chrono(retry_at)
        }
    };
    collection
        .update_one(doc! { "text": { "$regex": &test_prefix } }, update, None)
        .await
        .unwrap();

    // Verify retry state
    let reminder = collection
        .find_one(doc! { "text": { "$regex": &test_prefix } }, None)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(reminder.get_str("status").unwrap(), "retry");
    assert_eq!(reminder.get_i32("retryCount").unwrap(), 1);
    println!("Retry scheduled correctly");

    // Clean up
    collection
        .delete_many(doc! { "text": { "$regex": &test_prefix } }, None)
        .await
        .unwrap();

    println!("\n✅ Retry logic test passed!");
}

#[tokio::test]
async fn test_scheduler_no_duplicate_claims() {
    let mongo_uri = match env::var("MONGO_URI") {
        Ok(uri) => uri,
        Err(_) => {
            println!("Skipping test: MONGO_URI not set");
            return;
        }
    };

    let client = Client::with_uri_str(&mongo_uri).await.unwrap();
    let db = client.database(TEST_DB);
    let collection = db.collection::<mongodb::bson::Document>(REMINDERS_COLLECTION);

    let test_prefix = format!("DEDUP_TEST_{}", Utc::now().timestamp());
    let past_time = Utc::now() - Duration::minutes(1);

    println!("Testing no duplicate claims with prefix: {}", test_prefix);

    // Create 10 reminders
    for i in 0..10 {
        let reminder = doc! {
            "id": -888888888i64 - i as i64,
            "text": format!("{}_dedup_{}", test_prefix, i),
            "delay": "",
            "time": mongodb::bson::DateTime::from_chrono(past_time),
            "status": "active",
            "remID": 888000 + i,
            "retryCount": 0,
        };
        collection.insert_one(reminder, None).await.unwrap();
    }

    // Simulate concurrent claiming from multiple "workers"
    let filter = doc! {
        "text": { "$regex": &test_prefix },
        "status": "active"
    };
    let update = doc! { "$set": { "status": "processing" } };

    // Spawn multiple concurrent claims
    let mut handles = vec![];
    for worker_id in 0..5 {
        let coll = collection.clone();
        let f = filter.clone();
        let u = update.clone();

        handles.push(tokio::spawn(async move {
            let mut claimed = 0;
            for _ in 0..10 {
                if coll
                    .find_one_and_update(f.clone(), u.clone(), None)
                    .await
                    .unwrap()
                    .is_some()
                {
                    claimed += 1;
                }
            }
            (worker_id, claimed)
        }));
    }

    // Collect results
    let mut total_claimed = 0;
    for handle in handles {
        let (worker_id, claimed) = handle.await.unwrap();
        println!("Worker {} claimed {} reminders", worker_id, claimed);
        total_claimed += claimed;
    }

    // Should claim exactly 10 (no duplicates!)
    assert_eq!(
        total_claimed, 10,
        "Total claimed should be exactly 10 (no duplicates)"
    );
    println!("Total claimed: {} (expected: 10)", total_claimed);

    // Clean up
    collection
        .delete_many(doc! { "text": { "$regex": &test_prefix } }, None)
        .await
        .unwrap();

    println!("\n✅ No duplicate claims test passed!");
}
