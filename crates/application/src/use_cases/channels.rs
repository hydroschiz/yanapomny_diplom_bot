use domain::{ExternalChannelSubscription, Platform, UserId};

use crate::{
    ApplicationResult, Clock, ExternalChannelSubscriptionRepository, StreamPlatformGateway,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveExternalChannelSubscriptionCommand {
    pub user_id: UserId,
    pub platform: Platform,
    pub channel_id: String,
    pub channel_name: String,
    pub url: String,
}

pub struct SaveExternalChannelSubscriptionUseCase<'a, R, C> {
    subscriptions: &'a R,
    clock: &'a C,
}

impl<'a, R, C> SaveExternalChannelSubscriptionUseCase<'a, R, C>
where
    R: ExternalChannelSubscriptionRepository,
    C: Clock,
{
    pub const fn new(subscriptions: &'a R, clock: &'a C) -> Self {
        Self {
            subscriptions,
            clock,
        }
    }

    pub async fn execute(
        &self,
        command: SaveExternalChannelSubscriptionCommand,
    ) -> ApplicationResult<ExternalChannelSubscription> {
        let subscriptions = self
            .subscriptions
            .list_external_channel_subscriptions(command.user_id)
            .await?;

        if let Some(mut subscription) = subscriptions
            .iter()
            .find(|subscription| {
                subscription.platform == command.platform
                    && subscription.channel_id == command.channel_id
            })
            .cloned()
        {
            subscription.channel_name = command.channel_name;
            subscription.url = command.url;
            self.subscriptions
                .save_external_channel_subscription(&subscription)
                .await?;
            return Ok(subscription);
        }

        let sub_num = subscriptions
            .iter()
            .map(|subscription| subscription.sub_num)
            .max()
            .unwrap_or(0)
            + 1;
        let subscription = ExternalChannelSubscription::new(
            command.user_id,
            command.platform,
            command.channel_id,
            command.channel_name,
            command.url,
            sub_num,
            self.clock.now(),
        );
        self.subscriptions
            .save_external_channel_subscription(&subscription)
            .await?;
        Ok(subscription)
    }
}

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
