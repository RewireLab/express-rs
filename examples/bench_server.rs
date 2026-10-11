//! Benchmark server for express-rs.
//!
//! Run with `cargo run --release --example bench_server`. Serves the
//! four benchmark endpoints on port 3001 by default, mirroring
//! bench/express_app.js endpoint for endpoint. Override with `PORT_RS`.

use express_rs::router::NextCall;
use express_rs::App;

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

    let port = std::env::var("PORT_RS")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(3001);
    eprintln!("express-rs on {port}");
    app.listen(port).await?;
    Ok(())
}
