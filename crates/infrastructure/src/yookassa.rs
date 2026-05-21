use application::{ApplicationError, ApplicationResult, PaymentGateway};
use async_trait::async_trait;
use domain::{Currency, Payment};
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
}

#[async_trait]
impl PaymentGateway for HttpYooKassaPaymentGateway {
    async fn create_payment(&self, payment: &Payment) -> ApplicationResult<String> {
        let amount = serde_json::json!({
            "value": format!("{}.00", payment.amount.amount),
            "currency": currency_code(payment.amount.currency),
        });
        let request = serde_json::json!({
            "amount": amount,
            "capture": true,
            "confirmation": {
                "type": "redirect",
                "return_url": self.return_url,
            },
            "metadata": {
                "payment_id": payment.id.as_str(),
            },
        });

        let response = self
            .client
            .post(YOOKASSA_CREATE_PAYMENT_URL)
            .basic_auth(&self.shop_id, Some(&self.secret_key))
            .header("Idempotence-Key", payment.id.as_str())
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
