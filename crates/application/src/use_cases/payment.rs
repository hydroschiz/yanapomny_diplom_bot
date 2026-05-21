use domain::{Money, Payment, PaymentId, PaymentProvider, PaymentStatus};

use crate::{ApplicationError, ApplicationResult, Clock, PaymentGateway, PaymentRepository};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatePaymentCommand {
    pub payment_id: PaymentId,
    pub provider: PaymentProvider,
    pub amount: Money,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedPayment {
    pub payment: Payment,
    pub confirmation_url: String,
}

pub struct CreatePaymentUseCase<'a, R, G, C> {
    payments: &'a R,
    gateway: &'a G,
    clock: &'a C,
}

impl<'a, R, G, C> CreatePaymentUseCase<'a, R, G, C>
where
    R: PaymentRepository,
    G: PaymentGateway,
    C: Clock,
{
    pub const fn new(payments: &'a R, gateway: &'a G, clock: &'a C) -> Self {
        Self {
            payments,
            gateway,
            clock,
        }
    }

    pub async fn execute(
        &self,
        command: CreatePaymentCommand,
    ) -> ApplicationResult<CreatedPayment> {
        let payment = Payment::new(
            command.payment_id,
            command.provider,
            command.amount,
            self.clock.now(),
        );
        self.payments.save_payment(&payment).await?;
        let confirmation_url = self.gateway.create_payment(&payment).await?;
        Ok(CreatedPayment {
            payment,
            confirmation_url,
        })
    }
}

pub struct ProcessYooKassaWebhookUseCase<'a, R> {
    payments: &'a R,
}

impl<'a, R> ProcessYooKassaWebhookUseCase<'a, R>
where
    R: PaymentRepository,
{
    pub const fn new(payments: &'a R) -> Self {
        Self { payments }
    }

    pub async fn execute(
        &self,
        payment_id: &PaymentId,
        status: PaymentStatus,
    ) -> ApplicationResult<Payment> {
        let mut payment = self
            .payments
            .find_payment(payment_id)
            .await?
            .ok_or_else(|| ApplicationError::NotFound {
                entity: "payment",
                id: payment_id.to_string(),
            })?;
        payment.update_status(status);
        self.payments.save_payment(&payment).await?;
        Ok(payment)
    }
}
