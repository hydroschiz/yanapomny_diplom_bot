use chrono::Duration;
use domain::{
    tariff_for_months, ChatId, Money, Months, Payment, PaymentId, PaymentProvider, PaymentStatus,
    PaymentTransaction, Subscription, SubscriptionPolicy, UserId,
};

use crate::{
    ApplicationError, ApplicationResult, Clock, IdGenerator, Notification, Notifier,
    PaymentCachePort, PaymentGatewayPort, PaymentRepository, PaymentTransactionRepository,
    PendingPayment, ReferralRepository, SubscriptionRepository,
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
        let expires_at = now + Duration::minutes(30);

        if let Some(pending) = self
            .pending_cache
            .pending_payment_for_user(command.user_id)
            .await?
        {
            if pending.months == Some(command.months) {
                if let Some(transaction) = self
                    .transactions
                    .find_payment_transaction(&pending.payment_id)
                    .await?
                {
                    self.pending_cache
                        .refresh_pending_payment(&pending, expires_at)
                        .await?;
                    return Ok(CreatedSubscriptionPayment {
                        transaction,
                        confirmation_url: pending.confirmation_url,
                    });
                }
            }

            self.pending_cache
                .delete_pending_payment(&pending.payment_id)
                .await?;
        }

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
            .remember_pending_payment(&PendingPayment::new(
                payment_id,
                command.user_id,
                Some(command.months),
                confirmation_url.clone(),
                expires_at,
            ))
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

pub struct ProcessSubscriptionPaymentWebhookUseCase<'a, T, S, P, R, N, C> {
    transactions: &'a T,
    subscriptions: &'a S,
    payment_cache: &'a P,
    referrals: &'a R,
    notifier: &'a N,
    clock: &'a C,
}

impl<'a, T, S, P, R, N, C> ProcessSubscriptionPaymentWebhookUseCase<'a, T, S, P, R, N, C>
where
    T: PaymentTransactionRepository,
    S: SubscriptionRepository,
    P: PaymentCachePort,
    R: ReferralRepository,
    N: Notifier,
    C: Clock,
{
    pub const fn new(
        transactions: &'a T,
        subscriptions: &'a S,
        payment_cache: &'a P,
        referrals: &'a R,
        notifier: &'a N,
        clock: &'a C,
    ) -> Self {
        Self {
            transactions,
            subscriptions,
            payment_cache,
            referrals,
            notifier,
            clock,
        }
    }

    pub async fn execute(
        &self,
        payment_id: &PaymentId,
        status: PaymentStatus,
    ) -> ApplicationResult<SubscriptionPaymentStatus> {
        self.execute_with_provider_payment_id(payment_id, None, status)
            .await
    }

    pub async fn execute_with_provider_payment_id(
        &self,
        payment_id: &PaymentId,
        provider_payment_id: Option<&str>,
        status: PaymentStatus,
    ) -> ApplicationResult<SubscriptionPaymentStatus> {
        let mut transaction = self
            .load_transaction(payment_id, provider_payment_id)
            .await?;

        if transaction.provider_payment_id.is_none() {
            if let Some(provider_payment_id) = provider_payment_id {
                transaction.set_provider_payment_id(provider_payment_id);
            }
        }

        transaction.update_status(status.clone(), self.clock.now());
        self.transactions
            .save_payment_transaction(&transaction)
            .await?;

        let event = payment_status_event(&status);
        if !self
            .payment_cache
            .notify_once(
                &transaction.payment_id,
                event,
                self.clock.now() + Duration::hours(24),
            )
            .await?
        {
            return Ok(SubscriptionPaymentStatus {
                transaction,
                subscription: None,
            });
        }

        match status {
            PaymentStatus::Succeeded => self.fulfill_succeeded(transaction).await,
            PaymentStatus::Canceled | PaymentStatus::Failed => {
                self.payment_cache
                    .delete_pending_payment(&transaction.payment_id)
                    .await?;
                self.notify_payment_canceled(transaction.user_id).await?;
                Ok(SubscriptionPaymentStatus {
                    transaction,
                    subscription: None,
                })
            }
            PaymentStatus::WaitingForCapture => {
                self.notify_payment_waiting(transaction.user_id).await?;
                Ok(SubscriptionPaymentStatus {
                    transaction,
                    subscription: None,
                })
            }
            PaymentStatus::Pending | PaymentStatus::Unknown(_) => Ok(SubscriptionPaymentStatus {
                transaction,
                subscription: None,
            }),
        }
    }

    async fn load_transaction(
        &self,
        payment_id: &PaymentId,
        provider_payment_id: Option<&str>,
    ) -> ApplicationResult<PaymentTransaction> {
        if let Some(transaction) = self
            .transactions
            .find_payment_transaction(payment_id)
            .await?
        {
            return Ok(transaction);
        }

        if let Some(provider_payment_id) = provider_payment_id {
            if let Some(transaction) = self
                .transactions
                .find_payment_transaction_by_provider_payment_id(provider_payment_id)
                .await?
            {
                return Ok(transaction);
            }
        }

        Err(ApplicationError::NotFound {
            entity: "payment_transaction",
            id: payment_id.to_string(),
        })
    }

    async fn fulfill_succeeded(
        &self,
        transaction: PaymentTransaction,
    ) -> ApplicationResult<SubscriptionPaymentStatus> {
        let lock_until = self.clock.now() + Duration::minutes(10);
        if !self
            .payment_cache
            .try_acquire_fulfill_lock(&transaction.payment_id, lock_until)
            .await?
        {
            return Ok(SubscriptionPaymentStatus {
                transaction,
                subscription: None,
            });
        }

        let payment_id = transaction.payment_id.clone();
        let result = self.fulfill_succeeded_locked(transaction).await;
        let _ = self.payment_cache.release_fulfill_lock(&payment_id).await;

        result
    }

    async fn fulfill_succeeded_locked(
        &self,
        mut transaction: PaymentTransaction,
    ) -> ApplicationResult<SubscriptionPaymentStatus> {
        self.payment_cache
            .delete_pending_payment(&transaction.payment_id)
            .await?;

        if transaction.fulfilled {
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

        self.notify_payment_succeeded(transaction.user_id, &subscription)
            .await?;
        self.reward_referrer(transaction.user_id).await?;

        Ok(SubscriptionPaymentStatus {
            transaction,
            subscription: Some(subscription),
        })
    }

    async fn reward_referrer(&self, invited_id: UserId) -> ApplicationResult<()> {
        let Some(mut referral) = self.referrals.find_referral_by_invited(invited_id).await? else {
            return Ok(());
        };

        if referral.is_rewarded() {
            return Ok(());
        }

        let now = self.clock.now();
        let referrer_chat_id = ChatId::new(referral.referrer_id.value());
        let mut subscription = self
            .subscriptions
            .find_subscription(referrer_chat_id)
            .await?
            .unwrap_or_else(|| {
                Subscription::new_trial(referrer_chat_id, now, SubscriptionPolicy { trial_days: 0 })
            });
        subscription.link_user(referral.referrer_id);
        subscription.extend(Months::new(1)?, now);
        self.subscriptions.save_subscription(&subscription).await?;

        referral.mark_rewarded(now);
        self.referrals.save_referral(&referral).await?;

        self.notifier
            .notify(Notification::Text {
                chat_id: referrer_chat_id,
                text: format!(
                    "🎁 Бонус по реферальной программе!\n\nВаш друг оформил подписку, и вы получили +1 месяц бесплатно.\n\nПодписка активна до {}.",
                    subscription.expires_at.format("%d.%m.%Y")
                ),
            })
            .await?;
        Ok(())
    }

    async fn notify_payment_succeeded(
        &self,
        user_id: UserId,
        subscription: &Subscription,
    ) -> ApplicationResult<()> {
        self.notifier
            .notify(Notification::Text {
                chat_id: ChatId::new(user_id.value()),
                text: format!(
                    "🎉 Спасибо за покупку подписки!\n\nСтатус: активна\nДействует до: {}\n\nЕсли что-то непонятно — воспользуйтесь командой /help.",
                    subscription.expires_at.format("%d.%m.%Y")
                ),
            })
            .await
    }

    async fn notify_payment_canceled(&self, user_id: UserId) -> ApplicationResult<()> {
        self.notifier
            .notify(Notification::Text {
                chat_id: ChatId::new(user_id.value()),
                text: "❌ Оплата отменена.".to_string(),
            })
            .await
    }

    async fn notify_payment_waiting(&self, user_id: UserId) -> ApplicationResult<()> {
        self.notifier
            .notify(Notification::Text {
                chat_id: ChatId::new(user_id.value()),
                text: "⏳ Платёж ожидает подтверждения...".to_string(),
            })
            .await
    }
}

fn payment_status_event(status: &PaymentStatus) -> &'static str {
    match status {
        PaymentStatus::Succeeded => "succeeded",
        PaymentStatus::WaitingForCapture => "waiting",
        PaymentStatus::Canceled => "canceled",
        PaymentStatus::Failed => "failed",
        PaymentStatus::Pending => "pending",
        PaymentStatus::Unknown(_) => "unknown",
    }
}
