// Deletion Guard — safety checks for deletion operations
// Includes: cancellation word detection, confirmation word detection, and negation/question guards

/// Check if user text is a cancellation request
/// Cancellation words (精确列表): 取消、取消操作、不要了、停止、别执行、不删除、cancel、no
pub fn is_agent_cancellation_text(text: &str) -> bool {
    let normalized = text
        .to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>();

    matches!(
        normalized.as_str(),
        "取消"
            | "取消操作"
            | "不要了"
            | "停止"
            | "别执行"
            | "不删除"
            | "cancel"
            | "no"
    )
}

/// Check if user text is a confirmation request
/// Confirmation words include: 确认、确定、同意、yes、can、ok等
pub fn is_agent_confirmation_text(text: &str) -> bool {
    let normalized = text
        .to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>();

    matches!(
        normalized.as_str(),
        "确认"
            | "确定"
            | "同意"
            | "yes"
            | "ok"
            | "可以"
            | "go"
            | "yes."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cancellation_exact_matches() {
        assert!(is_agent_cancellation_text("取消"));
        assert!(is_agent_cancellation_text("停止"));
        assert!(is_agent_cancellation_text("cancel"));
        assert!(is_agent_cancellation_text("no"));
    }

    #[test]
    fn test_cancellation_case_insensitive() {
        assert!(is_agent_cancellation_text("CANCEL"));
        assert!(is_agent_cancellation_text("Cancel"));
    }

    #[test]
    fn test_cancellation_spaces_ignored() {
        assert!(is_agent_cancellation_text("取 消"));
        assert!(is_agent_cancellation_text(" cancel "));
    }

    #[test]
    fn test_cancellation_partial_rejected() {
        assert!(!is_agent_cancellation_text("不要"));
        assert!(!is_agent_cancellation_text("取消一下"));
    }

    #[test]
    fn test_confirmation_exact_matches() {
        assert!(is_agent_confirmation_text("确认"));
        assert!(is_agent_confirmation_text("yes"));
        assert!(is_agent_confirmation_text("ok"));
    }

    #[test]
    fn test_confirmation_case_insensitive() {
        assert!(is_agent_confirmation_text("YES"));
        assert!(is_agent_confirmation_text("OK"));
    }

    #[test]
    fn test_confirmation_spaces_ignored() {
        assert!(is_agent_confirmation_text("确 认"));
        assert!(is_agent_confirmation_text(" yes "));
    }
}
