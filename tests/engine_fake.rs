use std::sync::Arc;
use std::time::Duration;

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
    first_sink_write: Arc<Notify>,
    release_first_write: Arc<Notify>,
}

#[async_trait::async_trait]
impl LightSink for FakeSink {
    async fn write(&mut self, colors: ZoneColors) -> anyhow::Result<()> {
        let first = self.writes.lock().await.is_empty();
        if first {
            self.first_write_started.notify_one();
            self.first_sink_write.notify_one();
            self.release_first_write.notified().await;
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
async fn latest_channel_can_take_a_ready_value_without_waiting() {
    let (sender, mut receiver) = latest_channel();
    assert_eq!(receiver.try_recv(), None);
    sender.send(7).unwrap();
    sender.send(9).unwrap();

    assert_eq!(receiver.try_recv(), Some(9));
    assert_eq!(receiver.try_recv(), None);
}

#[tokio::test(start_paused = true)]
async fn engine_writes_first_newest_and_black() {
    let writes = Arc::new(Mutex::new(Vec::new()));
    let first_write_started = Arc::new(Notify::new());
    let first_sink_write = Arc::new(Notify::new());
    let release_first_write = Arc::new(Notify::new());
    let frames_emitted = Arc::new(Notify::new());
    let cancellation = Arc::new(Notify::new());
    let source = FakeSource {
        frames: vec![solid(RED), solid(GREEN), solid(BLUE)].into_iter(),
        first_write_started: first_write_started.clone(),
        frames_emitted: frames_emitted.clone(),
    };
    let sink = FakeSink {
        writes: writes.clone(),
        first_write_started: first_write_started.clone(),
        first_sink_write: first_sink_write.clone(),
        release_first_write: release_first_write.clone(),
    };
    let engine = tokio::spawn(run_engine(
        source,
        sink,
        ZoneLayout::g560_default(),
        SamplerConfig::default(),
        cancellation.clone().notified_owned(),
    ));
    tokio::time::advance(Duration::from_millis(20)).await;
    first_sink_write.notified().await;
    frames_emitted.notified().await;
    release_first_write.notify_one();
    for _ in 0..20 {
        tokio::time::advance(Duration::from_millis(20)).await;
        for _ in 0..4 {
            tokio::task::yield_now().await;
        }
        if writes.lock().await.last() == Some(&ZoneColors([BLUE; 4])) {
            break;
        }
    }
    assert_eq!(writes.lock().await.last(), Some(&ZoneColors([BLUE; 4])));
    cancellation.notify_one();
    let stats = engine.await.unwrap().unwrap();

    assert_eq!(stats.captured_frames, 3);
    assert!(stats.rendered_updates >= 2);
    assert!(stats.dropped_frames >= 1);
    let writes = writes.lock().await;
    assert_ne!(writes[0], ZoneColors::BLACK);
    assert_ne!(writes[0], ZoneColors([RED; 4]));
    assert!(!writes.contains(&ZoneColors([GREEN; 4])));
    assert!(writes.contains(&ZoneColors([BLUE; 4])));
    assert_eq!(*writes.last().unwrap(), ZoneColors::BLACK);
}
