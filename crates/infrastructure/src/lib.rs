//! Infrastructure layer.
//!
//! Concrete adapters for application ports. The legacy root crate still owns the
//! production MongoDB/Redis/YooKassa/VK wiring while this crate grows behind the
//! same contracts.

pub mod clock;
pub mod id;
pub mod llm;
pub mod memory;
pub mod mongo;
pub mod redis;
pub mod twitch;
pub mod yookassa;

pub use clock::SystemClock;
pub use id::UuidPaymentIdGenerator;
pub use llm::HttpLlmInterpreter;
pub use memory::{InMemoryPendingPayment, InMemoryStore};
pub use mongo::MongoStore;
pub use redis::RedisPaymentCache;
pub use twitch::TwitchGateway;
pub use yookassa::HttpYooKassaPaymentGateway;
