//! Link and MCP-level behaviour against a fake add-on (no Blender needed).
mod support;
use omnix_blender::{error::LinkError, link::Link};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use support::fake::{Fake, Reply};
use support::*;

fn echo() -> support::fake::Handler {
    Arc::new(|m, req| Reply::Ok(json!({"method": m, "params": req["params"]})))
}

#[tokio::test]
async fn round_trip_and_hello() {
    let d = short_tempdir();
    let f = Fake::start(&d.path().join("f.sock"), echo());
    let link = Link::new(&f.path);
    let h = link.hello().await.unwrap();
    assert_eq!(h.blender, "5.3.0");
    let r = link
        .call("scene_info", json!({"a": 1}), Duration::from_secs(5), true)
        .await
        .unwrap();
    assert_eq!(r["params"]["a"], 1);
}

#[tokio::test]
async fn no_socket_is_not_running() {
    let d = short_tempdir();
    let link = Link::new(d.path().join("missing.sock"));
    assert!(matches!(link.hello().await, Err(LinkError::NotRunning)));
}

#[tokio::test]
async fn too_long_path_is_rejected_clearly() {
    let link = Link::new(format!("/tmp/{}", "x".repeat(120)));
    match link.hello().await {
        Err(LinkError::BadPath(m)) => assert!(m.contains("103")),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn protocol_version_mismatch_is_reported() {
    let d = short_tempdir();
    let f = Fake::start(&d.path().join("f.sock"), echo());
    *f.hello_v.lock().unwrap() = 2;
    let e = Link::new(&f.path).hello().await.unwrap_err();
    assert!(
        matches!(e, LinkError::VersionMismatch { theirs: 2, .. }),
        "{e:?}"
    );
    assert!(e.to_string().contains("install-addon"));
}

#[tokio::test]
async fn remote_errors_keep_their_code() {
    let d = short_tempdir();
    let f = Fake::start(
        &d.path().join("f.sock"),
        Arc::new(|_, _| Reply::Err("not_found", "object 'X' does not exist")),
    );
    let e = Link::new(&f.path)
        .call("get_object", json!({}), Duration::from_secs(5), true)
        .await
        .unwrap_err();
    assert_eq!(e.code(), "not_found");
    assert!(e.to_string().contains("'X'"));
}

#[tokio::test]
async fn silent_blender_times_out_instead_of_hanging() {
    let d = short_tempdir();
    let f = Fake::start(&d.path().join("f.sock"), Arc::new(|_, _| Reply::Silent));
    let t = std::time::Instant::now();
    let e = Link::new(&f.path)
        .call("scene_info", json!({}), Duration::from_millis(300), true)
        .await
        .unwrap_err();
    assert!(matches!(e, LinkError::Timeout(_)), "{e:?}");
    assert!(t.elapsed() < Duration::from_secs(6));
}

#[tokio::test]
async fn oversized_response_line_is_a_protocol_error() {
    let d = short_tempdir();
    let big = vec![b'x'; 17 * 1024 * 1024];
    let f = Fake::start(
        &d.path().join("f.sock"),
        Arc::new(move |_, _| Reply::Raw(big.clone())),
    );
    let e = Link::new(&f.path)
        .call("scene_info", json!({}), Duration::from_secs(20), false)
        .await
        .unwrap_err();
    assert!(matches!(e, LinkError::Protocol(_)), "{e:?}");
}

#[tokio::test]
async fn garbage_response_is_a_protocol_error_and_next_call_reconnects() {
    let d = short_tempdir();
    let n = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let n2 = n.clone();
    let f = Fake::start(
        &d.path().join("f.sock"),
        Arc::new(move |_, _| {
            if n2.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                Reply::Raw(b"not json\n".to_vec())
            } else {
                Reply::Ok(json!(1))
            }
        }),
    );
    let link = Link::new(&f.path);
    assert!(matches!(
        link.call("m", json!({}), Duration::from_secs(5), false)
            .await,
        Err(LinkError::Protocol(_))
    ));
    assert_eq!(
        link.call("m", json!({}), Duration::from_secs(5), false)
            .await
            .unwrap(),
        json!(1)
    );
}

#[tokio::test]
async fn reconnects_after_the_addon_restarts() {
    let d = short_tempdir();
    let path = d.path().join("f.sock");
    let f = Fake::start(&path, echo());
    let link = Link::new(&path);
    link.call("a", json!({}), Duration::from_secs(5), false)
        .await
        .unwrap();
    drop(f); // add-on goes away
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(matches!(
        link.call("a", json!({}), Duration::from_secs(5), false)
            .await,
        Err(LinkError::NotRunning)
    ));
    let _f2 = Fake::start(&path, echo());
    // Even a mutating (non-retry) call succeeds: the dead connection is detected before sending.
    link.call("a", json!({}), Duration::from_secs(5), false)
        .await
        .unwrap();
}

#[tokio::test]
async fn busy_and_timeout_codes_pass_through_mcp() {
    let d = short_tempdir();
    let f = Fake::start(
        &d.path().join("f.sock"),
        Arc::new(|_, _| Reply::Err("busy", "Blender is still working")),
    );
    let c = connect_client(&f.path).await;
    err(&c, "scene_info", json!({}), "busy").await;
    let _ = c.cancel().await;
}

#[tokio::test]
async fn bad_arguments_are_rejected_before_reaching_blender() {
    let d = short_tempdir();
    let f = Fake::start(&d.path().join("f.sock"), echo());
    let c = connect_client(&f.path).await;
    let p = rmcp::model::CallToolRequestParams::new("list_objects")
        .with_arguments(json!({"limit": "ten"}).as_object().unwrap().clone());
    let r = c.call_tool(p).await;
    assert!(
        r.is_err() || r.unwrap().is_error == Some(true),
        "wrong type must be rejected"
    );
    let p = rmcp::model::CallToolRequestParams::new("run_python"); // missing required `code`
    let r = c.call_tool(p).await;
    assert!(r.is_err() || r.unwrap().is_error == Some(true));
    assert_eq!(f.calls("list_objects") + f.calls("run_python"), 0);
    let r = ok(&c, "list_objects", json!({"limit": 5})).await;
    assert_eq!(r["params"]["limit"], 5);
    assert!(
        !r["params"].as_object().unwrap().contains_key("type"),
        "None fields must be dropped: {r}"
    );
    let _ = c.cancel().await;
}
