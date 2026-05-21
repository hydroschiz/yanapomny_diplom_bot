use domain::{
    ChatId, SnoozeDuration, SubscriptionStatus, TimePreferences, User, UserId, UserPreferences,
};

use crate::{
    ApplicationResult, Clock, SubscriptionRepository, UserPreferencesRepository, UserRepository,
};

pub struct EnsureUserUseCase<'a, R> {
    users: &'a R,
}

impl<'a, R> EnsureUserUseCase<'a, R>
where
    R: UserRepository,
{
    pub const fn new(users: &'a R) -> Self {
        Self { users }
    }

    pub async fn execute(&self, user_id: UserId) -> ApplicationResult<User> {
        if let Some(user) = self.users.find_user(user_id).await? {
            return Ok(user);
        }

        let user = User::new(user_id);
        self.users.save_user(&user).await?;
        Ok(user)
    }
}

pub struct SetUserTimezoneUseCase<'a, R> {
    users: &'a R,
}

pub struct UpdatePreferencesUseCase<'a, R> {
    preferences: &'a R,
}

impl<'a, R> UpdatePreferencesUseCase<'a, R>
where
    R: UserPreferencesRepository,
{
    pub const fn new(preferences: &'a R) -> Self {
        Self { preferences }
    }

    pub async fn execute(
        &self,
        preferences: UserPreferences,
    ) -> ApplicationResult<UserPreferences> {
        self.preferences.save_preferences(&preferences).await?;
        Ok(preferences)
    }
}

impl<'a, R> SetUserTimezoneUseCase<'a, R>
where
    R: UserRepository,
{
    pub const fn new(users: &'a R) -> Self {
        Self { users }
    }

    pub async fn execute(
        &self,
        user_id: UserId,
        preferences: TimePreferences,
    ) -> ApplicationResult<User> {
        let mut user = self
            .users
            .find_user(user_id)
            .await?
            .unwrap_or_else(|| User::new(user_id));
        user.time_preferences = preferences;
        self.users.save_user(&user).await?;
        Ok(user)
    }
}

pub struct SetSnoozeButtonsUseCase<'a, R> {
    users: &'a R,
}

impl<'a, R> SetSnoozeButtonsUseCase<'a, R>
where
    R: UserRepository,
{
    pub const fn new(users: &'a R) -> Self {
        Self { users }
    }

    pub async fn execute(
        &self,
        user_id: UserId,
        buttons: Vec<SnoozeDuration>,
    ) -> ApplicationResult<User> {
        let mut user = self
            .users
            .find_user(user_id)
            .await?
            .unwrap_or_else(|| User::new(user_id));
        user.set_snooze_buttons(buttons);
        self.users.save_user(&user).await?;
        Ok(user)
    }
}

pub struct SetAutoSnoozeUseCase<'a, R> {
    users: &'a R,
}

impl<'a, R> SetAutoSnoozeUseCase<'a, R>
where
    R: UserRepository,
{
    pub const fn new(users: &'a R) -> Self {
        Self { users }
    }

    pub async fn execute(
        &self,
        user_id: UserId,
        auto_snooze: SnoozeDuration,
    ) -> ApplicationResult<User> {
        let mut user = self
            .users
            .find_user(user_id)
            .await?
            .unwrap_or_else(|| User::new(user_id));
        user.set_auto_snooze(auto_snooze);
        self.users.save_user(&user).await?;
        Ok(user)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileView {
    pub user: User,
    pub subscription_status: Option<SubscriptionStatus>,
}

pub struct GetProfileUseCase<'a, U, S, C> {
    users: &'a U,
    subscriptions: &'a S,
    clock: &'a C,
}

impl<'a, U, S, C> GetProfileUseCase<'a, U, S, C>
where
    U: UserRepository,
    S: SubscriptionRepository,
    C: Clock,
{
    pub const fn new(users: &'a U, subscriptions: &'a S, clock: &'a C) -> Self {
        Self {
            users,
            subscriptions,
            clock,
        }
    }

    pub async fn execute(
        &self,
        user_id: UserId,
        chat_id: ChatId,
    ) -> ApplicationResult<ProfileView> {
        let user = EnsureUserUseCase::new(self.users).execute(user_id).await?;
        let subscription_status = self
            .subscriptions
            .find_subscription(chat_id)
            .await?
            .map(|subscription| subscription.status(self.clock.now()));

        Ok(ProfileView {
            user,
            subscription_status,
        })
    }
}
