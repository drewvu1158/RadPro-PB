//! Camera capture on a background thread, with a simulated feed as fallback.
//!
//! The worker owns the camera. It publishes a *downscaled* preview frame at a
//! modest rate (15 fps) into a shared slot, and only grabs a full-resolution
//! frame when the UI asks for a capture. That keeps the idle kiosk cheap:
//! the preview texture is small and never re-uploaded when nothing changed,
//! and the full frame exists only for the instant a shot is taken.
//!
//! All camera calls happen on the worker thread. Media Foundation's COM
//! initialization is per-thread while `nokhwa` tracks it globally, so calling
//! into the camera from the UI thread would leave the worker uninitialized.

use image::{ImageBuffer, Rgb, RgbImage, RgbaImage};
use nokhwa::utils::ApiBackend;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Longest preview edge. 640x480 is plenty for a kiosk preview and costs a
/// quarter of the texture upload of a 1280x960 frame.
const PREVIEW_SIZE: (u32, u32) = (640, 480);
/// Preview frames per second. The countdown is drawn by the UI, not the
/// camera, so a smooth 60 fps feed is not needed.
const PREVIEW_INTERVAL: Duration = Duration::from_millis(66);
/// Resolution of the simulated capture, matching a typical webcam.
const SIM_CAPTURE_SIZE: (u32, u32) = (1280, 960);

#[derive(Default)]
pub struct PreviewState {
    /// Monotonic frame counter; the UI re-uploads the texture when it changes.
    pub id: u64,
    pub image: Option<Arc<RgbImage>>,
    /// Human-readable note, e.g. "camera unavailable, showing simulated feed".
    pub note: Option<String>,
    /// Camera names reported by the platform backend, in index order.
    pub cameras: Vec<String>,
    /// Name of the camera currently streaming, if any.
    pub active: Option<String>,
}

pub type CaptureResult = Result<RgbaImage, String>;

enum Command {
    Capture(mpsc::Sender<CaptureResult>),
    Reconnect { camera_index: u32, simulate: bool },
    Shutdown,
}

pub struct CameraClient {
    pub latest: Arc<Mutex<PreviewState>>,
    commands: mpsc::Sender<Command>,
    worker: Option<JoinHandle<()>>,
}

impl CameraClient {
    pub fn start(camera_index: u32, simulate: bool) -> Self {
        let latest = Arc::new(Mutex::new(PreviewState::default()));
        let (cmd_tx, cmd_rx) = mpsc::channel::<Command>();
        let worker_latest = Arc::clone(&latest);
        let worker = std::thread::spawn(move || {
            run_worker(camera_index, simulate, &worker_latest, cmd_rx);
        });
        Self {
            latest,
            commands: cmd_tx,
            worker: Some(worker),
        }
    }

    /// Ask the worker for the next frame as a full-resolution RGBA image.
    pub fn request_capture(&self) -> Option<mpsc::Receiver<CaptureResult>> {
        let (tx, rx) = mpsc::channel();
        self.commands.send(Command::Capture(tx)).ok().map(|()| rx)
    }

    /// Re-open the camera on the existing worker thread. Applies index and
    /// simulated-mode changes without restarting the app.
    pub fn reconnect(&self, camera_index: u32, simulate: bool) {
        let _ = self.commands.send(Command::Reconnect {
            camera_index,
            simulate,
        });
    }

    pub fn shutdown(&mut self) {
        let _ = self.commands.send(Command::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for CameraClient {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn with_state(latest: &Arc<Mutex<PreviewState>>, f: impl FnOnce(&mut PreviewState)) {
    if let Ok(mut state) = latest.lock() {
        f(&mut state);
    }
}

fn run_worker(
    camera_index: u32,
    simulate: bool,
    latest: &Arc<Mutex<PreviewState>>,
    commands: mpsc::Receiver<Command>,
) {
    let mut source = open_source(camera_index, simulate, latest);

    let mut id = 0u64;
    let mut next_preview = Instant::now();
    loop {
        // Sleep until the next preview is due, but wake up immediately for a
        // capture request, a reconnect, or shutdown.
        let wait = next_preview.saturating_duration_since(Instant::now());
        match commands.recv_timeout(wait) {
            Ok(Command::Shutdown) => return,
            Ok(Command::Capture(reply)) => {
                let result = source.capture();
                if let Err(note) = &result {
                    with_state(latest, |s| s.note = Some(note.clone()));
                }
                let _ = reply.send(result);
                // Keep the preview schedule untouched by captures.
                continue;
            }
            Ok(Command::Reconnect {
                camera_index,
                simulate,
            }) => {
                // Release the existing stream before opening its replacement.
                // Assignment alone opens the RHS while the old camera is held.
                drop(source);
                source = open_source(camera_index, simulate, latest);
                next_preview = Instant::now();
                continue;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }

        next_preview = Instant::now() + PREVIEW_INTERVAL;
        match source.preview() {
            Ok(frame) => {
                id += 1;
                with_state(latest, |state| {
                    state.id = id;
                    state.image = Some(Arc::new(frame));
                });
            }
            Err(note) => {
                with_state(latest, |s| {
                    s.active = None;
                    s.note = Some(format!("{note}; showing simulated feed"));
                });
                // Fall back to simulation so the kiosk never sits dead.
                source = Source::Simulated(Simulator::new());
                next_preview = Instant::now() + Duration::from_millis(100);
            }
        }
    }
}

/// Open the requested camera (or the simulator), and publish what was found
/// so the settings screen can list the available cameras.
fn open_source(camera_index: u32, simulate: bool, latest: &Arc<Mutex<PreviewState>>) -> Source {
    // List cameras even in simulated mode so the settings screen can offer
    // them before the user switches back to a real camera.
    let cameras = match nokhwa::query(ApiBackend::MediaFoundation) {
        Ok(list) => list.iter().map(|cam| cam.human_name()).collect(),
        Err(_) => Vec::new(),
    };

    if simulate {
        with_state(latest, |s| {
            s.cameras = cameras;
            s.active = None;
            s.note = None;
        });
        return Source::Simulated(Simulator::new());
    }

    match RealCamera::open(camera_index) {
        Ok(camera) => {
            let active = cameras.get(camera_index as usize).cloned();
            with_state(latest, |s| {
                s.cameras = cameras;
                s.active = active;
                s.note = None;
            });
            Source::Real(camera)
        }
        Err(note) => {
            let note = format!("{note}; showing simulated feed");
            with_state(latest, |s| {
                s.cameras = cameras;
                s.active = None;
                s.note = Some(note);
            });
            Source::Simulated(Simulator::new())
        }
    }
}

enum Source {
    Real(RealCamera),
    Simulated(Simulator),
}

impl Source {
    /// A small frame for the on-screen preview.
    fn preview(&mut self) -> Result<RgbImage, String> {
        match self {
            Source::Real(camera) => {
                let frame = camera.grab()?;
                Ok(fit_within(&frame, PREVIEW_SIZE))
            }
            Source::Simulated(sim) => Ok(sim.frame(PREVIEW_SIZE)),
        }
    }

    /// A full-resolution frame for a saved shot.
    fn capture(&mut self) -> Result<RgbaImage, String> {
        let rgb = match self {
            Source::Real(camera) => camera.grab()?,
            Source::Simulated(sim) => sim.frame(SIM_CAPTURE_SIZE),
        };
        Ok(image::DynamicImage::ImageRgb8(rgb).to_rgba8())
    }
}

/// Downscale `img` to fit inside `max`, preserving aspect ratio. Never upscales.
fn fit_within(img: &RgbImage, max: (u32, u32)) -> RgbImage {
    let (w, h) = img.dimensions();
    if w <= max.0 && h <= max.1 {
        return img.clone();
    }
    let scale = (max.0 as f32 / w as f32).min(max.1 as f32 / h as f32);
    let target = (
        ((w as f32 * scale) as u32).max(1),
        ((h as f32 * scale) as u32).max(1),
    );
    // Nearest is plenty for a 15 fps preview and much cheaper than a filter.
    image::imageops::resize(
        img,
        target.0,
        target.1,
        image::imageops::FilterType::Nearest,
    )
}

struct RealCamera {
    camera: nokhwa::Camera,
}

impl RealCamera {
    fn open(index: u32) -> Result<Self, String> {
        use nokhwa::pixel_format::RgbFormat;
        use nokhwa::utils::{
            CameraFormat, CameraIndex, FrameFormat, RequestedFormat, RequestedFormatType,
        };

        // Cameras vary a lot in which formats they advertise. Ask for a
        // preview-sized stream first: the strip cells are 640x480, so decoding
        // a larger frame would only burn CPU. Fall back to whatever the
        // backend can offer.
        let attempts = [
            RequestedFormatType::Closest(CameraFormat::new_from(640, 480, FrameFormat::MJPEG, 30)),
            RequestedFormatType::HighestFrameRate(30),
            RequestedFormatType::None,
        ];
        let mut errors = Vec::new();
        for request in attempts {
            let requested = RequestedFormat::new::<RgbFormat>(request);
            match nokhwa::Camera::new(CameraIndex::Index(index), requested) {
                Ok(mut camera) => match camera.open_stream() {
                    Ok(()) => return Ok(Self { camera }),
                    Err(e) => errors.push(format!("{request}: start failed: {e}")),
                },
                Err(e) => errors.push(format!("{request}: {e}")),
            }
        }
        Err(format!(
            "could not open camera {index} ({})",
            errors.join("; ")
        ))
    }

    fn grab(&mut self) -> Result<RgbImage, String> {
        let frame = self.camera.frame().map_err(|e| e.to_string())?;
        let buffer = frame
            .decode_image::<nokhwa::pixel_format::RgbFormat>()
            .map_err(|e| e.to_string())?;
        Ok(buffer)
    }
}

/// Probe the camera backend and report what it sees.
///
/// Runs on a fresh thread so the Media Foundation COM apartment matches the
/// one the camera worker uses.
pub fn report_cameras() -> String {
    std::thread::spawn(|| {
        let mut out = String::new();
        match nokhwa::query(ApiBackend::MediaFoundation) {
            Ok(cameras) => {
                out.push_str(&format!(
                    "Media Foundation reports {} camera(s):\n",
                    cameras.len()
                ));
                for (position, camera) in cameras.iter().enumerate() {
                    out.push_str(&format!(
                        "  index {position}: {} ({:?})\n",
                        camera.human_name(),
                        camera.index()
                    ));
                }
                if cameras.is_empty() {
                    out.push_str(
                        "  (none) Check Windows Settings > Privacy > Camera and that no \
                         other app is holding the camera.\n",
                    );
                }
                for position in 0..cameras.len() {
                    out.push_str(&format!("Opening index {position} ... "));
                    match RealCamera::open(position as u32) {
                        Ok(mut camera) => {
                            out.push_str(&format!(
                                "ok, format {:?}\n",
                                camera.camera.camera_format()
                            ));
                            match camera.grab() {
                                Ok(frame) => out.push_str(&format!(
                                    "  first frame: {}x{}\n",
                                    frame.width(),
                                    frame.height()
                                )),
                                Err(e) => out.push_str(&format!("  frame failed: {e}\n")),
                            }
                            // Confirm the stream actually delivers frames.
                            let started = Instant::now();
                            let mut frames = 0;
                            for _ in 0..5 {
                                match camera.grab() {
                                    Ok(_) => frames += 1,
                                    Err(e) => {
                                        out.push_str(&format!("  stream failed: {e}\n"));
                                        break;
                                    }
                                }
                            }
                            if frames > 0 {
                                out.push_str(&format!(
                                    "  {frames} more frames in {:.2}s\n",
                                    started.elapsed().as_secs_f32()
                                ));
                            }
                        }
                        Err(e) => out.push_str(&format!("failed: {e}\n")),
                    }
                }
            }
            Err(e) => out.push_str(&format!("Media Foundation query failed: {e}\n")),
        }
        out
    })
    .join()
    .unwrap_or_else(|_| "camera probe thread panicked\n".to_string())
}

/// Animated test pattern used when no real camera is available.
///
/// The expensive gradient is baked once per size; each frame is a cheap clone
/// plus the moving ball and stripe.
struct Simulator {
    start: Instant,
    frame_count: u64,
    preview_bg: RgbImage,
    capture_bg: RgbImage,
}

impl Simulator {
    fn new() -> Self {
        Self {
            start: Instant::now(),
            frame_count: 0,
            preview_bg: background(PREVIEW_SIZE),
            capture_bg: background(SIM_CAPTURE_SIZE),
        }
    }

    fn frame(&mut self, size: (u32, u32)) -> RgbImage {
        self.frame_count += 1;
        let t = self.start.elapsed().as_secs_f32();
        let (w, h) = size;
        let mut img = if size == PREVIEW_SIZE {
            self.preview_bg.clone()
        } else {
            self.capture_bg.clone()
        };

        // Bouncing ball so motion is obvious at a glance.
        let cx = (w as f32 * (0.5 + 0.42 * (t * 0.9).sin())) as i32;
        let cy = (h as f32 * (0.5 + 0.38 * (t * 1.7).cos())) as i32;
        let r: i32 = (h as f32 * 0.064) as i32;
        for dy in -r..=r {
            for dx in -r..=r {
                if dx * dx + dy * dy > r * r {
                    continue;
                }
                let x = cx + dx;
                let y = cy + dy;
                if x >= 0 && x < w as i32 && y >= 0 && y < h as i32 {
                    let depth = 1.0 - (dx * dx + dy * dy) as f32 / (r * r) as f32;
                    let v = (140.0 + 115.0 * depth) as u8;
                    img.put_pixel(x as u32, y as u32, Rgb([v, (v as f32 * 0.72) as u8, 60]));
                }
            }
        }

        // Simulated "timestamp" stripe so successive shots differ.
        let stripe = (self.frame_count / 30 % 10) as u32;
        let stripe_h = (h / 28).max(6);
        let cell = (w / 40).max(4);
        for y in h.saturating_sub(stripe_h)..h {
            for x in 0..w {
                if (x / cell + stripe) % 2 == 0 {
                    img.put_pixel(x, y, Rgb([230, 230, 235]));
                }
            }
        }
        img
    }
}

/// Static blue-grey gradient background, computed once.
fn background((w, h): (u32, u32)) -> RgbImage {
    let mut img = ImageBuffer::from_pixel(w, h, Rgb([24, 28, 38]));
    for y in 0..h {
        let fy = y as f32 / h as f32;
        let glow = ((fy - 0.5).abs() * -3.0).exp() * 90.0;
        for x in 0..w {
            let fx = x as f32 / w as f32;
            let band = ((fx * 6.0).sin() * 0.5 + 0.5) * 60.0;
            img.put_pixel(
                x,
                y,
                Rgb([
                    (40.0 + band + glow * 0.4) as u8,
                    (46.0 + band * 0.6 + glow * 0.3) as u8,
                    (64.0 + band * 0.9 + glow * 0.5) as u8,
                ]),
            );
        }
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_within_downscales_and_keeps_aspect() {
        let wide = RgbImage::from_pixel(1280, 720, Rgb([1, 2, 3]));
        let fitted = fit_within(&wide, (640, 480));
        assert_eq!(fitted.dimensions(), (640, 360));

        let small = RgbImage::from_pixel(320, 240, Rgb([1, 2, 3]));
        let untouched = fit_within(&small, (640, 480));
        assert_eq!(untouched.dimensions(), (320, 240));
    }

    #[test]
    fn simulated_worker_publishes_preview_and_full_capture() {
        let mut client = CameraClient::start(0, true);

        let deadline = Instant::now() + Duration::from_secs(3);
        let mut preview = None;
        while Instant::now() < deadline {
            if let Ok(state) = client.latest.lock() {
                if let Some(image) = &state.image {
                    preview = Some(Arc::clone(image));
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let preview = preview.expect("worker should publish a preview frame");
        assert!(preview.width() <= PREVIEW_SIZE.0);
        assert!(preview.height() <= PREVIEW_SIZE.1);

        let rx = client.request_capture().expect("capture request");
        let shot = rx
            .recv_timeout(Duration::from_secs(3))
            .expect("worker should answer a capture request")
            .expect("capture should succeed");
        assert_eq!(shot.dimensions(), SIM_CAPTURE_SIZE);
        client.shutdown();
    }

    #[test]
    fn reconnect_switches_back_to_simulation() {
        let mut client = CameraClient::start(0, true);
        std::thread::sleep(Duration::from_millis(120));
        client.reconnect(0, true);
        std::thread::sleep(Duration::from_millis(120));
        // The worker should still answer captures after a reconnect.
        let rx = client.request_capture().expect("capture request");
        let shot = rx
            .recv_timeout(Duration::from_secs(3))
            .expect("worker should answer after reconnect")
            .expect("capture should succeed");
        assert_eq!(shot.dimensions(), SIM_CAPTURE_SIZE);
        client.shutdown();
    }

    /// Opt-in hardware check of the actual GUI worker, including reopening the
    /// same device. The standalone device probe does not exercise that path.
    #[test]
    #[ignore = "requires the configured physical webcam to be connected and available"]
    fn configured_camera_streams_after_reconnect() {
        let settings = crate::settings::Settings::load(&crate::settings::default_settings_path())
            .expect("saved camera settings");
        assert!(!settings.simulate, "select a real camera in Settings first");
        let mut client = CameraClient::start(settings.camera_index, false);
        for reconnect in [false, true] {
            let previous_id = client.latest.lock().unwrap().id;
            if reconnect {
                client.reconnect(settings.camera_index, false);
            }
            // Commands are FIFO: the capture reply proves reconnect completed.
            let shot = client
                .request_capture()
                .expect("live worker")
                .recv_timeout(Duration::from_secs(10))
                .expect("real camera capture reply")
                .expect("real camera capture");
            assert!(shot.width() > 0 && shot.height() > 0);

            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                {
                    let state = client.latest.lock().unwrap();
                    assert!(state.note.is_none(), "camera error: {:?}", state.note);
                    if state.id > previous_id {
                        assert!(state.active.is_some(), "must not use simulated fallback");
                        assert!(state.image.is_some());
                        println!(
                            "{}: {}, captured {}x{}, preview frame {}",
                            if reconnect {
                                "reconnected"
                            } else {
                                "connected"
                            },
                            state.active.as_deref().unwrap(),
                            shot.width(),
                            shot.height(),
                            state.id,
                        );
                        break;
                    }
                }
                assert!(Instant::now() < deadline, "no new real preview frame");
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        client.shutdown();
    }
}
