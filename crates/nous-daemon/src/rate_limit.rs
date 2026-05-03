use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use nous_core::config::RateLimitConfig;
use tower_governor::{governor::GovernorConfigBuilder, key_extractor::PeerIpKeyExtractor};

pub fn apply(config: &RateLimitConfig, router: Router) -> Router {
    let period = Duration::from_secs(60) / config.requests_per_minute;
    let governor_config = Arc::new(
        GovernorConfigBuilder::default()
            .per_nanosecond(period.as_nanos() as u64)
            .burst_size(config.burst_size)
            .finish()
            .expect("invalid rate-limit config: burst_size must be > 0 and period must be > 0ns"),
    );
    router.layer(tower_governor::GovernorLayer::<PeerIpKeyExtractor, _, _>::new(governor_config))
}
