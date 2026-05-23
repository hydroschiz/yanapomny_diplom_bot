use application::{ApplicationError, ApplicationResult, PaymentGateway, PaymentGatewayPort};
use async_trait::async_trait;
use domain::{Currency, Money, Payment, PaymentTransaction};
use serde::Deserialize;

const YOOKASSA_CREATE_PAYMENT_URL: &str = "https://api.yookassa.ru/v3/payments";

#[derive(Clone)]
pub struct HttpYooKassaPaymentGateway {
    client: reqwest::Client,
    shop_id: String,
    secret_key: String,
    return_url: String,
}

impl HttpYooKassaPaymentGateway {
    pub fn new(
        shop_id: impl Into<String>,
        secret_key: impl Into<String>,
        return_url: impl Into<String>,
    ) -> Self {
        Self {
            client: reqwest::Client::new(),
            shop_id: shop_id.into(),
            secret_key: secret_key.into(),
            return_url: return_url.into(),
        }
    }

    async fn create_redirect_payment(
        &self,
        amount: Money,
        idempotence_key: &str,
        metadata: serde_json::Value,
        description: Option<String>,
    ) -> ApplicationResult<String> {
        let amount = serde_json::json!({
            "value": format!("{}.00", amount.amount),
            "currency": currency_code(amount.currency),
        });
        let mut request = serde_json::json!({
            "amount": amount,
            "capture": true,
            "confirmation": {
                "type": "redirect",
                "return_url": self.return_url,
            },
            "metadata": metadata,
        });
        if let (Some(description), Some(request)) = (description, request.as_object_mut()) {
            request.insert(
                "description".to_string(),
                serde_json::Value::String(description),
            );
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
            payment.id.as_str(),
            serde_json::json!({
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
        let mut metadata = serde_json::json!({
            "payment_id": transaction.payment_id.as_str(),
            "user_id": transaction.user_id.to_string(),
        });
        if let (Some(months), Some(metadata)) = (transaction.months, metadata.as_object_mut()) {
            metadata.insert(
                "months".to_string(),
                serde_json::Value::String(months.to_string()),
            );
        }
        let description = transaction
            .months
            .map(|months| format!("Подписка на {} мес.", months));

        self.create_redirect_payment(
            transaction.amount,
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
