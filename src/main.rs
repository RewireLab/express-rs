use express_rs::App;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = App::new();

    app.get("/", |_req, mut res, _next, _params| async move {
        res.status(200).send("Hello from express-rs!");
        Ok(res)
    });

    app.use_error_handler(|err, _req, mut res, _next, _params| async move {
        res.status(err.status()).send(&err.message());
        Ok(res)
    });

    println!("Server listening on http://127.0.0.1:3000");
    app.listen(3000).await?;
    Ok(())
}
