#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCommand {
    pub command: BotCommand,
    pub args: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BotCommand {
    Start,
    Help,
    Yan,
    Utc,
    Setup,
    Pay,
    List,
    Subs,
    Profile,
    Ref,
    Remind(String),
    Unknown(String),
}

pub fn parse_command(text: &str) -> Option<ParsedCommand> {
    let text = text.trim();
    if !text.starts_with('/') {
        return None;
    }

    let mut parts = text.splitn(2, char::is_whitespace);
    let raw = parts.next().unwrap_or_default();
    let args = parts.next().unwrap_or_default().trim().to_string();
    let name = raw
        .trim_start_matches('/')
        .split('@')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();

    if name.is_empty() {
        return None;
    }

    let command = match name.as_str() {
        "start" => BotCommand::Start,
        "help" => BotCommand::Help,
        "yan" => BotCommand::Yan,
        "utc" => BotCommand::Utc,
        "setup" => BotCommand::Setup,
        "pay" => BotCommand::Pay,
        "list" => BotCommand::List,
        "subs" => BotCommand::Subs,
        "profile" => BotCommand::Profile,
        "ref" => BotCommand::Ref,
        "remind" => BotCommand::Remind(args.clone()),
        _ => BotCommand::Unknown(name),
    };

    Some(ParsedCommand { command, args })
}
