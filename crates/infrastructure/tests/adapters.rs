use application::{
    Clock, DialogState, DialogStateStore, EnsureSubscriptionUseCase, EnsureUserUseCase,
    IdGenerator, PaymentCachePort, PaymentGatewayPort, PaymentTransactionRepository,
    SubscriptionRepository, UserRepository,
};
use chrono::{Duration, Utc};
use domain::{ChatId, Money, Months, PaymentId, PaymentTransaction, SubscriptionPolicy, UserId};
use infrastructure::{InMemoryStore, SystemClock, UuidPaymentIdGenerator};

#[test]
fn system_clock_returns_current_time() {
    let before = Utc::now();
    let now = SystemClock.now();
    let after = Utc::now();

    assert!(now >= before);
    assert!(now <= after);
}

#[test]
fn uuid_payment_id_generator_returns_unique_ids() {
    let generator = UuidPaymentIdGenerator;

    let first = generator.new_payment_id();
    let second = generator.new_payment_id();

    assert_ne!(first, second);
    assert!(!first.as_str().is_empty());
}

#[tokio::test]
async fn in_memory_store_backs_user_and_subscription_use_cases() {
    let store = InMemoryStore::new();
    let user_id = UserId::new(42);
    let chat_id = ChatId::new(42);

    let user = EnsureUserUseCase::new(&store)
        .execute(user_id)
        .await
        .unwrap();
    let subscription =
        EnsureSubscriptionUseCase::new(&store, &SystemClock, SubscriptionPolicy { trial_days: 7 })
            .execute(chat_id)
            .await
            .unwrap();

    assert_eq!(user.id, user_id);
    assert_eq!(store.user_count(), 1);
    assert_eq!(subscription.chat_id, chat_id);
    assert_eq!(store.subscription_count(), 1);
    assert!(store.find_user(user_id).await.unwrap().is_some());
    assert!(store.find_subscription(chat_id).await.unwrap().is_some());
}

#[tokio::test]
async fn in_memory_dialog_store_defaults_to_idle_and_roundtrips_state() {
    let store = InMemoryStore::new();
    let user_id = UserId::new(7);

    assert_eq!(store.get_state(user_id).await.unwrap(), DialogState::Idle);

    store
        .set_state(user_id, DialogState::AwaitingUtc)
        .await
        .unwrap();

    assert_eq!(
        store.get_state(user_id).await.unwrap(),
        DialogState::AwaitingUtc
    );
}

#[tokio::test]
async fn in_memory_payment_adapters_store_transactions_and_pending_cache() {
    let store = InMemoryStore::new();
    let payment_id = PaymentId::new("payment-1");
    let user_id = UserId::new(11);
    let now = Utc::now();
    let transaction = PaymentTransaction::new(
        payment_id.clone(),
        user_id,
        Money::rub(195),
        Some(Months::THREE),
        now,
    );

    store.save_payment_transaction(&transaction).await.unwrap();
    store
        .remember_pending_payment(&payment_id, user_id, now + Duration::minutes(15))
        .await
        .unwrap();
    let payment_url = store.create_payment(&transaction).await.unwrap();

    assert_eq!(
        store
            .find_payment_transaction(&payment_id)
            .await
            .unwrap()
            .unwrap()
            .payment_id,
        payment_id
    );
    assert_eq!(store.pending_payment(&payment_id).unwrap().user_id, user_id);
    assert!(payment_url.contains(payment_id.as_str()));
}
