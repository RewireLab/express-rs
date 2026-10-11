//! Integration tests for the HTTP foundation.
//!
//! These tests start a real server on a random port and make actual HTTP
//! requests over TCP.

use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Start a server with the given app builder and return its port.
async fn start_server<F>(build: F) -> u16
where
    F: FnOnce(&mut express_rs::App) + Send + 'static,
{
    // Find a free port
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    // Start the server
    tokio::spawn(async move {
        let mut app = express_rs::App::new();
        build(&mut app);
        app.listen(port).await.unwrap();
    });

    // Wait for the server to start
    tokio::time::sleep(Duration::from_millis(100)).await;
    port
}

/// Send a raw HTTP request and read the full response.
async fn send_request(port: u16, request: &str) -> String {
    let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port))
        .await
        .unwrap();
    stream.write_all(request.as_bytes()).await.unwrap();

    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = stream.read(&mut chunk).await.unwrap();
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    String::from_utf8_lossy(&buf).to_string()
}

/// Send a request and read the response, using Connection: close.
async fn request(port: u16, method: &str, path: &str, body: Option<&str>) -> String {
    let mut req = format!(
        "{} {} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n",
        method, path
    );
    if let Some(b) = body {
        req.push_str(&format!("Content-Length: {}\r\n", b.len()));
        req.push_str("\r\n");
        req.push_str(b);
    } else {
        req.push_str("\r\n");
    }
    send_request(port, &req).await
}

#[tokio::test]
async fn test_basic_get_request() {
    let port = start_server(|app| {
        app.get("/", |_req, mut res, _next, _params| async move {
            res.status(200).send("Hello, World!");
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/", None).await;
    assert!(
        response.contains("200 OK"),
        "Expected 200 OK, got: {}",
        response
    );
    assert!(
        response.contains("Hello, World!"),
        "Expected body, got: {}",
        response
    );
    assert!(
        response.contains("text/html"),
        "Expected text/html content type, got: {}",
        response
    );
}

#[tokio::test]
async fn test_custom_status_code() {
    let port = start_server(|app| {
        app.get("/missing", |_req, mut res, _next, _params| async move {
            res.status(404).send("Not Found");
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/missing", None).await;
    assert!(
        response.contains("404 Not Found"),
        "Expected 404, got: {}",
        response
    );
    assert!(
        response.contains("Not Found"),
        "Expected body, got: {}",
        response
    );
}

#[tokio::test]
async fn test_custom_headers() {
    let port = start_server(|app| {
        app.get("/", |_req, mut res, _next, _params| async move {
            res.set("X-Custom", "test-value")
                .set("X-Another", "another-value")
                .send("OK");
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/", None).await;
    assert!(
        response.contains("x-custom: test-value"),
        "Expected X-Custom header, got: {}",
        response
    );
    assert!(
        response.contains("x-another: another-value"),
        "Expected X-Another header, got: {}",
        response
    );
}

#[tokio::test]
async fn test_post_with_body() {
    let port = start_server(|app| {
        app.post("/submit", |req, mut res, _next, _params| async move {
            let body = req.body_text();
            res.send(&format!("Received: {}", body));
            Ok(res)
        });
    })
    .await;

    let response = request(port, "POST", "/submit", Some("hello world")).await;
    assert!(
        response.contains("200 OK"),
        "Expected 200, got: {}",
        response
    );
    assert!(
        response.contains("Received: hello world"),
        "Expected echoed body, got: {}",
        response
    );
}

#[tokio::test]
async fn test_query_parameters() {
    let port = start_server(|app| {
        app.get("/search", |req, mut res, _next, _params| async move {
            let name = req.query_param("name").unwrap_or("unknown");
            let count = req.query_param("count").unwrap_or("0");
            res.send(&format!("name={}, count={}", name, count));
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/search?name=alice&count=42", None).await;
    assert!(
        response.contains("name=alice"),
        "Expected name=alice, got: {}",
        response
    );
    assert!(
        response.contains("count=42"),
        "Expected count=42, got: {}",
        response
    );
}

#[tokio::test]
async fn test_query_with_encoding() {
    let port = start_server(|app| {
        app.get("/search", |req, mut res, _next, _params| async move {
            let q = req.query_param("q").unwrap_or("");
            res.send(&format!("q={}", q));
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/search?q=hello%20world", None).await;
    assert!(
        response.contains("q=hello world"),
        "Expected decoded query, got: {}",
        response
    );
}

#[tokio::test]
async fn test_multiple_sequential_requests() {
    let port = start_server(|app| {
        app.get("/", |_req, mut res, _next, _params| async move {
            res.send("OK");
            Ok(res)
        });
    })
    .await;

    for i in 0..5 {
        let response = request(port, "GET", "/", None).await;
        assert!(
            response.contains("200 OK"),
            "Request {} failed: {}",
            i,
            response
        );
    }
}

#[tokio::test]
async fn test_keep_alive() {
    let port = start_server(|app| {
        app.get("/", |_req, mut res, _next, _params| async move {
            res.send("OK");
            Ok(res)
        });
    })
    .await;

    let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port))
        .await
        .unwrap();

    // Send two requests on the same connection
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .await
        .unwrap();

    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let n = stream.read(&mut chunk).await.unwrap();
    buf.extend_from_slice(&chunk[..n]);
    let response1 = String::from_utf8_lossy(&buf).to_string();
    assert!(
        response1.contains("200 OK"),
        "First request failed: {}",
        response1
    );

    // Second request on same connection
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();

    let mut buf = Vec::new();
    loop {
        let n = stream.read(&mut chunk).await.unwrap();
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let response2 = String::from_utf8_lossy(&buf).to_string();
    assert!(
        response2.contains("200 OK"),
        "Second request failed: {}",
        response2
    );
}

#[tokio::test]
async fn test_empty_body() {
    let port = start_server(|app| {
        app.post("/", |req, mut res, _next, _params| async move {
            let body = req.body_text();
            res.send(&format!("Body length: {}", body.len()));
            Ok(res)
        });
    })
    .await;

    let response = request(port, "POST", "/", Some("")).await;
    assert!(
        response.contains("Body length: 0"),
        "Expected empty body, got: {}",
        response
    );
}

#[tokio::test]
async fn test_large_body() {
    let port = start_server(|app| {
        app.post("/", |req, mut res, _next, _params| async move {
            let body = req.body_text();
            res.send(&format!("Body length: {}", body.len()));
            Ok(res)
        });
    })
    .await;

    let large_body = "x".repeat(100_000);
    let response = request(port, "POST", "/", Some(&large_body)).await;
    assert!(
        response.contains("Body length: 100000"),
        "Expected 100000 body length, got: {}",
        response
    );
}

#[tokio::test]
async fn test_json_response() {
    let port = start_server(|app| {
        app.get("/api/data", |_req, mut res, _next, _params| async move {
            res.status(200)
                .json_str(r#"{"message":"hello","count":42}"#);
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/api/data", None).await;
    assert!(
        response.contains("200 OK"),
        "Expected 200, got: {}",
        response
    );
    assert!(
        response.contains("application/json"),
        "Expected JSON content type, got: {}",
        response
    );
    assert!(
        response.contains(r#"{"message":"hello","count":42}"#),
        "Expected JSON body, got: {}",
        response
    );
}

#[tokio::test]
async fn test_concurrent_requests() {
    let port = start_server(|app| {
        app.get("/", |_req, mut res, _next, _params| async move {
            res.send("OK");
            Ok(res)
        });
    })
    .await;

    let mut handles = Vec::new();
    for _ in 0..10 {
        let port = port;
        handles.push(tokio::spawn(async move {
            request(port, "GET", "/", None).await
        }));
    }

    for handle in handles {
        let response = handle.await.unwrap();
        assert!(
            response.contains("200 OK"),
            "Concurrent request failed: {}",
            response
        );
    }
}

#[tokio::test]
async fn test_malformed_request_does_not_crash_server() {
    let port = start_server(|app| {
        app.get("/", |_req, mut res, _next, _params| async move {
            res.send("Still alive");
            Ok(res)
        });
    })
    .await;

    // Send a malformed request
    let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port))
        .await
        .unwrap();
    stream.write_all(b"NOT HTTP\r\n\r\n").await.unwrap();

    let mut chunk = [0u8; 4096];
    let _ = stream.read(&mut chunk).await.unwrap();
    // Server should have closed the connection or returned an error

    // Verify server is still alive
    tokio::time::sleep(Duration::from_millis(50)).await;
    let response = request(port, "GET", "/", None).await;
    assert!(
        response.contains("Still alive"),
        "Server crashed after malformed request: {}",
        response
    );
}

// ============================================================
// Phase 2: Routing Tests
// ============================================================

#[tokio::test]
async fn test_routing_different_methods() {
    let port = start_server(|app| {
        app.get("/resource", |_req, mut res, _next, _params| async move {
            res.send("GET");
            Ok(res)
        });
        app.post("/resource", |_req, mut res, _next, _params| async move {
            res.send("POST");
            Ok(res)
        });
        app.put("/resource", |_req, mut res, _next, _params| async move {
            res.send("PUT");
            Ok(res)
        });
        app.delete("/resource", |_req, mut res, _next, _params| async move {
            res.send("DELETE");
            Ok(res)
        });
    })
    .await;

    assert!(request(port, "GET", "/resource", None)
        .await
        .contains("GET"));
    assert!(request(port, "POST", "/resource", None)
        .await
        .contains("POST"));
    assert!(request(port, "PUT", "/resource", None)
        .await
        .contains("PUT"));
    assert!(request(port, "DELETE", "/resource", None)
        .await
        .contains("DELETE"));
}

#[tokio::test]
async fn test_routing_method_not_allowed() {
    let port = start_server(|app| {
        app.get("/resource", |_req, mut res, _next, _params| async move {
            res.send("GET");
            Ok(res)
        });
    })
    .await;

    // POST to a GET-only route should not match
    let response = request(port, "POST", "/resource", None).await;
    assert!(
        !response.contains("GET"),
        "POST should not match GET route, got: {}",
        response
    );
}

#[tokio::test]
async fn test_routing_different_paths() {
    let port = start_server(|app| {
        app.get("/users", |_req, mut res, _next, _params| async move {
            res.send("users list");
            Ok(res)
        });
        app.get("/users/123", |_req, mut res, _next, _params| async move {
            res.send("user 123");
            Ok(res)
        });
        app.get("/posts", |_req, mut res, _next, _params| async move {
            res.send("posts list");
            Ok(res)
        });
    })
    .await;

    assert!(request(port, "GET", "/users", None)
        .await
        .contains("users list"));
    assert!(request(port, "GET", "/users/123", None)
        .await
        .contains("user 123"));
    assert!(request(port, "GET", "/posts", None)
        .await
        .contains("posts list"));
}

#[tokio::test]
async fn test_middleware_use_all_paths() {
    let port = start_server(|app| {
        app.r#use(|_req, mut res, next, _params| async move {
            res.set("X-Middleware", "hit");
            next(express_rs::router::NextCall::Continue).await;
            Ok(res)
        });
        app.get("/", |_req, mut res, _next, _params| async move {
            res.send("OK");
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/", None).await;
    assert!(
        response.contains("x-middleware: hit"),
        "Expected middleware header, got: {}",
        response
    );
    assert!(response.contains("OK"), "Expected body, got: {}", response);
}

#[tokio::test]
async fn test_middleware_use_path_prefix() {
    let port = start_server(|app| {
        app.use_with_path("/api", |_req, mut res, next, _params| async move {
            res.set("X-API", "true");
            next(express_rs::router::NextCall::Continue).await;
            Ok(res)
        });
        app.get("/api/users", |_req, mut res, _next, _params| async move {
            res.send("users");
            Ok(res)
        });
        app.get("/other", |_req, mut res, _next, _params| async move {
            res.send("other");
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/api/users", None).await;
    assert!(
        response.contains("x-api: true"),
        "Expected API header, got: {}",
        response
    );
    assert!(
        response.contains("users"),
        "Expected body, got: {}",
        response
    );

    let response = request(port, "GET", "/other", None).await;
    assert!(
        !response.contains("x-api: true"),
        "Should not have API header, got: {}",
        response
    );
    assert!(
        response.contains("other"),
        "Expected body, got: {}",
        response
    );
}

#[tokio::test]
async fn test_next_continues_to_next_middleware() {
    let port = start_server(|app| {
        app.r#use(|_req, mut res, next, _params| async move {
            res.set("X-First", "1");
            next(express_rs::router::NextCall::Continue).await;
            Ok(res)
        });
        app.r#use(|_req, mut res, next, _params| async move {
            res.set("X-Second", "2");
            next(express_rs::router::NextCall::Continue).await;
            Ok(res)
        });
        app.get("/", |_req, mut res, _next, _params| async move {
            res.send("done");
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/", None).await;
    assert!(
        response.contains("x-first: 1"),
        "Expected first middleware, got: {}",
        response
    );
    assert!(
        response.contains("x-second: 2"),
        "Expected second middleware, got: {}",
        response
    );
    assert!(
        response.contains("done"),
        "Expected body, got: {}",
        response
    );
}

#[tokio::test]
async fn test_no_next_stops_processing() {
    let port = start_server(|app| {
        app.r#use(|_req, mut res, _next, _params| async move {
            res.set("X-Blocked", "true");
            res.send("blocked");
            Ok(res)
        });
        app.get("/", |_req, mut res, _next, _params| async move {
            res.send("should not reach");
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/", None).await;
    assert!(
        response.contains("x-blocked: true"),
        "Expected blocked header, got: {}",
        response
    );
    assert!(
        response.contains("blocked"),
        "Expected blocked body, got: {}",
        response
    );
    assert!(
        !response.contains("should not reach"),
        "Should not reach handler, got: {}",
        response
    );
}

#[tokio::test]
async fn test_trailing_slash_optional() {
    let port = start_server(|app| {
        app.get("/users", |_req, mut res, _next, _params| async move {
            res.send("users");
            Ok(res)
        });
    })
    .await;

    // Both /users and /users/ should match
    assert!(request(port, "GET", "/users", None).await.contains("users"));
    assert!(request(port, "GET", "/users/", None)
        .await
        .contains("users"));
}

#[tokio::test]
async fn test_app_all_matches_all_methods() {
    let port = start_server(|app| {
        app.all("/resource", |_req, mut res, _next, _params| async move {
            res.send("any method");
            Ok(res)
        });
    })
    .await;

    for method in &["GET", "POST", "PUT", "DELETE", "PATCH", "OPTIONS"] {
        let response = request(port, method, "/resource", None).await;
        assert!(
            response.contains("any method"),
            "{} should match app.all, got: {}",
            method,
            response
        );
    }
}

#[tokio::test]
async fn test_route_ordering_first_match_wins() {
    let port = start_server(|app| {
        app.get("/resource", |_req, mut res, _next, _params| async move {
            res.send("first");
            Ok(res)
        });
        app.get("/resource", |_req, mut res, _next, _params| async move {
            res.send("second");
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/resource", None).await;
    assert!(
        response.contains("first"),
        "Expected first route to win, got: {}",
        response
    );
    assert!(
        !response.contains("second"),
        "Second route should not run, got: {}",
        response
    );
}

#[tokio::test]
async fn test_middleware_runs_before_route_handler() {
    let port = start_server(|app| {
        app.r#use(|_req, mut res, next, _params| async move {
            res.set("X-Before", "true");
            next(express_rs::router::NextCall::Continue).await;
            Ok(res)
        });
        app.get("/", |_req, mut res, _next, _params| async move {
            res.set("X-Handler", "true");
            res.send("OK");
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/", None).await;
    assert!(
        response.contains("x-before: true"),
        "Expected before header, got: {}",
        response
    );
    assert!(
        response.contains("x-handler: true"),
        "Expected handler header, got: {}",
        response
    );
}

#[tokio::test]
async fn test_head_request() {
    let port = start_server(|app| {
        app.get("/resource", |_req, mut res, _next, _params| async move {
            res.send("body");
            Ok(res)
        });
    })
    .await;

    let response = request(port, "HEAD", "/resource", None).await;
    assert!(
        response.contains("200 OK"),
        "Expected 200, got: {}",
        response
    );
}

#[tokio::test]
async fn test_middleware_can_modify_response() {
    let port = start_server(|app| {
        app.r#use(|_req, mut res, next, _params| async move {
            next(express_rs::router::NextCall::Continue).await;
            res.set("X-After", "true");
            Ok(res)
        });
        app.get("/", |_req, mut res, _next, _params| async move {
            res.send("OK");
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/", None).await;
    assert!(
        response.contains("x-after: true"),
        "Expected after header, got: {}",
        response
    );
    assert!(response.contains("OK"), "Expected body, got: {}", response);
}

#[tokio::test]
async fn test_multiple_middleware_and_route() {
    let port = start_server(|app| {
        app.r#use(|_req, mut res, next, _params| async move {
            res.set("X-M1", "1");
            next(express_rs::router::NextCall::Continue).await;
            Ok(res)
        });
        app.r#use(|_req, mut res, next, _params| async move {
            res.set("X-M2", "2");
            next(express_rs::router::NextCall::Continue).await;
            Ok(res)
        });
        app.get("/", |_req, mut res, _next, _params| async move {
            res.set("X-Handler", "H");
            res.send("OK");
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/", None).await;
    assert!(
        response.contains("x-m1: 1"),
        "Expected M1 header, got: {}",
        response
    );
    assert!(
        response.contains("x-m2: 2"),
        "Expected M2 header, got: {}",
        response
    );
    assert!(
        response.contains("x-handler: H"),
        "Expected handler header, got: {}",
        response
    );
    assert!(response.contains("OK"), "Expected body, got: {}", response);
}

// ============================================================
// Phase 3: Route Parameter Tests
// ============================================================

#[tokio::test]
async fn test_route_param_basic() {
    let port = start_server(|app| {
        app.get("/user/:id", |_req, mut res, _next, params| async move {
            let id = params.get("id").map_or("", |v| v);
            res.send(&format!("user {}", id));
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/user/123", None).await;
    assert!(
        response.contains("user 123"),
        "Expected param capture, got: {}",
        response
    );
}

#[tokio::test]
async fn test_route_param_multiple() {
    let port = start_server(|app| {
        app.get(
            "/user/:userId/post/:postId",
            |_req, mut res, _next, params| async move {
                let user = params.get("userId").map_or("", |v| v);
                let post = params.get("postId").map_or("", |v| v);
                res.send(&format!("user={} post={}", user, post));
                Ok(res)
            },
        );
    })
    .await;

    let response = request(port, "GET", "/user/42/post/99", None).await;
    assert!(
        response.contains("user=42"),
        "Expected userId param, got: {}",
        response
    );
    assert!(
        response.contains("post=99"),
        "Expected postId param, got: {}",
        response
    );
}

#[tokio::test]
async fn test_route_param_single_segment_only() {
    let port = start_server(|app| {
        app.get("/user/:id", |_req, mut res, _next, params| async move {
            let id = params.get("id").map_or("", |v| v);
            res.send(&format!("user {}", id));
            Ok(res)
        });
    })
    .await;

    // /user/123/edit should NOT match /user/:id (param matches single segment only)
    let response = request(port, "GET", "/user/123/edit", None).await;
    assert!(
        !response.contains("user 123"),
        "Should not match multi-segment, got: {}",
        response
    );
}

#[tokio::test]
async fn test_route_param_with_static_prefix() {
    let port = start_server(|app| {
        app.get(
            "/api/users/:id",
            |_req, mut res, _next, params| async move {
                let id = params.get("id").map_or("", |v| v);
                res.send(&format!("api user {}", id));
                Ok(res)
            },
        );
    })
    .await;

    let response = request(port, "GET", "/api/users/456", None).await;
    assert!(
        response.contains("api user 456"),
        "Expected param after static prefix, got: {}",
        response
    );
}

#[tokio::test]
async fn test_wildcard_splat() {
    let port = start_server(|app| {
        app.get("/files/*splat", |_req, mut res, _next, params| async move {
            let splat = params.get("splat").map_or("", |v| v);
            res.send(&format!("files: {}", splat));
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/files/a/b/c", None).await;
    assert!(
        response.contains("files: a/b/c"),
        "Expected wildcard capture, got: {}",
        response
    );
}

#[tokio::test]
async fn test_wildcard_single_segment() {
    let port = start_server(|app| {
        app.get("/files/*splat", |_req, mut res, _next, params| async move {
            let splat = params.get("splat").map_or("", |v| v);
            res.send(&format!("files: {}", splat));
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/files/readme.txt", None).await;
    assert!(
        response.contains("files: readme.txt"),
        "Expected wildcard single segment, got: {}",
        response
    );
}

#[tokio::test]
async fn test_optional_param_present() {
    let port = start_server(|app| {
        app.get(
            "/user/:id{/:op}",
            |_req, mut res, _next, params| async move {
                let id = params.get("id").map_or("", |v| v);
                let op = params.get("op").map_or("view", |v| v);
                res.send(&format!("{} {}", op, id));
                Ok(res)
            },
        );
    })
    .await;

    let response = request(port, "GET", "/user/123/edit", None).await;
    assert!(
        response.contains("edit 123"),
        "Expected optional param present, got: {}",
        response
    );
}

#[tokio::test]
async fn test_optional_param_absent() {
    let port = start_server(|app| {
        app.get(
            "/user/:id{/:op}",
            |_req, mut res, _next, params| async move {
                let id = params.get("id").map_or("", |v| v);
                let op = params.get("op").map_or("view", |v| v);
                res.send(&format!("{} {}", op, id));
                Ok(res)
            },
        );
    })
    .await;

    let response = request(port, "GET", "/user/123", None).await;
    assert!(
        response.contains("view 123"),
        "Expected optional param absent, got: {}",
        response
    );
}

#[tokio::test]
async fn test_next_route_skips_to_next_route() {
    let port = start_server(|app| {
        app.get("/resource", |_req, mut res, next, _params| async move {
            res.set("X-First", "1");
            next(express_rs::router::NextCall::Route).await;
            Ok(res)
        });
        app.get("/resource", |_req, mut res, _next, _params| async move {
            res.send("second");
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/resource", None).await;
    assert!(
        response.contains("x-first: 1"),
        "Expected first route header, got: {}",
        response
    );
    assert!(
        response.contains("second"),
        "Expected second route body, got: {}",
        response
    );
}

#[tokio::test]
async fn test_param_with_middleware() {
    let port = start_server(|app| {
        app.r#use(|_req, mut res, next, _params| async move {
            res.set("X-Middleware", "true");
            next(express_rs::router::NextCall::Continue).await;
            Ok(res)
        });
        app.get("/user/:id", |_req, mut res, _next, params| async move {
            let id = params.get("id").map_or("", |v| v);
            res.send(&format!("user {}", id));
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/user/789", None).await;
    assert!(
        response.contains("x-middleware: true"),
        "Expected middleware header, got: {}",
        response
    );
    assert!(
        response.contains("user 789"),
        "Expected param capture, got: {}",
        response
    );
}

#[tokio::test]
async fn test_param_decoding() {
    let port = start_server(|app| {
        app.get("/user/:name", |_req, mut res, _next, params| async move {
            let name = params.get("name").map_or("", |v| v);
            res.send(&format!("user {}", name));
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/user/foo%20bar", None).await;
    assert!(
        response.contains("user foo bar"),
        "Expected decoded param, got: {}",
        response
    );
}

#[tokio::test]
async fn test_dot_delimiter_in_path() {
    let port = start_server(|app| {
        app.get(
            "/:name.:format",
            |_req, mut res, _next, params| async move {
                let name = params.get("name").map_or("", |v| v);
                let format = params.get("format").map_or("none", |v| v);
                res.send(&format!("{} as {}", name, format));
                Ok(res)
            },
        );
    })
    .await;

    let response = request(port, "GET", "/foo.json", None).await;
    assert!(
        response.contains("foo as json"),
        "Expected dot-delimited params, got: {}",
        response
    );
}

#[tokio::test]
async fn test_optional_format_suffix() {
    let port = start_server(|app| {
        app.get(
            "/:name{.:format}",
            |_req, mut res, _next, params| async move {
                let name = params.get("name").map_or("", |v| v);
                let format = params.get("format").map_or("html", |v| v);
                res.send(&format!("{} as {}", name, format));
                Ok(res)
            },
        );
    })
    .await;

    let bare = request(port, "GET", "/foo", None).await;
    assert!(
        bare.contains("foo as html"),
        "Expected default format, got: {}",
        bare
    );

    let typed = request(port, "GET", "/foo.json", None).await;
    assert!(
        typed.contains("foo as json"),
        "Expected captured format, got: {}",
        typed
    );
}

#[tokio::test]
async fn test_escaped_literals_in_path() {
    let port = start_server(|app| {
        app.get(
            "/:user\\(:op\\)",
            |_req, mut res, _next, params| async move {
                let user = params.get("user").map_or("", |v| v);
                let op = params.get("op").map_or("", |v| v);
                res.send(&format!("{} {}", op, user));
                Ok(res)
            },
        );
    })
    .await;

    let response = request(port, "GET", "/tj(edit)", None).await;
    assert!(
        response.contains("edit tj"),
        "Expected escaped literals, got: {}",
        response
    );
}

#[tokio::test]
async fn test_param_adjacent_to_static_text() {
    let port = start_server(|app| {
        app.get(
            "/api/v:version/users",
            |_req, mut res, _next, params| async move {
                let version = params.get("version").map_or("", |v| v);
                res.send(&format!("v{}", version));
                Ok(res)
            },
        );
    })
    .await;

    let response = request(port, "GET", "/api/v2/users", None).await;
    assert!(
        response.contains("v2"),
        "Expected param after static text, got: {}",
        response
    );
}

#[tokio::test]
async fn test_wildcard_captures_multiple_segments() {
    let port = start_server(|app| {
        app.get("/files/*splat", |_req, mut res, _next, params| async move {
            let splat = params.get("splat").map_or("", |v| v);
            res.send(splat);
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/files/a/b/c.txt", None).await;
    assert!(
        response.contains("a/b/c.txt"),
        "Expected multi-segment wildcard, got: {}",
        response
    );
}

#[tokio::test]
async fn test_query_string_is_not_part_of_the_path() {
    let port = start_server(|app| {
        app.get("/users/:id", |_req, mut res, _next, params| async move {
            let id = params.get("id").map_or("", |v| v);
            res.send(&format!("user {}", id));
            Ok(res)
        });
    })
    .await;

    // A query string must never confuse route matching.
    let response = request(port, "GET", "/users/77?verbose=true", None).await;
    assert!(
        response.contains("user 77"),
        "Expected query to be ignored for matching, got: {}",
        response
    );
}

// ============================================================
// Phase 4/5: Response and Request API Tests
// ============================================================

#[tokio::test]
async fn test_send_defaults_to_text_html() {
    let port = start_server(|app| {
        app.get("/", |_req, mut res, _next, _params| async move {
            res.send("<p>hi</p>");
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/", None).await;
    assert!(
        response.contains("text/html"),
        "Expected text/html, got: {}",
        response
    );
    assert!(
        response.contains("charset=utf-8"),
        "Expected utf-8 charset, got: {}",
        response
    );
}

#[tokio::test]
async fn test_send_keeps_existing_content_type() {
    let port = start_server(|app| {
        app.get("/", |_req, mut res, _next, _params| async move {
            res.set("content-type", "application/xml");
            res.send("<a/>");
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/", None).await;
    assert!(
        response.contains("application/xml"),
        "Expected content type preserved, got: {}",
        response
    );
}

#[tokio::test]
async fn test_json_serializes_a_typed_value() {
    let port = start_server(|app| {
        app.get("/", |_req, mut res, _next, _params| async move {
            let data = serde_json::json!({"name": "tj", "roles": ["admin"]});
            res.json(&data);
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/", None).await;
    assert!(
        response.contains("application/json"),
        "Expected JSON content type, got: {}",
        response
    );
    assert!(
        response.contains(r#"{"name":"tj","roles":["admin"]}"#),
        "Expected serialized body, got: {}",
        response
    );
}

#[tokio::test]
async fn test_send_status_sends_reason_phrase() {
    let port = start_server(|app| {
        app.get("/gone", |_req, mut res, _next, _params| async move {
            res.send_status(410);
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/gone", None).await;
    assert!(
        response.contains("410 Gone"),
        "Expected 410, got: {}",
        response
    );
    assert!(
        response.contains("Gone"),
        "Expected reason phrase, got: {}",
        response
    );
}

#[tokio::test]
async fn test_content_type_resolves_extension() {
    let port = start_server(|app| {
        app.get("/", |_req, mut res, _next, _params| async move {
            res.content_type("json").send("{}");
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/", None).await;
    assert!(
        response.contains("application/json"),
        "Expected application/json, got: {}",
        response
    );
}

#[tokio::test]
async fn test_javascript_is_text_javascript() {
    let port = start_server(|app| {
        app.get("/", |_req, mut res, _next, _params| async move {
            res.content_type("js").send("1");
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/", None).await;
    assert!(
        response.contains("text/javascript"),
        "Express 5 serves .js as text/javascript, got: {}",
        response
    );
}

#[tokio::test]
async fn test_set_normalises_content_type_charset() {
    let port = start_server(|app| {
        app.get("/", |_req, mut res, _next, _params| async move {
            res.set("content-type", "text/html").send("x");
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/", None).await;
    assert!(
        response.contains("text/html; charset=utf-8"),
        "Expected charset appended once, got: {}",
        response
    );
}

#[tokio::test]
async fn test_get_reads_a_header_back() {
    let port = start_server(|app| {
        app.get("/", |req, mut res, _next, _params| async move {
            let host = req.header("host").unwrap_or("none").to_string();
            res.set("x-seen-host", &host);
            res.send("ok");
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/", None).await;
    assert!(
        response.contains("x-seen-host: localhost"),
        "Expected echoed request header, got: {}",
        response
    );
}

#[tokio::test]
async fn test_append_joins_values() {
    let port = start_server(|app| {
        app.get("/", |_req, mut res, _next, _params| async move {
            res.set("link", "<http://a>");
            res.append("link", "<http://b>");
            res.send("ok");
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/", None).await;
    assert!(
        response.contains("<http://a>, <http://b>"),
        "Expected joined header values, got: {}",
        response
    );
}

#[tokio::test]
async fn test_vary_appends_once() {
    let port = start_server(|app| {
        app.get("/", |_req, mut res, _next, _params| async move {
            res.vary("Accept").vary("accept").vary("Accept-Encoding");
            res.send("ok");
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/", None).await;
    assert!(
        response.contains("vary: Accept, Accept-Encoding"),
        "Expected deduplicated Vary, got: {}",
        response
    );
}

#[tokio::test]
async fn test_redirect_sets_location_and_defaults_to_302() {
    let port = start_server(|app| {
        app.get("/old", |_req, mut res, _next, _params| async move {
            res.redirect(302, "/new");
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/old", None).await;
    assert!(response.contains("302"), "Expected 302, got: {}", response);
    assert!(
        response.contains("location: /new"),
        "Expected Location, got: {}",
        response
    );
    assert!(
        response.contains("Found. Redirecting to /new"),
        "Expected plain-text body, got: {}",
        response
    );
}

#[tokio::test]
async fn test_redirect_negotiates_html_for_browsers() {
    let port = start_server(|app| {
        app.get("/old", |_req, mut res, _next, _params| async move {
            res.redirect(301, "/new");
            Ok(res)
        });
    })
    .await;

    let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port))
        .await
        .unwrap();
    stream
        .write_all(b"GET /old HTTP/1.1\r\nHost: localhost\r\nAccept: text/html\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();

    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = stream.read(&mut chunk).await.unwrap();
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let response = String::from_utf8_lossy(&buf);
    assert!(response.contains("301"), "Expected 301, got: {}", response);
    assert!(
        response.contains("<!DOCTYPE html>"),
        "Expected HTML redirect body, got: {}",
        response
    );
}

#[tokio::test]
async fn test_redirect_defaults_when_no_accept_header_is_meaningful() {
    let port = start_server(|app| {
        app.get("/x", |_req, mut res, _next, _params| async move {
            res.redirect(307, "/y");
            Ok(res)
        });
    })
    .await;

    // No Accept header at all still yields a body, as Express's default matches.
    let response = request(port, "GET", "/x", None).await;
    assert!(response.contains("307"), "Expected 307, got: {}", response);
}

#[tokio::test]
async fn test_location_encodes_the_url() {
    let port = start_server(|app| {
        app.get("/", |_req, mut res, _next, _params| async move {
            res.location("/my file");
            res.send("ok");
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/", None).await;
    assert!(
        response.contains("location: /my%20file"),
        "Expected encoded Location, got: {}",
        response
    );
}

#[tokio::test]
async fn test_send_sets_content_length() {
    let port = start_server(|app| {
        app.get("/", |_req, mut res, _next, _params| async move {
            res.send("hello");
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/", None).await;
    assert!(
        response.contains("content-length: 5"),
        "Expected content-length 5, got: {}",
        response
    );
}

#[tokio::test]
async fn test_send_generates_a_weak_etag() {
    let port = start_server(|app| {
        app.get("/", |_req, mut res, _next, _params| async move {
            res.send("hello");
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/", None).await;
    assert!(
        response.contains("etag: W/\""),
        "Expected weak ETag, got: {}",
        response
    );
}

#[tokio::test]
async fn test_conditional_request_returns_304() {
    let port = start_server(|app| {
        app.get("/", |_req, mut res, _next, _params| async move {
            res.send("hello");
            Ok(res)
        });
    })
    .await;

    // First pass to learn the ETag.
    let first = request(port, "GET", "/", None).await;
    let etag = first
        .lines()
        .find(|l| l.to_lowercase().starts_with("etag:"))
        .and_then(|l| l.split_once(':'))
        .map(|(_, v)| v.trim().to_string())
        .expect("expected an ETag");

    // Replay with If-None-Match; Express turns this into a 304.
    let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port))
        .await
        .unwrap();
    let req = format!(
        "GET / HTTP/1.1\r\nHost: localhost\r\nIf-None-Match: {}\r\nConnection: close\r\n\r\n",
        etag
    );
    stream.write_all(req.as_bytes()).await.unwrap();

    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = stream.read(&mut chunk).await.unwrap();
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let response = String::from_utf8_lossy(&buf).to_string();

    assert!(response.contains("304"), "Expected 304, got: {}", response);
    assert!(
        !response.contains("content-length: 5"),
        "304 must not carry the body length, got: {}",
        response
    );
}

#[tokio::test]
async fn test_head_request_has_headers_but_no_body() {
    let port = start_server(|app| {
        app.get("/", |_req, mut res, _next, _params| async move {
            res.send("hello");
            Ok(res)
        });
    })
    .await;

    let response = request(port, "HEAD", "/", None).await;
    assert!(
        response.contains("200 OK"),
        "Expected 200, got: {}",
        response
    );
    assert!(
        response.contains("content-length: 5"),
        "HEAD must keep Content-Length, got: {}",
        response
    );
    // The raw response ends after the headers.
    assert!(
        !response.contains("hello"),
        "HEAD must not send the body, got: {}",
        response
    );
}

#[tokio::test]
async fn test_204_strips_content_headers() {
    let port = start_server(|app| {
        app.get("/", |_req, mut res, _next, _params| async move {
            res.status(204).send("");
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/", None).await;
    assert!(response.contains("204"), "Expected 204, got: {}", response);
    assert!(
        !response.contains("content-length"),
        "204 must not carry Content-Length, got: {}",
        response
    );
}

#[tokio::test]
async fn test_request_accepts_negotiates() {
    let port = start_server(|app| {
        app.get("/", |req, mut res, _next, _params| async move {
            match req.accepts(&["json", "html"]) {
                Some(kind) => {
                    res.send(&format!("chosen: {}", kind));
                    Ok(res)
                }
                None => {
                    res.status(406).send("not acceptable");
                    Ok(res)
                }
            }
        });
    })
    .await;

    let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port))
        .await
        .unwrap();
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nAccept: application/json\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();

    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = stream.read(&mut chunk).await.unwrap();
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let response = String::from_utf8_lossy(&buf).to_string();
    assert!(
        response.contains("chosen: json"),
        "Expected json chosen, got: {}",
        response
    );
}

#[tokio::test]
async fn test_request_accepts_returns_406_when_nothing_matches() {
    let port = start_server(|app| {
        app.get("/", |req, mut res, _next, _params| async move {
            if req.accepts(&["json"]).is_none() {
                res.status(406);
            }
            res.send("done");
            Ok(res)
        });
    })
    .await;

    let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port))
        .await
        .unwrap();
    stream
        .write_all(
            b"GET / HTTP/1.1\r\nHost: localhost\r\nAccept: text/html\r\nConnection: close\r\n\r\n",
        )
        .await
        .unwrap();

    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = stream.read(&mut chunk).await.unwrap();
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let response = String::from_utf8_lossy(&buf).to_string();
    assert!(response.contains("406"), "Expected 406, got: {}", response);
}

#[tokio::test]
async fn test_request_host_and_hostname() {
    let port = start_server(|app| {
        app.get("/", |req, mut res, _next, _params| async move {
            res.send(&format!(
                "{}|{}",
                req.host().unwrap_or("?"),
                req.hostname().unwrap_or("?")
            ));
            Ok(res)
        });
    })
    .await;

    let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port))
        .await
        .unwrap();
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: example.com:8080\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();

    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = stream.read(&mut chunk).await.unwrap();
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let response = String::from_utf8_lossy(&buf).to_string();
    assert!(
        response.contains("example.com:8080|example.com"),
        "Expected host with port and hostname without, got: {}",
        response
    );
}

#[tokio::test]
async fn test_request_xhr_and_is() {
    let port = start_server(|app| {
        app.post("/", |req, mut res, _next, _params| async move {
            let xhr = req.xhr();
            let json = req.is(&["json"]).is_some();
            res.send(&format!("xhr={} json={}", xhr, json));
            Ok(res)
        });
    })
    .await;

    let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port))
        .await
        .unwrap();
    stream
        .write_all(
            b"POST / HTTP/1.1\r\nHost: localhost\r\nX-Requested-With: XMLHttpRequest\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
        )
        .await
        .unwrap();

    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = stream.read(&mut chunk).await.unwrap();
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let response = String::from_utf8_lossy(&buf).to_string();
    assert!(
        response.contains("xhr=true json=true"),
        "Expected xhr and json detection, got: {}",
        response
    );
}

#[tokio::test]
async fn test_status_out_of_range_is_ignored() {
    let port = start_server(|app| {
        app.get("/", |_req, mut res, _next, _params| async move {
            // Express throws here; express-rs ignores the invalid code and
            // keeps the previous status.
            res.status(99);
            res.status(200);
            res.send("ok");
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/", None).await;
    assert!(
        response.contains("200 OK"),
        "Expected 200, got: {}",
        response
    );
}

// ============================================================
// Phase 6: Error Middleware Tests
// ============================================================

#[tokio::test]
async fn test_next_err_reaches_error_handler() {
    let port = start_server(|app| {
        app.get("/boom", |_req, _res, next, _params| async move {
            next(express_rs::router::NextCall::Error(express_rs::Error::new(
                "boom",
            )))
            .await;
            Ok(express_rs::Response::new())
        });
        app.use_error_handler(|err, _req, mut res, _next, _params| async move {
            res.send(&format!("handled: {}", err.message()));
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/boom", None).await;
    assert!(
        response.contains("handled: boom"),
        "Expected error handler, got: {}",
        response
    );
}

#[tokio::test]
async fn test_async_handler_returning_err_is_forwarded() {
    let port = start_server(|app| {
        app.get("/fail", |_req, _res, _next, _params| async move {
            // Express 5 forwards a rejected promise here.
            Err(express_rs::Error::new("async failure"))
        });
        app.use_error_handler(|err, _req, mut res, _next, _params| async move {
            res.send(&format!("caught: {}", err.message()));
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/fail", None).await;
    assert!(
        response.contains("caught: async failure"),
        "Expected caught error, got: {}",
        response
    );
}

#[tokio::test]
async fn test_error_skips_route_layers() {
    let port = start_server(|app| {
        // The erroring route.
        app.get("/x", |_req, _res, next, _params| async move {
            next(express_rs::router::NextCall::Error(express_rs::Error::new(
                "fail",
            )))
            .await;
            Ok(express_rs::Response::new())
        });
        // A later route for the same path must NOT run while the error is
        // pending — Express skips every route layer once an error exists.
        app.get("/x", |_req, mut res, _next, _params| async move {
            res.send("second route should not run");
            Ok(res)
        });
        app.use_error_handler(|_err, _req, mut res, _next, _params| async move {
            res.send("error middleware");
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/x", None).await;
    assert!(
        response.contains("error middleware"),
        "Expected error handler, got: {}",
        response
    );
    assert!(
        !response.contains("second route should not run"),
        "A route must not run while an error is pending, got: {}",
        response
    );
}

#[tokio::test]
async fn test_plain_middleware_is_skipped_while_error_pending() {
    let port = start_server(|app| {
        app.get("/e", |_req, _res, next, _params| async move {
            next(express_rs::router::NextCall::Error(express_rs::Error::new(
                "nope",
            )))
            .await;
            Ok(express_rs::Response::new())
        });
        // Ordinary middleware registered after the error must be skipped.
        app.r#use(|_req, mut res, next, _params| async move {
            next(express_rs::router::NextCall::Continue).await;
            res.send("ordinary middleware ran");
            Ok(res)
        });
        app.use_error_handler(|_err, _req, mut res, _next, _params| async move {
            res.send("error handler");
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/e", None).await;
    assert!(
        response.contains("error handler"),
        "Expected error handler, got: {}",
        response
    );
    assert!(
        !response.contains("ordinary middleware ran"),
        "Ordinary middleware must not run while an error is pending, got: {}",
        response
    );
}

#[tokio::test]
async fn test_error_handler_only_runs_with_a_pending_error() {
    let port = start_server(|app| {
        // An error handler registered before any error must not run for a
        // normal request.
        app.use_error_handler(|_err, _req, mut res, _next, _params| async move {
            res.send("error handler ran unexpectedly");
            Ok(res)
        });
        app.get("/ok", |_req, mut res, _next, _params| async move {
            res.send("normal");
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/ok", None).await;
    assert!(
        response.contains("normal"),
        "Expected normal handler, got: {}",
        response
    );
    assert!(
        !response.contains("error handler ran unexpectedly"),
        "Error handler must not run without an error, got: {}",
        response
    );
}

#[tokio::test]
async fn test_unhandled_error_becomes_500() {
    let port = start_server(|app| {
        app.get("/oops", |_req, _res, _next, _params| async move {
            Err(express_rs::Error::new("kaboom"))
        });
    })
    .await;

    let response = request(port, "GET", "/oops", None).await;
    assert!(response.contains("500"), "Expected 500, got: {}", response);
    assert!(
        response.contains("kaboom"),
        "Expected the message, got: {}",
        response
    );
}

#[tokio::test]
async fn test_error_status_is_honoured() {
    let port = start_server(|app| {
        app.get("/missing", |_req, _res, _next, _params| async move {
            Err(express_rs::Error::with_status(404, "not here"))
        });
    })
    .await;

    let response = request(port, "GET", "/missing", None).await;
    assert!(response.contains("404"), "Expected 404, got: {}", response);
    assert!(
        response.contains("not here"),
        "Expected the message, got: {}",
        response
    );
}

#[tokio::test]
async fn test_headers_set_before_next_err_persist() {
    let port = start_server(|app| {
        app.get("/x", |_req, mut res, next, _params| async move {
            // Express keeps a single res object for the whole request, so
            // headers set before an error survive into the error handler.
            res.set("x-partial", "true");
            next(express_rs::router::NextCall::Error(express_rs::Error::new(
                "partial",
            )))
            .await;
            Ok(res)
        });
        app.use_error_handler(|err, _req, mut res, _next, _params| async move {
            res.send(&err.message());
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/x", None).await;
    assert!(
        response.contains("partial"),
        "Expected the message, got: {}",
        response
    );
    assert!(
        response.contains("x-partial: true"),
        "Headers set before next(err) should persist, got: {}",
        response
    );
}

#[tokio::test]
async fn test_returning_err_loses_headers_set_before_it() {
    let port = start_server(|app| {
        app.get("/x", |_req, mut res, _next, _params| async move {
            // The ownership model forces choosing between a response and an
            // error, so anything set before the Err is dropped. This is the
            // documented divergence from Express, where res is one object.
            res.set("x-partial", "true");
            Err(express_rs::Error::new("dropped"))
        });
        app.use_error_handler(|err, _req, mut res, _next, _params| async move {
            res.send(&err.message());
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/x", None).await;
    assert!(
        response.contains("dropped"),
        "Expected the message, got: {}",
        response
    );
    assert!(
        !response.contains("x-partial"),
        "Returning Err drops the half-built response, got: {}",
        response
    );
}

#[tokio::test]
async fn test_error_handler_can_resume_with_next() {
    let port = start_server(|app| {
        app.get("/x", |_req, _res, next, _params| async move {
            next(express_rs::router::NextCall::Error(express_rs::Error::new(
                "recoverable",
            )))
            .await;
            Ok(express_rs::Response::new())
        });
        // Swallow the error and continue normal dispatch.
        app.use_error_handler(|_err, _req, _res, next, _params| async move {
            next(express_rs::router::NextCall::Continue).await;
            Ok(express_rs::Response::new())
        });
        app.r#use(|_req, mut res, next, _params| async move {
            next(express_rs::router::NextCall::Continue).await;
            res.send("recovered");
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/x", None).await;
    assert!(
        response.contains("recovered"),
        "Expected recovery, got: {}",
        response
    );
}

#[tokio::test]
async fn test_failing_error_handler_propagates_to_500() {
    let port = start_server(|app| {
        app.get("/x", |_req, _res, _next, _params| async move {
            Err(express_rs::Error::new("first"))
        });
        // This error handler fails too, so nothing handles it.
        app.use_error_handler(|_err, _req, _res, _next, _params| async move {
            Err(express_rs::Error::new("second"))
        });
    })
    .await;

    let response = request(port, "GET", "/x", None).await;
    assert!(response.contains("500"), "Expected 500, got: {}", response);
    assert!(
        response.contains("second"),
        "Expected the propagated message, got: {}",
        response
    );
}

#[tokio::test]
async fn test_first_error_handler_wins() {
    let port = start_server(|app| {
        app.get("/x", |_req, _res, next, _params| async move {
            next(express_rs::router::NextCall::Error(express_rs::Error::new(
                "e",
            )))
            .await;
            Ok(express_rs::Response::new())
        });
        app.use_error_handler(|_err, _req, mut res, _next, _params| async move {
            res.send("first");
            Ok(res)
        });
        app.use_error_handler(|_err, _req, mut res, _next, _params| async move {
            res.send("second");
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/x", None).await;
    assert!(
        response.contains("first"),
        "Expected the first error handler, got: {}",
        response
    );
    assert!(
        !response.contains("second"),
        "Second must not run, got: {}",
        response
    );
}

#[tokio::test]
async fn test_error_body_is_text_plain_with_hardening_headers() {
    let port = start_server(|app| {
        app.get("/x", |_req, _res, _next, _params| async move {
            Err(express_rs::Error::new("bad"))
        });
    })
    .await;

    let response = request(port, "GET", "/x", None).await;
    assert!(
        response.contains("text/plain"),
        "Expected text/plain, got: {}",
        response
    );
    assert!(
        response.contains("x-content-type-options: nosniff"),
        "Expected nosniff, got: {}",
        response
    );
    assert!(
        response.contains("content-security-policy"),
        "Expected a CSP header, got: {}",
        response
    );
}

#[tokio::test]
async fn test_head_on_unhandled_error_keeps_headers_only() {
    let port = start_server(|app| {
        app.get("/x", |_req, _res, _next, _params| async move {
            Err(express_rs::Error::new("hidden"))
        });
    })
    .await;

    let response = request(port, "HEAD", "/x", None).await;
    assert!(response.contains("500"), "Expected 500, got: {}", response);
    assert!(
        !response.contains("hidden"),
        "HEAD must not send the error body, got: {}",
        response
    );
}

#[tokio::test]
async fn test_error_in_middleware_reaches_error_handler() {
    let port = start_server(|app| {
        app.r#use(|_req, _res, _next, _params| async move {
            Err(express_rs::Error::new("middleware failure"))
        });
        app.get("/", |_req, mut res, _next, _params| async move {
            res.send("never");
            Ok(res)
        });
        app.use_error_handler(|err, _req, mut res, _next, _params| async move {
            res.send(&format!("from middleware: {}", err.message()));
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/", None).await;
    assert!(
        response.contains("from middleware: middleware failure"),
        "Expected the middleware error, got: {}",
        response
    );
}

#[tokio::test]
async fn test_next_route_still_works_with_error_handling() {
    let port = start_server(|app| {
        app.get("/r", |_req, mut res, next, _params| async move {
            res.set("x-first", "1");
            next(express_rs::router::NextCall::Route).await;
            Ok(res)
        });
        app.get("/r", |_req, mut res, _next, _params| async move {
            res.send("second route");
            Ok(res)
        });
        app.use_error_handler(|err, _req, mut res, _next, _params| async move {
            res.send(&format!("error: {}", err.message()));
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/r", None).await;
    assert!(
        response.contains("x-first: 1"),
        "Expected the first route header, got: {}",
        response
    );
    assert!(
        response.contains("second route"),
        "Expected the second route, got: {}",
        response
    );
}

// ============================================================
// Phase 6: ports of Express's app.router.js error scenarios
// ============================================================

/// Mirrors Express's "should break out of app.router" test: an error in
/// one route skips every later route layer and is caught by middleware.
#[tokio::test]
async fn test_error_breaks_out_of_the_router() {
    let port = start_server(|app| {
        app.get("/foo{/:bar}", |_req, _res, next, _params| async move {
            next(express_rs::router::NextCall::Continue).await;
            Ok(express_rs::Response::new())
        });
        // Never matches /foo, and must not run even if it did.
        app.get("/bar", |_req, mut res, _next, _params| async move {
            res.send("wrong");
            Ok(res)
        });
        app.get("/foo", |_req, _res, next, _params| async move {
            next(express_rs::router::NextCall::Error(express_rs::Error::new(
                "fail",
            )))
            .await;
            Ok(express_rs::Response::new())
        });
        // A later /foo route must be skipped while the error is pending.
        app.get("/foo", |_req, mut res, _next, _params| async move {
            res.send("must not run");
            Ok(res)
        });
        app.use_error_handler(|err, _req, mut res, _next, _params| async move {
            res.set("content-type", "application/json");
            res.send(&format!("{{\"error\":\"{}\"}}", err.message()));
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/foo", None).await;
    assert!(
        response.contains(r#"{"error":"fail"}"#),
        "Expected the error JSON, got: {}",
        response
    );
    assert!(
        !response.contains("must not run"),
        "A route after the erroring route must be skipped, got: {}",
        response
    );
    assert!(
        !response.contains("wrong"),
        "An unrelated route must not run, got: {}",
        response
    );
}

/// Mirrors Express's "should call handler in same route, if exists".
///
/// Express supports this because one route holds a stack of handlers and
/// its own dispatch runs error handlers registered alongside them.
/// express-rs registers one handler per route layer, so this scenario is
/// spelled with ordinary middleware instead — the observable result for an
/// error raised mid-chain is the same.
#[tokio::test]
async fn test_error_handler_downstream_of_the_failing_one_runs() {
    let port = start_server(|app| {
        app.get("/foo", |_req, _res, next, _params| async move {
            next(express_rs::router::NextCall::Error(express_rs::Error::new(
                "boom!",
            )))
            .await;
            Ok(express_rs::Response::new())
        });
        app.r#use(|_req, _res, next, _params| async move {
            // Ordinary middleware, skipped while the error is pending.
            next(express_rs::router::NextCall::Continue).await;
            Ok(express_rs::Response::new())
        });
        app.use_error_handler(|err, _req, mut res, _next, _params| async move {
            res.send(&format!("route go {}", err.message()));
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/foo", None).await;
    assert!(
        response.contains("route go boom!"),
        "Expected the downstream error handler, got: {}",
        response
    );
}

/// Express 5 forwards a rejected promise to error middleware. A handler
/// returning `Err` is the Rust equivalent.
#[tokio::test]
async fn test_rejected_promise_equivalent_is_forwarded() {
    let port = start_server(|app| {
        app.get("/", |_req, _res, _next, _params| async move {
            Err(express_rs::Error::new("boom!"))
        });
        app.use_error_handler(|err, _req, mut res, _next, _params| async move {
            res.send(&format!("saw {}: {}", "Error", err.message()));
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/", None).await;
    assert!(
        response.contains("saw Error: boom!"),
        "Expected the rejected value forwarded, got: {}",
        response
    );
}

// ============================================================
// Phase 7: Body Parsing Tests
// ============================================================

/// Send a request with extra headers and a raw byte body.
async fn request_raw(
    port: u16,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<&[u8]>,
) -> String {
    let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port))
        .await
        .unwrap();
    let mut req = format!(
        "{} {} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n",
        method, path
    );
    for (k, v) in headers {
        req.push_str(&format!("{}: {}\r\n", k, v));
    }
    if let Some(b) = body {
        req.push_str(&format!("Content-Length: {}\r\n", b.len()));
        req.push_str("\r\n");
        stream.write_all(req.as_bytes()).await.unwrap();
        stream.write_all(b).await.unwrap();
    } else {
        req.push_str("\r\n");
        stream.write_all(req.as_bytes()).await.unwrap();
    }

    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = stream.read(&mut chunk).await.unwrap();
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    String::from_utf8_lossy(&buf).to_string()
}

fn gzip(data: &[u8]) -> Vec<u8> {
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;

    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(data).unwrap();
    encoder.finish().unwrap()
}

fn zlib(data: &[u8]) -> Vec<u8> {
    use flate2::write::ZlibEncoder;
    use flate2::Compression;
    use std::io::Write;

    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(data).unwrap();
    encoder.finish().unwrap()
}

fn brotli(data: &[u8]) -> Vec<u8> {
    use std::io::Write;

    let mut buf = Vec::new();
    {
        let mut writer = brotli::CompressorWriter::new(&mut buf, 4096, 11, 22);
        writer.write_all(data).unwrap();
        writer.flush().unwrap();
    }
    buf
}

#[tokio::test]
async fn test_json_body_is_parsed() {
    let port = start_server(|app| {
        app.r#use(express_rs::body::json(Default::default()));
        app.post("/user", |req, mut res, _next, _params| async move {
            match req.parsed_body() {
                Some(express_rs::body::ParsedBody::Json(v)) => {
                    res.send(&format!("name={}", v["name"].as_str().unwrap_or("?")));
                    Ok(res)
                }
                _ => {
                    res.send("no json body");
                    Ok(res)
                }
            }
        });
    })
    .await;

    let response = request_raw(
        port,
        "POST",
        "/user",
        &[("Content-Type", "application/json")],
        Some(br#"{"name":"tj"}"#),
    )
    .await;
    assert!(
        response.contains("name=tj"),
        "Expected parsed JSON, got: {}",
        response
    );
}

#[tokio::test]
async fn test_json_typed_deserialization() {
    let port = start_server(|app| {
        app.r#use(express_rs::body::json(Default::default()));
        app.post("/login", |req, mut res, _next, _params| async move {
            #[derive(serde::Deserialize)]
            struct Login {
                user: String,
            }
            match req.body_json::<Login>() {
                Some(Ok(login)) => {
                    res.send(&format!("hello {}", login.user));
                    Ok(res)
                }
                _ => {
                    res.send("bad login");
                    Ok(res)
                }
            }
        });
    })
    .await;

    let response = request_raw(
        port,
        "POST",
        "/login",
        &[("Content-Type", "application/json")],
        Some(br#"{"user":"tobi"}"#),
    )
    .await;
    assert!(
        response.contains("hello tobi"),
        "Expected typed JSON, got: {}",
        response
    );
}

#[tokio::test]
async fn test_json_strict_rejects_primitives() {
    let port = start_server(|app| {
        app.r#use(express_rs::body::json(Default::default()));
        app.post("/", |_req, mut res, _next, _params| async move {
            res.send("parsed");
            Ok(res)
        });
    })
    .await;

    let response = request_raw(
        port,
        "POST",
        "/",
        &[("Content-Type", "application/json")],
        Some(b"42"),
    )
    .await;
    assert!(response.contains("400"), "Expected 400, got: {}", response);
}

#[tokio::test]
async fn test_json_non_strict_accepts_primitives() {
    let port = start_server(|app| {
        let mut options = express_rs::body::JsonOptions::default();
        options.strict = false;
        app.r#use(express_rs::body::json(options));
        app.post("/", |req, mut res, _next, _params| async move {
            match req.parsed_body() {
                Some(express_rs::body::ParsedBody::Json(v)) => {
                    res.send(&format!("value={}", v));
                    Ok(res)
                }
                _ => {
                    res.send("none");
                    Ok(res)
                }
            }
        });
    })
    .await;

    let response = request_raw(
        port,
        "POST",
        "/",
        &[("Content-Type", "application/json")],
        Some(b"42"),
    )
    .await;
    assert!(
        response.contains("value=42"),
        "Expected primitive JSON, got: {}",
        response
    );
}

#[tokio::test]
async fn test_json_malformed_is_a_400_with_kind() {
    let port = start_server(|app| {
        app.r#use(express_rs::body::json(Default::default()));
        app.post("/", |_req, mut res, _next, _params| async move {
            res.send("parsed");
            Ok(res)
        });
        app.use_error_handler(|err, _req, mut res, _next, _params| async move {
            res.status(err.status());
            res.send(&format!("{}:{}", err.status(), err.kind().unwrap_or("?")));
            Ok(res)
        });
    })
    .await;

    let response = request_raw(
        port,
        "POST",
        "/",
        &[("Content-Type", "application/json")],
        Some(b"{oops"),
    )
    .await;
    assert!(response.contains("400"), "Expected 400, got: {}", response);
    assert!(
        response.contains("entity.parse.failed"),
        "Expected the error kind, got: {}",
        response
    );
}

#[tokio::test]
async fn test_json_skipped_for_other_content_types() {
    let port = start_server(|app| {
        app.r#use(express_rs::body::json(Default::default()));
        app.post("/", |req, mut res, _next, _params| async move {
            match req.parsed_body() {
                Some(_) => {
                    res.send("parsed");
                    Ok(res)
                }
                None => {
                    res.send("skipped");
                    Ok(res)
                }
            }
        });
    })
    .await;

    let response = request_raw(
        port,
        "POST",
        "/",
        &[("Content-Type", "text/plain")],
        Some(b"{}"),
    )
    .await;
    assert!(
        response.contains("skipped"),
        "Expected the parser to skip, got: {}",
        response
    );
}

#[tokio::test]
async fn test_json_skipped_without_body() {
    let port = start_server(|app| {
        app.r#use(express_rs::body::json(Default::default()));
        app.post("/", |req, mut res, _next, _params| async move {
            match req.parsed_body() {
                Some(_) => {
                    res.send("parsed");
                    Ok(res)
                }
                None => {
                    res.send("no body");
                    Ok(res)
                }
            }
        });
    })
    .await;

    let response = request_raw(
        port,
        "POST",
        "/",
        &[("Content-Type", "application/json")],
        None,
    )
    .await;
    assert!(
        response.contains("no body"),
        "Expected skip without body, got: {}",
        response
    );
}

#[tokio::test]
async fn test_json_limit_is_a_413() {
    let port = start_server(|app| {
        let mut options = express_rs::body::JsonOptions::default();
        options.limit = 4;
        app.r#use(express_rs::body::json(options));
        app.post("/", |_req, mut res, _next, _params| async move {
            res.send("parsed");
            Ok(res)
        });
    })
    .await;

    let response = request_raw(
        port,
        "POST",
        "/",
        &[("Content-Type", "application/json")],
        Some(br#"{"a":12345}"#),
    )
    .await;
    assert!(response.contains("413"), "Expected 413, got: {}", response);
}

#[tokio::test]
async fn test_json_gzip_body_is_decompressed() {
    let port = start_server(|app| {
        app.r#use(express_rs::body::json(Default::default()));
        app.post("/", |req, mut res, _next, _params| async move {
            match req.parsed_body() {
                Some(express_rs::body::ParsedBody::Json(v)) => {
                    res.send(&format!("a={}", v["a"]));
                    Ok(res)
                }
                _ => {
                    res.send("none");
                    Ok(res)
                }
            }
        });
    })
    .await;

    let compressed = gzip(br#"{"a":1}"#);
    let response = request_raw(
        port,
        "POST",
        "/",
        &[
            ("Content-Type", "application/json"),
            ("Content-Encoding", "gzip"),
        ],
        Some(&compressed),
    )
    .await;
    assert!(
        response.contains("a=1"),
        "Expected decompressed JSON, got: {}",
        response
    );
}

#[tokio::test]
async fn test_json_deflate_body_is_decompressed() {
    let port = start_server(|app| {
        app.r#use(express_rs::body::json(Default::default()));
        app.post("/", |req, mut res, _next, _params| async move {
            match req.parsed_body() {
                Some(express_rs::body::ParsedBody::Json(v)) => {
                    res.send(&format!("a={}", v["a"]));
                    Ok(res)
                }
                _ => {
                    res.send("none");
                    Ok(res)
                }
            }
        });
    })
    .await;

    let compressed = zlib(br#"{"a":2}"#);
    let response = request_raw(
        port,
        "POST",
        "/",
        &[
            ("Content-Type", "application/json"),
            ("Content-Encoding", "deflate"),
        ],
        Some(&compressed),
    )
    .await;
    assert!(
        response.contains("a=2"),
        "Expected decompressed JSON, got: {}",
        response
    );
}

#[tokio::test]
async fn test_json_brotli_body_is_decompressed() {
    let port = start_server(|app| {
        app.r#use(express_rs::body::json(Default::default()));
        app.post("/", |req, mut res, _next, _params| async move {
            match req.parsed_body() {
                Some(express_rs::body::ParsedBody::Json(v)) => {
                    res.send(&format!("a={}", v["a"]));
                    Ok(res)
                }
                _ => {
                    res.send("none");
                    Ok(res)
                }
            }
        });
    })
    .await;

    let compressed = brotli(br#"{"a":3}"#);
    let response = request_raw(
        port,
        "POST",
        "/",
        &[
            ("Content-Type", "application/json"),
            ("Content-Encoding", "br"),
        ],
        Some(&compressed),
    )
    .await;
    assert!(
        response.contains("a=3"),
        "Expected decompressed JSON, got: {}",
        response
    );
}

#[tokio::test]
async fn test_unknown_content_encoding_is_a_415() {
    let port = start_server(|app| {
        app.r#use(express_rs::body::json(Default::default()));
        app.post("/", |_req, mut res, _next, _params| async move {
            res.send("parsed");
            Ok(res)
        });
    })
    .await;

    let response = request_raw(
        port,
        "POST",
        "/",
        &[
            ("Content-Type", "application/json"),
            ("Content-Encoding", "compress"),
        ],
        Some(b"{}"),
    )
    .await;
    assert!(response.contains("415"), "Expected 415, got: {}", response);
}

#[tokio::test]
async fn test_inflate_false_rejects_encoded_bodies() {
    let port = start_server(|app| {
        let mut options = express_rs::body::JsonOptions::default();
        options.inflate = false;
        app.r#use(express_rs::body::json(options));
        app.post("/", |_req, mut res, _next, _params| async move {
            res.send("parsed");
            Ok(res)
        });
    })
    .await;

    let compressed = gzip(b"{}");
    let response = request_raw(
        port,
        "POST",
        "/",
        &[
            ("Content-Type", "application/json"),
            ("Content-Encoding", "gzip"),
        ],
        Some(&compressed),
    )
    .await;
    assert!(response.contains("415"), "Expected 415, got: {}", response);
}

#[tokio::test]
async fn test_json_rejects_non_utf_charset() {
    let port = start_server(|app| {
        app.r#use(express_rs::body::json(Default::default()));
        app.post("/", |_req, mut res, _next, _params| async move {
            res.send("parsed");
            Ok(res)
        });
    })
    .await;

    let response = request_raw(
        port,
        "POST",
        "/",
        &[("Content-Type", "application/json; charset=iso-8859-1")],
        Some(b"{}"),
    )
    .await;
    assert!(response.contains("415"), "Expected 415, got: {}", response);
}

#[tokio::test]
async fn test_text_body_is_parsed() {
    let port = start_server(|app| {
        app.r#use(express_rs::body::text(Default::default()));
        app.post("/", |req, mut res, _next, _params| async move {
            match req.parsed_body() {
                Some(express_rs::body::ParsedBody::Text(t)) => {
                    res.send(&format!("text={}", t));
                    Ok(res)
                }
                _ => {
                    res.send("none");
                    Ok(res)
                }
            }
        });
    })
    .await;

    let response = request_raw(
        port,
        "POST",
        "/",
        &[("Content-Type", "text/plain")],
        Some("hello world".as_bytes()),
    )
    .await;
    assert!(
        response.contains("text=hello world"),
        "Expected text body, got: {}",
        response
    );
}

#[tokio::test]
async fn test_text_custom_type_option() {
    let port = start_server(|app| {
        let mut options = express_rs::body::TextOptions::default();
        options.types = vec!["text/csv".to_string()];
        app.r#use(express_rs::body::text(options));
        app.post("/", |req, mut res, _next, _params| async move {
            match req.parsed_body() {
                Some(express_rs::body::ParsedBody::Text(t)) => {
                    res.send(&format!("csv={}", t));
                    Ok(res)
                }
                _ => {
                    res.send("none");
                    Ok(res)
                }
            }
        });
    })
    .await;

    let response = request_raw(
        port,
        "POST",
        "/",
        &[("Content-Type", "text/csv")],
        Some("a,b,c".as_bytes()),
    )
    .await;
    assert!(
        response.contains("csv=a,b,c"),
        "Expected CSV body, got: {}",
        response
    );
}

#[tokio::test]
async fn test_form_simple_pairs() {
    let port = start_server(|app| {
        app.r#use(express_rs::body::urlencoded(Default::default()));
        app.post("/", |req, mut res, _next, _params| async move {
            match req.parsed_body() {
                Some(express_rs::body::ParsedBody::Form(v)) => {
                    let name = v["name"].as_str().unwrap_or("?");
                    let age = v["age"].as_str().unwrap_or("?");
                    res.send(&format!("{}:{}", name, age));
                    Ok(res)
                }
                _ => {
                    res.send("none");
                    Ok(res)
                }
            }
        });
    })
    .await;

    let response = request_raw(
        port,
        "POST",
        "/",
        &[("Content-Type", "application/x-www-form-urlencoded")],
        Some("name=tj&age=30".as_bytes()),
    )
    .await;
    assert!(
        response.contains("tj:30"),
        "Expected form values, got: {}",
        response
    );
}

#[tokio::test]
async fn test_form_repeated_keys_become_arrays() {
    let port = start_server(|app| {
        app.r#use(express_rs::body::urlencoded(Default::default()));
        app.post("/", |req, mut res, _next, _params| async move {
            match req.parsed_body() {
                Some(express_rs::body::ParsedBody::Form(v)) => {
                    res.send(&format!(
                        "n={}",
                        v["tag"].as_array().map(|a| a.len()).unwrap_or(0)
                    ));
                    Ok(res)
                }
                _ => {
                    res.send("none");
                    Ok(res)
                }
            }
        });
    })
    .await;

    let response = request_raw(
        port,
        "POST",
        "/",
        &[("Content-Type", "application/x-www-form-urlencoded")],
        Some("tag=a&tag=b&tag=c".as_bytes()),
    )
    .await;
    assert!(
        response.contains("n=3"),
        "Expected array of three, got: {}",
        response
    );
}

#[tokio::test]
async fn test_form_extended_nests_brackets() {
    let port = start_server(|app| {
        let mut options = express_rs::body::FormOptions::default();
        options.extended = true;
        app.r#use(express_rs::body::urlencoded(options));
        app.post("/", |req, mut res, _next, _params| async move {
            match req.parsed_body() {
                Some(express_rs::body::ParsedBody::Form(v)) => {
                    res.send(&format!(
                        "city={}",
                        v["user"]["address"]["city"].as_str().unwrap_or("?")
                    ));
                    Ok(res)
                }
                _ => {
                    res.send("none");
                    Ok(res)
                }
            }
        });
    })
    .await;

    let response = request_raw(
        port,
        "POST",
        "/",
        &[("Content-Type", "application/x-www-form-urlencoded")],
        Some("user[address][city]=SF".as_bytes()),
    )
    .await;
    assert!(
        response.contains("city=SF"),
        "Expected nested form, got: {}",
        response
    );
}

#[tokio::test]
async fn test_form_too_many_parameters_is_a_413() {
    let port = start_server(|app| {
        let mut options = express_rs::body::FormOptions::default();
        options.parameter_limit = 2;
        app.r#use(express_rs::body::urlencoded(options));
        app.post("/", |_req, mut res, _next, _params| async move {
            res.send("parsed");
            Ok(res)
        });
    })
    .await;

    let response = request_raw(
        port,
        "POST",
        "/",
        &[("Content-Type", "application/x-www-form-urlencoded")],
        Some("a=1&b=2&c=3".as_bytes()),
    )
    .await;
    assert!(response.contains("413"), "Expected 413, got: {}", response);
}

#[tokio::test]
async fn test_form_depth_overflow_is_a_400() {
    let port = start_server(|app| {
        let mut options = express_rs::body::FormOptions::default();
        options.extended = true;
        options.depth = 1;
        app.r#use(express_rs::body::urlencoded(options));
        app.post("/", |_req, mut res, _next, _params| async move {
            res.send("parsed");
            Ok(res)
        });
    })
    .await;

    let response = request_raw(
        port,
        "POST",
        "/",
        &[("Content-Type", "application/x-www-form-urlencoded")],
        Some("a[b][c]=1".as_bytes()),
    )
    .await;
    assert!(response.contains("400"), "Expected 400, got: {}", response);
}

#[tokio::test]
async fn test_verify_failure_is_a_403() {
    let port = start_server(|app| {
        let mut options = express_rs::body::JsonOptions::default();
        options.verify = Some(std::sync::Arc::new(|_, _, _| Err("reject".to_string())));
        app.r#use(express_rs::body::json(options));
        app.post("/", |_req, mut res, _next, _params| async move {
            res.send("parsed");
            Ok(res)
        });
    })
    .await;

    let response = request_raw(
        port,
        "POST",
        "/",
        &[("Content-Type", "application/json")],
        Some(b"{}"),
    )
    .await;
    assert!(response.contains("403"), "Expected 403, got: {}", response);
}

#[tokio::test]
async fn test_custom_type_option_matches_vendor_json() {
    let port = start_server(|app| {
        let mut options = express_rs::body::JsonOptions::default();
        options.types = vec!["application/vnd.api+json".to_string()];
        app.r#use(express_rs::body::json(options));
        app.post("/", |req, mut res, _next, _params| async move {
            match req.parsed_body() {
                Some(express_rs::body::ParsedBody::Json(v)) => {
                    res.send(&format!("t={}", v["type"].as_str().unwrap_or("?")));
                    Ok(res)
                }
                _ => {
                    res.send("none");
                    Ok(res)
                }
            }
        });
    })
    .await;

    let response = request_raw(
        port,
        "POST",
        "/",
        &[("Content-Type", "application/vnd.api+json")],
        Some(br#"{"type":"user"}"#),
    )
    .await;
    assert!(
        response.contains("t=user"),
        "Expected vendor JSON parsed, got: {}",
        response
    );
}

#[tokio::test]
async fn test_parsers_compose_with_routing_and_errors() {
    let port = start_server(|app| {
        app.r#use(express_rs::body::json(Default::default()));
        app.post("/items/:id", |req, mut res, _next, params| async move {
            let id = params.get("id").map_or("", |v| v).to_string();
            match req.parsed_body() {
                Some(express_rs::body::ParsedBody::Json(v)) => {
                    res.send(&format!("{}:{}", id, v["q"]));
                    Ok(res)
                }
                _ => Err(express_rs::Error::with_status(400, "need json")),
            }
        });
        app.use_error_handler(|err, _req, mut res, _next, _params| async move {
            res.status(err.status());
            res.send(&err.message());
            Ok(res)
        });
    })
    .await;

    let ok = request_raw(
        port,
        "POST",
        "/items/7",
        &[("Content-Type", "application/json")],
        Some(br#"{"q":1}"#),
    )
    .await;
    assert!(ok.contains("7:1"), "Expected routed JSON, got: {}", ok);

    let missing = request_raw(port, "POST", "/items/7", &[], Some(b"{}")).await;
    assert!(
        missing.contains("400"),
        "Expected 400 without JSON, got: {}",
        missing
    );
    assert!(
        missing.contains("need json"),
        "Expected error message, got: {}",
        missing
    );
}

// ============================================================
// Phase 8: Static File Tests
// ============================================================

/// Build a fixture tree and return its root.
fn fixture_root(tag: &str) -> std::path::PathBuf {
    let dir =
        std::env::temp_dir().join(format!("express-rs-static-{}-{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("docs")).unwrap();
    std::fs::write(dir.join("hello.txt"), "hello static").unwrap();
    std::fs::write(dir.join("data.json"), r#"{"a":1}"#).unwrap();
    std::fs::write(dir.join(".hidden"), "secret").unwrap();
    std::fs::write(dir.join("docs").join("index.html"), "<h1>docs</h1>").unwrap();
    std::fs::write(dir.join("docs").join("note.txt"), "nested note").unwrap();
    dir
}

fn serve_at(
    app: &mut express_rs::App,
    mount: &str,
    root: std::path::PathBuf,
    options: express_rs::static_files::StaticOptions,
) {
    app.r#use(express_rs::static_files::serve_static_at(
        mount, root, options,
    ));
}

#[tokio::test]
async fn test_static_serves_a_file() {
    let root = fixture_root("basic");
    let port = start_server(move |app| {
        serve_at(
            app,
            "/static",
            root.clone(),
            express_rs::static_files::StaticOptions::with_index(),
        );
    })
    .await;

    let response = request(port, "GET", "/static/hello.txt", None).await;
    assert!(
        response.contains("200 OK"),
        "Expected 200, got: {}",
        response
    );
    assert!(
        response.contains("hello static"),
        "Expected body, got: {}",
        response
    );
    assert!(
        response.contains("text/plain"),
        "Expected text/plain, got: {}",
        response
    );
    assert!(
        response.contains("accept-ranges: bytes"),
        "Expected Accept-Ranges, got: {}",
        response
    );
}

#[tokio::test]
async fn test_static_json_content_type() {
    let root = fixture_root("json");
    let port = start_server(move |app| {
        serve_at(
            app,
            "/static",
            root.clone(),
            express_rs::static_files::StaticOptions::with_index(),
        );
    })
    .await;

    let response = request(port, "GET", "/static/data.json", None).await;
    assert!(
        response.contains("application/json"),
        "Expected JSON type, got: {}",
        response
    );
    assert!(
        response.contains(r#"{"a":1}"#),
        "Expected body, got: {}",
        response
    );
}

#[tokio::test]
async fn test_static_cache_headers() {
    let root = fixture_root("cache");
    let port = start_server(move |app| {
        let mut options = express_rs::static_files::StaticOptions::with_index();
        options.max_age_secs = 3600;
        options.immutable = true;
        serve_at(app, "/static", root.clone(), options);
    })
    .await;

    let response = request(port, "GET", "/static/hello.txt", None).await;
    assert!(
        response.contains("cache-control: public, max-age=3600, immutable"),
        "Expected Cache-Control, got: {}",
        response
    );
    assert!(
        response.contains("etag: W/\""),
        "Expected weak ETag, got: {}",
        response
    );
    assert!(
        response.contains("last-modified:"),
        "Expected Last-Modified, got: {}",
        response
    );
}

#[tokio::test]
async fn test_static_missing_falls_through() {
    let root = fixture_root("fallthrough");
    let port = start_server(move |app| {
        serve_at(
            app,
            "/static",
            root.clone(),
            express_rs::static_files::StaticOptions::with_index(),
        );
        app.get(
            "/static/missing.txt",
            |_req, mut res, _next, _params| async move {
                res.send("fallback");
                Ok(res)
            },
        );
    })
    .await;

    let response = request(port, "GET", "/static/missing.txt", None).await;
    assert!(
        response.contains("fallback"),
        "Expected fallthrough, got: {}",
        response
    );
}

#[tokio::test]
async fn test_static_missing_no_fallthrough_is_404() {
    let root = fixture_root("no-fallthrough");
    let port = start_server(move |app| {
        let mut options = express_rs::static_files::StaticOptions::with_index();
        options.fallthrough = false;
        serve_at(app, "/static", root.clone(), options);
    })
    .await;

    let response = request(port, "GET", "/static/missing.txt", None).await;
    assert!(response.contains("404"), "Expected 404, got: {}", response);
}

#[tokio::test]
async fn test_static_traversal_is_403() {
    let root = fixture_root("traversal");
    let port = start_server(move |app| {
        serve_at(
            app,
            "/static",
            root.clone(),
            express_rs::static_files::StaticOptions::with_index(),
        );
    })
    .await;

    let response = request(port, "GET", "/static/%2e%2e/hello.txt", None).await;
    assert!(response.contains("403"), "Expected 403, got: {}", response);
    assert!(
        !response.contains("hello static"),
        "Must not serve outside root, got: {}",
        response
    );
}

#[tokio::test]
async fn test_static_dotfile_ignored_by_default() {
    let root = fixture_root("dotfile");
    let port = start_server(move |app| {
        serve_at(
            app,
            "/static",
            root.clone(),
            express_rs::static_files::StaticOptions::with_index(),
        );
        app.get(
            "/static/.hidden",
            |_req, mut res, _next, _params| async move {
                res.send("fallback");
                Ok(res)
            },
        );
    })
    .await;

    let response = request(port, "GET", "/static/.hidden", None).await;
    assert!(
        response.contains("fallback"),
        "Expected ignore fallthrough, got: {}",
        response
    );
    assert!(
        !response.contains("secret"),
        "Must not serve dotfile, got: {}",
        response
    );
}

#[tokio::test]
async fn test_static_dotfile_allowed() {
    let root = fixture_root("dotfile-allow");
    let port = start_server(move |app| {
        let mut options = express_rs::static_files::StaticOptions::with_index();
        options.dotfiles = express_rs::static_files::Dotfiles::Allow;
        serve_at(app, "/static", root.clone(), options);
    })
    .await;

    let response = request(port, "GET", "/static/.hidden", None).await;
    assert!(
        response.contains("secret"),
        "Expected dotfile body, got: {}",
        response
    );
}

#[tokio::test]
async fn test_static_dotfile_denied_is_403() {
    let root = fixture_root("dotfile-deny");
    let port = start_server(move |app| {
        let mut options = express_rs::static_files::StaticOptions::with_index();
        options.dotfiles = express_rs::static_files::Dotfiles::Deny;
        serve_at(app, "/static", root.clone(), options);
    })
    .await;

    let response = request(port, "GET", "/static/.hidden", None).await;
    assert!(response.contains("403"), "Expected 403, got: {}", response);
}

#[tokio::test]
async fn test_static_directory_serves_index() {
    let root = fixture_root("index");
    let port = start_server(move |app| {
        serve_at(
            app,
            "/static",
            root.clone(),
            express_rs::static_files::StaticOptions::with_index(),
        );
    })
    .await;

    let response = request(port, "GET", "/static/docs/", None).await;
    assert!(
        response.contains("<h1>docs</h1>"),
        "Expected index, got: {}",
        response
    );
    assert!(
        response.contains("text/html"),
        "Expected HTML type, got: {}",
        response
    );
}

#[tokio::test]
async fn test_static_directory_redirects_to_slash() {
    let root = fixture_root("redirect");
    let port = start_server(move |app| {
        serve_at(
            app,
            "/static",
            root.clone(),
            express_rs::static_files::StaticOptions::with_index(),
        );
    })
    .await;

    let response = request(port, "GET", "/static/docs", None).await;
    assert!(response.contains("301"), "Expected 301, got: {}", response);
    assert!(
        response.contains("location: /static/docs/"),
        "Expected Location, got: {}",
        response
    );
}

#[tokio::test]
async fn test_static_directory_no_redirect_falls_through() {
    let root = fixture_root("no-redirect");
    let port = start_server(move |app| {
        let mut options = express_rs::static_files::StaticOptions::with_index();
        options.redirect = false;
        serve_at(app, "/static", root.clone(), options);
        app.get("/static/docs", |_req, mut res, _next, _params| async move {
            res.send("fallback");
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/static/docs", None).await;
    assert!(
        response.contains("fallback"),
        "Expected fallthrough, got: {}",
        response
    );
}

#[tokio::test]
async fn test_static_conditional_returns_304() {
    let root = fixture_root("fresh");
    let port = start_server(move |app| {
        serve_at(
            app,
            "/static",
            root.clone(),
            express_rs::static_files::StaticOptions::with_index(),
        );
    })
    .await;

    let first = request(port, "GET", "/static/hello.txt", None).await;
    let etag = first
        .lines()
        .find(|l| l.to_lowercase().starts_with("etag:"))
        .and_then(|l| l.split_once(':'))
        .map(|(_, v)| v.trim().to_string())
        .expect("expected an ETag");

    let response = request_raw(
        port,
        "GET",
        "/static/hello.txt",
        &[("If-None-Match", &etag)],
        None,
    )
    .await;
    assert!(response.contains("304"), "Expected 304, got: {}", response);
    assert!(
        !response.contains("hello static"),
        "304 must not carry a body, got: {}",
        response
    );
}

#[tokio::test]
async fn test_static_range_returns_206() {
    let root = fixture_root("range");
    let port = start_server(move |app| {
        serve_at(
            app,
            "/static",
            root.clone(),
            express_rs::static_files::StaticOptions::with_index(),
        );
    })
    .await;

    let response = request_raw(
        port,
        "GET",
        "/static/hello.txt",
        &[("Range", "bytes=0-4")],
        None,
    )
    .await;
    assert!(response.contains("206"), "Expected 206, got: {}", response);
    assert!(
        response.contains("content-range: bytes 0-4/12"),
        "Expected Content-Range, got: {}",
        response
    );
    assert!(
        response.ends_with("hello"),
        "Expected partial body, got: {}",
        response
    );
}

#[tokio::test]
async fn test_static_bad_range_is_416() {
    let root = fixture_root("range-416");
    let port = start_server(move |app| {
        serve_at(
            app,
            "/static",
            root.clone(),
            express_rs::static_files::StaticOptions::with_index(),
        );
    })
    .await;

    let response = request_raw(
        port,
        "GET",
        "/static/hello.txt",
        &[("Range", "bytes=99-")],
        None,
    )
    .await;
    assert!(response.contains("416"), "Expected 416, got: {}", response);
}

#[tokio::test]
async fn test_static_post_falls_through() {
    let root = fixture_root("post");
    let port = start_server(move |app| {
        serve_at(
            app,
            "/static",
            root.clone(),
            express_rs::static_files::StaticOptions::with_index(),
        );
        app.post(
            "/static/hello.txt",
            |_req, mut res, _next, _params| async move {
                res.send("posted");
                Ok(res)
            },
        );
    })
    .await;

    let response = request(port, "POST", "/static/hello.txt", Some("")).await;
    assert!(
        response.contains("posted"),
        "Expected POST to skip static, got: {}",
        response
    );
}

#[tokio::test]
async fn test_static_post_no_fallthrough_is_405() {
    let root = fixture_root("post-405");
    let port = start_server(move |app| {
        let mut options = express_rs::static_files::StaticOptions::with_index();
        options.fallthrough = false;
        serve_at(app, "/static", root.clone(), options);
    })
    .await;

    let response = request(port, "POST", "/static/hello.txt", Some("")).await;
    assert!(response.contains("405"), "Expected 405, got: {}", response);
    assert!(
        response.contains("allow: GET, HEAD"),
        "Expected Allow, got: {}",
        response
    );
}

#[tokio::test]
async fn test_static_outside_mount_falls_through() {
    let root = fixture_root("mount");
    let port = start_server(move |app| {
        serve_at(
            app,
            "/static",
            root.clone(),
            express_rs::static_files::StaticOptions::with_index(),
        );
        app.get("/other", |_req, mut res, _next, _params| async move {
            res.send("other");
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/other", None).await;
    assert!(
        response.contains("other"),
        "Expected other route, got: {}",
        response
    );
}

#[tokio::test]
async fn test_static_mount_boundary_is_exact() {
    let root = fixture_root("boundary");
    let port = start_server(move |app| {
        serve_at(
            app,
            "/static",
            root.clone(),
            express_rs::static_files::StaticOptions::with_index(),
        );
        app.get(
            "/staticfiles/x",
            |_req, mut res, _next, _params| async move {
                res.send("not static");
                Ok(res)
            },
        );
    })
    .await;

    let response = request(port, "GET", "/staticfiles/x", None).await;
    assert!(
        response.contains("not static"),
        "Expected boundary respect, got: {}",
        response
    );
}

#[tokio::test]
async fn test_static_symlink_escape_is_blocked() {
    let root = fixture_root("symlink");
    let outside = root.parent().unwrap().join("outside-secret.txt");
    std::fs::write(&outside, "TOP-SECRET").unwrap();
    std::os::unix::fs::symlink(&outside, root.join("evil.txt")).unwrap();
    let port = start_server(move |app| {
        serve_at(
            app,
            "/static",
            root.clone(),
            express_rs::static_files::StaticOptions::with_index(),
        );
    })
    .await;

    let response = request(port, "GET", "/static/evil.txt", None).await;
    assert!(
        !response.contains("TOP-SECRET"),
        "Must not serve outside root, got: {}",
        response
    );
}

// ============================================================
// Phase 9: Cookie Tests
// ============================================================

#[tokio::test]
async fn test_cookie_defaults_to_path_root() {
    let port = start_server(|app| {
        app.get("/", |_req, mut res, _next, _params| async move {
            res.cookie(
                "sid",
                "abc123",
                &express_rs::cookies::CookieOptions::default(),
            );
            res.send("ok");
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/", None).await;
    assert!(
        response.contains("set-cookie: sid=abc123; Path=/"),
        "Expected default Set-Cookie, got: {}",
        response
    );
}

#[tokio::test]
async fn test_cookie_encodes_the_value() {
    let port = start_server(|app| {
        app.get("/", |_req, mut res, _next, _params| async move {
            res.cookie(
                "sid",
                "a b&c=d",
                &express_rs::cookies::CookieOptions::default(),
            );
            res.send("ok");
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/", None).await;
    assert!(
        response.contains("set-cookie: sid=a%20b%26c%3Dd; Path=/"),
        "Expected encoded value, got: {}",
        response
    );
}

#[tokio::test]
async fn test_cookie_sends_attributes() {
    let port = start_server(|app| {
        app.get("/", |_req, mut res, _next, _params| async move {
            let options = express_rs::cookies::CookieOptions {
                max_age_secs: Some(3600),
                path: Some("/app".to_string()),
                domain: Some("example.com".to_string()),
                secure: true,
                http_only: true,
                same_site: Some(express_rs::cookies::SameSite::Strict),
                ..Default::default()
            };
            res.cookie("sid", "abc", &options);
            res.send("ok");
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/", None).await;
    assert!(
        response.contains("set-cookie: sid=abc; Domain=example.com; Path=/app; Expires="),
        "Expected attribute order, got: {}",
        response
    );
    for attribute in ["Max-Age=3600", "HttpOnly", "Secure", "SameSite=Strict"] {
        assert!(
            response.contains(attribute),
            "Expected {}, got: {}",
            attribute,
            response
        );
    }
}

#[tokio::test]
async fn test_multiple_cookies_are_separate_headers() {
    let port = start_server(|app| {
        app.get("/", |_req, mut res, _next, _params| async move {
            let options = express_rs::cookies::CookieOptions::default();
            res.cookie("sid", "a", &options);
            res.cookie("theme", "dark", &options);
            res.send("ok");
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/", None).await;
    assert!(
        response.contains("set-cookie: sid=a; Path=/"),
        "Expected first cookie, got: {}",
        response
    );
    assert!(
        response.contains("set-cookie: theme=dark; Path=/"),
        "Expected second cookie on its own header, got: {}",
        response
    );
}

#[tokio::test]
async fn test_clear_cookie_sends_epoch_expiry() {
    let port = start_server(|app| {
        app.get("/", |_req, mut res, _next, _params| async move {
            res.clear_cookie("sid", &express_rs::cookies::CookieOptions::default());
            res.send("ok");
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/", None).await;
    assert!(
        response.contains("set-cookie: sid=; Path=/; Expires=Thu, 01 Jan 1970 00:00:00 GMT"),
        "Expected epoch expiry, got: {}",
        response
    );
    assert!(
        !response.contains("Max-Age"),
        "clear_cookie must not carry a lifetime, got: {}",
        response
    );
}

#[tokio::test]
async fn test_request_cookies_are_parsed() {
    let port = start_server(|app| {
        app.get("/echo", |req, mut res, _next, _params| async move {
            let cookies = req.cookies();
            let mut names: Vec<&String> = cookies.keys().collect();
            names.sort();
            let mut body = String::new();
            for name in names {
                body.push_str(&format!("{}={}; ", name, cookies.get(name).unwrap()));
            }
            res.send(body.trim_end());
            Ok(res)
        });
    })
    .await;

    let response = request_raw(
        port,
        "GET",
        "/echo",
        &[("Cookie", "sid=abc123; theme=dark%20mode")],
        None,
    )
    .await;
    assert!(
        response.contains("sid=abc123"),
        "Expected parsed sid, got: {}",
        response
    );
    assert!(
        response.contains("theme=dark mode"),
        "Expected percent-decoded theme, got: {}",
        response
    );
}

#[tokio::test]
async fn test_request_cookies_quoting_and_duplicates() {
    let port = start_server(|app| {
        app.get("/echo", |req, mut res, _next, _params| async move {
            let cookies = req.cookies();
            let quoted = cookies.get("a").map(String::as_str).unwrap_or("missing");
            let empty = cookies.get("b").map(String::as_str).unwrap_or("missing");
            let flag = cookies.contains_key("flag");
            res.send(&format!("a={} b={} flag={}", quoted, empty, flag));
            Ok(res)
        });
    })
    .await;

    let response = request_raw(
        port,
        "GET",
        "/echo",
        &[("Cookie", "a=\"quoted value\"; b=; flag; a=second")],
        None,
    )
    .await;
    assert!(
        response.contains("a=quoted value"),
        "Expected quotes stripped, got: {}",
        response
    );
    assert!(
        response.contains("b= flag=false"),
        "Expected empty b and skipped flag, got: {}",
        response
    );
    assert!(
        !response.contains("a=second"),
        "First duplicate must win, got: {}",
        response
    );
}

#[tokio::test]
async fn test_request_without_cookies_is_empty() {
    let port = start_server(|app| {
        app.get("/echo", |req, mut res, _next, _params| async move {
            res.send(&format!("count={}", req.cookies().len()));
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/echo", None).await;
    assert!(
        response.contains("count=0"),
        "Expected no cookies, got: {}",
        response
    );
}

#[tokio::test]
async fn test_cookie_round_trip_through_the_client() {
    let port = start_server(|app| {
        app.get("/set", |_req, mut res, _next, _params| async move {
            res.cookie(
                "sess",
                "héllo wörld & =",
                &express_rs::cookies::CookieOptions::default(),
            );
            res.send("ok");
            Ok(res)
        });
        app.get("/echo", |req, mut res, _next, _params| async move {
            let value = req.cookies().get("sess").cloned().unwrap_or_default();
            res.send(&value);
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/set", None).await;
    let sent = response
        .split("set-cookie: sess=")
        .nth(1)
        .and_then(|rest| rest.split("\r\n").next())
        .unwrap_or("")
        .to_string();
    assert!(
        sent.contains("%C3%A9"),
        "Expected encoded value, got: {}",
        sent
    );

    let response = request_raw(
        port,
        "GET",
        "/echo",
        &[("Cookie", &format!("sess={}", sent))],
        None,
    )
    .await;
    let body = response.split("\r\n\r\n").nth(1).unwrap_or("");
    assert_eq!(body, "héllo wörld & =");
}

// ============================================================
// Phase 10: File Sending Tests
// ============================================================

/// Create a fixture tree for send_file tests.
///
/// `base/root/` holds the files to serve; `base/outside.txt` sits
/// outside the root for traversal tests.
fn send_file_fixture(tag: &str) -> std::path::PathBuf {
    let base = std::env::temp_dir().join(format!("express-rs-send-{}-{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("root")).unwrap();
    std::fs::write(base.join("root").join("report.txt"), "hello send").unwrap();
    std::fs::write(base.join("root").join(".secret"), "hidden").unwrap();
    std::fs::write(base.join("outside.txt"), "outside").unwrap();
    base
}

#[tokio::test]
async fn test_send_file_serves_default_headers() {
    let base = send_file_fixture("headers");
    let port = start_server(move |app| {
        let root = base.join("root");
        app.get("/file", move |req, res, _next, _params| {
            let root = root.clone();
            async move {
                let res = express_rs::static_files::send_file(
                    &req,
                    res,
                    root.join("report.txt"),
                    &express_rs::static_files::SendFileOptions::default(),
                )
                .await?;
                Ok(res)
            }
        });
    })
    .await;

    let response = request(port, "GET", "/file", None).await;
    assert!(response.contains("200 OK"), "got: {}", response);
    assert!(response.contains("text/plain"), "got: {}", response);
    assert!(
        response.contains("cache-control: public, max-age=0"),
        "got: {}",
        response
    );
    assert!(
        response.contains("accept-ranges: bytes"),
        "got: {}",
        response
    );
    assert!(response.contains("etag: W/"), "got: {}", response);
    assert!(response.contains("last-modified:"), "got: {}", response);
    let body = response.split("\r\n\r\n").nth(1).unwrap_or("").trim_end();
    assert_eq!(
        body, "hello send",
        "Expected file contents, got: {}",
        response
    );
}

#[tokio::test]
async fn test_send_file_errors_reach_the_error_middleware() {
    let base = send_file_fixture("errors");
    let port = start_server(move |app| {
        let root = base.join("root");
        let missing = root.clone();
        app.get("/missing", move |req, res, _next, _params| {
            let root = missing.clone();
            async move {
                let res = express_rs::static_files::send_file(
                    &req,
                    res,
                    root.join("nope.txt"),
                    &express_rs::static_files::SendFileOptions::default(),
                )
                .await?;
                Ok(res)
            }
        });
        let dir = root.clone();
        app.get("/dir", move |req, res, _next, _params| {
            let root = dir.clone();
            async move {
                let res = express_rs::static_files::send_file(
                    &req,
                    res,
                    root.clone(),
                    &express_rs::static_files::SendFileOptions::default(),
                )
                .await?;
                Ok(res)
            }
        });
        app.get("/relative", move |req, res, _next, _params| async move {
            let res = express_rs::static_files::send_file(
                &req,
                res,
                "report.txt",
                &express_rs::static_files::SendFileOptions::default(),
            )
            .await?;
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/missing", None).await;
    assert!(
        response.contains("404 Not Found"),
        "Expected 404, got: {}",
        response
    );

    let response = request(port, "GET", "/dir", None).await;
    assert!(
        response.contains("404 Not Found"),
        "A directory is a 404, got: {}",
        response
    );

    let response = request(port, "GET", "/relative", None).await;
    assert!(
        response.contains("500 Internal Server Error"),
        "Expected 500 for a relative path, got: {}",
        response
    );
    assert!(
        response.contains("path must be absolute"),
        "Expected the TypeError message, got: {}",
        response
    );
}

#[tokio::test]
async fn test_send_file_with_root_confines_the_path() {
    let base = send_file_fixture("root");
    let port = start_server(move |app| {
        let root = base.join("root");
        let ok = root.clone();
        app.get("/ok", move |req, res, _next, _params| {
            let root = ok.clone();
            async move {
                let res = express_rs::static_files::send_file(
                    &req,
                    res,
                    "report.txt",
                    &express_rs::static_files::SendFileOptions::with_root(root),
                )
                .await?;
                Ok(res)
            }
        });
        let traverse = root.clone();
        app.get("/traverse", move |req, res, _next, _params| {
            let root = traverse.clone();
            async move {
                let res = express_rs::static_files::send_file(
                    &req,
                    res,
                    "../outside.txt",
                    &express_rs::static_files::SendFileOptions::with_root(root),
                )
                .await?;
                Ok(res)
            }
        });
        let absolute = root.clone();
        app.get("/absolute", move |req, res, _next, _params| {
            let root = absolute.clone();
            async move {
                let res = express_rs::static_files::send_file(
                    &req,
                    res,
                    root.join("report.txt"),
                    &express_rs::static_files::SendFileOptions::with_root(&root),
                )
                .await?;
                Ok(res)
            }
        });
        let missing = root.clone();
        app.get("/missing", move |req, res, _next, _params| {
            let root = missing.clone();
            async move {
                let res = express_rs::static_files::send_file(
                    &req,
                    res,
                    "nope.txt",
                    &express_rs::static_files::SendFileOptions::with_root(root),
                )
                .await?;
                Ok(res)
            }
        });
    })
    .await;

    let response = request(port, "GET", "/ok", None).await;
    let body = response.split("\r\n\r\n").nth(1).unwrap_or("").trim_end();
    assert_eq!(
        body, "hello send",
        "Expected the rooted file, got: {}",
        response
    );

    let response = request(port, "GET", "/traverse", None).await;
    assert!(
        response.contains("403 Forbidden"),
        "Expected traversal refused, got: {}",
        response
    );
    assert!(
        !response.contains("outside"),
        "Must not serve outside root, got: {}",
        response
    );

    let response = request(port, "GET", "/absolute", None).await;
    assert!(
        response.contains("500 Internal Server Error"),
        "Absolute path with root must be refused, got: {}",
        response
    );

    let response = request(port, "GET", "/missing", None).await;
    assert!(
        response.contains("404 Not Found"),
        "Expected 404, got: {}",
        response
    );
}

#[tokio::test]
async fn test_send_file_dotfile_policy() {
    let base = send_file_fixture("dotfiles");
    let port = start_server(move |app| {
        let root = base.join("root");
        let make_options = |dotfiles| express_rs::static_files::SendFileOptions {
            dotfiles,
            ..Default::default()
        };
        let ignored = make_options(express_rs::static_files::Dotfiles::Ignore);
        let denied = make_options(express_rs::static_files::Dotfiles::Deny);
        let allowed = make_options(express_rs::static_files::Dotfiles::Allow);
        let ignored_root = root.clone();
        let denied_root = root.clone();
        let allowed_root = root.clone();

        app.get("/ignore", move |req, res, _next, _params| {
            let root = ignored_root.clone();
            let options = ignored.clone();
            async move {
                let res =
                    express_rs::static_files::send_file(&req, res, root.join(".secret"), &options)
                        .await?;
                Ok(res)
            }
        });
        app.get("/deny", move |req, res, _next, _params| {
            let root = denied_root.clone();
            let options = denied.clone();
            async move {
                let res =
                    express_rs::static_files::send_file(&req, res, root.join(".secret"), &options)
                        .await?;
                Ok(res)
            }
        });
        app.get("/allow", move |req, res, _next, _params| {
            let root = allowed_root.clone();
            let options = allowed.clone();
            async move {
                let res =
                    express_rs::static_files::send_file(&req, res, root.join(".secret"), &options)
                        .await?;
                Ok(res)
            }
        });
    })
    .await;

    let response = request(port, "GET", "/ignore", None).await;
    assert!(
        response.contains("404 Not Found"),
        "Ignore must hide dotfiles, got: {}",
        response
    );

    let response = request(port, "GET", "/deny", None).await;
    assert!(
        response.contains("403 Forbidden"),
        "Deny must answer 403, got: {}",
        response
    );

    let response = request(port, "GET", "/allow", None).await;
    let body = response.split("\r\n\r\n").nth(1).unwrap_or("").trim_end();
    assert_eq!(
        body, "hidden",
        "Allow must serve dotfiles, got: {}",
        response
    );
}

#[tokio::test]
async fn test_send_file_conditional_request_is_304() {
    let base = send_file_fixture("conditional");
    let port = start_server(move |app| {
        let root = base.join("root");
        app.get("/file", move |req, res, _next, _params| {
            let root = root.clone();
            async move {
                let res = express_rs::static_files::send_file(
                    &req,
                    res,
                    root.join("report.txt"),
                    &express_rs::static_files::SendFileOptions::default(),
                )
                .await?;
                Ok(res)
            }
        });
    })
    .await;

    let first = request(port, "GET", "/file", None).await;
    let etag = first
        .split("\r\n")
        .find(|l| l.to_lowercase().starts_with("etag:"))
        .and_then(|l| l.split_once(':'))
        .map(|(_, v)| v.trim().to_string())
        .expect("ETag on first response");

    let second = request_raw(port, "GET", "/file", &[("If-None-Match", &etag)], None).await;
    assert!(
        second.contains("304 Not Modified"),
        "Expected 304, got: {}",
        second
    );
}

#[tokio::test]
async fn test_send_file_range_request_is_206() {
    let base = send_file_fixture("range");
    let port = start_server(move |app| {
        let root = base.join("root");
        app.get("/file", move |req, res, _next, _params| {
            let root = root.clone();
            async move {
                let res = express_rs::static_files::send_file(
                    &req,
                    res,
                    root.join("report.txt"),
                    &express_rs::static_files::SendFileOptions::default(),
                )
                .await?;
                Ok(res)
            }
        });
    })
    .await;

    let response = request_raw(port, "GET", "/file", &[("Range", "bytes=0-4")], None).await;
    assert!(
        response.contains("206 Partial Content"),
        "got: {}",
        response
    );
    assert!(
        response.contains("content-range: bytes 0-4/10"),
        "got: {}",
        response
    );
    let body = response.split("\r\n\r\n").nth(1).unwrap_or("").trim_end();
    assert_eq!(
        body, "hello",
        "Expected the first five bytes, got: {}",
        response
    );
    assert!(
        !response.contains("hello send"),
        "Full body must not appear for a range, got: {}",
        response
    );
}

#[tokio::test]
async fn test_send_file_cache_options_and_header_hook() {
    let base = send_file_fixture("cache");
    let port = start_server(move |app| {
        let root = base.join("root");
        app.get("/file", move |req, res, _next, _params| {
            let root = root.clone();
            let options = express_rs::static_files::SendFileOptions {
                max_age_secs: 3600,
                immutable: true,
                on_headers: Some(std::sync::Arc::new(|res, _info| {
                    res.set("x-custom", "hook");
                })),
                ..Default::default()
            };
            async move {
                let res = express_rs::static_files::send_file(
                    &req,
                    res,
                    root.join("report.txt"),
                    &options,
                )
                .await?;
                Ok(res)
            }
        });
    })
    .await;

    let response = request(port, "GET", "/file", None).await;
    assert!(
        response.contains("cache-control: public, max-age=3600, immutable"),
        "got: {}",
        response
    );
    assert!(response.contains("x-custom: hook"), "got: {}", response);
}

#[tokio::test]
async fn test_download_sets_content_disposition() {
    let base = send_file_fixture("download");
    let port = start_server(move |app| {
        let root = base.join("root");
        let explicit = root.clone();
        app.get("/named", move |req, res, _next, _params| {
            let root = explicit.clone();
            async move {
                let res = express_rs::static_files::download(
                    &req,
                    res,
                    root.join("report.txt"),
                    Some("report.txt"),
                    &express_rs::static_files::SendFileOptions::default(),
                )
                .await?;
                Ok(res)
            }
        });
        let default_root = root.clone();
        app.get("/default", move |req, res, _next, _params| {
            let root = default_root.clone();
            async move {
                let res = express_rs::static_files::download(
                    &req,
                    res,
                    root.join("report.txt"),
                    None,
                    &express_rs::static_files::SendFileOptions::default(),
                )
                .await?;
                Ok(res)
            }
        });
        let unicode_root = root.clone();
        app.get("/unicode", move |req, res, _next, _params| {
            let root = unicode_root.clone();
            async move {
                let res = express_rs::static_files::download(
                    &req,
                    res,
                    root.join("report.txt"),
                    Some("ファイル.txt"),
                    &express_rs::static_files::SendFileOptions::default(),
                )
                .await?;
                Ok(res)
            }
        });
    })
    .await;

    let response = request(port, "GET", "/named", None).await;
    assert!(
        response.contains("content-disposition: attachment; filename=\"report.txt\""),
        "got: {}",
        response
    );
    let body = response.split("\r\n\r\n").nth(1).unwrap_or("").trim_end();
    assert_eq!(
        body, "hello send",
        "Expected the file body, got: {}",
        response
    );

    let response = request(port, "GET", "/default", None).await;
    assert!(
        response.contains("content-disposition: attachment; filename=\"report.txt\""),
        "Expected basename default, got: {}",
        response
    );

    let response = request(port, "GET", "/unicode", None).await;
    assert!(
        response.contains(
            "content-disposition: attachment; filename=\"____.txt\"; \
             filename*=UTF-8''%E3%83%95%E3%82%A1%E3%82%A4%E3%83%AB.txt"
        ),
        "Expected RFC 5987 encoding, got: {}",
        response
    );
}

// ============================================================
// Phase 11: Content Negotiation Tests (res.format)
// ============================================================

#[tokio::test]
async fn test_format_picks_the_best_type_by_quality() {
    let port = start_server(|app| {
        app.get("/negotiate", |req, res, _next, _params| async move {
            let res = express_rs::response::format(&req, res, |f| {
                f.on("json", |mut res| {
                    res.send(r#"{"picked":"json"}"#);
                    Ok(res)
                })
                .on("html", |mut res| {
                    res.send("<p>html</p>");
                    Ok(res)
                })
            })?;
            Ok(res)
        });
    })
    .await;

    let response = request_raw(
        port,
        "GET",
        "/negotiate",
        &[("Accept", "text/html;q=0.5, application/json")],
        None,
    )
    .await;
    assert!(response.contains("200 OK"), "got: {}", response);
    assert!(
        response.contains("content-type: application/json"),
        "got: {}",
        response
    );
    assert!(
        response.contains("vary: Accept"),
        "Expected Vary: Accept, got: {}",
        response
    );
    assert!(
        response.contains("picked"),
        "Expected the json body, got: {}",
        response
    );
    assert!(
        !response.contains("html</p>"),
        "Only the chosen handler must run, got: {}",
        response
    );
}

#[tokio::test]
async fn test_format_expands_extension_keys() {
    let port = start_server(|app| {
        app.get("/negotiate", |req, res, _next, _params| async move {
            let res = express_rs::response::format(&req, res, |f| {
                f.on("json", |mut res| {
                    res.send("json body");
                    Ok(res)
                })
            })?;
            Ok(res)
        });
    })
    .await;

    let response = request_raw(
        port,
        "GET",
        "/negotiate",
        &[("Accept", "application/json")],
        None,
    )
    .await;
    assert!(
        response.contains("content-type: application/json"),
        "Expected the extension expanded, got: {}",
        response
    );
    assert!(response.contains("json body"), "got: {}", response);
}

#[tokio::test]
async fn test_format_accepts_full_type_keys() {
    let port = start_server(|app| {
        app.get("/negotiate", |req, res, _next, _params| async move {
            let res = express_rs::response::format(&req, res, |f| {
                f.on("text/html", |mut res| {
                    res.send("<p>html</p>");
                    Ok(res)
                })
            })?;
            Ok(res)
        });
    })
    .await;

    let response = request_raw(port, "GET", "/negotiate", &[("Accept", "text/html")], None).await;
    assert!(
        response.contains("content-type: text/html"),
        "got: {}",
        response
    );
    assert!(response.contains("<p>html</p>"), "got: {}", response);
}

#[tokio::test]
async fn test_format_default_runs_when_nothing_matches() {
    let port = start_server(|app| {
        app.get("/negotiate", |req, res, _next, _params| async move {
            let res = express_rs::response::format(&req, res, |f| {
                f.on("json", |mut res| {
                    res.send("json body");
                    Ok(res)
                })
                .default(|mut res| {
                    res.send("plain fallback");
                    Ok(res)
                })
            })?;
            Ok(res)
        });
    })
    .await;

    let response = request_raw(port, "GET", "/negotiate", &[("Accept", "text/csv")], None).await;
    assert!(response.contains("200 OK"), "got: {}", response);
    assert!(
        response.contains("plain fallback"),
        "Expected the default body, got: {}",
        response
    );
    assert!(
        response.contains("vary: Accept"),
        "Vary must be set even on the default, got: {}",
        response
    );
}

#[tokio::test]
async fn test_format_406_without_default() {
    let port = start_server(|app| {
        app.get("/negotiate", |req, res, _next, _params| async move {
            let res = express_rs::response::format(&req, res, |f| {
                f.on("json", |mut res| {
                    res.send("json body");
                    Ok(res)
                })
            })?;
            Ok(res)
        });
    })
    .await;

    let response = request_raw(port, "GET", "/negotiate", &[("Accept", "text/csv")], None).await;
    assert!(
        response.contains("406 Not Acceptable"),
        "Expected 406, got: {}",
        response
    );
    // Vary is lost here: the 406 travels as Err, and a returned Err
    // discards the headers the response carried — the documented
    // behaviour of error propagation in this framework.
}

#[tokio::test]
async fn test_format_without_accept_header_picks_first_offered() {
    let port = start_server(|app| {
        app.get("/negotiate", |req, res, _next, _params| async move {
            let res = express_rs::response::format(&req, res, |f| {
                f.on("json", |mut res| {
                    res.send("json body");
                    Ok(res)
                })
                .on("html", |mut res| {
                    res.send("<p>html</p>");
                    Ok(res)
                })
            })?;
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/negotiate", None).await;
    assert!(
        response.contains("json body"),
        "First offered wins without Accept, got: {}",
        response
    );
}

// ============================================================
// Phase 12: Raw Body Tests
// ============================================================

#[tokio::test]
async fn test_raw_parser_stores_bytes() {
    let port = start_server(|app| {
        app.r#use(express_rs::body::raw(
            express_rs::body::RawOptions::default(),
        ));
        app.post("/echo", |req, mut res, _next, _params| async move {
            match req.parsed_body() {
                Some(express_rs::body::ParsedBody::Raw(bytes)) => {
                    res.send(&String::from_utf8_lossy(&bytes));
                }
                _ => {
                    res.send("none");
                }
            }
            Ok(res)
        });
    })
    .await;

    let response = request_raw(
        port,
        "POST",
        "/echo",
        &[("Content-Type", "application/octet-stream")],
        Some(b"raw bytes!"),
    )
    .await;
    let body = response.split("\r\n\r\n").nth(1).unwrap_or("").trim_end();
    assert_eq!(body, "raw bytes!", "got: {}", response);
}

#[tokio::test]
async fn test_raw_parser_skips_other_types() {
    let port = start_server(|app| {
        app.r#use(express_rs::body::raw(
            express_rs::body::RawOptions::default(),
        ));
        app.post("/echo", |req, mut res, _next, _params| async move {
            match req.parsed_body() {
                Some(_) => {
                    res.send("parsed");
                }
                None => {
                    res.send("none");
                }
            }
            Ok(res)
        });
    })
    .await;

    let response = request_raw(
        port,
        "POST",
        "/echo",
        &[("Content-Type", "text/plain")],
        Some(b"just text"),
    )
    .await;
    let body = response.split("\r\n\r\n").nth(1).unwrap_or("").trim_end();
    assert_eq!(
        body, "none",
        "text/plain must not parse as raw, got: {}",
        response
    );
}

#[tokio::test]
async fn test_raw_parser_enforces_limit() {
    let port = start_server(|app| {
        app.r#use(express_rs::body::raw(express_rs::body::RawOptions {
            limit: 8,
            ..Default::default()
        }));
        app.post("/echo", |_req, mut res, _next, _params| async move {
            res.send("ok");
            Ok(res)
        });
    })
    .await;

    let response = request_raw(
        port,
        "POST",
        "/echo",
        &[("Content-Type", "application/octet-stream")],
        Some(b"0123456789ab"),
    )
    .await;
    assert!(
        response.contains("413"),
        "Expected 413 over the limit, got: {}",
        response
    );
}

// ============================================================
// Phase 13: zstd Content-Encoding Tests
// ============================================================

fn zstd_compress(data: &[u8]) -> Vec<u8> {
    zstd::stream::encode_all(data, 3).unwrap()
}

#[tokio::test]
async fn test_json_zstd_body_is_decompressed() {
    let port = start_server(|app| {
        app.r#use(express_rs::body::json(Default::default()));
        app.post("/", |req, mut res, _next, _params| async move {
            match req.parsed_body() {
                Some(express_rs::body::ParsedBody::Json(v)) => {
                    res.send(&format!("a={}", v["a"]));
                    Ok(res)
                }
                _ => {
                    res.send("none");
                    Ok(res)
                }
            }
        });
    })
    .await;

    let compressed = zstd_compress(br#"{"a":1}"#);
    let response = request_raw(
        port,
        "POST",
        "/",
        &[
            ("Content-Type", "application/json"),
            ("Content-Encoding", "zstd"),
        ],
        Some(&compressed),
    )
    .await;
    assert!(
        response.contains("a=1"),
        "Expected decompressed JSON, got: {}",
        response
    );
}

#[tokio::test]
async fn test_zstd_body_over_the_limit_is_413() {
    let port = start_server(|app| {
        app.r#use(express_rs::body::json(express_rs::body::JsonOptions {
            limit: 5,
            ..Default::default()
        }));
        app.post("/", |_req, mut res, _next, _params| async move {
            res.send("ok");
            Ok(res)
        });
    })
    .await;

    let compressed = zstd_compress(br#"{"a":"0123456789"}"#);
    let response = request_raw(
        port,
        "POST",
        "/",
        &[
            ("Content-Type", "application/json"),
            ("Content-Encoding", "zstd"),
        ],
        Some(&compressed),
    )
    .await;
    assert!(
        response.contains("413"),
        "Expected 413 over the limit, got: {}",
        response
    );
}

// ============================================================
// Phase 14: Parameter Callback Tests (app.param)
// ============================================================

#[tokio::test]
async fn test_param_callback_runs_before_the_route() {
    let port = start_server(|app| {
        app.param("id", |_req, mut res, next, _params| async move {
            res.append("x-order", "param");
            next(express_rs::router::NextCall::Continue).await;
            Ok(res)
        });
        app.get("/users/:id", |_req, mut res, _next, params| async move {
            res.append("x-order", "route");
            res.send(params.get("id").map_or("", |v| v));
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/users/7", None).await;
    assert!(
        response.contains("x-order: param, route"),
        "Expected param before route, got: {}",
        response
    );
    let body = response.split("\r\n\r\n").nth(1).unwrap_or("").trim_end();
    assert_eq!(body, "7");
}

#[tokio::test]
async fn test_param_callback_error_reaches_error_middleware() {
    let port = start_server(|app| {
        app.param("id", |_req, res, next, params| async move {
            let valid = params
                .get("id")
                .and_then(|v| v.parse::<u64>().ok())
                .is_some();
            if valid {
                next(express_rs::router::NextCall::Continue).await;
                Ok(res)
            } else {
                Err(express_rs::Error::with_status(400, "invalid id"))
            }
        });
        app.get("/users/:id", |_req, mut res, _next, _params| async move {
            res.send("route reached");
            Ok(res)
        });
        app.use_error_handler(|err, _req, mut res, _next, _params| async move {
            res.status(err.status()).send(&err.message());
            Ok(res)
        });
    })
    .await;

    let bad = request(port, "GET", "/users/abc", None).await;
    assert!(bad.contains("400 Bad Request"), "got: {}", bad);
    assert!(
        bad.contains("invalid id"),
        "Expected the callback error, got: {}",
        bad
    );
    assert!(
        !bad.contains("route reached"),
        "The route must not run after a failed callback, got: {}",
        bad
    );

    let good = request(port, "GET", "/users/5", None).await;
    assert!(
        good.contains("route reached"),
        "Expected the route to run, got: {}",
        good
    );
}

#[tokio::test]
async fn test_param_callback_runs_once_per_request() {
    let port = start_server(|app| {
        app.param("id", |_req, mut res, next, _params| async move {
            res.append("x-trace", "P");
            next(express_rs::router::NextCall::Continue).await;
            Ok(res)
        });
        app.use_with_path("/u/:id", |_req, mut res, next, _params| async move {
            res.append("x-trace", "M");
            next(express_rs::router::NextCall::Continue).await;
            Ok(res)
        });
        app.get("/u/:id", |_req, mut res, _next, _params| async move {
            res.append("x-trace", "R");
            res.send("done");
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/u/9", None).await;
    assert!(
        response.contains("x-trace: P, M, R"),
        "Expected the callback exactly once, got: {}",
        response
    );
}

#[tokio::test]
async fn test_param_callback_short_circuits_without_next() {
    let port = start_server(|app| {
        app.param("id", |_req, mut res, next, params| async move {
            if params.get("id").is_some_and(|v| v == "blocked") {
                res.status(401).send("blocked by callback");
                return Ok(res);
            }
            next(express_rs::router::NextCall::Continue).await;
            Ok(res)
        });
        app.get("/items/:id", |_req, mut res, _next, _params| async move {
            res.send("route reached");
            Ok(res)
        });
    })
    .await;

    let blocked = request(port, "GET", "/items/blocked", None).await;
    assert!(blocked.contains("401 Unauthorized"), "got: {}", blocked);
    assert!(
        !blocked.contains("route reached"),
        "The route must not run, got: {}",
        blocked
    );

    let passed = request(port, "GET", "/items/open", None).await;
    assert!(passed.contains("route reached"), "got: {}", passed);
}

// ============================================================
// Phase 15: Route-Scoped Error Handler Tests
// ============================================================

#[tokio::test]
async fn test_route_error_handler_catches_its_own_route() {
    let port = start_server(|app| {
        app.get_with_error(
            "/users/:id",
            |_req, _res, _next, _params| async move {
                Err(express_rs::Error::with_status(400, "bad id"))
            },
            |err, _req, mut res, _next, _params| async move {
                res.status(err.status())
                    .send(&format!("route: {}", err.message()));
                Ok(res)
            },
        );
        app.get("/ok", |_req, mut res, _next, _params| async move {
            res.send("fine");
            Ok(res)
        });
        app.use_error_handler(|err, _req, mut res, _next, _params| async move {
            res.status(err.status())
                .send(&format!("GLOBAL: {}", err.message()));
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/users/1", None).await;
    assert!(response.contains("400 Bad Request"), "got: {}", response);
    assert!(
        response.contains("route: bad id"),
        "Expected the route-scoped handler, got: {}",
        response
    );
    assert!(
        !response.contains("GLOBAL"),
        "The global handler must not run, got: {}",
        response
    );

    let ok = request(port, "GET", "/ok", None).await;
    assert!(ok.contains("fine"), "got: {}", ok);
}

#[tokio::test]
async fn test_external_errors_skip_route_error_handlers() {
    let port = start_server(|app| {
        app.r#use(|_req, _res, _next, _params| async move {
            Err(express_rs::Error::with_status(418, "from middleware"))
        });
        app.get_with_error(
            "/users/:id",
            |_req, mut res, _next, _params| async move {
                res.send("route reached");
                Ok(res)
            },
            |err, _req, mut res, _next, _params| async move {
                res.status(err.status()).send("route handler ran");
                Ok(res)
            },
        );
        app.use_error_handler(|err, _req, mut res, _next, _params| async move {
            res.status(err.status())
                .send(&format!("GLOBAL: {}", err.message()));
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/users/1", None).await;
    assert!(response.contains("418"), "got: {}", response);
    assert!(
        response.contains("GLOBAL: from middleware"),
        "Expected the global handler, got: {}",
        response
    );
    assert!(
        !response.contains("route handler ran"),
        "The route-scoped handler must not catch foreign errors, got: {}",
        response
    );
}

#[tokio::test]
async fn test_route_error_handler_can_continue() {
    let port = start_server(|app| {
        app.get_with_error(
            "/x",
            |_req, _res, _next, _params| async move {
                Err(express_rs::Error::with_status(500, "boom"))
            },
            |_err, _req, mut res, next, _params| async move {
                res.set("x-recovered", "yes");
                next(express_rs::router::NextCall::Continue).await;
                Ok(res)
            },
        );
        app.use_with_path("/x", |_req, mut res, _next, _params| async move {
            res.send("after recovery");
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/x", None).await;
    assert!(response.contains("x-recovered: yes"), "got: {}", response);
    assert!(
        response.contains("after recovery"),
        "Expected dispatch to continue, got: {}",
        response
    );
}

// ============================================================
// Phase 16: Mounted Router Tests
// ============================================================

use express_rs::router::Router as SubRouter;

#[tokio::test]
async fn test_mounted_router_serves_relative_paths() {
    let port = start_server(|app| {
        let mut api = SubRouter::new();
        api.get("/users", |_req, mut res, _next, _params| async move {
            res.send("all users");
            Ok(res)
        });
        api.get("/users/:id", |_req, mut res, _next, params| async move {
            res.send(&format!("user {}", params.get("id").map_or("", |v| v)));
            Ok(res)
        });
        app.mount("/api", api);
    })
    .await;

    let response = request(port, "GET", "/api/users", None).await;
    let body = response.split("\r\n\r\n").nth(1).unwrap_or("").trim_end();
    assert_eq!(body, "all users", "got: {}", response);

    let response = request(port, "GET", "/api/users/9", None).await;
    let body = response.split("\r\n\r\n").nth(1).unwrap_or("").trim_end();
    assert_eq!(body, "user 9", "got: {}", response);
}

#[tokio::test]
async fn test_mounted_router_falls_through_to_parent() {
    let port = start_server(|app| {
        let mut api = SubRouter::new();
        api.get("/a", |_req, mut res, _next, _params| async move {
            res.send("sub a");
            Ok(res)
        });
        app.mount("/api", api);
        app.get("/api/b", |_req, mut res, _next, _params| async move {
            res.send("parent b");
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/api/a", None).await;
    let body = response.split("\r\n\r\n").nth(1).unwrap_or("").trim_end();
    assert_eq!(body, "sub a");

    let response = request(port, "GET", "/api/b", None).await;
    let body = response.split("\r\n\r\n").nth(1).unwrap_or("").trim_end();
    assert_eq!(
        body, "parent b",
        "Expected parent fallthrough, got: {}",
        response
    );
}

#[tokio::test]
async fn test_next_router_exits_to_parent() {
    let port = start_server(|app| {
        let mut api = SubRouter::new();
        api.r#use(|_req, mut res, next, _params| async move {
            res.set("x-from", "sub-middleware");
            next(express_rs::router::NextCall::Router).await;
            Ok(res)
        });
        api.get("/x", |_req, mut res, _next, _params| async move {
            res.send("sub x");
            Ok(res)
        });
        app.mount("/api", api);
        app.get("/api/x", |_req, mut res, _next, _params| async move {
            res.send("parent x");
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/api/x", None).await;
    assert!(
        response.contains("x-from: sub-middleware"),
        "Expected the sub middleware to run, got: {}",
        response
    );
    let body = response.split("\r\n\r\n").nth(1).unwrap_or("").trim_end();
    assert_eq!(
        body, "parent x",
        "next('router') must skip the sub route, got: {}",
        response
    );
}

#[tokio::test]
async fn test_mounted_error_reaches_parent_error_handler() {
    let port = start_server(|app| {
        let mut api = SubRouter::new();
        api.get("/forbidden", |_req, _res, _next, _params| async move {
            Err(express_rs::Error::with_status(403, "sub says no"))
        });
        app.mount("/api", api);
        app.use_error_handler(|err, _req, mut res, _next, _params| async move {
            res.status(err.status())
                .send(&format!("parent: {}", err.message()));
            Ok(res)
        });
    })
    .await;

    let response = request(port, "GET", "/api/forbidden", None).await;
    assert!(response.contains("403 Forbidden"), "got: {}", response);
    let body = response.split("\r\n\r\n").nth(1).unwrap_or("").trim_end();
    assert_eq!(body, "parent: sub says no", "got: {}", response);
}

#[tokio::test]
async fn test_mount_params_reach_sub_routes() {
    let port = start_server(|app| {
        let mut api = SubRouter::new();
        api.get("/users", |_req, mut res, _next, params| async move {
            res.send(&format!("org={}", params.get("org").map_or("none", |v| v)));
            Ok(res)
        });
        app.mount("/org/:org", api);
    })
    .await;

    let response = request(port, "GET", "/org/acme/users", None).await;
    let body = response.split("\r\n\r\n").nth(1).unwrap_or("").trim_end();
    assert_eq!(body, "org=acme", "got: {}", response);
}
