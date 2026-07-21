use std::time::Instant;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use tokio::sync::{mpsc, oneshot};

use crate::{LightSink, LightUpdateStatus, ZoneColors};

pub struct EngineWrite {
    pub colors: ZoneColors,
    pub captured_at: Option<Instant>,
    pub response: oneshot::Sender<Result<LightUpdateStatus>>,
}

pub struct EngineBlackout {
    pub response: oneshot::Sender<Result<()>>,
}

pub enum ProxyCommand {
    Write(EngineWrite),
    Blackout(EngineBlackout),
}

/// Small `LightSink` that forwards every write to the service task
/// so the engine can operate against a real sink owned elsewhere.
pub struct ProxySink {
    tx: mpsc::Sender<ProxyCommand>,
}

impl ProxySink {
    pub fn new(tx: mpsc::Sender<ProxyCommand>) -> Self {
        Self { tx }
    }
}

#[async_trait]
impl LightSink for ProxySink {
    async fn write(&mut self, colors: ZoneColors) -> Result<()> {
        let (response, rx) = oneshot::channel();
        self.tx
            .send(ProxyCommand::Write(EngineWrite {
                colors,
                captured_at: None,
                response,
            }))
            .await
            .map_err(|_| anyhow!("service task dropped proxy sink receiver"))?;
        rx.await
            .map_err(|_| anyhow!("service task dropped proxy write response"))??;
        Ok(())
    }

    async fn write_update(
        &mut self,
        colors: ZoneColors,
        captured_at: Instant,
    ) -> Result<LightUpdateStatus> {
        let (response, rx) = oneshot::channel();
        self.tx
            .send(ProxyCommand::Write(EngineWrite {
                colors,
                captured_at: Some(captured_at),
                response,
            }))
            .await
            .map_err(|_| anyhow!("service task dropped proxy sink receiver"))?;
        rx.await
            .map_err(|_| anyhow!("service task dropped proxy write response"))?
    }

    async fn blackout(&mut self) -> Result<()> {
        let (response, rx) = oneshot::channel();
        self.tx
            .send(ProxyCommand::Blackout(EngineBlackout { response }))
            .await
            .map_err(|_| anyhow!("service task dropped proxy sink receiver"))?;
        rx.await
            .map_err(|_| anyhow!("service task dropped proxy blackout response"))??;
        Ok(())
    }
}
