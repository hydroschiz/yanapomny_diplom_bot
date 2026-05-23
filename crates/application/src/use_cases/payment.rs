use chrono::Duration;
use domain::{
    tariff_for_months, ChatId, Money, Months, Payment, PaymentId, PaymentProvider, PaymentStatus,
    PaymentTransaction, Subscription, SubscriptionPolicy, UserId,
};

use crate::{
    ApplicationError, ApplicationResult, Clock, IdGenerator, PaymentCachePort, PaymentGatewayPort,
    PaymentRepository, PaymentTransactionRepository, SubscriptionRepository,
};

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
    G: PaymentGatewayPort,
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

        // Convert Payment to a minimal PaymentTransaction for gateway
        let transaction = PaymentTransaction::new(
            payment.id.clone(),
            UserId::new(0), // Not used for simple payments
            payment.amount,
            None,
            self.clock.now(),
        );
        let (_, confirmation_url) = self.gateway.create_payment(&transaction).await?;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CreateSubscriptionPaymentCommand {
    pub user_id: UserId,
    pub months: Months,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedSubscriptionPayment {
    pub transaction: PaymentTransaction,
    pub confirmation_url: String,
}

pub struct CreateSubscriptionPaymentUseCase<'a, T, G, P, I, C> {
    transactions: &'a T,
    gateway: &'a G,
    pending_cache: &'a P,
    ids: &'a I,
    clock: &'a C,
}

impl<'a, T, G, P, I, C> CreateSubscriptionPaymentUseCase<'a, T, G, P, I, C>
where
    T: PaymentTransactionRepository,
    G: PaymentGatewayPort,
    P: PaymentCachePort,
    I: IdGenerator,
    C: Clock,
{
    pub const fn new(
        transactions: &'a T,
        gateway: &'a G,
        pending_cache: &'a P,
        ids: &'a I,
        clock: &'a C,
    ) -> Self {
        Self {
            transactions,
            gateway,
            pending_cache,
            ids,
            clock,
        }
    }

    pub async fn execute(
        &self,
        command: CreateSubscriptionPaymentCommand,
    ) -> ApplicationResult<CreatedSubscriptionPayment> {
        let tariff = tariff_for_months(command.months).ok_or_else(|| {
            ApplicationError::Conflict(format!("unsupported tariff: {} months", command.months))
        })?;
        let now = self.clock.now();
        let payment_id = self.ids.new_payment_id();
        let mut transaction = PaymentTransaction::new(
            payment_id.clone(),
            command.user_id,
            tariff.price,
            Some(command.months),
            now,
        );
        transaction.provider = Some("yookassa".to_string());
        transaction.idempotence_key = Some(payment_id.to_string());
        self.transactions
            .save_payment_transaction(&transaction)
            .await?;

        let (provider_payment_id, confirmation_url) =
            self.gateway.create_payment(&transaction).await?;

        // Store the provider's payment ID so we can check status later
        transaction.set_provider_payment_id(&provider_payment_id);
        self.transactions
            .save_payment_transaction(&transaction)
            .await?;

        self.pending_cache
            .remember_pending_payment(&payment_id, command.user_id, now + Duration::minutes(30))
            .await?;

        Ok(CreatedSubscriptionPayment {
            transaction,
            confirmation_url,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscriptionPaymentStatus {
    pub transaction: PaymentTransaction,
    pub subscription: Option<Subscription>,
}

pub struct CheckSubscriptionPaymentUseCase<'a, T, G, S, C> {
    transactions: &'a T,
    gateway: Option<&'a G>,
    subscriptions: &'a S,
    clock: &'a C,
}

impl<'a, T, G, S, C> CheckSubscriptionPaymentUseCase<'a, T, G, S, C>
where
    T: PaymentTransactionRepository,
    G: PaymentGatewayPort,
    S: SubscriptionRepository,
    C: Clock,
{
    pub const fn new(
        transactions: &'a T,
        gateway: Option<&'a G>,
        subscriptions: &'a S,
        clock: &'a C,
    ) -> Self {
        Self {
            transactions,
            gateway,
            subscriptions,
            clock,
        }
    }

    pub async fn execute(
        &self,
        payment_id: &PaymentId,
    ) -> ApplicationResult<SubscriptionPaymentStatus> {
        let mut transaction = self.load_transaction(payment_id).await?;

        // If transaction is not yet succeeded, check with payment gateway
        if transaction.status != PaymentStatus::Succeeded {
            if let Some(gateway) = self.gateway {
                // Use provider's payment_id if available (new flow),
                // otherwise fall back to our payment_id (legacy compatibility)
                let provider_id = transaction
                    .provider_payment_id
                    .as_ref()
                    .map(|id| PaymentId::new(id.as_str()))
                    .unwrap_or_else(|| payment_id.clone());

                let gateway_status = gateway.get_payment_status(&provider_id).await?;
                if gateway_status == PaymentStatus::Succeeded {
                    let now = self.clock.now();
                    transaction.update_status(gateway_status, now);
                    self.transactions
                        .save_payment_transaction(&transaction)
                        .await?;
                }
            }
        }

        self.fulfill_if_succeeded(transaction).await
    }

    async fn load_transaction(
        &self,
        payment_id: &PaymentId,
    ) -> ApplicationResult<PaymentTransaction> {
        self.transactions
            .find_payment_transaction(payment_id)
            .await?
            .ok_or_else(|| ApplicationError::NotFound {
                entity: "payment_transaction",
                id: payment_id.to_string(),
            })
    }

    async fn fulfill_if_succeeded(
        &self,
        mut transaction: PaymentTransaction,
    ) -> ApplicationResult<SubscriptionPaymentStatus> {
        if transaction.status != PaymentStatus::Succeeded || transaction.fulfilled {
            return Ok(SubscriptionPaymentStatus {
                transaction,
                subscription: None,
            });
        }

        let Some(months) = transaction.months else {
            return Err(ApplicationError::Conflict(
                "payment transaction has no subscription period".to_string(),
            ));
        };
        let now = self.clock.now();
        let chat_id = ChatId::new(transaction.user_id.value());
        let mut subscription = self
            .subscriptions
            .find_subscription(chat_id)
            .await?
            .unwrap_or_else(|| {
                Subscription::new_trial(chat_id, now, SubscriptionPolicy { trial_days: 0 })
            });
        subscription.link_user(transaction.user_id);
        subscription.extend(months, now);
        self.subscriptions.save_subscription(&subscription).await?;
        transaction.mark_fulfilled(now);
        self.transactions
            .save_payment_transaction(&transaction)
            .await?;

        Ok(SubscriptionPaymentStatus {
            transaction,
            subscription: Some(subscription),
        })
    }
}

pub struct ProcessSubscriptionPaymentWebhookUseCase<'a, T, G, S, C> {
    transactions: &'a T,
    gateway: Option<&'a G>,
    subscriptions: &'a S,
    clock: &'a C,
}

impl<'a, T, G, S, C> ProcessSubscriptionPaymentWebhookUseCase<'a, T, G, S, C>
where
    T: PaymentTransactionRepository,
    G: PaymentGatewayPort,
    S: SubscriptionRepository,
    C: Clock,
{
    pub const fn new(
        transactions: &'a T,
        gateway: Option<&'a G>,
        subscriptions: &'a S,
        clock: &'a C,
    ) -> Self {
        Self {
            transactions,
            gateway,
            subscriptions,
            clock,
        }
    }

    pub async fn execute(
        &self,
        payment_id: &PaymentId,
        status: PaymentStatus,
    ) -> ApplicationResult<SubscriptionPaymentStatus> {
        let mut transaction = self
            .transactions
            .find_payment_transaction(payment_id)
            .await?
            .ok_or_else(|| ApplicationError::NotFound {
                entity: "payment_transaction",
                id: payment_id.to_string(),
            })?;
        transaction.update_status(status, self.clock.now());
        self.transactions
            .save_payment_transaction(&transaction)
            .await?;
        CheckSubscriptionPaymentUseCase::new(
            self.transactions,
            self.gateway,
            self.subscriptions,
            self.clock,
        )
        .fulfill_if_succeeded(transaction)
        .await
    }
}
