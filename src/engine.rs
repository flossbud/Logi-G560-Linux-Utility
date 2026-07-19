use std::{future::Future, time::Instant};

use anyhow::Result;

use crate::{Region, RgbFrame, SamplerConfig, ZoneColors, latest_channel, sampler::sample_zones};

#[async_trait::async_trait]
pub trait FrameSource: Send {
    async fn next_frame(&mut self) -> Result<Option<RgbFrame>>;
}

#[async_trait::async_trait]
pub trait LightSink: Send {
    async fn write(&mut self, colors: ZoneColors) -> Result<()>;
}

pub struct EngineStats {
    pub captured_frames: u64,
    pub rendered_updates: u64,
    pub dropped_frames: u64,
    pub capture_to_write: hdrhistogram::Histogram<u64>,
}

#[derive(Clone)]
struct SampledUpdate {
    captured_at: Instant,
    colors: ZoneColors,
}

pub async fn run_engine<S, L, C>(
    mut source: S,
    mut sink: L,
    regions: [Region; 4],
    config: SamplerConfig,
    cancellation: C,
) -> Result<EngineStats>
where
    S: FrameSource + 'static,
    L: LightSink + 'static,
    C: Future<Output = ()> + Send,
{
    let (sender, mut receiver) = latest_channel::<SampledUpdate>();
    let writer = tokio::spawn(async move {
        let mut rendered_updates = 0_u64;
        let mut capture_to_write = hdrhistogram::Histogram::<u64>::new(3)?;
        let mut write_result = Ok(());

        while let Some(update) = receiver.recv().await {
            if let Err(error) = sink.write(update.colors).await {
                write_result = Err(error);
                break;
            }
            rendered_updates += 1;
            let latency_micros = u64::try_from(update.captured_at.elapsed().as_micros())
                .unwrap_or(u64::MAX)
                .max(1);
            capture_to_write.record(latency_micros)?;
        }

        let blackout_result = sink.write(ZoneColors::BLACK).await;
        write_result.and(blackout_result)?;
        Ok::<_, anyhow::Error>((rendered_updates, capture_to_write))
    });

    tokio::pin!(cancellation);
    let mut captured_frames = 0_u64;
    loop {
        let frame = tokio::select! {
            () = &mut cancellation => break,
            frame = source.next_frame() => frame?,
        };
        let Some(frame) = frame else {
            break;
        };

        captured_frames += 1;
        let captured_at = Instant::now();
        let colors =
            tokio::task::spawn_blocking(move || sample_zones(&frame, &regions, config)).await?;
        if sender
            .send(SampledUpdate {
                captured_at,
                colors,
            })
            .is_err()
        {
            break;
        }
    }

    drop(sender);
    let (rendered_updates, capture_to_write) = writer.await??;
    Ok(EngineStats {
        captured_frames,
        rendered_updates,
        dropped_frames: captured_frames.saturating_sub(rendered_updates),
        capture_to_write,
    })
}
