use std::fmt;

use chrono::{DateTime, Utc};

use crate::{DomainError, Months, PaymentId, SubscriptionId, UserId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Currency {
    Rub,
}

impl fmt::Display for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rub => f.write_str("RUB"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Money {
    pub amount: i64,
    pub currency: Currency,
}

impl Money {
    pub const fn rub(amount: i64) -> Self {
        Self {
            amount,
            currency: Currency::Rub,
        }
    }

    pub fn new(amount: i64, currency: Currency) -> Result<Self, DomainError> {
        if amount < 0 {
            return Err(DomainError::InvalidMoneyAmount(amount));
        }
        Ok(Self { amount, currency })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tariff {
    pub months: Months,
    pub price: Money,
    pub price_per_month: Money,
}

pub const TARIFFS: &[Tariff] = &[
    Tariff {
        months: Months::THREE,
        price: Money::rub(195),
        price_per_month: Money::rub(65),
    },
    Tariff {
        months: Months::SIX,
        price: Money::rub(360),
        price_per_month: Money::rub(60),
    },
    Tariff {
        months: Months::TWELVE,
        price: Money::rub(660),
        price_per_month: Money::rub(55),
    },
];

pub fn tariff_for_months(months: Months) -> Option<&'static Tariff> {
    TARIFFS.iter().find(|tariff| tariff.months == months)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaymentStatus {
    Pending,
    WaitingForCapture,
    Succeeded,
    Canceled,
    Failed,
    Unknown(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaymentProvider {
    YooKassa,
}

impl fmt::Display for PaymentProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::YooKassa => f.write_str("yookassa"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Payment {
    pub id: PaymentId,
    pub subscription_id: Option<SubscriptionId>,
    pub provider: PaymentProvider,
    pub provider_payment_id: Option<String>,
    pub amount: Money,
    pub status: PaymentStatus,
    pub created_at: DateTime<Utc>,
}

impl Payment {
    pub fn new(
        id: PaymentId,
        provider: PaymentProvider,
        amount: Money,
        created_at: DateTime<Utc>,
    ) -> Self {
        Self {
            id,
            subscription_id: None,
            provider,
            provider_payment_id: None,
            amount,
            status: PaymentStatus::Pending,
            created_at,
        }
    }

    pub fn link_subscription(&mut self, subscription_id: SubscriptionId) {
        self.subscription_id = Some(subscription_id);
    }

    pub fn set_provider_payment_id(&mut self, value: impl Into<String>) {
        self.provider_payment_id = Some(value.into());
    }

    pub fn update_status(&mut self, status: PaymentStatus) {
        self.status = status;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaymentTransaction {
    pub payment_id: PaymentId,
    pub user_id: UserId,
    pub amount: Money,
    pub months: Option<Months>,
    pub status: PaymentStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub fulfilled: bool,
    pub fulfilled_at: Option<DateTime<Utc>>,
    pub idempotence_key: Option<String>,
    pub provider: Option<String>,
    /// YooKassa's internal payment ID (separate from our generated payment_id)
    pub provider_payment_id: Option<String>,
}

impl PaymentTransaction {
    pub fn new(
        payment_id: PaymentId,
        user_id: UserId,
        amount: Money,
        months: Option<Months>,
        now: DateTime<Utc>,
    ) -> Self {
        Self {
            payment_id,
            user_id,
            amount,
            months,
            status: PaymentStatus::Pending,
            created_at: now,
            updated_at: now,
            fulfilled: false,
            fulfilled_at: None,
            idempotence_key: None,
            provider: None,
            provider_payment_id: None,
        }
    }

    pub fn set_provider_payment_id(&mut self, id: impl Into<String>) {
        self.provider_payment_id = Some(id.into());
    }

    pub fn update_status(&mut self, status: PaymentStatus, now: DateTime<Utc>) {
        self.status = status;
        self.updated_at = now;
    }

    pub fn mark_fulfilled(&mut self, now: DateTime<Utc>) {
        self.fulfilled = true;
        self.fulfilled_at = Some(now);
        self.updated_at = now;
    }
}
