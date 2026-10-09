#![cfg(feature = "server")]

//! `POST /v1/guard/decisions`: a caller's guard decision reaches the ledger
//! unchanged; a body that does not decode is refused and never recorded.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use synapse::gateway::Gateway;
use synapse::ledger::event::GuardDecision;
use synapse::ledger::{InMemoryLedger, LedgerHandle, LedgerStore};
use synapse::pricing::PricingTable;
use synapse::providers::Catalog;
use synapse::routing::table::RouteTable;
use synapse::server::router;
use tower::ServiceExt;

/// Identical to `WIRE` in `src/ledger/event.rs` and in marketplace `guardDecision.spec.ts`.
const WIRE: &str = r#"{"namespace":"acme","workspace":"ws-9","runId":"thread-1","requestId":"6f1c2b9e-1d2a-4c55-9f0e-3b7a2d1c4e88","timestamp":"2026-10-09T12:00:00Z","type":"guard_decision","slot":"route","guard":"harness_intent","policy":"harness-intent","verdict":"delegate","score":0.97,"latencyMs":84,"argsHash":"8832acbc74cc8fd7c43b6cbd7d7ffa2eaa483de177cef59a6adc46cd5c503e1c"}"#;

fn gateway() -> (Arc<Gateway>, Arc<InMemoryLedger>) {
    let routes = RouteTable::from_toml_str("[routes.\"dummy\"]\nlegs = []").unwrap();
    let catalog = Catalog::build(
        &HashMap::new(),
        &routes.referenced_providers(),
        Duration::from_secs(5),
    )
    .unwrap();
    let store = Arc::new(InMemoryLedger::default());
    let gw = Gateway::builder()
        .routes(routes)
        .catalog(catalog)
        .pricing(PricingTable::default())
        .ledger(LedgerHandle::spawn(
            store.clone() as Arc<dyn LedgerStore>,
            64,
        ))
        .default_tenant("unattributed")
        .build()
        .unwrap();
    (Arc::new(gw), store)
}

fn post(body: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/v1/guard/decisions")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

async fn recorded(store: &InMemoryLedger) -> Vec<GuardDecision> {
    for _ in 0..100 {
        {
            let rows = store.guards.lock();
            if !rows.is_empty() {
                return rows.clone();
            }
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    store.guards.lock().clone()
}

#[tokio::test]
async fn a_guard_decision_reaches_the_ledger_unchanged() {
    let (gw, store) = gateway();
    let res = router(gw).oneshot(post(WIRE)).await.unwrap();
    assert_eq!(res.status(), StatusCode::ACCEPTED);
    let expected: GuardDecision = serde_json::from_str(WIRE).unwrap();
    assert_eq!(recorded(&store).await, vec![expected]);
}

#[tokio::test]
async fn a_decision_without_args_hash_is_refused_and_not_recorded() {
    let (gw, store) = gateway();
    let mut body: serde_json::Value = serde_json::from_str(WIRE).unwrap();
    body.as_object_mut().unwrap().remove("argsHash");
    let res = router(gw).oneshot(post(&body.to_string())).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(store.guards.lock().is_empty());
}
