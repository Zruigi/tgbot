use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, Default, Serialize)]
pub struct TenantWorkerTelemetry {
    pub claimed_jobs: u64,
    pub completed_jobs: u64,
    pub failed_jobs: u64,
    pub in_flight_jobs: u64,
    pub last_claimed_at: Option<DateTime<Utc>>,
    pub last_completed_at: Option<DateTime<Utc>>,
    pub last_failed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct GlobalWorkerTelemetry {
    pub scheduler_started_at: Option<DateTime<Utc>>,
    pub last_poll_at: Option<DateTime<Utc>>,
    pub last_successful_poll_at: Option<DateTime<Utc>>,
    pub last_poll_error_at: Option<DateTime<Utc>>,
    pub total_polls: u64,
    pub total_claimed_jobs: u64,
    pub total_completed_jobs: u64,
    pub total_failed_jobs: u64,
    pub in_flight_jobs: u64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct WorkerTelemetrySnapshot {
    pub global: GlobalWorkerTelemetry,
    pub tenants: HashMap<String, TenantWorkerTelemetry>,
}

#[derive(Clone, Default)]
pub struct WorkerTelemetry {
    state: Arc<RwLock<WorkerTelemetrySnapshot>>,
}

impl WorkerTelemetry {
    pub fn mark_scheduler_started(&self) {
        self.with_state(|state| {
            state.global.scheduler_started_at = Some(Utc::now());
        });
    }

    pub fn mark_poll_started(&self) {
        self.with_state(|state| {
            state.global.total_polls = state.global.total_polls.saturating_add(1);
            state.global.last_poll_at = Some(Utc::now());
        });
    }

    pub fn mark_poll_succeeded(&self) {
        self.with_state(|state| {
            state.global.last_successful_poll_at = Some(Utc::now());
        });
    }

    pub fn mark_poll_failed(&self) {
        self.with_state(|state| {
            state.global.last_poll_error_at = Some(Utc::now());
        });
    }

    pub fn mark_claimed(&self, tenant_id: &str) {
        self.with_state(|state| {
            let now = Utc::now();
            state.global.total_claimed_jobs = state.global.total_claimed_jobs.saturating_add(1);
            state.global.in_flight_jobs = state.global.in_flight_jobs.saturating_add(1);
            let tenant = state.tenants.entry(tenant_id.to_string()).or_default();
            tenant.claimed_jobs = tenant.claimed_jobs.saturating_add(1);
            tenant.in_flight_jobs = tenant.in_flight_jobs.saturating_add(1);
            tenant.last_claimed_at = Some(now);
        });
    }

    pub fn mark_dropped(&self, tenant_id: &str) {
        self.with_state(|state| {
            state.global.in_flight_jobs = state.global.in_flight_jobs.saturating_sub(1);
            if let Some(tenant) = state.tenants.get_mut(tenant_id) {
                tenant.in_flight_jobs = tenant.in_flight_jobs.saturating_sub(1);
            }
        });
    }

    pub fn mark_completed(&self, tenant_id: &str) {
        self.with_state(|state| {
            let now = Utc::now();
            state.global.total_completed_jobs = state.global.total_completed_jobs.saturating_add(1);
            state.global.in_flight_jobs = state.global.in_flight_jobs.saturating_sub(1);
            let tenant = state.tenants.entry(tenant_id.to_string()).or_default();
            tenant.completed_jobs = tenant.completed_jobs.saturating_add(1);
            tenant.in_flight_jobs = tenant.in_flight_jobs.saturating_sub(1);
            tenant.last_completed_at = Some(now);
        });
    }

    pub fn mark_failed(&self, tenant_id: &str) {
        self.with_state(|state| {
            let now = Utc::now();
            state.global.total_failed_jobs = state.global.total_failed_jobs.saturating_add(1);
            state.global.in_flight_jobs = state.global.in_flight_jobs.saturating_sub(1);
            let tenant = state.tenants.entry(tenant_id.to_string()).or_default();
            tenant.failed_jobs = tenant.failed_jobs.saturating_add(1);
            tenant.in_flight_jobs = tenant.in_flight_jobs.saturating_sub(1);
            tenant.last_failed_at = Some(now);
        });
    }

    pub fn snapshot(&self) -> WorkerTelemetrySnapshot {
        self.state
            .read()
            .map(|state| state.clone())
            .unwrap_or_default()
    }

    fn with_state(&self, update: impl FnOnce(&mut WorkerTelemetrySnapshot)) {
        if let Ok(mut state) = self.state.write() {
            update(&mut state);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracks_scheduler_polls_and_job_lifecycle() {
        let telemetry = WorkerTelemetry::default();
        telemetry.mark_scheduler_started();
        telemetry.mark_poll_started();
        telemetry.mark_poll_succeeded();
        telemetry.mark_claimed("tenant-a");
        telemetry.mark_completed("tenant-a");
        telemetry.mark_claimed("tenant-a");
        telemetry.mark_failed("tenant-a");

        let snapshot = telemetry.snapshot();
        assert!(snapshot.global.scheduler_started_at.is_some());
        assert_eq!(snapshot.global.total_polls, 1);
        assert_eq!(snapshot.global.total_claimed_jobs, 2);
        assert_eq!(snapshot.global.total_completed_jobs, 1);
        assert_eq!(snapshot.global.total_failed_jobs, 1);
        assert_eq!(snapshot.global.in_flight_jobs, 0);
        let tenant = snapshot.tenants.get("tenant-a").unwrap();
        assert_eq!(tenant.claimed_jobs, 2);
        assert_eq!(tenant.completed_jobs, 1);
        assert_eq!(tenant.failed_jobs, 1);
        assert_eq!(tenant.in_flight_jobs, 0);
    }

    #[test]
    fn dropped_jobs_do_not_underflow_in_flight_counts() {
        let telemetry = WorkerTelemetry::default();
        telemetry.mark_dropped("tenant-a");

        let snapshot = telemetry.snapshot();
        assert_eq!(snapshot.global.in_flight_jobs, 0);
        assert!(snapshot.tenants.is_empty());
    }
}
