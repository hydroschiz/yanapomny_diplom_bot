use domain::{ChatId, Subscription, SubscriptionPolicy};

use crate::{ApplicationResult, Clock, SubscriptionRepository};

pub struct EnsureSubscriptionUseCase<'a, R, C> {
    subscriptions: &'a R,
    clock: &'a C,
    policy: SubscriptionPolicy,
}

impl<'a, R, C> EnsureSubscriptionUseCase<'a, R, C>
where
    R: SubscriptionRepository,
    C: Clock,
{
    pub const fn new(subscriptions: &'a R, clock: &'a C, policy: SubscriptionPolicy) -> Self {
        Self {
            subscriptions,
            clock,
            policy,
        }
    }

    pub async fn execute(&self, chat_id: ChatId) -> ApplicationResult<Subscription> {
        if let Some(subscription) = self.subscriptions.find_subscription(chat_id).await? {
            return Ok(subscription);
        }

        let subscription = Subscription::new_trial(chat_id, self.clock.now(), self.policy);
        self.subscriptions.save_subscription(&subscription).await?;
        Ok(subscription)
    }
}
