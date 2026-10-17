//! Cooperative run cancellation and deadlines; dropping a call cannot undo tool effects.
use std::future::Future;
use std::time::Duration;
use tokio::sync::watch;

#[derive(Debug, Clone)]
pub struct Cancellation(watch::Sender<bool>);

impl Default for Cancellation {
    fn default() -> Self {
        Self(watch::channel(false).0)
    }
}

impl Cancellation {
    pub fn cancel(&self) {
        self.0.send_replace(true);
    }

    pub async fn cancelled(&self) {
        let mut receiver = self.0.subscribe();
        let _ = receiver.wait_for(|cancelled| *cancelled).await;
    }
}

#[derive(Debug, Clone, Default)]
pub struct RemainingBudget {
    pub tokens: Option<u64>,
    pub usd: Option<f64>,
    pub duration: Option<Duration>,
}

#[derive(Debug, Clone, Copy)]
pub enum Stop {
    Cancelled,
    Deadline,
}

pub async fn controlled<T>(
    cancellation: &Cancellation,
    deadline: Option<tokio::time::Instant>,
    future: impl Future<Output = T>,
) -> Result<T, Stop> {
    tokio::select! {
        biased;
        _ = cancellation.cancelled() => Err(Stop::Cancelled),
        _ = async {
            match deadline {
                Some(at) => tokio::time::sleep_until(at).await,
                None => std::future::pending::<()>().await,
            }
        } => Err(Stop::Deadline),
        result = future => Ok(result),
    }
}
