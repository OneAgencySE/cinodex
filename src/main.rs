#[macro_use]
extern crate lazy_static;

extern crate dotenv;
extern crate dotenv_codegen;

use client::CINodeClient;
use extractor::FileExtractor;

mod client;
mod extractor;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv::dotenv().ok();
    let access = dotenv_codegen::dotenv!("ACCESS");
    let client = CINodeClient::init(access).await?;
    FileExtractor::new(client).extract().await;
    Ok(())
}
