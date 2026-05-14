use application::IdGenerator;
use domain::PaymentId;
use uuid::Uuid;

#[derive(Debug, Default, Clone, Copy)]
pub struct UuidPaymentIdGenerator;

impl IdGenerator for UuidPaymentIdGenerator {
    fn new_payment_id(&self) -> PaymentId {
        PaymentId::new(Uuid::new_v4().to_string())
    }
}
