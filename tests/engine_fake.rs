use std::sync::Arc;

use logilightshow::{
    FrameSource, LightSink, Rgb8, RgbFrame, SamplerConfig, ZoneColors, ZoneLayout, latest_channel,
    run_engine,
};
use tokio::sync::{Mutex, Notify};

const RED: Rgb8 = Rgb8 { r: 255, g: 0, b: 0 };
const GREEN: Rgb8 = Rgb8 { r: 0, g: 255, b: 0 };
const BLUE: Rgb8 = Rgb8 { r: 0, g: 0, b: 255 };

fn solid(color: Rgb8) -> RgbFrame {
    let mut pixels = Vec::with_capacity(12);
    for _ in 0..4 {
        pixels.extend_from_slice(&[color.r, color.g, color.b]);
    }
    RgbFrame::new(2, 2, 6, pixels).unwrap()
}

struct FakeSource {
    frames: std::vec::IntoIter<RgbFrame>,
    first_write_started: Arc<Notify>,
    frames_emitted: Arc<Notify>,
}

#[async_trait::async_trait]
impl FrameSource for FakeSource {
    async fn next_frame(&mut self) -> anyhow::Result<Option<RgbFrame>> {
        if self.frames.len() == 2 {
            self.first_write_started.notified().await;
        }
        let frame = self.frames.next();
        match frame {
            Some(frame) => Ok(Some(frame)),
            None => {
                self.frames_emitted.notify_one();
                std::future::pending().await
            }
        }
    }
}

struct FakeSink {
    writes: Arc<Mutex<Vec<ZoneColors>>>,
    first_write_started: Arc<Notify>,
    release_first_write: Arc<Notify>,
    second_write_started: Arc<Notify>,
}

#[async_trait::async_trait]
impl LightSink for FakeSink {
    async fn write(&mut self, colors: ZoneColors) -> anyhow::Result<()> {
        let first = self.writes.lock().await.is_empty();
        if first {
            self.first_write_started.notify_one();
            self.release_first_write.notified().await;
        } else {
            self.second_write_started.notify_one();
        }
        self.writes.lock().await.push(colors);
        Ok(())
    }
}

#[tokio::test]
async fn latest_channel_replaces_unread_values() {
    let (sender, mut receiver) = latest_channel();
    sender.send(1).unwrap();
    sender.send(2).unwrap();
    sender.send(3).unwrap();

    assert_eq!(receiver.recv().await, Some(3));
}

#[tokio::test]
async fn engine_writes_first_newest_and_black() {
    let writes = Arc::new(Mutex::new(Vec::new()));
    let first_write_started = Arc::new(Notify::new());
    let release_first_write = Arc::new(Notify::new());
    let frames_emitted = Arc::new(Notify::new());
    let cancellation = Arc::new(Notify::new());
    let second_write_started = Arc::new(Notify::new());
    let source = FakeSource {
        frames: vec![solid(RED), solid(GREEN), solid(BLUE)].into_iter(),
        first_write_started: first_write_started.clone(),
        frames_emitted: frames_emitted.clone(),
    };
    let sink = FakeSink {
        writes: writes.clone(),
        first_write_started,
        release_first_write: release_first_write.clone(),
        second_write_started: second_write_started.clone(),
    };
    let engine = tokio::spawn(run_engine(
        source,
        sink,
        ZoneLayout::g560_default(),
        SamplerConfig::default(),
        cancellation.clone().notified_owned(),
    ));
    frames_emitted.notified().await;
    release_first_write.notify_one();
    second_write_started.notified().await;
    cancellation.notify_one();
    let stats = engine.await.unwrap().unwrap();

    assert_eq!(stats.captured_frames, 3);
    assert_eq!(stats.rendered_updates, 2);
    assert_eq!(stats.dropped_frames, 1);
    assert_eq!(
        *writes.lock().await,
        vec![
            ZoneColors([RED; 4]),
            ZoneColors([BLUE; 4]),
            ZoneColors::BLACK
        ]
    );
}
