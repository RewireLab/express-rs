# express-rs

**Express.js-style HTTP, rebuilt in Rust.**

An independent Rust implementation inspired by Express.js. This is not an official Express.js project.

## Why this exists

The point of express-rs is to *rewire* an existing idea and learn from the process: study how Express works, rebuild the important pieces in Rust, measure the result, and document what changed and what did not survive the translation. It is a study in framework internals, not an attempt to displace Express.

## Status

**Phase 4/5: Request + Response** — complete.

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
- **Response:** `send()`, `json()`, `sendStatus()`, `set()`/`get()`/`append()`, `content_type()`, `location()`, `redirect()`, `vary()`, `status()`
- **Request:** `header()`, `accepts()`, `acceptsEncodings()`/`acceptsCharsets()`/`acceptsLanguages()`, `is()`, `host()`/`hostname()`, `protocol()`/`secure()`, `xhr()`
- **Framing:** `Content-Length`, weak `ETag`, conditional-request `304`, `204`/`205` stripping, HEAD body suppression
- **Content types:** extension resolution including Express 5's `text/javascript` for `.js`

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
  mime.rs      — Content type resolution and charset handling
  etag.rs      — Entity tag generation and comparison
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
| **Finalisation in the server** | Express applies `Content-Length`, ETag, freshness, and HEAD suppression inside `res.send()`, where the response can see the request. Here the handler owns the response, so the server applies those steps after it returns, in the same order Express does. |
| **HeadBody** | A body that reports a length but yields no bytes. hyper derives framing from the body it is handed, so simply emptying the body for HEAD would also drop `Content-Length`. |
| **serde for JSON** | `res.json(&value)` serialises any `Serialize` type rather than taking a pre-built string, which is the useful shape in Rust. `json_str()` covers the raw case. |
| **sha1 for ETag** | Express's ETag format is `"<hexlen>-<27 chars of base64 SHA-1>"`; matching it needs a real SHA-1 rather than a stand-in hash. |
| **HEAD falls back to GET** | Express's `Route#_handlesMethod` maps HEAD onto GET when no HEAD handler is registered, so `app.get()` routes still answer HEAD requests. |

### Dependencies

| Crate | Purpose |
|---|---|
| `tokio` | Async runtime (rt-multi-thread, net, macros) |
| `hyper` | HTTP/1.1 server implementation |
| `hyper-util` | Tokio I/O adapter for hyper |
| `http` | HTTP types (Method, StatusCode, HeaderMap, Uri) |
| `http-body` | `Body` trait, needed for the HEAD body type |
| `http-body-util` | Body collection utilities |
| `bytes` | Efficient byte buffers |
| `serde` / `serde_json` | JSON serialisation for `res.json()` |
| `sha1` | ETag generation |

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
| `res.send()` | ✅ | string, typed JSON, bytes |
| `res.json()` | ✅ | any `Serialize` value |
| `res.sendStatus()` | ✅ | reason phrase as text |
| `res.set()` / `get()` / `append()` | ✅ | |
| `res.content_type()` | ✅ | Express's `type()`; `type` is a Rust keyword |
| `res.location()` / `redirect()` | ✅ | Express 5 `redirect(status, url)` order |
| `res.vary()` | ✅ | de-duplicates on a second add |
| `res.status()` validation | ⚠️ | out-of-range codes ignored, not thrown |
| Weak ETag on `send()` | ✅ | matches the `etag` package format |
| Conditional request → 304 | ✅ | `If-None-Match` |
| HEAD body suppression | ✅ | keeps `Content-Length` |
| `req.header()` | ✅ | `referer`/`referrer` interchangeable |
| `req.accepts()` family | ✅ | returns the offered string, not the MIME type |
| `req.is()` | ✅ | full type or extension |
| `req.host()` / `hostname()` | ✅ | Express 5 keeps the port |
| `req.xhr()` | ✅ | |
| `req.fresh()` | ✅ | ETag branch; date comparison not implemented |
| Strict routing | ⚠️ | flag accepted, not enforced |
| `next('router')` | ❌ | not implemented |
| Error middleware | ❌ | not implemented |
| `app.param()` callbacks | ❌ | not implemented |
| `res.cookie()` | ❌ | not implemented |
| `res.format()` | ❌ | not implemented |
| `res.sendFile()` | ❌ | not implemented |
| Body parsing (JSON, form) | ❌ | not implemented |
| Static files | ❌ | not implemented |

### Known differences from Express

- Wildcard params are joined with `/` into a single string rather than exposed as an array.
- `req.params` is delivered to handlers as a `HashMap<String, String>` argument instead of living on the request object.
- Strict routing is accepted but not yet enforced; trailing slashes are always optional.
- Handlers must return the `Response`, because Rust ownership makes an out-parameter awkward across an await point.
- `res.status()` ignores out-of-range codes instead of throwing `TypeError`/`RangeError`, because a handler here returns the `Response` rather than a `Result` and has no channel to report the failure.
- `res.redirect()` takes the Express 5 `(status, url)` order only; the legacy `(url, status)` order is gone.
- `content_type()` replaces `type()`, which is a reserved word in Rust.
- `append()` comma-joins values into one header, so `Set-Cookie` is not supported.
- `protocol()` always reports `http`; `X-Forwarded-Proto` is not trusted without a trust-proxy setting.
- `req.fresh()` compares ETags but not `If-Modified-Since` timestamps, which would need a date parser.
- `accepts()` returns the string that was offered rather than the resolved MIME type.

## Testing

```bash
cargo test
```

Integration tests start a real server on a random port and make actual HTTP requests over TCP.

- 52 unit tests across the path matcher, MIME resolution, ETags, and request negotiation
- 68 integration tests covering the HTTP foundation, routing, middleware, path syntax, and the response/request API

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
