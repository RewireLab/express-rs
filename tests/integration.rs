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
        app.get("/", |_req, mut res, _next| async move {
            res.status(200).text("Hello, World!");
            res
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
        response.contains("text/plain"),
        "Expected text/plain content type, got: {}",
        response
    );
}

#[tokio::test]
async fn test_custom_status_code() {
    let port = start_server(|app| {
        app.get("/missing", |_req, mut res, _next| async move {
            res.status(404).text("Not Found");
            res
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
        app.get("/", |_req, mut res, _next| async move {
            res.set_header("X-Custom", "test-value")
                .set_header("X-Another", "another-value")
                .text("OK");
            res
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
        app.post("/submit", |req, mut res, _next| async move {
            let body = req.body_text();
            res.text(&format!("Received: {}", body));
            res
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
        app.get("/search", |req, mut res, _next| async move {
            let name = req.query_param("name").unwrap_or("unknown");
            let count = req.query_param("count").unwrap_or("0");
            res.text(&format!("name={}, count={}", name, count));
            res
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
        app.get("/search", |req, mut res, _next| async move {
            let q = req.query_param("q").unwrap_or("");
            res.text(&format!("q={}", q));
            res
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
        app.get("/", |_req, mut res, _next| async move {
            res.text("OK");
            res
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
        app.get("/", |_req, mut res, _next| async move {
            res.text("OK");
            res
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
        app.post("/", |req, mut res, _next| async move {
            let body = req.body_text();
            res.text(&format!("Body length: {}", body.len()));
            res
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
        app.post("/", |req, mut res, _next| async move {
            let body = req.body_text();
            res.text(&format!("Body length: {}", body.len()));
            res
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
        app.get("/api/data", |_req, mut res, _next| async move {
            res.status(200).json(r#"{"message":"hello","count":42}"#);
            res
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
        app.get("/", |_req, mut res, _next| async move {
            res.text("OK");
            res
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
        app.get("/", |_req, mut res, _next| async move {
            res.text("Still alive");
            res
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
        app.get("/resource", |_req, mut res, _next| async move {
            res.text("GET");
            res
        });
        app.post("/resource", |_req, mut res, _next| async move {
            res.text("POST");
            res
        });
        app.put("/resource", |_req, mut res, _next| async move {
            res.text("PUT");
            res
        });
        app.delete("/resource", |_req, mut res, _next| async move {
            res.text("DELETE");
            res
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
        app.get("/resource", |_req, mut res, _next| async move {
            res.text("GET");
            res
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
        app.get("/users", |_req, mut res, _next| async move {
            res.text("users list");
            res
        });
        app.get("/users/123", |_req, mut res, _next| async move {
            res.text("user 123");
            res
        });
        app.get("/posts", |_req, mut res, _next| async move {
            res.text("posts list");
            res
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
        app.r#use(|_req, mut res, next| async move {
            res.set_header("X-Middleware", "hit");
            next().await;
            res
        });
        app.get("/", |_req, mut res, _next| async move {
            res.text("OK");
            res
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
        app.use_with_path("/api", |_req, mut res, next| async move {
            res.set_header("X-API", "true");
            next().await;
            res
        });
        app.get("/api/users", |_req, mut res, _next| async move {
            res.text("users");
            res
        });
        app.get("/other", |_req, mut res, _next| async move {
            res.text("other");
            res
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
        app.r#use(|_req, mut res, next| async move {
            res.set_header("X-First", "1");
            next().await;
            res
        });
        app.r#use(|_req, mut res, next| async move {
            res.set_header("X-Second", "2");
            next().await;
            res
        });
        app.get("/", |_req, mut res, _next| async move {
            res.text("done");
            res
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
        app.r#use(|_req, mut res, _next| async move {
            res.set_header("X-Blocked", "true");
            res.text("blocked");
            res
        });
        app.get("/", |_req, mut res, _next| async move {
            res.text("should not reach");
            res
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
        app.get("/users", |_req, mut res, _next| async move {
            res.text("users");
            res
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
        app.all("/resource", |_req, mut res, _next| async move {
            res.text("any method");
            res
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
        app.get("/resource", |_req, mut res, _next| async move {
            res.text("first");
            res
        });
        app.get("/resource", |_req, mut res, _next| async move {
            res.text("second");
            res
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
        app.r#use(|_req, mut res, next| async move {
            res.set_header("X-Before", "true");
            next().await;
            res
        });
        app.get("/", |_req, mut res, _next| async move {
            res.set_header("X-Handler", "true");
            res.text("OK");
            res
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
        app.get("/resource", |_req, mut res, _next| async move {
            res.text("body");
            res
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
        app.r#use(|_req, mut res, next| async move {
            next().await;
            res.set_header("X-After", "true");
            res
        });
        app.get("/", |_req, mut res, _next| async move {
            res.text("OK");
            res
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
        app.r#use(|_req, mut res, next| async move {
            res.set_header("X-M1", "1");
            next().await;
            res
        });
        app.r#use(|_req, mut res, next| async move {
            res.set_header("X-M2", "2");
            next().await;
            res
        });
        app.get("/", |_req, mut res, _next| async move {
            res.set_header("X-Handler", "H");
            res.text("OK");
            res
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
