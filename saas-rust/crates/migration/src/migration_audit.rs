use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationFailure {
    pub source_table: String,
    pub source_key: String,
    pub error: String,
    pub disposition: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_count: Option<i64>,
}

impl MigrationFailure {
    pub fn new(source_table: &str, source_key: String, error: String) -> Self {
        Self {
            source_table: source_table.to_string(),
            source_key,
            error,
            disposition: "unresolved".to_string(),
            source_count: None,
            target_count: None,
        }
    }

    pub fn with_counts(mut self, source_count: i64, target_count: i64) -> Self {
        self.source_count = Some(source_count);
        self.target_count = Some(target_count);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_failure_requires_explicit_disposition() {
        let failure = MigrationFailure::new(
            "messages",
            "tenant_id=tenant-a::msg_id=42".to_string(),
            "invalid timestamp".to_string(),
        );

        assert_eq!(failure.source_table, "messages");
        assert_eq!(failure.source_key, "tenant_id=tenant-a::msg_id=42");
        assert_eq!(failure.error, "invalid timestamp");
        assert_eq!(failure.disposition, "unresolved");
        assert_eq!(failure.source_count, None);
        assert_eq!(failure.target_count, None);
    }

    #[test]
    fn counts_are_attached_without_changing_disposition() {
        let failure = MigrationFailure::new(
            "messages",
            "msg_id=42".to_string(),
            "invalid timestamp".to_string(),
        )
        .with_counts(100, 99);

        assert_eq!(failure.source_count, Some(100));
        assert_eq!(failure.target_count, Some(99));
        assert_eq!(failure.disposition, "unresolved");
    }
}
