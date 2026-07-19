use tokio::sync::watch;

pub struct LatestSender<T> {
    inner: watch::Sender<Option<T>>,
}

pub struct LatestReceiver<T> {
    inner: watch::Receiver<Option<T>>,
}

pub fn latest_channel<T>() -> (LatestSender<T>, LatestReceiver<T>) {
    let (sender, receiver) = watch::channel(None);
    (
        LatestSender { inner: sender },
        LatestReceiver { inner: receiver },
    )
}

impl<T> LatestSender<T> {
    pub fn send(&self, value: T) -> Result<(), T> {
        self.inner
            .send(Some(value))
            .map_err(|error| error.0.expect("latest channel sends only present values"))
    }
}

impl<T: Clone> LatestReceiver<T> {
    pub async fn recv(&mut self) -> Option<T> {
        loop {
            if self.inner.changed().await.is_err() {
                return None;
            }

            if let Some(value) = self.inner.borrow_and_update().clone() {
                return Some(value);
            }
        }
    }
}
