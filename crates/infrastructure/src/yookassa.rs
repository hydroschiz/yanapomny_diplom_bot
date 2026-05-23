use application::{ApplicationError, ApplicationResult, PaymentGateway, PaymentGatewayPort};
use async_trait::async_trait;
use domain::{Currency, Money, Payment, PaymentTransaction, UserId};
use serde::Deserialize;
use serde_json::{json, Value};

const YOOKASSA_CREATE_PAYMENT_URL: &str = "https://api.yookassa.ru/v3/payments";

/// Fiscalization receipt configuration as required by YooKassa for shops with
/// receipts enabled. Mirrors legacy `src/api/payments.rs::init_payment`
/// behaviour driven by `YK_*` environment variables.
#[derive(Debug, Clone)]
pub struct YooKassaReceiptConfig {
    pub vat_code: u8,
    pub payment_subject: String,
    pub payment_mode: String,
    pub tax_system_code: Option<u8>,
    pub email_suffix: String,
}

impl YooKassaReceiptConfig {
    pub fn new(
        vat_code: u8,
        payment_subject: impl Into<String>,
        payment_mode: impl Into<String>,
        tax_system_code: Option<u8>,
        email_suffix: impl Into<String>,
    ) -> Self {
        Self {
            vat_code,
            payment_subject: payment_subject.into(),
            payment_mode: payment_mode.into(),
            tax_system_code,
            email_suffix: email_suffix.into(),
        }
    }

    fn customer_email(&self, user_id: UserId) -> String {
        format!("vk-{}@{}", user_id.value(), self.email_suffix)
    }

    fn build(&self, user_id: UserId, amount: &Value, description: &str) -> Value {
        let mut item = json!({
            "description": description,
            "quantity": "1.00",
            "amount": amount,
            "vat_code": self.vat_code,
            "payment_subject": self.payment_subject,
            "payment_mode": self.payment_mode,
        });
        if let Some(item) = item.as_object_mut() {
            item.retain(|_, value| !value.is_null());
        }

        let mut receipt = json!({
            "customer": {
                "email": self.customer_email(user_id),
            },
            "items": [item],
        });
        if let (Some(tax_system_code), Some(receipt)) =
            (self.tax_system_code, receipt.as_object_mut())
        {
            receipt.insert("tax_system_code".to_string(), json!(tax_system_code));
        }
        receipt
    }
}

#[derive(Clone)]
pub struct HttpYooKassaPaymentGateway {
    client: reqwest::Client,
    shop_id: String,
    secret_key: String,
    return_url: String,
    receipt: Option<YooKassaReceiptConfig>,
}

impl HttpYooKassaPaymentGateway {
    pub fn new(
        shop_id: impl Into<String>,
        secret_key: impl Into<String>,
        return_url: impl Into<String>,
        receipt: Option<YooKassaReceiptConfig>,
    ) -> Self {
        Self {
            client: reqwest::Client::new(),
            shop_id: shop_id.into(),
            secret_key: secret_key.into(),
            return_url: return_url.into(),
            receipt,
        }
    }

    async fn create_redirect_payment(
        &self,
        amount: Money,
        user_id: Option<UserId>,
        idempotence_key: &str,
        metadata: Value,
        description: Option<String>,
    ) -> ApplicationResult<String> {
        let amount_value = json!({
            "value": format!("{}.00", amount.amount),
            "currency": currency_code(amount.currency),
        });
        let mut request = json!({
            "amount": amount_value,
            "capture": true,
            "confirmation": {
                "type": "redirect",
                "return_url": self.return_url,
            },
            "metadata": metadata,
        });
        if let Some(request) = request.as_object_mut() {
            if let Some(description) = description.as_deref() {
                request.insert("description".to_string(), Value::String(description.into()));
            }
            if let (Some(receipt), Some(user_id), Some(description)) =
                (self.receipt.as_ref(), user_id, description.as_deref())
            {
                let receipt = receipt.build(user_id, &amount_value, description);
                request.insert("receipt".to_string(), receipt);
            }
        }

        let response = self
            .client
            .post(YOOKASSA_CREATE_PAYMENT_URL)
            .basic_auth(&self.shop_id, Some(&self.secret_key))
            .header("Idempotence-Key", idempotence_key)
            .json(&request)
            .send()
            .await
            .map_err(|err| ApplicationError::ExternalService(err.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(ApplicationError::ExternalService(format!(
                "YooKassa create payment failed: {status} {body}"
            )));
        }

        let response: CreatePaymentResponse = response
            .json()
            .await
            .map_err(|err| ApplicationError::ExternalService(err.to_string()))?;
        response.confirmation.confirmation_url.ok_or_else(|| {
            ApplicationError::ExternalService(
                "YooKassa response has no confirmation_url".to_string(),
            )
        })
    }
}

#[async_trait]
impl PaymentGateway for HttpYooKassaPaymentGateway {
    async fn create_payment(&self, payment: &Payment) -> ApplicationResult<String> {
        self.create_redirect_payment(
            payment.amount,
            None,
            payment.id.as_str(),
            json!({
                "payment_id": payment.id.as_str(),
            }),
            None,
        )
        .await
    }
}

#[async_trait]
impl PaymentGatewayPort for HttpYooKassaPaymentGateway {
    async fn create_payment(&self, transaction: &PaymentTransaction) -> ApplicationResult<String> {
        let mut metadata = json!({
            "payment_id": transaction.payment_id.as_str(),
            "user_id": transaction.user_id.to_string(),
        });
        if let (Some(months), Some(metadata)) = (transaction.months, metadata.as_object_mut()) {
            metadata.insert("months".to_string(), Value::String(months.to_string()));
        }
        let description = transaction
            .months
            .map(|months| format!("Подписка на {} мес.", months));

        self.create_redirect_payment(
            transaction.amount,
            Some(transaction.user_id),
            transaction
                .idempotence_key
                .as_deref()
                .unwrap_or(transaction.payment_id.as_str()),
            metadata,
            description,
        )
        .await
    }
}

#[derive(Debug, Deserialize)]
struct CreatePaymentResponse {
    confirmation: ConfirmationResponse,
}

#[derive(Debug, Deserialize)]
struct ConfirmationResponse {
    confirmation_url: Option<String>,
}

fn currency_code(value: Currency) -> &'static str {
    match value {
        Currency::Rub => "RUB",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::UserId;

    #[test]
    fn receipt_includes_legacy_required_fields() {
        let receipt =
            YooKassaReceiptConfig::new(1, "service", "full_payment", Some(1), "yanapomnyu.ru");
        let amount = json!({ "value": "195.00", "currency": "RUB" });

        let value = receipt.build(UserId::new(42), &amount, "Подписка на 3 мес.");

        assert_eq!(
            value
                .get("customer")
                .and_then(|customer| customer.get("email"))
                .and_then(|email| email.as_str()),
            Some("vk-42@yanapomnyu.ru"),
        );
        assert_eq!(value.get("tax_system_code"), Some(&json!(1)));

        let items = value.get("items").and_then(Value::as_array).unwrap();
        assert_eq!(items.len(), 1);
        let item = &items[0];
        assert_eq!(
            item.get("description").and_then(Value::as_str),
            Some("Подписка на 3 мес.")
        );
        assert_eq!(item.get("quantity").and_then(Value::as_str), Some("1.00"));
        assert_eq!(item.get("vat_code"), Some(&json!(1)));
        assert_eq!(
            item.get("payment_subject").and_then(Value::as_str),
            Some("service")
        );
        assert_eq!(
            item.get("payment_mode").and_then(Value::as_str),
            Some("full_payment")
        );
        assert_eq!(item.get("amount"), Some(&amount));
    }

    #[test]
    fn receipt_omits_tax_system_code_when_not_set() {
        let receipt =
            YooKassaReceiptConfig::new(1, "service", "full_payment", None, "yanapomnyu.ru");
        let amount = json!({ "value": "195.00", "currency": "RUB" });

        let value = receipt.build(UserId::new(7), &amount, "Подписка на 3 мес.");

        assert!(value.get("tax_system_code").is_none());
    }
}
