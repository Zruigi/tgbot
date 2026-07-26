use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const TELEGRAM_AUTO_REPLY_CONFIG_KEY: &str = "telegram_auto_reply";
pub const MAX_AUTO_REPLY_RULES: usize = 50;
pub const MAX_TRIGGER_CHARS: usize = 256;
pub const MAX_REPLY_CHARS: usize = 4_096;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TelegramAutoReplyConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub rules: Vec<TelegramAutoReplyRule>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TelegramAutoReplyRule {
    pub match_type: TelegramAutoReplyMatch,
    pub trigger: String,
    pub reply: String,
    #[serde(default)]
    pub case_sensitive: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TelegramAutoReplyMatch {
    Exact,
    Contains,
    Prefix,
}

pub fn normalize_auto_reply_config(
    mut config: TelegramAutoReplyConfig,
) -> Result<TelegramAutoReplyConfig, String> {
    if config.rules.len() > MAX_AUTO_REPLY_RULES {
        return Err(format!(
            "at most {MAX_AUTO_REPLY_RULES} auto-reply rules are allowed"
        ));
    }

    let mut identities = HashSet::with_capacity(config.rules.len());
    for rule in &mut config.rules {
        rule.trigger = rule.trigger.trim().to_string();
        rule.reply = rule.reply.trim().to_string();
        if rule.trigger.is_empty() {
            return Err("auto-reply trigger must not be empty".into());
        }
        if rule.trigger.chars().count() > MAX_TRIGGER_CHARS {
            return Err(format!(
                "auto-reply trigger must not exceed {MAX_TRIGGER_CHARS} characters"
            ));
        }
        if rule.reply.is_empty() {
            return Err("auto-reply text must not be empty".into());
        }
        if rule.reply.chars().count() > MAX_REPLY_CHARS {
            return Err(format!(
                "auto-reply text must not exceed {MAX_REPLY_CHARS} characters"
            ));
        }

        let trigger_identity = if rule.case_sensitive {
            rule.trigger.clone()
        } else {
            rule.trigger.to_lowercase()
        };
        if !identities.insert((rule.match_type, rule.case_sensitive, trigger_identity)) {
            return Err("duplicate auto-reply rule".into());
        }
    }

    Ok(config)
}

pub fn select_auto_reply<'a>(
    config: &'a TelegramAutoReplyConfig,
    inbound_text: &str,
) -> Option<&'a str> {
    if !config.enabled {
        return None;
    }

    let inbound_text = inbound_text.trim();
    if inbound_text.is_empty() || inbound_text.starts_with('/') {
        return None;
    }

    for rule in &config.rules {
        let matches = if rule.case_sensitive {
            matches_rule(rule.match_type, inbound_text, &rule.trigger)
        } else {
            matches_rule(
                rule.match_type,
                &inbound_text.to_lowercase(),
                &rule.trigger.to_lowercase(),
            )
        };
        if matches {
            return Some(&rule.reply);
        }
    }

    None
}

fn matches_rule(match_type: TelegramAutoReplyMatch, text: &str, trigger: &str) -> bool {
    match match_type {
        TelegramAutoReplyMatch::Exact => text == trigger,
        TelegramAutoReplyMatch::Contains => text.contains(trigger),
        TelegramAutoReplyMatch::Prefix => text.starts_with(trigger),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(
        match_type: TelegramAutoReplyMatch,
        trigger: &str,
        reply: &str,
    ) -> TelegramAutoReplyRule {
        TelegramAutoReplyRule {
            match_type,
            trigger: trigger.into(),
            reply: reply.into(),
            case_sensitive: false,
        }
    }

    #[test]
    fn normalizes_and_selects_rules_in_order() {
        let config = normalize_auto_reply_config(TelegramAutoReplyConfig {
            enabled: true,
            rules: vec![
                rule(TelegramAutoReplyMatch::Exact, "  Opening Hours ", " Exact "),
                rule(TelegramAutoReplyMatch::Contains, "hours", "Contains"),
            ],
        })
        .unwrap();

        assert_eq!(select_auto_reply(&config, "opening hours"), Some("Exact"));
        assert_eq!(
            select_auto_reply(&config, "What are your HOURS?"),
            Some("Contains")
        );
    }

    #[test]
    fn supports_prefix_and_case_sensitive_matching() {
        let mut prefix = rule(TelegramAutoReplyMatch::Prefix, "Order:", "Received");
        prefix.case_sensitive = true;
        let config = TelegramAutoReplyConfig {
            enabled: true,
            rules: vec![prefix],
        };

        assert_eq!(select_auto_reply(&config, "Order: 42"), Some("Received"));
        assert_eq!(select_auto_reply(&config, "order: 42"), None);
    }

    #[test]
    fn disabled_non_matching_and_commands_do_not_reply() {
        let rule = rule(TelegramAutoReplyMatch::Contains, "price", "Reply");
        let disabled = TelegramAutoReplyConfig {
            enabled: false,
            rules: vec![rule.clone()],
        };
        let enabled = TelegramAutoReplyConfig {
            enabled: true,
            rules: vec![rule],
        };

        assert_eq!(select_auto_reply(&disabled, "price"), None);
        assert_eq!(select_auto_reply(&enabled, "unknown"), None);
        assert_eq!(select_auto_reply(&enabled, "/price"), None);
    }

    #[test]
    fn rejects_invalid_and_duplicate_rules() {
        let duplicate = TelegramAutoReplyConfig {
            enabled: true,
            rules: vec![
                rule(TelegramAutoReplyMatch::Exact, "PRICE", "One"),
                rule(TelegramAutoReplyMatch::Exact, "price", "Two"),
            ],
        };
        assert_eq!(
            normalize_auto_reply_config(duplicate).unwrap_err(),
            "duplicate auto-reply rule"
        );

        let empty = TelegramAutoReplyConfig {
            enabled: true,
            rules: vec![rule(TelegramAutoReplyMatch::Exact, " ", "Reply")],
        };
        assert!(normalize_auto_reply_config(empty).is_err());

        let too_long = TelegramAutoReplyConfig {
            enabled: true,
            rules: vec![rule(
                TelegramAutoReplyMatch::Exact,
                "trigger",
                &"x".repeat(MAX_REPLY_CHARS + 1),
            )],
        };
        assert!(normalize_auto_reply_config(too_long).is_err());
    }

    #[test]
    fn rejects_unknown_json_fields() {
        assert!(
            serde_json::from_value::<TelegramAutoReplyConfig>(serde_json::json!({
                "enabled": true,
                "rules": [],
                "unexpected": true
            }))
            .is_err()
        );
    }
}
