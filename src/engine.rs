use std::{future::Future, sync::Arc, time::Instant};

use anyhow::{Context, Result, anyhow};
use tokio::sync::watch;

use crate::{Region, RgbFrame, SamplerConfig, ZoneColors, latest_channel, sampler::sample_zones};

#[async_trait::async_trait]
pub trait FrameSource: Send {
    async fn next_frame(&mut self) -> Result<Option<RgbFrame>>;

    async fn shutdown(&mut self) -> Result<()> {
        Ok(())
    }
}

#[async_trait::async_trait]
pub trait LightSink: Send {
    async fn write(&mut self, colors: ZoneColors) -> Result<()>;
}

#[derive(Debug)]
pub struct EngineStats {
    pub captured_frames: u64,
    pub rendered_updates: u64,
    pub dropped_frames: u64,
    pub capture_to_write: hdrhistogram::Histogram<u64>,
}

#[derive(Clone)]
struct CapturedFrame {
    captured_at: Instant,
    frame: RgbFrame,
}

#[derive(Clone)]
struct SampledUpdate {
    captured_at: Instant,
    colors: ZoneColors,
}

pub async fn run_engine<S, L, C>(
    source: S,
    sink: L,
    regions: [Region; 4],
    config: SamplerConfig,
    cancellation: C,
) -> Result<EngineStats>
where
    S: FrameSource + 'static,
    L: LightSink + 'static,
    C: Future<Output = ()> + Send,
{
    run_engine_with_sampler(source, sink, regions, config, cancellation, sample_zones).await
}

async fn run_engine_with_sampler<S, L, C, F>(
    mut source: S,
    mut sink: L,
    regions: [Region; 4],
    config: SamplerConfig,
    cancellation: C,
    sampler: F,
) -> Result<EngineStats>
where
    S: FrameSource + 'static,
    L: LightSink + 'static,
    C: Future<Output = ()> + Send,
    F: Fn(&RgbFrame, &[Region; 4], SamplerConfig) -> ZoneColors + Send + Sync + 'static,
{
    let (cancel_sender, cancel_receiver) = watch::channel(false);
    let (frame_sender, mut frame_receiver) = latest_channel::<CapturedFrame>();
    let capture_cancel = cancel_receiver.clone();
    let capture_cancel_sender = cancel_sender.clone();
    let capture = tokio::spawn(async move {
        let mut captured = 0_u64;
        let mut replacements = 0_u64;
        let mut cancel = capture_cancel;
        let capture_result: Result<()> = loop {
            let next = tokio::select! {
                biased;
                changed = cancel.changed() => {
                    if changed.is_err() || *cancel.borrow() { break Ok(()); }
                    continue;
                }
                next = source.next_frame() => next,
            };
            match next {
                Ok(Some(frame)) => {
                    captured += 1;
                    match frame_sender.send(CapturedFrame {
                        captured_at: Instant::now(),
                        frame,
                    }) {
                        Ok(replaced) => replacements += u64::from(replaced),
                        Err(_) => break Ok(()),
                    }
                }
                Ok(None) => break Ok(()),
                Err(error) => {
                    let _ = capture_cancel_sender.send(true);
                    break Err(error);
                }
            }
        };
        let shutdown_result = source
            .shutdown()
            .await
            .context("frame source shutdown failed");
        if shutdown_result.is_err() {
            let _ = capture_cancel_sender.send(true);
        }
        match (capture_result, shutdown_result) {
            (Ok(()), Ok(())) => Ok((captured, replacements)),
            (Err(error), Ok(())) => Err(error),
            (Ok(()), Err(error)) => Err(error),
            (Err(primary), Err(shutdown)) => Err(anyhow!(
                "{primary:#}; frame source shutdown also failed: {shutdown:#}"
            )),
        }
    });

    let (update_sender, mut update_receiver) = latest_channel::<SampledUpdate>();
    let writer = tokio::spawn(async move {
        let mut rendered = 0_u64;
        let mut histogram = hdrhistogram::Histogram::<u64>::new(3)?;
        let mut write_error = None;
        while let Some(update) = update_receiver.recv().await {
            match sink.write(update.colors).await {
                Ok(()) => {
                    rendered += 1;
                    let micros = u64::try_from(update.captured_at.elapsed().as_micros())
                        .unwrap_or(u64::MAX)
                        .max(1);
                    if let Err(error) = histogram.record(micros) {
                        write_error =
                            Some(anyhow::Error::new(error).context("latency record failed"));
                        break;
                    }
                }
                Err(error) => {
                    write_error = Some(error);
                    break;
                }
            }
        }
        let blackout = sink.write(ZoneColors::BLACK).await;
        match (write_error, blackout) {
            (None, Ok(())) => Ok((rendered, histogram)),
            (Some(error), Ok(())) => Err(error.context("light update failed")),
            (None, Err(error)) => Err(error.context("blackout failed")),
            (Some(error), Err(blackout)) => Err(anyhow!(
                "light update failed: {error:#}; blackout also failed: {blackout:#}"
            )),
        }
    });

    tokio::pin!(cancellation);
    let sampler = Arc::new(sampler);
    let mut sampled_replacements = 0_u64;
    let mut sampling_error = None;
    let mut cancel = cancel_receiver;
    'sampling: loop {
        let captured = tokio::select! {
            biased;
            () = &mut cancellation => {
                let _ = cancel_sender.send(true);
                break;
            }
            changed = cancel.changed() => {
                if changed.is_err() || *cancel.borrow() { break; }
                continue;
            }
            captured = frame_receiver.recv() => captured,
        };
        let Some(captured) = captured else { break };
        let CapturedFrame { captured_at, frame } = captured;
        let sampler = sampler.clone();
        let sample = tokio::task::spawn_blocking(move || sampler(&frame, &regions, config));
        tokio::pin!(sample);
        let colors = tokio::select! {
            biased;
            () = &mut cancellation => {
                let _ = cancel_sender.send(true);
                break 'sampling;
            }
            changed = cancel.changed() => {
                if changed.is_err() || *cancel.borrow() { break 'sampling; }
                continue 'sampling;
            }
            result = &mut sample => match result {
                Ok(colors) => colors,
                Err(error) => {
                    sampling_error = Some(anyhow::Error::new(error).context("sampling task failed"));
                    let _ = cancel_sender.send(true);
                    break 'sampling;
                }
            },
        };
        if *cancel.borrow() {
            break;
        }
        match update_sender.send(SampledUpdate {
            captured_at,
            colors,
        }) {
            Ok(replaced) => sampled_replacements += u64::from(replaced),
            Err(_) => break,
        }
    }

    let _ = cancel_sender.send(true);
    drop(frame_receiver);
    let capture_join = capture.await;
    drop(update_sender);
    let writer_result = writer.await.context("writer task failed to join")?;

    let mut capture_stats = None;
    let capture_error = match capture_join {
        Ok(Ok(stats)) => {
            capture_stats = Some(stats);
            None
        }
        Ok(Err(error)) => Some(error.context("frame capture failed")),
        Err(error) => Some(anyhow::Error::new(error).context("capture task failed to join")),
    };
    let primary_error = sampling_error.or(capture_error);
    match (primary_error, writer_result) {
        (Some(primary), Err(shutdown)) => Err(anyhow!(
            "{primary:#}; engine shutdown also failed: {shutdown:#}"
        )),
        (Some(primary), Ok(_)) => Err(primary),
        (None, Err(error)) => Err(error),
        (None, Ok((rendered_updates, capture_to_write))) => {
            let (captured_frames, frame_replacements) =
                capture_stats.expect("successful capture has statistics");
            Ok(EngineStats {
                captured_frames,
                rendered_updates,
                dropped_frames: frame_replacements + sampled_replacements,
                capture_to_write,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    use tokio::sync::Notify;

    use super::{FrameSource, LightSink, run_engine_with_sampler};
    use crate::{Region, Rgb8, RgbFrame, SamplerConfig, ZoneColors};

    fn frame(value: u8) -> RgbFrame {
        RgbFrame::new(1, 1, 3, vec![value, 0, 0]).unwrap()
    }

    fn regions() -> [Region; 4] {
        [Region {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        }; 4]
    }

    struct TestSource {
        frames: std::vec::IntoIter<RgbFrame>,
        error: bool,
        pending_at_end: bool,
        release_after_first: Option<Arc<Notify>>,
        ended: Option<Arc<Notify>>,
        shutdowns: Option<Arc<AtomicUsize>>,
    }

    #[async_trait::async_trait]
    impl FrameSource for TestSource {
        async fn next_frame(&mut self) -> anyhow::Result<Option<RgbFrame>> {
            if self.frames.len() == 2
                && let Some(release) = self.release_after_first.take()
            {
                release.notified().await;
            }
            if let Some(frame) = self.frames.next() {
                return Ok(Some(frame));
            }
            if self.error {
                self.error = false;
                anyhow::bail!("fake capture failure");
            }
            if self.pending_at_end {
                return std::future::pending().await;
            }
            if let Some(ended) = self.ended.take() {
                ended.notify_one();
            }
            Ok(None)
        }

        async fn shutdown(&mut self) -> anyhow::Result<()> {
            if let Some(shutdowns) = &self.shutdowns {
                shutdowns.fetch_add(1, Ordering::SeqCst);
            }
            Ok(())
        }
    }

    struct RecordingSink {
        writes: Arc<Mutex<Vec<ZoneColors>>>,
        fail_blackout: bool,
    }

    #[async_trait::async_trait]
    impl LightSink for RecordingSink {
        async fn write(&mut self, colors: ZoneColors) -> anyhow::Result<()> {
            self.writes.lock().unwrap().push(colors);
            if self.fail_blackout && colors == ZoneColors::BLACK {
                anyhow::bail!("fake blackout failure");
            }
            Ok(())
        }
    }

    #[tokio::test]
    async fn capture_replaces_pending_frames_while_sampling_is_blocked() {
        let writes = Arc::new(Mutex::new(Vec::new()));
        let started = Arc::new(Notify::new());
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let sampled = Arc::new(Mutex::new(Vec::new()));
        let ended = Arc::new(Notify::new());
        let sampler = {
            let started = started.clone();
            let gate = gate.clone();
            let sampled = sampled.clone();
            move |frame: &RgbFrame, _: &[Region; 4], _: SamplerConfig| {
                let value = frame.pixels[0];
                sampled.lock().unwrap().push(value);
                if value == 1 {
                    started.notify_waiters();
                    let (lock, ready) = &*gate;
                    let guard = lock.lock().unwrap();
                    drop(ready.wait_while(guard, |open| !*open).unwrap());
                }
                ZoneColors(
                    [Rgb8 {
                        r: value,
                        g: 0,
                        b: 0,
                    }; 4],
                )
            }
        };
        let engine = tokio::spawn(run_engine_with_sampler(
            TestSource {
                frames: vec![frame(1), frame(2), frame(3)].into_iter(),
                error: false,
                pending_at_end: false,
                release_after_first: Some(started.clone()),
                ended: Some(ended.clone()),
                shutdowns: None,
            },
            RecordingSink {
                writes,
                fail_blackout: false,
            },
            regions(),
            SamplerConfig::default(),
            std::future::pending(),
            sampler,
        ));
        started.notified().await;
        ended.notified().await;
        let (lock, ready) = &*gate;
        *lock.lock().unwrap() = true;
        ready.notify_one();
        let stats = engine.await.unwrap().unwrap();

        assert_eq!(*sampled.lock().unwrap(), vec![1, 3]);
        assert_eq!(stats.dropped_frames, 1);
    }

    #[tokio::test]
    async fn cancellation_during_sampling_never_publishes_the_sample() {
        let writes = Arc::new(Mutex::new(Vec::new()));
        let started = Arc::new(Notify::new());
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let cancellation = Arc::new(Notify::new());
        let sampler = {
            let started = started.clone();
            let gate = gate.clone();
            move |_: &RgbFrame, _: &[Region; 4], _: SamplerConfig| {
                started.notify_waiters();
                let (lock, ready) = &*gate;
                let guard = lock.lock().unwrap();
                drop(ready.wait_while(guard, |open| !*open).unwrap());
                ZoneColors([Rgb8 { r: 9, g: 0, b: 0 }; 4])
            }
        };
        let engine = tokio::spawn(run_engine_with_sampler(
            TestSource {
                frames: vec![frame(1)].into_iter(),
                error: false,
                pending_at_end: true,
                release_after_first: None,
                ended: None,
                shutdowns: None,
            },
            RecordingSink {
                writes: writes.clone(),
                fail_blackout: false,
            },
            regions(),
            SamplerConfig::default(),
            cancellation.clone().notified_owned(),
            sampler,
        ));
        started.notified().await;
        cancellation.notify_one();
        tokio::task::yield_now().await;
        let (lock, ready) = &*gate;
        *lock.lock().unwrap() = true;
        ready.notify_one();
        engine.await.unwrap().unwrap();

        assert_eq!(*writes.lock().unwrap(), vec![ZoneColors::BLACK]);
    }

    #[tokio::test]
    async fn source_error_still_awaits_blackout() {
        let writes = Arc::new(Mutex::new(Vec::new()));
        let result = run_engine_with_sampler(
            TestSource {
                frames: Vec::new().into_iter(),
                error: true,
                pending_at_end: false,
                release_after_first: None,
                ended: None,
                shutdowns: None,
            },
            RecordingSink {
                writes: writes.clone(),
                fail_blackout: false,
            },
            regions(),
            SamplerConfig::default(),
            std::future::pending(),
            |_, _, _| ZoneColors::BLACK,
        )
        .await;

        assert!(result.unwrap_err().to_string().contains("capture"));
        assert_eq!(*writes.lock().unwrap(), vec![ZoneColors::BLACK]);
    }

    #[tokio::test]
    async fn sampling_panic_still_awaits_blackout() {
        let writes = Arc::new(Mutex::new(Vec::new()));
        let result = run_engine_with_sampler(
            TestSource {
                frames: vec![frame(1)].into_iter(),
                error: false,
                pending_at_end: false,
                release_after_first: None,
                ended: None,
                shutdowns: None,
            },
            RecordingSink {
                writes: writes.clone(),
                fail_blackout: false,
            },
            regions(),
            SamplerConfig::default(),
            std::future::pending(),
            |_, _, _| panic!("fake sampling panic"),
        )
        .await;

        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("sampling task failed")
        );
        assert_eq!(*writes.lock().unwrap(), vec![ZoneColors::BLACK]);
    }

    #[tokio::test]
    async fn blackout_failure_is_propagated() {
        let result = run_engine_with_sampler(
            TestSource {
                frames: Vec::new().into_iter(),
                error: false,
                pending_at_end: false,
                release_after_first: None,
                ended: None,
                shutdowns: None,
            },
            RecordingSink {
                writes: Arc::new(Mutex::new(Vec::new())),
                fail_blackout: true,
            },
            regions(),
            SamplerConfig::default(),
            std::future::pending(),
            |_, _, _| ZoneColors::BLACK,
        )
        .await;

        assert!(result.unwrap_err().to_string().contains("blackout failed"));
    }

    #[tokio::test]
    async fn engine_invokes_frame_source_shutdown_on_teardown() {
        let shutdowns = Arc::new(AtomicUsize::new(0));
        run_engine_with_sampler(
            TestSource {
                frames: Vec::new().into_iter(),
                error: false,
                pending_at_end: false,
                release_after_first: None,
                ended: None,
                shutdowns: Some(shutdowns.clone()),
            },
            RecordingSink {
                writes: Arc::new(Mutex::new(Vec::new())),
                fail_blackout: false,
            },
            regions(),
            SamplerConfig::default(),
            std::future::pending(),
            |_, _, _| ZoneColors::BLACK,
        )
        .await
        .unwrap();

        assert_eq!(shutdowns.load(Ordering::SeqCst), 1);
    }
}
