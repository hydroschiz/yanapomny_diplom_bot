use std::fmt;

use crate::DomainError;

macro_rules! id_type {
    ($name:ident, $inner:ty) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name($inner);

        impl $name {
            pub const fn new(value: $inner) -> Self {
                Self(value)
            }

            pub const fn value(self) -> $inner {
                self.0
            }
        }

        impl From<$inner> for $name {
            fn from(value: $inner) -> Self {
                Self::new(value)
            }
        }

        impl From<$name> for $inner {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

id_type!(UserId, i64);
id_type!(ChatId, i64);
id_type!(TaskId, i64);
id_type!(ReminderId, i32);
id_type!(DeliveryEventId, i64);
id_type!(SubscriptionId, i64);
id_type!(ExternalChannelSubscriptionId, i64);
id_type!(IntentLogId, i64);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PaymentId(String);

impl PaymentId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Display for PaymentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for PaymentId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl From<&str> for PaymentId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Months(u8);

impl Months {
    pub const THREE: Self = Self(3);
    pub const SIX: Self = Self(6);
    pub const TWELVE: Self = Self(12);

    pub fn new(value: i32) -> Result<Self, DomainError> {
        if !(1..=120).contains(&value) {
            return Err(DomainError::InvalidMonths(value));
        }
        Ok(Self(value as u8))
    }

    pub const fn value(self) -> u8 {
        self.0
    }
}

impl fmt::Display for Months {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
