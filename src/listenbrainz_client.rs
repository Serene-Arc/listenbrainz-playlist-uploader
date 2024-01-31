use anyhow::Result;
use governor::{DefaultDirectRateLimiter, Quota, RateLimiter};
use log::debug;
use reqwest::header::AUTHORIZATION;
use reqwest::RequestBuilder;
use reqwest::{Client, Response, StatusCode};
use std::num::NonZeroU32;
use std::time::Duration;
use tokio::time::sleep;

pub struct ListenbrainzClient {
    pub request_client: Client,
    pub user_token: String,
    pub rate_limiter: DefaultDirectRateLimiter,
}

impl ListenbrainzClient {
    pub fn new(user_token: String) -> Self {
        ListenbrainzClient {
            request_client: Client::new(),
            user_token: user_token.to_string(),
            rate_limiter: RateLimiter::direct(
                Quota::per_second(NonZeroU32::new(3).unwrap())
                    .allow_burst(NonZeroU32::new(30).unwrap()),
            ),
        }
    }

    pub async fn take_request_builder(
        &mut self,
        request_builder: RequestBuilder,
    ) -> Result<Response> {
        let request_builder =
            request_builder.header(AUTHORIZATION, format!("Token {}", self.user_token));
        self.rate_limiter.until_ready().await;
        let out = request_builder
            .try_clone()
            .expect("Could not clone request builder")
            .send()
            .await;
        match out {
            Ok(r) => {
                if r.status() == StatusCode::from_u16(429).unwrap() {
                    debug!("Rate limiting error found, waiting and retrying");
                    sleep(Duration::from_secs(15)).await;
                    Ok(request_builder.send().await?)
                } else {
                    Ok(r)
                }
            }
            Err(e) => Err(anyhow::Error::from(e)),
        }
    }
}
