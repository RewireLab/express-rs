# express-rs

**An Express.js-inspired HTTP framework written in Rust.**

`express-rs` brings familiar Express 5 routing and middleware patterns to an asynchronous Rust server built on Tokio and Hyper. It is an independent project, not an official Express.js implementation, and aims for practical compatibility rather than a complete port.

## Features

- HTTP/1.1 server with asynchronous handlers and keep-alive connections
- Express-style routing, middleware, mounted routers, and route parameters
- Path parameters, wildcards, optional groups, case-sensitive and strict routing
- Request helpers for headers, content negotiation, cookies, and query data
- Response helpers for text, JSON, redirects, cookies, attachments, and conditional requests
- Error middleware and route-scoped error handling
- JSON, text, URL-encoded, and raw body parsers, with size limits and supported content encodings
- Static-file serving and file downloads with cache validators, dotfile policies, and byte ranges

## Getting started

Add the repository as a dependency in `Cargo.toml`:

```toml
[dependencies]
express-rs = { git = "https://github.com/RewireLab/express-rs" }
serde_json = "1"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

Create `src/main.rs`:

```rust
use express_rs::App;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = App::new();

    app.get("/", |_req, mut res, _next, _params| async move {
        res.send("Hello, world!");
        Ok(res)
    });

    app.get("/users/:id", |_req, mut res, _next, params| async move {
        let id = params.get("id").map_or("", String::as_str);
        res.json(&serde_json::json!({ "id": id }));
        Ok(res)
    });

    app.listen(3000).await?;
    Ok(())
}
```

Run the application with `cargo run`, then visit `http://localhost:3000`.

Handlers receive the request, response, continuation function, and matched parameters. They return `Result<Response, Error>`; returning `Err` forwards the error to error middleware.

## Compatibility

The API follows familiar Express conventions while using Rust-native types and ownership. This project implements a useful subset of Express 5; it does not promise drop-in compatibility.

Notable differences include:

- Route parameters are passed to handlers as a `HashMap<String, String>` rather than through `req.params`.
- Wildcard parameters are joined into one string rather than returned as an array.
- Handlers return the response, and failures use `Result`; Rust closures do not use JavaScript function arity to identify error handlers.
- Error middleware is registered with `use_error_handler()`. Route-scoped error handling pairs one route handler with one error handler.
- Some Express helpers have Rust names, such as `content_type()` in place of `type()`.
- Proxy headers are not trusted; `protocol()` reports `http` until trust-proxy support is added.
- Extended URL-encoded parsing supports common nested forms but not the full `qs` grammar.
- Static and file responses buffer the selected bytes in memory. Range responses read only the requested range.

The implementation also differs in smaller edge cases. See the source and tests for details before relying on exact Express error messages or option behavior.

## Performance

The included benchmark compares the Rust example server with a small Express 5 app using matching endpoints. This is a microbenchmark, not a general application performance claim.

Latest recorded run:

| Endpoint | express-rs (req/s) | Express 5 (req/s) | Throughput ratio | p99 express-rs | p99 Express 5 |
|---|---:|---:|---:|---:|---:|
| `GET /hello` | 44,031 | 17,952 | 2.45× | 0.0 ms | 1.0 ms |
| `GET /json` | 39,655 | 17,631 | 2.25× | 0.0 ms | 1.0 ms |
| `GET /user/42` | 45,625 | 16,177 | 2.82× | 0.0 ms | 1.0 ms |
| `GET /chain` (5 middleware) | 45,409 | 17,958 | 2.53× | 0.0 ms | 1.0 ms |

In this run, all endpoints passed the harness's 1.5× throughput threshold with zero errors. The aggregate throughput ratio was approximately 2.51×. Results vary by machine and run; p99 latency is rounded to milliseconds, and the load generator may limit the measured throughput of the faster server. Repeat measurements when evaluating changes.

The harness builds express-rs in release mode and measures both servers back-to-back with 8 `autocannon` connections, a 2-second warm-up, and a 10-second sample per endpoint. The recorded Express run used Node v26.8.1.

```sh
./bench/run.sh
```

To use alternate ports:

```sh
PORT_RS=3101 PORT_JS=3102 ./bench/run.sh
```

The harness requires Node.js and npm/npx. It can also be tuned with `CONNECTIONS`, `WARMUP`, `DURATION`, `ENDPOINTS`, and `MIN_RATIO` environment variables.

## Development

Run the formatter, compiler, linter, and test suite with:

```sh
cargo fmt --check
cargo check
cargo clippy
cargo test
```

Integration tests start the server on a random local port and exercise it over HTTP.

## Architecture

The server uses Hyper for HTTP/1.1 connections and Tokio for asynchronous I/O. The application layer provides the Express-like API; the router evaluates middleware and route layers in registration order. Request and response wrappers expose framework helpers while using types from Rust's `http` ecosystem.

Key implementation modules:

| Module | Responsibility |
|---|---|
| `app` | Application API and route registration |
| `server` | Listener, connection handling, and response finalization |
| `router` | Middleware stack, route dispatch, and continuation flow |
| `path` | Route pattern parsing and matching |
| `request`, `response` | Request and response helpers |
| `body`, `cookies` | Body parsing and cookie handling |
| `static_files` | Static serving, file sending, and downloads |
| `mime`, `etag`, `error` | Content types, entity tags, and errors |

## License

MIT

## Attribution

Express.js is MIT licensed. This project is an independent implementation; it does not copy Express source code. Its behavior is informed by public documentation, APIs, and test suites, including those for Express and `pillarjs/router`.
