use express_rs::App;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = App::new();

    app.handler(|_req, mut res| async move {
        res.status(200).text("Hello from express-rs!");
        res
    });

    println!("Server listening on http://127.0.0.1:3000");
    app.listen(3000).await?;
    Ok(())
}
