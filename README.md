# express-rs

**Express.js-style HTTP, rebuilt in Rust.**

An independent Rust implementation inspired by Express.js. This is not an official Express.js project.

## Why this exists

The point of express-rs is to *rewire* an existing idea and learn from the process: study how Express works, rebuild the important pieces in Rust, measure the result, and document what changed and what did not survive the translation. It is a study in framework internals, not an attempt to displace Express.

## Status

**Phase 3: Route Parameters + Wildcards** — complete.

The framework currently provides:

- TCP server with HTTP/1.1 support (via hyper)
- Request parsing: method, path, headers, body, query parameters
- Response writing: status, headers, text, JSON
- Keep-alive connections
- Async request processing
- Routing: `app.get()`, `app.post()`, `app.put()`, `app.patch()`, `app.delete()`, `app.options()`, `app.head()`, `app.all()`
- Middleware: `app.use()` with `next()` support
- Route ordering: first match wins
- Trailing slash handling: optional by default
- **Path parameters:** `:id`, percent-decoded
- **Wildcards:** `*splat`, capturing one or more segments
- **Optional groups:** `{/:op}` and optional suffixes `user{s}`, `{.:ext}`
- **Delimiters:** literal `.` and escaped `\(` `\)` inside paths
- **`next('route')`:** skip to the next matching route
- Case sensitivity toggle (`app.case_sensitive()`)

## Quick Start

```rust
use express_rs::App;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = App::new();

    app.get("/", |_req, mut res, _next, _params| async move {
        res.status(200).text("Hello, World!");
        res
    });

    app.get("/user/:id", |_req, res, _next, params| async move {
        let id = params.get("id").map_or("", |v| v);
        res.text(&format!("user {}", id));
        res
    });

    app.listen(3000).await?;
    Ok(())
}
```

## Architecture

```
src/
  lib.rs       — crate root, public API
  app.rs       — Application struct (user-facing API)
  server.rs    — HTTP server (TCP listener, connection handling)
  router.rs    — Router (layer stack, dispatch, next())
  path.rs      — Path pattern matcher (params, wildcards, optionals)
  request.rs   — Request wrapper (method, path, headers, body, query)
  response.rs  — Response wrapper (status, headers, body)
  error.rs     — Error types
```

### Request flow

```
TCP connection → hyper HTTP/1.1 parser → Incoming request
  → collect body → Request::new()
  → Router::handle()
    → iterate layer stack
    → for each matching layer, execute handlers
    → if handler calls next(), continue to next layer
    → if handler does not call next(), stop
  → Response::into_hyper_response() → hyper writes to TCP
```

### Design decisions

| Decision | Rationale |
|---|---|
| **hyper** for HTTP | Most mature Rust HTTP library. Handles HTTP/1.1 parsing, keep-alive, pipelining, connection management. We build the Express-like abstraction on top. |
| **tokio** for async | Standard Rust async runtime. Required by hyper. |
| **http** crate for types | `Method`, `StatusCode`, `HeaderMap`, `Uri` — standard HTTP types used across the ecosystem. |
| **http-body-util** | Body collection utilities (`BodyExt::collect`). |
| **hyper-util** | Tokio I/O adapter (`TokioIo`) for hyper connections. |
| **Handler returns Response** | The handler takes ownership of `Response`, mutates it, and returns it. This avoids lifetime issues with `&mut Response` in async contexts. |
| **Arc\<Request\>** | Request is wrapped in `Arc` so it can be shared across middleware, matching Express's behavior where the request object is shared between handlers. |
| **Router with layer stack** | Inspired by `pillarjs/router`. Layers are matched in order; `next()` continues to the next matching layer. |
| **Hand-written path matcher** | `path.rs` implements the Express 5 subset of `path-to-regexp`: params, wildcards, optional groups, delimiters, and escapes, with backtracking. No regex crate needed. |
| **Mandatory-variant pass** | Optional groups are resolved by first trying a variant with every group required, which binds `:name` and `.format` correctly instead of letting a greedy param swallow the delimiter. |

### Dependencies

| Crate | Purpose |
|---|---|
| `tokio` | Async runtime (rt-multi-thread, net, macros) |
| `hyper` | HTTP/1.1 server implementation |
| `hyper-util` | Tokio I/O adapter for hyper |
| `http` | HTTP types (Method, StatusCode, HeaderMap, Uri) |
| `http-body-util` | Body collection utilities |
| `bytes` | Efficient byte buffers |

## Compatibility matrix

| Feature | Status | Notes |
|---|---|---|
| HTTP server | ✅ | hyper-backed |
| Request parsing | ✅ | method, path, headers, body, query |
| Response writing | ✅ | status, headers, text, JSON |
| Keep-alive | ✅ | hyper handles this |
| `app.get()` / `app.post()` etc. | ✅ | |
| `app.all()` | ✅ | matches all methods |
| `app.use()` middleware | ✅ | with `next()` support |
| Route ordering | ✅ | first match wins |
| Trailing slash | ✅ | optional by default |
| Route parameters (`:id`) | ✅ | percent-decoded |
| Wildcards (`*splat`) | ✅ | one or more segments |
| Optional groups (`{/:op}`) | ✅ | including `user{s}`, `{.:ext}` |
| Dot delimiter / escapes | ✅ | `.:ext`, `\(`, `\)` |
| `next('route')` | ✅ | skip to next route |
| Case sensitivity | ✅ | `app.case_sensitive()` |
| Strict routing | ⚠️ | flag accepted, not enforced |
| `next('router')` | ❌ | not implemented |
| Error middleware | ❌ | not implemented |
| `app.param()` callbacks | ❌ | not implemented |
| `res.redirect()` | ❌ | not implemented |
| `res.cookie()` | ❌ | not implemented |
| Body parsing (JSON, form) | ❌ | not implemented |
| Static files | ❌ | not implemented |

### Known differences from Express

- Wildcard params are joined with `/` into a single string rather than exposed as an array.
- `req.params` is delivered to handlers as a `HashMap<String, String>` argument instead of living on the request object.
- Strict routing is accepted but not yet enforced; trailing slashes are always optional.
- Handlers must return the `Response`, because Rust ownership makes an out-parameter awkward across an await point.

## Testing

```bash
cargo test
```

Integration tests start a real server on a random port and make actual HTTP requests over TCP.

- 17 unit tests for the path matcher
- 44 integration tests covering the HTTP foundation, routing, middleware, and path syntax

```bash
cargo fmt --check   # formatting
cargo check         # compilation
cargo clippy        # lints
cargo test          # tests
```

## Benchmarks

Not yet run. The plan is to measure hello-world, JSON, parameterized, and middleware-chain endpoints against Express, Fastify, Axum, and Actix Web using `oha` or `wrk`, and publish the methodology alongside the numbers.

## License

MIT

## Attribution

Express.js is MIT licensed. This is an independent Rust implementation inspired by Express.js — no Express source code is copied. Behavior was derived from the public API, the documentation, and reading the Express and `pillarjs/router` sources and their test suites.
