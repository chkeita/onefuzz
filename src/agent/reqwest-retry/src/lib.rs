// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use anyhow::Result;
use async_trait::async_trait;
use backoff::{self, future::retry_notify, ExponentialBackoff};
use onefuzz_telemetry::warn;
use reqwest::{Response, StatusCode};
use std::{
    sync::atomic::{AtomicI32, Ordering},
    time::Duration,
};

use std::error::Error as StdError;
use std::io::ErrorKind;

const DEFAULT_RETRY_PERIOD: Duration = Duration::from_secs(2);
const MAX_ELAPSED_TIME: Duration = Duration::from_secs(30);
const MAX_RETRY_ATTEMPTS: i32 = 5;
const MAX_RETRY_ERROR_MESSAGE: &str = "Maximum number of attempts reached for this request";

fn is_transient_socket_error(error: &reqwest::Error) -> bool {
    let source = error.source();
    if let Some(err) = source {
        if let Some(io_error) = err.downcast_ref::<std::io::Error>() {
            match io_error.kind() {
                ErrorKind::ConnectionAborted
                | ErrorKind::ConnectionReset
                | ErrorKind::ConnectionRefused
                | ErrorKind::TimedOut
                | ErrorKind::NotConnected => {
                    return true;
                }
                _ => (),
            }
        }
    }
    false
}

fn to_backoff_response(
    result: Result<Response, reqwest::Error>,
) -> Result<Response, backoff::Error<anyhow::Error>> {
    match result {
        Err(error) => {
            if error.is_connect() || is_transient_socket_error(&error) {
                Err(backoff::Error::Transient(anyhow::Error::from(error)))
            } else {
                Err(backoff::Error::Permanent(anyhow::Error::from(error)))
            }
        }
        Ok(response) => match response.status() {
            status if status.is_success() => Ok(response),
            StatusCode::REQUEST_TIMEOUT
            | StatusCode::TOO_MANY_REQUESTS
            | StatusCode::INTERNAL_SERVER_ERROR
            | StatusCode::BAD_GATEWAY
            | StatusCode::SERVICE_UNAVAILABLE
            | StatusCode::GATEWAY_TIMEOUT => Ok(response
                .error_for_status()
                .map_err(|error| backoff::Error::Transient(anyhow::Error::from(error)))?),
            _ => Ok(response),
        },
    }
}

pub async fn send_retry_reqwest_default<
    F: Fn() -> Result<reqwest::RequestBuilder> + Send + Sync,
>(
    build_request: F,
) -> Result<Response> {
    send_retry_reqwest(
        build_request,
        DEFAULT_RETRY_PERIOD,
        MAX_ELAPSED_TIME,
        MAX_RETRY_ATTEMPTS,
        to_backoff_response,
    )
    .await
}

pub async fn send_retry_reqwest<
    F: Fn() -> Result<reqwest::RequestBuilder> + Send + Sync,
    F2: Fn(Result<Response, reqwest::Error>) -> Result<Response, backoff::Error<anyhow::Error>>
        + Send
        + Sync,
>(
    build_request: F,
    retry_period: Duration,
    max_elapsed_time: Duration,
    max_retry: i32,
    error_mapper: F2,
) -> Result<Response> {
    let counter = AtomicI32::new(0);
    let op = || async {
        if counter.fetch_add(1, Ordering::SeqCst) >= max_retry {
            Result::<Response, backoff::Error<anyhow::Error>>::Err(backoff::Error::Permanent(
                anyhow::Error::msg(MAX_RETRY_ERROR_MESSAGE),
            ))
        } else {
            let request = build_request().map_err(backoff::Error::Permanent)?;
            let response = request.send().await;
            let mapped = error_mapper(response)?;

            Result::<Response, backoff::Error<anyhow::Error>>::Ok(mapped)
        }
    };

    let backoff_settings = ExponentialBackoff {
        current_interval: retry_period,
        initial_interval: retry_period,
        max_elapsed_time: Some(max_elapsed_time),
        ..ExponentialBackoff::default()
    };

    let notify = |err, _| warn!("Transient error: {}", err);
    let result = retry_notify(backoff_settings, op, notify).await?;
    Ok(result)
}

#[async_trait]
pub trait SendRetry {
    async fn send_retry<
        F: Fn(Result<Response, reqwest::Error>) -> Result<Response, backoff::Error<anyhow::Error>>
            + Send
            + Sync,
    >(
        self,
        retry_period: Duration,
        max_elapsed_time: Duration,
        max_retry: i32,
        error_mapper: F,
    ) -> Result<Response>;
    async fn send_retry_default(self) -> Result<Response>;
}

#[async_trait]
impl SendRetry for reqwest::RequestBuilder {
    async fn send_retry_default(self) -> Result<Response> {
        self.send_retry(
            DEFAULT_RETRY_PERIOD,
            MAX_ELAPSED_TIME,
            MAX_RETRY_ATTEMPTS,
            to_backoff_response,
        )
        .await
    }

    async fn send_retry<
        F: Fn(Result<Response, reqwest::Error>) -> Result<Response, backoff::Error<anyhow::Error>>
            + Send
            + Sync,
    >(
        self,
        retry_period: Duration,
        max_elapsed_time: Duration,
        max_retry: i32,
        response_mapper: F,
    ) -> Result<Response> {
        let result = send_retry_reqwest(
            || {
                self.try_clone().ok_or_else(|| {
                    anyhow::Error::msg("This request cannot be retried because it cannot be cloned")
                })
            },
            retry_period,
            max_elapsed_time,
            max_retry,
            response_mapper,
        )
        .await?;

        Ok(result)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[tokio::test]
    async fn retry_should_pass() -> Result<()> {
        reqwest::Client::new()
            .get("https://www.microsoft.com")
            .send_retry_default()
            .await?
            .error_for_status()?;

        Ok(())
    }

    #[tokio::test]
    async fn retry_should_fail() -> Result<()> {
        let invalid_url = "http://127.0.0.1:81/test.txt";
        let resp = reqwest::Client::new()
            .get(invalid_url)
            .send_retry(
                Duration::from_secs(1),
                Duration::from_secs(300),
                2i32,
                to_backoff_response,
            )
            .await;

        if let Err(err) = &resp {
            let as_text = format!("{}", err);
            assert!(as_text.contains(MAX_RETRY_ERROR_MESSAGE));
        } else {
            anyhow::bail!("response to {} was expected to fail", invalid_url);
        }

        Ok(())
    }
}
