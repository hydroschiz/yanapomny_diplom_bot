use domain::{ExternalChannelSubscription, UserId};

use crate::{ApplicationResult, ExternalChannelSubscriptionRepository, StreamPlatformGateway};

pub struct CheckTwitchStreamsUseCase<'a, R, G> {
    subscriptions: &'a R,
    gateway: &'a G,
}

impl<'a, R, G> CheckTwitchStreamsUseCase<'a, R, G>
where
    R: ExternalChannelSubscriptionRepository,
    G: StreamPlatformGateway,
{
    pub const fn new(subscriptions: &'a R, gateway: &'a G) -> Self {
        Self {
            subscriptions,
            gateway,
        }
    }

    pub async fn execute(
        &self,
        user_id: UserId,
    ) -> ApplicationResult<Vec<ExternalChannelSubscription>> {
        let mut changed = Vec::new();
        for mut subscription in self
            .subscriptions
            .list_external_channel_subscriptions(user_id)
            .await?
        {
            let latest = self.gateway.latest_content_id(&subscription).await?;
            if latest != subscription.last_content_id {
                if let Some(content_id) = latest {
                    subscription.mark_content_seen(content_id);
                    self.subscriptions
                        .save_external_channel_subscription(&subscription)
                        .await?;
                    changed.push(subscription);
                }
            }
        }
        Ok(changed)
    }
}
