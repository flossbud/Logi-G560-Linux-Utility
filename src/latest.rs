use std::sync::{Arc, Mutex};

use tokio::sync::watch;

pub struct LatestSender<T> {
    inner: watch::Sender<Option<T>>,
    state: Arc<Mutex<LatestState>>,
}

pub struct LatestReceiver<T> {
    inner: watch::Receiver<Option<T>>,
    state: Arc<Mutex<LatestState>>,
}

#[derive(Default)]
struct LatestState {
    unread: bool,
}

pub fn latest_channel<T>() -> (LatestSender<T>, LatestReceiver<T>) {
    let (sender, receiver) = watch::channel(None);
    let state = Arc::new(Mutex::new(LatestState::default()));
    (
        LatestSender {
            inner: sender,
            state: state.clone(),
        },
        LatestReceiver {
            inner: receiver,
            state,
        },
    )
}

impl<T> LatestSender<T> {
    /// Sends a value, returning whether it replaced an unread value.
    pub fn send(&self, value: T) -> Result<bool, T> {
        let mut state = self.state.lock().expect("latest channel lock poisoned");
        let replaced = state.unread;
        match self.inner.send(Some(value)) {
            Ok(()) => {
                state.unread = true;
                Ok(replaced)
            }
            Err(error) => Err(error.0.expect("latest channel sends only present values")),
        }
    }
}

impl<T: Clone> LatestReceiver<T> {
    pub fn try_recv(&mut self) -> Option<T> {
        let mut state = self.state.lock().expect("latest channel lock poisoned");
        if !state.unread {
            return None;
        }
        let value = self.inner.borrow_and_update().clone();
        state.unread = false;
        value
    }

    pub async fn recv(&mut self) -> Option<T> {
        loop {
            if self.inner.changed().await.is_err() {
                return None;
            }

            let mut state = self.state.lock().expect("latest channel lock poisoned");
            if let Some(value) = self.inner.borrow_and_update().clone() {
                state.unread = false;
                return Some(value);
            }
        }
    }
}
