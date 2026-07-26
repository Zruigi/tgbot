//! Workers crate — async background job processing system.
//!
//! Provides a complete worker infrastructure:
//! - Postgres-backed job queue with reliable claiming (FOR UPDATE SKIP LOCKED)
//! - Scheduler that polls the queue and dispatches to workers
//! - Worker implementations for various task types
//!
//! # Quick start
//!
//! ```ignore
//! use tgbot_workers::{Scheduler, SchedulerConfig, Dispatcher};
//! use tgbot_workers::push_task_worker::PushTaskWorker;
//! use tgbot_workers::group_policy_worker::GroupPolicyWorker;
//! use tgbot_workers::ai_worker::AiWorker;
//! use tgbot_workers::ocr_worker::OcrWorker;
//! use tgbot_workers::monitor_worker::MonitorWorker;
//! use tgbot_workers::payment_worker::PaymentWorker;
//! use tgbot_integrations::{TelegramClient, LiteLLMClient, PaddleOCRClient, TronGridClient, BinanceClient};
//!
//! let pool = sqlx::postgres::PgPool::connect(&database_url).await?;
//!
//! // Create worker instances
//! let telegram = Arc::new(TelegramClient::from_env()?);
//! let litellm = Arc::new(LiteLLMClient::from_env()?);
//! let ocr = Arc::new(PaddleOCRClient::from_env()?);
//! let trongrid = Arc::new(TronGridClient::from_env()?);
//! let binance = Arc::new(BinanceClient::from_env()?);
//!
//! let workers = vec![
//!     Arc::new(PushTaskWorker::new(telegram.clone())) as Arc<dyn Worker>,
//!     Arc::new(GroupPolicyWorker::new()) as Arc<dyn Worker>,
//!     Arc::new(AiWorker::new(litellm.clone())) as Arc<dyn Worker>,
//!     Arc::new(OcrWorker::new(ocr.clone(), telegram.clone())) as Arc<dyn Worker>,
//!     Arc::new(MonitorWorker::new(telegram.clone(), trongrid.clone(), binance.clone())) as Arc<dyn Worker>,
//!     Arc::new(PaymentWorker::new(trongrid.clone())) as Arc<dyn Worker>,
//! ];
//!
//! let dispatcher = Dispatcher::new(workers);
//! let config = SchedulerConfig::default();
//! let scheduler = Scheduler::new(pool, config, dispatcher);
//! let handle = scheduler.start();
//! ```

pub mod ad_governance_worker;
pub mod ai_worker;
pub mod channel_reminder_worker;
pub mod group_message_policy_worker;
pub mod group_policy_worker;
pub mod job_queue;
pub mod monitor_worker;
pub mod ocr_worker;
pub mod payment_worker;
pub mod push_task_worker;
pub mod scheduler;
pub mod telegram_auto_reply_worker;
pub mod telegram_command_worker;

// Re-export commonly used types
pub use ad_governance_worker::AdGovernanceWorker;
pub use ai_worker::{AiWorker, AnalysisType};
pub use channel_reminder_worker::{
    cleanup_expired_channel_reminders, ChannelReminderCleanupSummary,
};
pub use group_message_policy_worker::GroupMessagePolicyWorker;
pub use group_policy_worker::GroupPolicyWorker;
pub use job_queue::{ClaimedJob, EnqueueOpts, Job, JobStatus, JobType};
pub use monitor_worker::{MonitorTaskType, MonitorWorker};
pub use ocr_worker::{OcrTaskType, OcrWorker};
pub use payment_worker::{PaymentTaskType, PaymentWorker};
pub use push_task_worker::{PushTaskWorker, TelegramClientResolver};
pub use scheduler::{
    run_cron_checks, schedule_group_policy_syncs, schedule_monitor_checks, schedule_push_tasks,
    Dispatcher, Scheduler, SchedulerConfig, Worker,
};
pub use telegram_auto_reply_worker::TelegramAutoReplyWorker;
pub use telegram_command_worker::TelegramCommandWorker;

use std::sync::Arc;
use tokio::sync::watch;
use tokio::task::JoinHandle;

// ---------------------------------------------------------------------------
// Worker handle (for graceful shutdown)
// ---------------------------------------------------------------------------

/// Handle to the running scheduler, used for graceful shutdown.
pub struct WorkerHandle {
    /// Sender to signal shutdown.
    shutdown_tx: watch::Sender<bool>,
    /// Handles to the spawned tasks.
    pub tasks: Vec<JoinHandle<()>>,
}

impl WorkerHandle {
    /// Signal all worker tasks to shut down and wait for them to complete.
    pub async fn shutdown(self) {
        let _ = self.shutdown_tx.send(true);
        for task in self.tasks {
            let _ = task.await;
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_worker_handle_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<WorkerHandle>();
    }

    #[test]
    fn test_worker_handle_is_sync() {
        fn assert_sync<T: Sync>() {}
        assert_sync::<WorkerHandle>();
    }
}
