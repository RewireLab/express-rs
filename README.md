# express-rs

**Express.js-style HTTP, rebuilt in Rust.**

An independent Rust implementation inspired by Express.js. This is not an official Express.js project.

## Why this exists

The point of express-rs is to *rewire* an existing idea and learn from the process: study how Express works, rebuild the important pieces in Rust, measure the result, and document what changed and what did not survive the translation. It is a study in framework internals, not an attempt to displace Express.

## Status

**Phase 12: Raw bodies** — complete.

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
- **Static files:** `static_files::serve_static()` and `serve_static_at()` with content types, `ETag`/`Last-Modified` caching, conditional requests, single ranges, index files, and trailing-slash redirects
- **Cookies:** `res.cookie()` and `res.clear_cookie()` with attribute options, and built-in `req.cookies()` parsing
- **File sending:** `static_files::send_file()` and `download()` with root confinement, dotfile policy, conditional requests, and ranges
- **Content negotiation:** `response::format()` picks a body by `Accept`, with extension keys, a default fallback, `Vary: Accept`, and 406 when nothing matches
- **Raw bodies:** `body::raw()` stores the undecoded bytes behind content-type gating, size limits, and inflation

## Quick Start

```rust
use express_rs::App;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = App::new();

    app.get("/", |_req, mut res, _next, _params| async move {
        res.status(200).send("Hello, World!");
        Ok(res)
    });

    app.get("/user/:id", |_req, mut res, _next, params| async move {
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
  cookies.rs   — Cookie parsing and `Set-Cookie` serialisation
  static_files.rs — Static file middleware (serve-static equivalent) plus `send_file()`/`download()` (send equivalent)
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
| **Canonical confinement for static files** | `send` checks `..` lexically and follows symlinks wherever they lead. Paths here are canonicalized and must stay under the root, so a symlink escape answers 404. |
| **Stat-based weak ETags for files** | `send` tags the stat result instead of hashing content, so large files never pay for a digest. Same shape here: `W/"<hexlen>-<hexmtime>"`. |
| **`max_age_secs` in seconds** | Express accepts milliseconds or strings like `"1d"`. Seconds as `u64` need no string parser and no float clamping. |
| **Cookies parsed eagerly on the request** | Express fills `req.cookies` through the `cookie-parser` middleware. The parse is cheap, so it happens in `Request::new` beside the query string and is always available. |
| **`Set-Cookie` bypasses `append()`** | `res.append()` comma-joins values, which would merge distinct cookies into one broken header. `res.cookie()` appends real header instances instead. |
| **No throw for invalid cookie options** | Express throws `TypeError` for a control-character name or `partitioned` without `secure`. A handler here has no error channel, so both cases are dropped silently. |
| **`send_file` errors are `Result`** | Express's `res.sendFile` without a callback forwards failures to `next(err)`; with a callback the handler inspects the error. `Result<Response, Error>` is both — return the `Err` to reach error middleware, or match on it. |
| **Root confinement for `send_file`** | Express resolves through `path.resolve`, which lets an absolute `path` bypass the `root` option entirely. Here an absolute path with a root is refused, and the resolved file must stay under the root — the same stance the static middleware takes on symlinks. |
| **Manual `Default` for option structs** | A derived `Default` zeroed every flag once already (`StaticOptions` in the static-files phase); `SendFileOptions` carries a hand-written impl so the send defaults — ETag, freshness, ranges, cache-control all on — survive. |
| **`format()` is a free function** | It must read the request's `Accept` header, and a `Response` method cannot see the request — the same reason finalisation lives in the server. `response::format(&req, res, |f| ...)`. |

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
| `httpdate` | `Last-Modified` formatting and `If-Modified-Since` parsing |

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
| `app.param()` callbacks | ❌ | not implemented |
| `res.cookie()` | ✅ | `CookieOptions` carries the attributes |
| `res.clearCookie()` | ✅ | `clear_cookie()`; forces epoch expiry |
| Cookie parsing (`req.cookies`) | ✅ | built in; Express needs the `cookie-parser` middleware |
| Signed cookies | ❌ | the `s:` HMAC prefix needs a per-app secret |
| `res.format()` | ✅ | `response::format()`; handlers take the `Response` only |
| `res.sendFile()` | ✅ | `static_files::send_file()`; errors surface as `Result` |
| `res.download()` | ✅ | `static_files::download()`; `Content-Disposition` attachment |
| Body parsing (JSON, text, form) | ✅ | `body::json()`, `body::text()`, `body::urlencoded()` |
| Extended form nesting | ⚠️ | `a[b]` subset, not full `qs` |
| Raw bodies | ✅ | `body::raw()` |
| `zstd` content encoding | ❌ | Node-gated in Express, unsupported here |
| Static files | ✅ | `static_files::serve_static()` / `serve_static_at()` |
| Dotfile policy | ✅ | ignore (default), allow, deny |
| Directory index + redirect | ✅ | `index.html` default, 301 to trailing slash |
| Static conditional + range | ✅ | ETag/date freshness, single ranges, 206/304/412/416 |

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

### Known differences in static files

- Symlink escapes answer 404. `send` follows symlinks outside the root; this confinement is stricter on purpose.
- Files are read fully into memory rather than streamed. Large files pay for one allocation per request.
- The 416 answer carries no `Content-Range` header and the error page shape follows the default error handler (plain text), not `send`'s HTML document.
- `If-Modified-Since` compares at whole-second precision.
- `max_age_secs` takes seconds; Express takes milliseconds or duration strings.
- Extension fallback (`extensions` option) is implemented. Directory listing was never part of `serve-static` and is not implemented either.

### Known differences in cookies

- Signed cookies (`s:`-prefixed HMAC values via cookie-parser) are not implemented; there is no per-app secret to sign with.
- The `encode`/`decode` options do not exist. Values always go out `encodeURIComponent`-encoded and come in percent-decoded; an escape that does not decode is kept raw rather than failing the request the way `cookie-parser` surfaces a 500.
- `max_age_secs` takes seconds and also derives `Expires` from the current time, matching what Express does with `maxAge`.
- `partitioned` without `secure` is dropped silently; Express throws `TypeError`.
- A name carrying a control character is refused (no header is set); Express throws `TypeError`. Other names the `cookie` package accepts — including oddities like `a=b` — pass through.
- `priority: Medium` is omitted from the header because it is the browser default, the same rule Express applies.
- `clear_cookie` drops a caller-supplied `max_age`. Express's `clearCookie` would let it overwrite the epoch expiry and keep the cookie alive.
- When a request carries the same cookie name twice, the first occurrence wins.

### Known differences in file sending

- Without `root`, `path` must be absolute; a relative path is a 500 carrying Express's "path must be absolute or specify root" message. Express throws the equivalent `TypeError` synchronously.
- With `root`, an absolute `path` is refused. Express's `path.resolve(root, path)` would let it escape the root outright; confinement here forbids it.
- The `dotfiles` policy applies to every component of the supplied path, so a dotfile deep inside an absolute path is hidden or refused the same way one under `root` is.
- `extensions`, `start`, and `end` from the `send` options are not supported.
- `Content-Disposition` is a subset of the `content-disposition` package: names within ISO-8859-1 are quoted with `"` and `\` escaped; names outside it gain a percent-encoded `filename*` parameter with a `_`-substituted fallback.
- `download()` sets the header before sending, so a send failure discards it, exactly as Express loses the header to `next(err)`.

### Known differences in content negotiation

- Handlers receive the `Response` only. Express passes `(req, res, next)`; here the request is in scope at the call site, and `next` has no equivalent — the handler returns the response or an `Err`.
- The fallback is a `.default()` method rather than a `default` key in the object.
- Keys go through `req.accepts()`, so extensions and full types both work and the chosen key is reported back as offered; the `Content-Type` is that key normalised to a bare MIME type.
- The 406 travels as an `Err`, so the `Vary: Accept` header `format()` had already set is discarded on the way to the error handler. Express's `finalhandler` writes onto the original response, where the header survives.
- The chosen handler's `send()`/`json()` applies the usual charset rules afterwards, so a text type gains `charset=utf-8` just as it would outside `format()`.

## Testing

```bash
cargo test
```

Integration tests start a real server on a random port and make actual HTTP requests over TCP.

- 115 unit tests across the path matcher, MIME, ETags, request negotiation, errors, body parsing, static helpers, cookies, content disposition, and type normalisation
- 157 integration tests covering the HTTP foundation, routing, middleware, path syntax, the response/request API, error propagation, body parsing (including raw), static files, cookies, file sending, and content negotiation

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
