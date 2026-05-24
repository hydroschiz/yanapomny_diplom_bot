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

pub struct ListExternalChannelSubscriptionsUseCase<'a, R> {
    subscriptions: &'a R,
}

impl<'a, R> ListExternalChannelSubscriptionsUseCase<'a, R>
where
    R: ExternalChannelSubscriptionRepository,
{
    pub const fn new(subscriptions: &'a R) -> Self {
        Self { subscriptions }
    }

    pub async fn execute(
        &self,
        user_id: UserId,
    ) -> ApplicationResult<Vec<ExternalChannelSubscription>> {
        Ok(numbered_external_channel_subscriptions(
            self.subscriptions
                .list_external_channel_subscriptions(user_id)
                .await?,
        ))
    }
}

pub struct CheckAllTwitchStreamsUseCase<'a, R, G> {
    subscriptions: &'a R,
    gateway: &'a G,
}

impl<'a, R, G> CheckAllTwitchStreamsUseCase<'a, R, G>
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

    pub async fn execute(&self) -> ApplicationResult<Vec<ExternalChannelSubscription>> {
        let mut changed = Vec::new();
        for mut subscription in self
            .subscriptions
            .list_all_external_channel_subscriptions()
            .await?
            .into_iter()
            .filter(|subscription| subscription.platform == Platform::Twitch)
        {
            let latest = self.gateway.latest_content_id(&subscription).await?;
            let is_live = latest.is_some();
            if latest != subscription.last_content_id || is_live != subscription.is_live {
                if let Some(content_id) = latest {
                    subscription.mark_content_seen(content_id);
                    subscription.set_live(true);
                    self.subscriptions
                        .save_external_channel_subscription(&subscription)
                        .await?;
                    changed.push(subscription);
                } else {
                    subscription.last_content_id = None;
                    subscription.set_live(false);
                    self.subscriptions
                        .save_external_channel_subscription(&subscription)
                        .await?;
                }
            }
        }
        Ok(changed)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeleteExternalChannelSubscriptionCommand {
    pub user_id: UserId,
    pub sub_num: i32,
}

pub struct DeleteExternalChannelSubscriptionUseCase<'a, R> {
    subscriptions: &'a R,
}

impl<'a, R> DeleteExternalChannelSubscriptionUseCase<'a, R>
where
    R: ExternalChannelSubscriptionRepository,
{
    pub const fn new(subscriptions: &'a R) -> Self {
        Self { subscriptions }
    }

    pub async fn execute(
        &self,
        command: DeleteExternalChannelSubscriptionCommand,
    ) -> ApplicationResult<Option<ExternalChannelSubscription>> {
        let subscription = numbered_external_channel_subscriptions(
            self.subscriptions
                .list_external_channel_subscriptions(command.user_id)
                .await?,
        )
        .into_iter()
        .find(|subscription| subscription.sub_num == command.sub_num);

        if let Some(subscription) = subscription.as_ref() {
            self.subscriptions
                .delete_external_channel_subscription(subscription)
                .await?;
        }

        Ok(subscription)
    }
}

fn numbered_external_channel_subscriptions(
    mut subscriptions: Vec<ExternalChannelSubscription>,
) -> Vec<ExternalChannelSubscription> {
    subscriptions.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.channel_id.cmp(&right.channel_id))
    });
    for (index, subscription) in subscriptions.iter_mut().enumerate() {
        subscription.sub_num = (index + 1) as i32;
    }
    subscriptions
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
