use std::{
    io::Cursor,
    thread,
    time::{Duration, Instant},
};

use pipewire as pw;
use pw::{properties::properties, spa};
use spa::{
    buffer::DataType,
    param::{
        ParamType,
        format::{FormatProperties, MediaSubtype, MediaType},
        format_utils,
        video::{VideoFormat, VideoInfoRaw},
    },
    pod::{Pod, Value, serialize::PodSerializer},
};
use tokio::sync::mpsc;

use super::{CaptureError, gstreamer::proportional_dimensions};
use crate::{FrameSource, LatestReceiver, LatestSender, RgbFrame, latest_channel};

const GAMESCOPE_NODE_NAME: &str = "gamescope";
const OUTPUT_WIDTH: u32 = 160;
const FRAME_INTERVAL: Duration = Duration::from_millis(50);

enum WorkerCommand {
    Stop,
}

struct WorkerData {
    format: VideoInfoRaw,
    frame_sender: LatestSender<RgbFrame>,
    error_sender: mpsc::UnboundedSender<CaptureError>,
    next_frame_due: Option<Instant>,
}

/// Portal-free capture of Gamescope's native PipeWire video source.
///
/// Gamescope offers both DMA-BUF and MemFd buffers. Advertising BGRx without
/// a modifier intentionally selects MemFd, which PipeWire maps into ordinary
/// CPU memory. This avoids the black GPU readback seen with Bazzite's current
/// Mesa/GStreamer stack.
pub struct GamescopeFrameSource {
    frame_receiver: LatestReceiver<RgbFrame>,
    error_receiver: mpsc::UnboundedReceiver<CaptureError>,
    command_sender: Option<pw::channel::Sender<WorkerCommand>>,
    worker: Option<thread::JoinHandle<()>>,
    shutdown: bool,
}

impl GamescopeFrameSource {
    pub async fn open() -> Result<Self, CaptureError> {
        let (frame_sender, frame_receiver) = latest_channel();
        let (error_sender, error_receiver) = mpsc::unbounded_channel();
        let (command_sender, command_receiver) = pw::channel::channel();
        let (startup_sender, startup_receiver) = std::sync::mpsc::sync_channel(1);
        let worker_error_sender = error_sender.clone();

        let worker = thread::Builder::new()
            .name("logilightshow-pipewire".to_owned())
            .spawn(move || {
                if let Err(message) = run_worker(
                    frame_sender,
                    error_sender,
                    command_receiver,
                    &startup_sender,
                ) && startup_sender.send(Err(message.clone())).is_err()
                {
                    let _ = worker_error_sender.send(pipewire_error("stream", message));
                }
            })
            .map_err(|error| pipewire_error("worker start", error))?;

        let startup = tokio::task::spawn_blocking(move || {
            startup_receiver.recv_timeout(Duration::from_secs(5))
        })
        .await
        .map_err(|error| pipewire_error("worker startup wait", error))?;

        match startup {
            Ok(Ok(())) => Ok(Self {
                frame_receiver,
                error_receiver,
                command_sender: Some(command_sender),
                worker: Some(worker),
                shutdown: false,
            }),
            Ok(Err(message)) => {
                let _ = worker.join();
                Err(pipewire_error("stream setup", message))
            }
            Err(error) => {
                let _ = command_sender.send(WorkerCommand::Stop);
                let _ = worker.join();
                Err(pipewire_error("worker startup wait", error))
            }
        }
    }

    pub async fn shutdown(&mut self) -> Result<(), CaptureError> {
        if self.shutdown {
            return Ok(());
        }
        self.shutdown = true;

        if let Some(sender) = self.command_sender.take() {
            let _ = sender.send(WorkerCommand::Stop);
        }
        if let Some(worker) = self.worker.take() {
            tokio::task::spawn_blocking(move || worker.join())
                .await
                .map_err(|error| pipewire_error("worker join task", error))?
                .map_err(|_| pipewire_error("worker join", "capture worker panicked"))?;
        }
        Ok(())
    }
}

impl Drop for GamescopeFrameSource {
    fn drop(&mut self) {
        if !self.shutdown
            && let Some(sender) = self.command_sender.take()
        {
            let _ = sender.send(WorkerCommand::Stop);
        }
    }
}

#[async_trait::async_trait]
impl FrameSource for GamescopeFrameSource {
    async fn next_frame(&mut self) -> anyhow::Result<Option<RgbFrame>> {
        tokio::select! {
            biased;
            error = self.error_receiver.recv() => match error {
                Some(error) => Err(error.into()),
                None => Ok(None),
            },
            frame = self.frame_receiver.recv() => Ok(frame),
        }
    }

    async fn shutdown(&mut self) -> anyhow::Result<()> {
        GamescopeFrameSource::shutdown(self)
            .await
            .map_err(Into::into)
    }
}

fn run_worker(
    frame_sender: LatestSender<RgbFrame>,
    error_sender: mpsc::UnboundedSender<CaptureError>,
    command_receiver: pw::channel::Receiver<WorkerCommand>,
    startup_sender: &std::sync::mpsc::SyncSender<Result<(), String>>,
) -> Result<(), String> {
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None).map_err(display)?;
    let context = pw::context::ContextRc::new(&mainloop, None).map_err(display)?;
    let core = context.connect_rc(None).map_err(display)?;

    let _commands = command_receiver.attach(mainloop.loop_(), {
        let mainloop = mainloop.clone();
        move |command| match command {
            WorkerCommand::Stop => mainloop.quit(),
        }
    });

    let stream = pw::stream::StreamBox::new(
        &core,
        "LogiLightShow Gamescope capture",
        properties! {
            *pw::keys::MEDIA_TYPE => "Video",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Screen",
            *pw::keys::TARGET_OBJECT => GAMESCOPE_NODE_NAME,
            *pw::keys::NODE_NAME => "logilightshow-gamescope-capture",
        },
    )
    .map_err(display)?;

    let _listener = stream
        .add_local_listener_with_user_data(WorkerData {
            format: VideoInfoRaw::new(),
            frame_sender,
            error_sender,
            next_frame_due: None,
        })
        .state_changed(|_, data, old, new| match new {
            pw::stream::StreamState::Error(message) => {
                let _ = data
                    .error_sender
                    .send(pipewire_error("Gamescope stream", message));
            }
            pw::stream::StreamState::Unconnected if old != pw::stream::StreamState::Unconnected => {
                let _ = data.error_sender.send(pipewire_error(
                    "Gamescope stream",
                    "capture node disconnected",
                ));
            }
            _ => {}
        })
        .param_changed(|_, data, id, param| {
            if id != ParamType::Format.as_raw() {
                return;
            }
            let Some(param) = param else {
                data.format = VideoInfoRaw::new();
                return;
            };
            let Ok((media_type, media_subtype)) = format_utils::parse_format(param) else {
                return;
            };
            if media_type != MediaType::Video || media_subtype != MediaSubtype::Raw {
                return;
            }
            if let Err(error) = data.format.parse(param) {
                let _ = data
                    .error_sender
                    .send(pipewire_error("format negotiation", error));
            }
        })
        .process(|stream, data| {
            let now = Instant::now();
            if !advance_frame_deadline(&mut data.next_frame_due, now) {
                let _ = stream.dequeue_buffer();
                return;
            }

            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            let datas = buffer.datas_mut();
            let Some(plane) = datas.first_mut() else {
                return;
            };
            match frame_from_mapped_bgrx(data.format, plane) {
                Ok(frame) => {
                    let _ = data.frame_sender.send(frame);
                }
                Err(error) => {
                    let _ = data.error_sender.send(error);
                }
            }
        })
        .register()
        .map_err(display)?;

    let format = spa::pod::object!(
        spa::utils::SpaTypes::ObjectParamFormat,
        ParamType::EnumFormat,
        spa::pod::property!(FormatProperties::MediaType, Id, MediaType::Video),
        spa::pod::property!(FormatProperties::MediaSubtype, Id, MediaSubtype::Raw),
        spa::pod::property!(FormatProperties::VideoFormat, Id, VideoFormat::BGRx),
    );
    let bytes = PodSerializer::serialize(Cursor::new(Vec::new()), &Value::Object(format))
        .map_err(display)?
        .0
        .into_inner();
    let pod = Pod::from_bytes(&bytes).ok_or_else(|| "invalid serialized format pod".to_owned())?;
    let mut params = [pod];

    stream
        .connect(
            spa::utils::Direction::Input,
            None,
            pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
            &mut params,
        )
        .map_err(display)?;
    startup_sender
        .send(Ok(()))
        .map_err(|_| "capture opener disappeared during setup".to_owned())?;

    mainloop.run();
    Ok(())
}

fn advance_frame_deadline(next_frame_due: &mut Option<Instant>, now: Instant) -> bool {
    if next_frame_due.is_some_and(|due| now < due) {
        return false;
    }
    let mut next_due = next_frame_due.unwrap_or_else(|| now + FRAME_INTERVAL);
    while next_due <= now {
        next_due += FRAME_INTERVAL;
    }
    *next_frame_due = Some(next_due);
    true
}

fn frame_from_mapped_bgrx(
    format: VideoInfoRaw,
    plane: &mut spa::buffer::Data,
) -> Result<RgbFrame, CaptureError> {
    const BYTES_PER_PIXEL: usize = 4;

    if format.format() != VideoFormat::BGRx {
        return Err(pipewire_error(
            "frame decode",
            format!("unsupported video format {:?}", format.format()),
        ));
    }
    if !matches!(plane.type_(), DataType::MemFd | DataType::MemPtr) {
        return Err(pipewire_error(
            "frame decode",
            format!(
                "expected CPU-mapped MemFd buffer, received {:?}",
                plane.type_()
            ),
        ));
    }

    let width = usize::try_from(format.size().width).map_err(frame_layout_error)?;
    let height = usize::try_from(format.size().height).map_err(frame_layout_error)?;
    let (target_width, target_height) =
        proportional_dimensions(format.size().width, format.size().height, OUTPUT_WIDTH)
            .ok_or_else(|| frame_layout_error("invalid Gamescope dimensions"))?;
    let target_width = usize::try_from(target_width).map_err(frame_layout_error)?;
    let target_height = usize::try_from(target_height).map_err(frame_layout_error)?;

    let offset = usize::try_from(plane.chunk().offset()).map_err(frame_layout_error)?;
    let stride = usize::try_from(plane.chunk().stride())
        .map_err(|_| frame_layout_error("negative Gamescope buffer stride"))?;
    let minimum_stride = width
        .checked_mul(BYTES_PER_PIXEL)
        .ok_or_else(|| frame_layout_error("Gamescope row size overflow"))?;
    if stride < minimum_stride {
        return Err(frame_layout_error(format!(
            "Gamescope stride {stride} is smaller than the {minimum_stride}-byte row"
        )));
    }
    let required = offset
        .checked_add(
            stride
                .checked_mul(height.saturating_sub(1))
                .ok_or_else(|| frame_layout_error("Gamescope frame size overflow"))?,
        )
        .and_then(|value| value.checked_add(minimum_stride))
        .ok_or_else(|| frame_layout_error("Gamescope frame size overflow"))?;
    let source = plane
        .data()
        .ok_or_else(|| frame_layout_error("PipeWire did not map the Gamescope MemFd"))?;
    if source.len() < required {
        return Err(frame_layout_error(format!(
            "mapped Gamescope buffer has {} bytes but its layout requires {required}",
            source.len()
        )));
    }

    let target_stride = target_width
        .checked_mul(3)
        .ok_or_else(|| frame_layout_error("scaled RGB row size overflow"))?;
    let mut pixels = vec![0; target_stride * target_height];
    for target_y in 0..target_height {
        let source_y = target_y * height / target_height;
        for target_x in 0..target_width {
            let source_x = target_x * width / target_width;
            let source_offset = offset + source_y * stride + source_x * BYTES_PER_PIXEL;
            let target_offset = target_y * target_stride + target_x * 3;
            pixels[target_offset] = source[source_offset + 2];
            pixels[target_offset + 1] = source[source_offset + 1];
            pixels[target_offset + 2] = source[source_offset];
        }
    }

    RgbFrame::new(target_width, target_height, target_stride, pixels).map_err(frame_layout_error)
}

fn pipewire_error(stage: &'static str, error: impl std::fmt::Display) -> CaptureError {
    CaptureError::PipeWire {
        stage,
        message: error.to_string(),
    }
}

fn frame_layout_error(error: impl std::fmt::Display) -> CaptureError {
    CaptureError::InvalidFrameLayout {
        message: error.to_string(),
    }
}

fn display(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::advance_frame_deadline;

    #[test]
    fn pacing_stays_at_twenty_fps_on_a_sixty_hz_source() {
        let start = Instant::now();
        let mut next_frame_due = None;
        let accepted = (0..60)
            .filter(|tick| {
                advance_frame_deadline(
                    &mut next_frame_due,
                    start + Duration::from_nanos(16_666_667 * tick),
                )
            })
            .count();

        assert_eq!(accepted, 20);
    }

    #[test]
    fn late_callback_does_not_move_the_long_term_schedule() {
        let start = Instant::now();
        let mut next_frame_due = None;

        assert!(advance_frame_deadline(&mut next_frame_due, start));
        assert!(advance_frame_deadline(
            &mut next_frame_due,
            start + Duration::from_millis(66)
        ));
        assert!(!advance_frame_deadline(
            &mut next_frame_due,
            start + Duration::from_millis(99)
        ));
        assert!(advance_frame_deadline(
            &mut next_frame_due,
            start + Duration::from_millis(100)
        ));
    }
}
