//! Integration tests for the HTTP foundation.
//!
//! These tests start a real server on a random port and make actual HTTP
//! requests over TCP.

use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Start a server with the given handler and return its port.
async fn start_server<H, F>(handler: H) -> u16
where
    H: Fn(express_rs::Request, express_rs::Response) -> F + Send + Sync + 'static,
    F: std::future::Future<Output = express_rs::Response> + Send + 'static,
{
    // Find a free port
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    // Start the server
    tokio::spawn(async move {
        let mut app = express_rs::App::new();
        app.handler(handler);
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
    let port = start_server(|_req, mut res| async move {
        res.status(200).text("Hello, World!");
        res
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
    let port = start_server(|_req, mut res| async move {
        res.status(404).text("Not Found");
        res
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
    let port = start_server(|_req, mut res| async move {
        res.set_header("X-Custom", "test-value")
            .set_header("X-Another", "another-value")
            .text("OK");
        res
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
    let port = start_server(|req, mut res| async move {
        let body = req.body_text();
        res.text(&format!("Received: {}", body));
        res
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
    let port = start_server(|req, mut res| async move {
        let name = req.query_param("name").unwrap_or("unknown");
        let count = req.query_param("count").unwrap_or("0");
        res.text(&format!("name={}, count={}", name, count));
        res
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
    let port = start_server(|req, mut res| async move {
        let q = req.query_param("q").unwrap_or("");
        res.text(&format!("q={}", q));
        res
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
    let port = start_server(|_req, mut res| async move {
        res.text("OK");
        res
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
    let port = start_server(|_req, mut res| async move {
        res.text("OK");
        res
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
    let port = start_server(|req, mut res| async move {
        let body = req.body_text();
        res.text(&format!("Body length: {}", body.len()));
        res
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
    let port = start_server(|req, mut res| async move {
        let body = req.body_text();
        res.text(&format!("Body length: {}", body.len()));
        res
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
    let port = start_server(|_req, mut res| async move {
        res.status(200).json(r#"{"message":"hello","count":42}"#);
        res
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
    let port = start_server(|_req, mut res| async move {
        res.text("OK");
        res
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
    let port = start_server(|_req, mut res| async move {
        res.text("Still alive");
        res
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
