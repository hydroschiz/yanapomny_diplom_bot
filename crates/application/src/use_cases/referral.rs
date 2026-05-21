use domain::{Referral, UserId};

use crate::{ApplicationResult, Clock, ReferralRepository};

pub struct CreateReferralUseCase<'a, R, C> {
    referrals: &'a R,
    clock: &'a C,
}

impl<'a, R, C> CreateReferralUseCase<'a, R, C>
where
    R: ReferralRepository,
    C: Clock,
{
    pub const fn new(referrals: &'a R, clock: &'a C) -> Self {
        Self { referrals, clock }
    }

    pub async fn execute(
        &self,
        referrer_id: UserId,
        invited_id: UserId,
    ) -> ApplicationResult<Referral> {
        if let Some(referral) = self
            .referrals
            .find_referral(referrer_id, invited_id)
            .await?
        {
            return Ok(referral);
        }

        let referral = Referral::new(referrer_id, invited_id, self.clock.now());
        self.referrals.save_referral(&referral).await?;
        Ok(referral)
    }
}

pub struct ConsumeReferralRewardUseCase<'a, R, C> {
    referrals: &'a R,
    clock: &'a C,
}

impl<'a, R, C> ConsumeReferralRewardUseCase<'a, R, C>
where
    R: ReferralRepository,
    C: Clock,
{
    pub const fn new(referrals: &'a R, clock: &'a C) -> Self {
        Self { referrals, clock }
    }

    pub async fn execute(
        &self,
        referrer_id: UserId,
        invited_id: UserId,
    ) -> ApplicationResult<Option<Referral>> {
        let Some(mut referral) = self
            .referrals
            .find_referral(referrer_id, invited_id)
            .await?
        else {
            return Ok(None);
        };

        if referral.is_rewarded() {
            return Ok(None);
        }

        referral.mark_rewarded(self.clock.now());
        self.referrals.save_referral(&referral).await?;
        Ok(Some(referral))
    }
}
