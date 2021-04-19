// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use crate::{jitter::random_delay, utils::CheckNotify};
use anyhow::Result;
use futures::Future;
use reqwest::Url;
use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
    time::Duration,
};
use storage_queue::{QueueClient, QueueClientImpl};
use tokio::{sync::Notify, task, task::JoinHandle};
use serde::Serialize;

const DEFAULT_HEARTBEAT_PERIOD: Duration = Duration::from_secs(60 * 5);

pub struct HeartbeatContext<TContext, T, TMessage> {
    pub state: TContext,
    pub queue_client: Box<dyn QueueClient<TMessage>>,
    pub pending_messages: Mutex<HashSet<T>>,
    pub cancelled: Notify,
}

pub struct HeartbeatClient<TContext, T, TMessage>
where
    TContext: Send+Sync,
    T: Clone + Send + Sync,
    TMessage: Send + Sync + Serialize
{
    pub context: Arc<HeartbeatContext<TContext, T, TMessage>>,
    pub heartbeat_process: JoinHandle<Result<()>>,
}

impl<TContext, T, TMessage> Drop for HeartbeatClient<TContext, T, TMessage>
where
    TContext: Send+Sync,
    T: Clone + Sync + Send,
    TMessage: Send + Sync + Serialize
{
    fn drop(&mut self) {
        self.context.cancelled.notify_one();
    }
}

impl<TContext, T, TMessage> HeartbeatClient<TContext, T, TMessage>
where
    TContext: Send+Sync,
    T: Clone + Sync + Send,
    TMessage: Send + Sync + Serialize
{
    pub fn drain_current_messages(context: Arc<HeartbeatContext<TContext, T, TMessage>>) -> Vec<T> {
        let lock = context.pending_messages.lock();
        let mut messages = lock.unwrap();
        let drain = messages.iter().cloned().collect::<Vec<T>>();
        messages.clear();
        drain
    }

    // pub async fn start_background_process<Fut>(
    //     queue_url: Url,
    //     messages: Arc<Mutex<HashSet<T>>>,
    //     cancelled: Arc<Notify>,
    //     heartbeat_period: Duration,
    //     flush: impl Fn(Arc<dyn QueueClient>, Arc<Mutex<HashSet<T>>>) -> Fut,
    // ) -> Result<()>
    // where
    //     Fut: Future<Output = ()> + Send,
    // {
    //     let queue_client = Arc::new(QueueClient::new(queue_url)?);
    //     flush(queue_client.clone(), messages.clone()).await;
    //     while !cancelled.is_notified(heartbeat_period).await {
    //         flush(queue_client.clone(), messages.clone()).await;
    //     }
    //     flush(queue_client.clone(), messages.clone()).await;
    //     Ok(())
    // }

    pub fn init_heartbeat<F, Fut>(
        context: TContext,
        queue_url: Url,
        heartbeat_period: Option<Duration>,
        flush: F,
    ) -> Result<HeartbeatClient<TContext, T, TMessage>>
    where
        F: Fn(Arc<HeartbeatContext<TContext, T, TMessage>>) -> Fut + Sync + Send + 'static,
        Fut: Future<Output = ()> + Send,
        T: 'static,
        TContext: Send + Sync + 'static,
    {
        let heartbeat_period = heartbeat_period.unwrap_or(DEFAULT_HEARTBEAT_PERIOD);

        let context = Arc::new(HeartbeatContext {
            state: context,
            queue_client:Box::new( QueueClientImpl::new(queue_url)?),
            pending_messages: Mutex::new(HashSet::<T>::new()),
            cancelled: Notify::new(),
        });

        let flush_context = context.clone();
        let heartbeat_process = task::spawn(async move {
            random_delay(heartbeat_period).await;
            flush(flush_context.clone()).await;
            while !flush_context.cancelled.is_notified(heartbeat_period).await {
                flush(flush_context.clone()).await;
            }
            flush(flush_context.clone()).await;
            Ok(())
        });

        Ok(HeartbeatClient {
            context,
            heartbeat_process,
        })
    }
}
