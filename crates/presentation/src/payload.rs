#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallbackPayload {
    SetupMenu,
    SetupSnooze,
    SetupAuto,
    SetupUtc,
    UtcCancel,
    UtcPage(usize),
    UtcSet(String),
    PayMenu,
    PayCancel,
    PaySelect(i32),
    PayYooKassa(i32),
    PayCheck(String),
    TextConfirm,
    TextCancel,
    ReminderConfirm,
    ReminderEdit,
    ReminderCancel,
    ReminderDeleteStart,
    ReminderDeleteBack,
    ReminderList,
    Snooze { reminder_id: i32, code: String },
    ReminderDone(i32),
    SubDelete,
    Subs,
    Profile,
    ProfileList,
    ProfileSetup,
    ProfileSubs,
    ProfileReferral,
    ProfilePay,
    BackMain,
    Unknown(String),
}

pub fn parse_payload(payload: &str) -> CallbackPayload {
    match payload {
        "setup_menu" => CallbackPayload::SetupMenu,
        "setup_snooze" => CallbackPayload::SetupSnooze,
        "setup_auto" => CallbackPayload::SetupAuto,
        "setup_utc" => CallbackPayload::SetupUtc,
        "utc_cancel" => CallbackPayload::UtcCancel,
        "pay_menu" => CallbackPayload::PayMenu,
        "pay_cancel" => CallbackPayload::PayCancel,
        "text_confirm" => CallbackPayload::TextConfirm,
        "text_cancel" => CallbackPayload::TextCancel,
        "reminder_confirm" => CallbackPayload::ReminderConfirm,
        "reminder_edit" => CallbackPayload::ReminderEdit,
        "reminder_cancel" => CallbackPayload::ReminderCancel,
        "reminder_delete_start" => CallbackPayload::ReminderDeleteStart,
        "reminder_delete_back" => CallbackPayload::ReminderDeleteBack,
        "reminder_list" => CallbackPayload::ReminderList,
        "sub_delete" => CallbackPayload::SubDelete,
        "subs" => CallbackPayload::Subs,
        "profile" | "profile_stub" => CallbackPayload::Profile,
        "profile_list" => CallbackPayload::ProfileList,
        "profile_setup" => CallbackPayload::ProfileSetup,
        "profile_subs" => CallbackPayload::ProfileSubs,
        "profile_referral" => CallbackPayload::ProfileReferral,
        "profile_pay" => CallbackPayload::ProfilePay,
        "back_main" => CallbackPayload::BackMain,
        _ => parse_prefixed_payload(payload),
    }
}

fn parse_prefixed_payload(payload: &str) -> CallbackPayload {
    if let Some(rest) = payload.strip_prefix("utc_page:") {
        if let Ok(page) = rest.parse() {
            return CallbackPayload::UtcPage(page);
        }
    }

    if let Some(rest) = payload.strip_prefix("utc_set:") {
        if !rest.trim().is_empty() {
            return CallbackPayload::UtcSet(rest.trim().to_string());
        }
    }

    if let Some(rest) = payload.strip_prefix("pay_select:") {
        if let Ok(months) = rest.parse() {
            return CallbackPayload::PaySelect(months);
        }
    }

    if let Some(rest) = payload.strip_prefix("pay_yk:") {
        if let Ok(months) = rest.parse() {
            return CallbackPayload::PayYooKassa(months);
        }
    }

    if let Some(rest) = payload.strip_prefix("pay_check:") {
        let payment_id = rest.trim();
        if !payment_id.is_empty() {
            return CallbackPayload::PayCheck(payment_id.to_string());
        }
    }

    if let Some(rest) = payload.strip_prefix("snooze:") {
        if let Some((id, code)) = rest.split_once(':') {
            if let Ok(reminder_id) = id.parse() {
                return CallbackPayload::Snooze {
                    reminder_id,
                    code: code.to_string(),
                };
            }
        }
    }

    if let Some(rest) = payload.strip_prefix("reminder_done:") {
        if let Ok(reminder_id) = rest.parse() {
            return CallbackPayload::ReminderDone(reminder_id);
        }
    }

    CallbackPayload::Unknown(payload.to_string())
}
