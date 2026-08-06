use serde_json::{json, Value};

/// 已识别的 Telegram 命令枚举。
#[derive(Debug, Clone, PartialEq)]
pub enum TelegramCommand {
    Start(Option<String>),
    Help,
    Wallets,
    WalletMonitor(Option<String>),
}

/// 命令回复，包含纯文本消息。
#[derive(Debug, Clone)]
pub struct CommandReply {
    pub text: String,
}

/// 解析私聊文本，提取命令。
///
/// - 支持 `/start`、`/help`，未知命令返回 `None`。
/// - 支持 `/start payload` 形式，payload 作为 `Start` 的附带参数；频道验证
///   深链由 `parse_channel_start_payload` 严格解析。
/// - 支持 `/command@bot_username` 形式；若 `@` 后用户名与 `bot_username` 不匹配，则返回 `None`。
/// - 纯函数，不访问环境变量、数据库或网络。
pub fn parse_telegram_command(text: &str, bot_username: &str) -> Option<TelegramCommand> {
    let text = text.trim();

    if !text.starts_with('/') {
        return None;
    }

    // 去掉前导 '/'
    let without_slash = &text[1..];

    // 第一步：按首个空白拆分 command token 与 payload
    let (cmd_token, payload) = if let Some(space_pos) = without_slash.find(char::is_whitespace) {
        without_slash.split_at(space_pos)
    } else {
        (without_slash, "")
    };

    let cmd_token = cmd_token.trim();

    // 拒绝空命令名（例如 "/" 或 "/  "）
    if cmd_token.is_empty() {
        return None;
    }

    // 第二步：在 cmd_token 内处理 @bot_username
    let (cmd_name, _) = if let Some(at_pos) = cmd_token.find('@') {
        let (name, suffix) = cmd_token.split_at(at_pos);
        // suffix 形如 "@bot_username"，去掉 '@' 后比较
        let bot_part = &suffix[1..]; // 去掉 '@'

        // 拒绝空 @ 后缀（例如 "/start@"）
        if bot_part.is_empty() {
            return None;
        }

        // bot_username 参数可带或不带前导 @
        let expected_bot = bot_username.trim_start_matches('@');

        if !bot_part.eq_ignore_ascii_case(expected_bot) {
            // @ 后的用户名不匹配，忽略此命令
            return None;
        }

        // 拒绝纯 @ 后缀而无命令名（例如 "/@MyTestBot"）
        let trimmed_name = name.trim();
        if trimmed_name.is_empty() {
            return None;
        }

        (trimmed_name, "")
    } else {
        (cmd_token, "")
    };

    let cmd_name = cmd_name.trim().to_lowercase();
    let payload = payload.trim();

    match cmd_name.as_str() {
        "start" => {
            let p = if payload.is_empty() {
                None
            } else {
                Some(payload.to_string())
            };
            Some(TelegramCommand::Start(p))
        }
        "help" => {
            // /help 不支持 payload，忽略其后的内容
            Some(TelegramCommand::Help)
        }
        "wallets" | "wallet" => Some(TelegramCommand::Wallets),
        "wallet_monitor" | "add_wallet" => Some(TelegramCommand::WalletMonitor(
            (!payload.is_empty()).then(|| payload.to_string()),
        )),
        _ => None,
    }
}

/// Parse the channel-verification deep-link payload used by `/start`.
///
/// The payload is intentionally strict: only `channel_<group_id>_<channel_id>`
/// is accepted, and the tenant-scoped database configuration remains the
/// authority for whether the pair is valid.
pub fn parse_channel_start_payload(payload: &str) -> Option<(i64, i64)> {
    let mut parts = payload.trim().split('_');
    if parts.next()? != "channel" {
        return None;
    }
    let group_id = parts.next()?.parse::<i64>().ok()?;
    let channel_id = parts.next()?.parse::<i64>().ok()?;
    if parts.next().is_some() || group_id == 0 || channel_id == 0 {
        return None;
    }
    Some((group_id, channel_id))
}

pub fn parse_unmute_start_payload(payload: &str) -> Option<i64> {
    let group_id = payload
        .trim()
        .strip_prefix("unmute_")?
        .parse::<i64>()
        .ok()?;
    (group_id != 0).then_some(group_id)
}

/// 根据命令和用户名字构建回复文本。
///
/// 回复使用简洁中文，仅列出已实现的 `/start` 和 `/help`，不添加按钮，不声称其他功能可用。
/// 尚未接通的深链参数会显式 fail-closed，不能退化成普通 `/start` 假成功。
pub fn build_command_reply(command: &TelegramCommand, first_name: &str) -> CommandReply {
    let text = match command {
        TelegramCommand::Start(Some(payload)) => {
            format!(
                "已识别 /start 深链参数 `{payload}`，但当前 Rust 命令 Worker 尚未接通对应业务流程，因此未执行任何操作。请使用正式 Webhook/MiniApp 入口。"
            )
        }
        TelegramCommand::Start(None) => {
            format!(
                "你好，{first_name}！欢迎使用本机器人。\n\n已实现的命令：\n/start - 开始使用机器人\n/help - 显示帮助信息\n/wallets - 查看地址监听"
            )
        }
        TelegramCommand::Help => {
            "已实现的命令：\n/start - 开始使用机器人\n/help - 显示帮助信息\n/wallets - 查看地址监听"
                .to_string()
        }
        TelegramCommand::Wallets => "正在读取当前租户的地址监听列表，请稍候。".to_string(),
        TelegramCommand::WalletMonitor(Some(address)) => {
            format!("正在登记地址 `{address}` 到当前租户的监听列表，请稍候。")
        }
        TelegramCommand::WalletMonitor(None) => {
            "请发送 `/wallet_monitor <TRC20或BSC地址>` 添加地址监听。".to_string()
        }
    };
    CommandReply { text }
}

pub fn start_menu_reply(first_name: &str) -> (String, Option<Value>) {
    (
        build_command_reply(&TelegramCommand::Start(None), first_name).text,
        Some(json!({
            "inline_keyboard": [[{
                "text": "📡 我的地址监听",
                "callback_data": "wallet_my_list"
            }]]
        })),
    )
}

// ---------------------------------------------------------------------------
// 单元测试
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    const BOT: &str = "MyTestBot";

    // ---- 普通命令 ----

    #[test]
    fn test_start_without_payload() {
        let cmd = parse_telegram_command("/start", BOT);
        assert_eq!(cmd, Some(TelegramCommand::Start(None)));
    }

    #[test]
    fn test_start_with_payload() {
        let cmd = parse_telegram_command("/start ref_abc123", BOT);
        assert_eq!(cmd, Some(TelegramCommand::Start(Some("ref_abc123".into()))));
    }

    #[test]
    fn parses_strict_channel_start_payload() {
        assert_eq!(
            parse_channel_start_payload("channel_-10001_-30001"),
            Some((-10001, -30001))
        );
        assert_eq!(parse_channel_start_payload("channel_bad_-30001"), None);
        assert_eq!(
            parse_channel_start_payload("channel_-10001_-30001_extra"),
            None
        );
        assert_eq!(parse_channel_start_payload("ref_abc123"), None);
    }

    #[test]
    fn parses_strict_unmute_start_payload() {
        assert_eq!(parse_unmute_start_payload("unmute_-10001"), Some(-10001));
        assert_eq!(parse_unmute_start_payload("unmute_10001"), Some(10001));
        assert_eq!(parse_unmute_start_payload("unmute_0"), None);
        assert_eq!(parse_unmute_start_payload("unmute_-10001_extra"), None);
        assert_eq!(parse_unmute_start_payload("channel_-10001_-30001"), None);
    }

    #[test]
    fn test_start_with_payload_extra_spaces() {
        let cmd = parse_telegram_command("/start   payload123   ", BOT);
        assert_eq!(cmd, Some(TelegramCommand::Start(Some("payload123".into()))));
    }

    #[test]
    fn test_help() {
        let cmd = parse_telegram_command("/help", BOT);
        assert_eq!(cmd, Some(TelegramCommand::Help));
    }

    #[test]
    fn test_help_with_extra_text() {
        // /help 忽略多余内容
        let cmd = parse_telegram_command("/help something", BOT);
        assert_eq!(cmd, Some(TelegramCommand::Help));
    }

    // ---- @bot 后缀匹配 ----

    #[test]
    fn test_start_with_bot_suffix_matching() {
        let cmd = parse_telegram_command("/start@MyTestBot", BOT);
        assert_eq!(cmd, Some(TelegramCommand::Start(None)));
    }

    #[test]
    fn test_start_with_bot_suffix_and_payload() {
        let cmd = parse_telegram_command("/start@MyTestBot ref_xyz", BOT);
        assert_eq!(cmd, Some(TelegramCommand::Start(Some("ref_xyz".into()))));
    }

    #[test]
    fn test_help_with_bot_suffix_matching() {
        let cmd = parse_telegram_command("/help@MyTestBot", BOT);
        assert_eq!(cmd, Some(TelegramCommand::Help));
    }

    // ---- @bot 后缀不匹配 ----

    #[test]
    fn test_start_with_wrong_bot_suffix() {
        let cmd = parse_telegram_command("/start@OtherBot", BOT);
        assert_eq!(cmd, None);
    }

    #[test]
    fn test_help_with_wrong_bot_suffix() {
        let cmd = parse_telegram_command("/help@OtherBot", BOT);
        assert_eq!(cmd, None);
    }

    #[test]
    fn test_start_with_wrong_bot_suffix_case_insensitive() {
        // 不匹配，因为 "OtherBot" != "MyTestBot"（忽略大小写比较）
        let cmd = parse_telegram_command("/start@otherbot", BOT);
        assert_eq!(cmd, None);
    }

    // ---- 未知命令 ----

    #[test]
    fn test_unknown_command() {
        let cmd = parse_telegram_command("/unknown", BOT);
        assert_eq!(cmd, None);
    }

    #[test]
    fn test_unknown_command_with_bot_suffix() {
        let cmd = parse_telegram_command("/unknown@MyTestBot", BOT);
        assert_eq!(cmd, None);
    }

    #[test]
    fn test_slash_only() {
        let cmd = parse_telegram_command("/", BOT);
        assert_eq!(cmd, None);
    }

    // ---- 非命令文本 ----

    #[test]
    fn test_plain_text_returns_none() {
        let cmd = parse_telegram_command("hello world", BOT);
        assert_eq!(cmd, None);
    }

    #[test]
    fn test_empty_string() {
        let cmd = parse_telegram_command("", BOT);
        assert_eq!(cmd, None);
    }

    // ---- build_command_reply ----

    #[test]
    fn test_start_reply_includes_greeting() {
        let reply = build_command_reply(&TelegramCommand::Start(None), "Alice");
        assert!(reply.text.contains("Alice"));
        assert!(reply.text.contains("/start"));
        assert!(reply.text.contains("/help"));
    }

    #[test]
    fn test_start_payload_fails_closed_instead_of_being_dropped() {
        let reply = build_command_reply(
            &TelegramCommand::Start(Some("unmute_-100123".into())),
            "Alice",
        );
        assert!(reply.text.contains("unmute_-100123"));
        assert!(reply.text.contains("未执行任何操作"));
        assert!(!reply.text.contains("欢迎使用"));
    }

    #[test]
    fn test_help_reply() {
        let reply = build_command_reply(&TelegramCommand::Help, "Bob");
        assert!(reply.text.contains("/start"));
        assert!(reply.text.contains("/help"));
        assert!(reply.text.contains("/wallets"));
    }

    #[test]
    fn parses_wallet_command_and_alias() {
        assert_eq!(
            parse_telegram_command("/wallets", BOT),
            Some(TelegramCommand::Wallets)
        );
        assert_eq!(
            parse_telegram_command("/wallet@MyTestBot", BOT),
            Some(TelegramCommand::Wallets)
        );
    }

    #[test]
    fn parses_wallet_monitor_command_and_alias() {
        assert_eq!(
            parse_telegram_command(
                "/wallet_monitor 0x0000000000000000000000000000000000000001",
                BOT,
            ),
            Some(TelegramCommand::WalletMonitor(Some(
                "0x0000000000000000000000000000000000000001".into(),
            )))
        );
        assert_eq!(
            parse_telegram_command("/add_wallet@MyTestBot", BOT),
            Some(TelegramCommand::WalletMonitor(None))
        );
    }

    #[test]
    fn start_menu_exposes_only_migrated_wallet_callback() {
        let (text, markup) = start_menu_reply("Alice");
        assert!(text.contains("/wallets"));
        assert_eq!(
            markup.expect("start menu markup")["inline_keyboard"][0][0]["callback_data"],
            "wallet_my_list"
        );
    }

    #[test]
    fn test_reply_no_buttons() {
        // 确认回复中不含按钮相关的标记
        let reply = build_command_reply(&TelegramCommand::Start(None), "Alice");
        assert!(!reply.text.contains("按钮"));
        assert!(!reply.text.contains("inline_keyboard"));
    }

    #[test]
    fn test_bot_suffix_matching_case_insensitive() {
        // 大小写不敏感匹配
        let cmd = parse_telegram_command("/start@mytestbot", BOT);
        assert_eq!(cmd, Some(TelegramCommand::Start(None)));
    }

    // ---- bot_username 带前导 @ ----

    #[test]
    fn test_bot_username_with_at_sign() {
        // bot_username 参数带前导 @
        let cmd = parse_telegram_command("/start@MyTestBot", "@MyTestBot");
        assert_eq!(cmd, Some(TelegramCommand::Start(None)));
    }

    #[test]
    fn test_bot_username_with_at_sign_and_payload() {
        let cmd = parse_telegram_command("/start@MyTestBot abc123", "@MyTestBot");
        assert_eq!(cmd, Some(TelegramCommand::Start(Some("abc123".into()))));
    }

    // ---- 空 @ 后缀 ----

    #[test]
    fn test_empty_at_suffix_start() {
        // "/start@" → 空 @ 后缀，拒绝
        let cmd = parse_telegram_command("/start@", BOT);
        assert_eq!(cmd, None);
    }

    #[test]
    fn test_empty_at_suffix_help() {
        let cmd = parse_telegram_command("/help@", BOT);
        assert_eq!(cmd, None);
    }

    #[test]
    fn test_empty_at_suffix_with_payload() {
        // "/start@ payload" → 空白拆分后 cmd_token="start@" → 空 @ 后缀
        let cmd = parse_telegram_command("/start@ payload", BOT);
        assert_eq!(cmd, None);
    }

    // ---- 纯 @ 后缀而无命令名 ----

    #[test]
    fn test_at_only_no_command() {
        // "/@MyTestBot" → 无命令名，拒绝
        let cmd = parse_telegram_command("/@MyTestBot", BOT);
        assert_eq!(cmd, None);
    }

    // ---- suffix + payload（已存在但之前因拆分次序错误而失败） ----

    #[test]
    fn test_bot_suffix_with_payload_multiple_words() {
        let cmd = parse_telegram_command("/start@MyTestBot ref_abc extra", BOT);
        assert_eq!(
            cmd,
            Some(TelegramCommand::Start(Some("ref_abc extra".into())))
        );
    }

    #[test]
    fn test_help_with_bot_suffix_and_payload() {
        // /help 忽略 payload
        let cmd = parse_telegram_command("/help@MyTestBot something", BOT);
        assert_eq!(cmd, Some(TelegramCommand::Help));
    }

    #[test]
    fn test_wrong_bot_suffix_with_payload() {
        // bot 不匹配时即使有 payload 也应返回 None
        let cmd = parse_telegram_command("/start@WrongBot payload", BOT);
        assert_eq!(cmd, None);
    }
}
