use std::collections::HashMap;
use std::future::Future;
use std::hash::Hash;
use std::result::Result;
use std::sync::{Arc, Mutex};

use futures_util::future::FutureExt;
use tokio::sync::oneshot;

/// Maintains a mapping from request IDs to the channels on which to send their response.
///
/// Provides helper methods for inserting response channels into the structure, while attempting to
/// send the request, propagating any errors that occur when sending the request onto the response
/// channel.
pub struct Awaiting<Id, Success, Error> {
    map: Arc<Mutex<HashMap<Id, oneshot::Sender<Result<Success, Error>>>>>,
}

impl<Id, Success, Error> Awaiting<Id, Success, Error>
where
    Id: Clone + Eq + Hash,
{
    pub fn new() -> Self {
        Self {
            map: Default::default(),
        }
    }

    pub fn register(&self, id: Id, sender: oneshot::Sender<Result<Success, Error>>) {
        self.map.lock().expect("poisoned").insert(id, sender);
    }

    pub fn deregister(&self, id: &Id) -> Option<oneshot::Sender<Result<Success, Error>>> {
        self.map.lock().expect("posoined").remove(&id)
    }

    pub fn send(&self, id: &Id, result: Result<Success, Error>) -> Option<Result<Success, Error>> {
        match self.deregister(id) {
            Some(sender) => {
                let _ = sender.send(result);
                None
            }
            None => Some(result),
        }
    }

    pub fn send_success(&self, id: &Id, success: Success) -> Option<Success> {
        match self.send(id, Ok(success)) {
            Some(Ok(success)) => Some(success),
            _ => None,
        }
    }

    pub async fn register_while<F, R, E>(
        self,
        id: Id,
        sender: oneshot::Sender<Result<Success, Error>>,
        future: F,
    ) where
        F: Future<Output = Result<R, E>>,
        E: Into<Error>,
    {
        self.register(id.clone(), sender);
        let future = future.map(move |result| {
            if let Err(error) = result {
                self.send(&id, Err(error.into()));
            }
        });
        future.await;
    }
}

impl<Id, Success, Error> Clone for Awaiting<Id, Success, Error> {
    fn clone(&self) -> Self {
        Self {
            map: self.map.clone(),
        }
    }
}
