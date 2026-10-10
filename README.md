# express-rs

**Express.js-style HTTP, rebuilt in Rust.**

An independent Rust implementation inspired by Express.js. This is not an official Express.js project.

## Why this exists

The point of express-rs is to *rewire* an existing idea and learn from the process: study how Express works, rebuild the important pieces in Rust, measure the result, and document what changed and what did not survive the translation. It is a study in framework internals, not an attempt to displace Express.

## Status

**Phase 7: Body parsing** — complete.

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
- **Error handling:** `use_error_handler()`, `next(NextCall::Error(..))`, `Err` from a handler
- **Framing:** `Content-Length`, weak `ETag`, conditional-request `304`, `204`/`205` stripping, HEAD body suppression
- **Content types:** extension resolution including Express 5's `text/javascript` for `.js`
- **Error middleware:** `use_error_handler()`, `next(err)`, and a handler returning `Err` as the equivalent of a rejected promise
- **Default error response:** 500 with the message, `Content-Security-Policy`, and `nosniff`, mirroring `finalhandler`
- **Body parsing:** `body::json()`, `body::text()`, `body::urlencoded()` with content-type gating, `gzip`/`deflate`/`br` decompression, charset checks, size limits, and typed errors

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
        res.send(&format!("user {}", id));
        Ok(res)
    });

    // Error middleware takes the error as its first argument.
    app.use_error_handler(|err, _req, mut res, _next, _params| async move {
        res.status(err.status()).send(&err.message());
        Ok(res)
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
  body.rs      — Body parsing middleware (JSON, text, forms)
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
| **HEAD falls back to GET** | Express's `Route#_handlesMethod` maps HEAD onto GET when no HEAD handler is registered, so `app.get()` routes still answer HEAD requests. |
| **Handlers return `Result<Response, Error>`** | Express 5 forwards a rejected promise to error middleware. `Err` is the Rust equivalent, and it is the only way an `async` handler can report failure given that it returns the response. |
| **Error handlers take the error by value** | A borrowed `&Error` cannot be captured by an `async` block, which would make `async fn` error handlers unwritable. |
| **sha1 for ETag** | Express's ETag format is `"<hexlen>-<27 chars of base64 SHA-1>"`; matching it needs a real SHA-1 rather than a stand-in hash. |
| **`use_error_handler` instead of arity** | Express tells handlers and error handlers apart by `fn.length === 4`. Rust closures cannot be inspected that way, so the distinction lives in the signature: `use_error_handler` takes the error as its first parameter and the compiler enforces it. |
| **Route layers skipped while an error is pending** | Express's `Router#handle` does `if (layerError) { match = false }`, so only middleware can catch an error. Replicated literally. |
| **Parsed body behind a lock** | Handlers receive the request behind an `Arc`, so the parsed body lives in an `RwLock<Option<ParsedBody>>`. Parsers store it; handlers clone it out. |
| **flate2 and brotli for decompression** | body-parser supports `gzip`, `deflate`, and `br` (plus `zstd` on new Node). `flate2` covers the first two from memory, `brotli` the third. |
| **Decompression capped at limit + 1** | The size limit applies to the decompressed stream, as `raw-body` sees it. Reading is capped so a compression bomb cannot exhaust memory. |
| **Hand-rolled form nesting** | `serde_urlencoded` stays flat, so extended `a[b]` nesting is a small parser with the same `parameterLimit` and `depth` defaults as `qs`. |

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
| `flate2` | `gzip` and `deflate` body decompression |
| `brotli` | `br` body decompression |

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
| `next(err)` → error middleware | ✅ | `NextCall::Error` or a returned `Err` |
| Error middleware arity | ⚠️ | `use_error_handler()`, not `fn.length === 4` |
| Route-scoped error handlers | ❌ | needs multi-handler routes |
| Rejected promise → error handler | ✅ | a handler returning `Err` |
| Default 500 on unhandled error | ✅ | message plus CSP and `nosniff` |
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
| Body parsing (JSON, text, form) | ✅ | `body::json()`, `body::text()`, `body::urlencoded()` |
| Extended form nesting | ⚠️ | `a[b]` subset, not full `qs` |
| Raw bodies | ❌ | no raw parser yet |
| `zstd` content encoding | ❌ | Node-gated in Express, unsupported here |
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

### Known differences in error handling

- Handlers return `Result<Response, Error>` rather than `Response`, so an `async` failure has somewhere to go. Express 5's rejected-promise forwarding maps onto this directly.
- Error middleware is registered with `use_error_handler()` and takes the error as its first argument. Express infers this from `fn.length === 4`, which Rust closures cannot expose.
- The error is passed **by value**, since a borrowed reference cannot be captured by an `async` block.
- Returning `Err` discards any headers the handler had already set. Express keeps a single `res` object for the whole request, so headers set before `next(err)` normally survive — and they still do here, as long as the handler returns `Ok(res)` rather than `Err`. `test_returning_err_loses_headers_set_before_it` pins this behaviour.
- **Route-scoped error handlers are not supported.** Express's `app.get(path, fn, errFn)` puts every handler in one route whose own dispatch runs error handlers. Here each registration is a separate layer, and a route layer is skipped the moment an error exists — so an error handler must be registered with `use_error_handler`. Supporting the Express shape needs multi-handler routes.

### Known differences in body parsing

- Extended `a[b]` nesting covers objects, arrays, `[]` appends, and numeric indices — not the full `qs` grammar (`charsetSentinel` and `interpretNumericEntities` do not exist here).
- Only `utf-8`, `iso-8859-1`, and `windows-1252` decode properly; other charsets fall back to lossy UTF-8.
- The JSON strict-violation message differs from V8's `SyntaxError` text; the status (400) and kind (`entity.parse.failed`) match.
- The error `kind` (Express's `.type`) is exposed as `kind()` because `type` is a Rust keyword.
- `verify` receives `(request, bytes, charset)` and returns `Result<(), String>` instead of throwing.

## Testing

```bash
cargo test
```

Integration tests start a real server on a random port and make actual HTTP requests over TCP.

- 81 unit tests across the path matcher, MIME, ETags, request negotiation, errors, and body parsing
- 111 integration tests covering the HTTP foundation, routing, middleware, path syntax, the response/request API, error propagation, and body parsing

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
