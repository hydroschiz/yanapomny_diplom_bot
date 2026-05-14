//! Infrastructure layer.
//!
//! Concrete adapters for application ports. The legacy root crate still owns the
//! production MongoDB/Redis/YooKassa/VK wiring while this crate grows behind the
//! same contracts.

pub mod clock;
pub mod id;
pub mod memory;

pub use clock::SystemClock;
pub use id::UuidPaymentIdGenerator;
pub use memory::{InMemoryPendingPayment, InMemoryStore};
