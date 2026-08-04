use crate::{
    config::Config,
    models::{DailyUsageSummary, UsageIncrement, UsageSummary},
    store::sqlite::SqliteStore,
};
use chrono::TimeZone;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct UsageStore {
    sqlite: SqliteStore,
}

impl UsageStore {
    pub fn new(config: Arc<Config>) -> Result<Self, String> {
        Ok(Self {
            sqlite: SqliteStore::new(config)?,
        })
    }

    /// Usage accounting is intentionally best-effort: an accounting failure must
    /// never turn an otherwise successful upstream response into a failed request.
    pub fn record(&self, increment: &UsageIncrement) -> Result<(), String> {
        self.sqlite.record_usage_increment(increment)
    }

    pub fn list(
        &self,
        owner_user_id: Option<i64>,
        period: UsagePeriod,
        provider_id: Option<&str>,
    ) -> Result<Vec<UsageSummary>, String> {
        self.sqlite.list_usage_summaries(
            owner_user_id,
            period.bucket_type(),
            period.bucket_key(),
            provider_id,
        )
    }

    pub fn list_daily(
        &self,
        owner_user_id: Option<i64>,
        days: u32,
    ) -> Result<Vec<DailyUsageSummary>, String> {
        let days = days.clamp(1, 90);
        let now = chrono::FixedOffset::east_opt(8 * 60 * 60)
            .expect("valid UTC+08 offset")
            .timestamp_opt(crate::support::time::now_unix() as i64, 0)
            .single()
            .expect("current timestamp is valid");
        let from = now.date_naive() - chrono::Days::new((days - 1) as u64);
        self.sqlite.list_daily_usage_summaries(
            owner_user_id,
            &from.format("%F").to_string(),
            &now.date_naive().format("%F").to_string(),
        )
    }
}

#[derive(Clone, Copy, Debug)]
pub enum UsagePeriod {
    Total,
    Today,
    ThisWeek,
}

impl UsagePeriod {
    pub fn parse(value: Option<&str>) -> Result<Self, String> {
        match value.unwrap_or("total") {
            "total" => Ok(Self::Total),
            "today" => Ok(Self::Today),
            "week" => Ok(Self::ThisWeek),
            value => Err(format!("unsupported usage period: {value}")),
        }
    }

    fn bucket_type(self) -> &'static str {
        match self {
            Self::Total => "total",
            Self::Today => "day",
            Self::ThisWeek => "week",
        }
    }

    fn bucket_key(self) -> String {
        SqliteStore::usage_bucket_key(self.bucket_type(), crate::support::time::now_unix() as i64)
    }
}

#[cfg(test)]
mod tests {
    use super::{UsagePeriod, UsageStore};
    use crate::{
        models::{TokenUsage, UsageIncrement},
        store::sqlite::SqliteStore,
    };
    use std::{
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn aggregates_by_user_provider_model_and_period() {
        let store = UsageStore {
            sqlite: SqliteStore::for_test(unique_test_db_path("usage")).unwrap(),
        };
        let timestamp = 1_785_657_600; // 2026-08-04T00:00:00+08:00
        let increment =
            |user: Option<i64>, provider: &str, model: &str, total: u64| UsageIncrement {
                owner_user_id: user,
                provider_id: provider.to_string(),
                model: model.to_string(),
                usage: TokenUsage {
                    input_tokens: total / 2,
                    output_tokens: total / 2,
                    total_tokens: total,
                    ..TokenUsage::default()
                },
                timestamp,
            };

        store.record(&increment(Some(7), "p1", "m1", 10)).unwrap();
        store.record(&increment(Some(7), "p1", "m1", 20)).unwrap();
        store.record(&increment(Some(7), "p1", "m2", 30)).unwrap();
        store.record(&increment(Some(8), "p1", "m1", 40)).unwrap();

        let rows = store
            .sqlite
            .list_usage_summaries(Some(7), "total", "all".to_string(), Some("p1"))
            .unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].model, None);
        assert_eq!(rows[0].usage.total_tokens, 60);
        assert_eq!(rows[0].request_count, 3);
        assert_eq!(rows[1].model.as_deref(), Some("m1"));
        assert_eq!(rows[1].usage.total_tokens, 30);
        assert_eq!(rows[2].model.as_deref(), Some("m2"));
        assert_eq!(rows[2].usage.total_tokens, 30);

        assert!(matches!(
            UsagePeriod::parse(Some("today")),
            Ok(UsagePeriod::Today)
        ));
        assert!(UsagePeriod::parse(Some("month")).is_err());
    }

    fn unique_test_db_path(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!("ai_gateway_{prefix}_{unique}.sqlite"))
    }
}
