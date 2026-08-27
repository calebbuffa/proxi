//! Integration tests for `Context` network and cache controls.
//!
//! These tests configure local PROJ state and do not require network access.

use proxi::Context;

#[test]
fn url_endpoint_round_trips() {
    let context = Context::new().expect("configured context");
    let distinct = "https://cdn.proj.org";
    context
        .set_url_endpoint(distinct)
        .expect("set url endpoint");
    assert_eq!(
        context.url_endpoint().as_deref(),
        Some(distinct),
        "get_url_endpoint should return what was set"
    );
}

#[test]
fn grid_cache_controls_do_not_panic_and_set_max_size() {
    let context = Context::new().expect("configured context");
    // These are stateful setters with no getter for size/TTL; we assert they
    // don't fault and that clear + enable round-trips do not error.
    context.grid_cache_set_enable(true);
    context.grid_cache_set_max_size_mb(512);
    context.grid_cache_set_ttl_seconds(3600);
    context.grid_cache_clear();
    context.grid_cache_set_enable(false);
    // Re-enable to leave a sane default.
    context.grid_cache_set_enable(true);
}

#[test]
fn network_enable_disable_round_trips() {
    let context = Context::new().expect("configured context");
    // Network defaults to disabled in proxi.
    context.set_network_enabled(true);
    assert!(context.network_enabled(), "network should be enabled");
    context.set_network_enabled(false);
    assert!(!context.network_enabled(), "network should be disabled");
}
