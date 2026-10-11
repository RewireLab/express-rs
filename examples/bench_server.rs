//! Benchmark server for express-rs.
//!
//! Run with `cargo run --release --example bench_server`. Serves the
//! four benchmark endpoints on port 3001, mirroring bench/express_app.js
//! endpoint for endpoint.

use express_rs::App;
use express_rs::router::NextCall;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = App::new();

    // A five-deep middleware chain, as the /chain endpoint's setup.
    for _ in 0..5 {
        app.r#use(|_req, mut res, next, _params| async move {
            res.set("x-chain", "1");
            next(NextCall::Continue).await;
            Ok(res)
        });
    }

    app.get("/hello", |_req, mut res, _next, _params| async move {
        res.send("Hello, World!");
        Ok(res)
    });

    app.get("/json", |_req, mut res, _next, _params| async move {
        res.json(&serde_json::json!({ "message": "Hello, World!" }));
        Ok(res)
    });

    app.get("/user/:id", |_req, mut res, _next, params| async move {
        res.send(&format!("user {}", params.get("id").map_or("", |v| v)));
        Ok(res)
    });

    app.get("/chain", |_req, mut res, _next, _params| async move {
        res.send("chained");
        Ok(res)
    });

    app.listen(3001).await?;
    Ok(())
}
