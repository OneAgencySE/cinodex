use core::panic;

use base64::encode;
use reqwest::{
    header::{HeaderMap, HeaderValue},
    Client,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Deserialize, Debug)]
struct Token {
    access_token: String,
    refresh_token: String,
}

pub const API_PATH: &'static str = "https://api.cinode.com/v0.1/companies/31";

#[derive(Clone)] // The inner values are not worth an arc, the client itself is wrapped in an ARC internally
pub struct CINodeClient {
    pub inner: reqwest::Client,
}

impl CINodeClient {
    pub async fn init(access: &str) -> Result<CINodeClient, Box<dyn std::error::Error>> {
        let _ = tokio::fs::create_dir("cache").await;
        let token = encode(&access);
        let client = reqwest::Client::new();
        let token = client
            .get("https://api.cinode.com/token")
            .header("Authorization", format!("Basic {}", token))
            .send()
            .await?
            .json::<Token>()
            .await?;

        let mut headers = HeaderMap::new();
        headers.append(
            "Authorization",
            HeaderValue::from_str(&format!("Bearer {}", token.access_token))?,
        );
        let client = Client::builder().default_headers(headers).build()?;

        Ok(CINodeClient { inner: client })
    }

    pub async fn get_cached<T: DeserializeOwned + Default + Serialize>(&self, path: String) -> T {
        let file_path = format!("cache/{:x}.json", md5::compute(&path));

        if let Ok(mut file) = tokio::fs::File::open(&file_path).await {
            let mut buffer = String::new();
            file.read_to_string(&mut buffer).await.unwrap();
            serde_json::from_str(&buffer).expect(&format!("disk-json error: {}", buffer))
        } else {
            let text_response = self
                .inner
                .get(&path)
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap();

            // Make sure the JSON is correct before printing it to disk
            // Return default if the json is invalid
            match serde_json::from_str::<T>(&text_response) {
                Ok(value) => {
                    let mut file = tokio::fs::File::create(&file_path).await.unwrap();
                    file.write_all(text_response.as_bytes()).await.unwrap();
                    file.flush().await.unwrap();
                    value
                }
                Err(e) => {
                    println!("Error: {}", e);
                    if text_response.contains("Too many requests for 24 hrs.") {
                        panic!("Too many requests for 24 hrs.");
                    } else {
                        T::default()
                    }
                }
            }
        }
    }
}
