use crate::request_metadata::{RequestMetadata, ResponseMetadata};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use std::collections::BTreeMap;
use std::sync::LazyLock;

static PROMETHEUS_HANDLE: LazyLock<PrometheusHandle> = LazyLock::new(|| {
    PrometheusBuilder::new()
        .install_recorder()
        .expect("failed to install Prometheus recorder")
});

pub fn init() {
    LazyLock::force(&PROMETHEUS_HANDLE);
}

pub fn render() -> String {
    PROMETHEUS_HANDLE.render()
}

pub fn record_request(request_metadata: &RequestMetadata) {
    let labels = request_metadata.labels();
    metrics::counter!("lacuna_provider_requests_total", &labels).increment(1);
}

pub fn record_response(request_metadata: &RequestMetadata, response_metadata: &ResponseMetadata) {
    let labels = request_metadata.labels();
    if let Some(tokens) = response_metadata.input_tokens {
        metrics::counter!("lacuna_provider_tokens_input_total", &labels).increment(tokens);
    }
    if let Some(tokens) = response_metadata.output_tokens {
        metrics::counter!("lacuna_provider_tokens_output_total", &labels).increment(tokens);
    }
    let total_tokens =
        response_metadata.input_tokens.unwrap_or(0) + response_metadata.output_tokens.unwrap_or(0);
    if total_tokens > 0 {
        metrics::counter!("lacuna_provider_tokens_total", &labels).increment(total_tokens);
    }
    if let Some(map) = &response_metadata.cache_creation_tokens {
        for (duration, tokens) in map {
            let mut l = labels.clone();
            l.push(("cache_duration".to_owned(), duration.clone()));
            metrics::counter!("lacuna_provider_tokens_cache_creation_total", &l).increment(*tokens);
        }
    }
    if let Some(tokens) = response_metadata.cache_read_input_tokens {
        metrics::counter!("lacuna_provider_tokens_cache_read_total", &labels).increment(tokens);
    }
    let cache_creation_tokens = |duration: &str| {
        response_metadata
            .cache_creation_tokens
            .as_ref()
            .and_then(|map| map.get(duration).copied())
    };
    tracing::info!(
        target: "lacuna::usage",
        labels = %serde_json::json!(labels.into_iter().collect::<BTreeMap<_, _>>()),
        input_tokens = response_metadata.input_tokens,
        output_tokens = response_metadata.output_tokens,
        cache_read_input_tokens = response_metadata.cache_read_input_tokens,
        cache_creation_5m_tokens = cache_creation_tokens("5m"),
        cache_creation_1h_tokens = cache_creation_tokens("1h"),
        cache_creation_unknown_tokens = cache_creation_tokens("unknown"),
        "usage"
    );
}
