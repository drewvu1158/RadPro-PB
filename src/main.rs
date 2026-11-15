//! Radiate Photobooth: a local photo booth for Windows.
//!
//! Usage:
//!   radiate-photobooth            launch with the saved camera settings
//!   radiate-photobooth --simulate launch with a simulated camera feed
//!
//! Part of a clean-room reimplementation: no Sparkbooth code, assets or
//! branding are used. See README.md for details.

mod alignment;
mod amf;
mod app;
mod booth;
mod camera;
mod design;
mod editor;
mod file_dialog;
mod layout;
mod library;
mod library_ui;
mod presets;
mod printing;
mod settings;
mod storage;
mod template;
mod ui_theme;

use app::SparkboothApp;
use eframe::egui;

fn main() -> eframe::Result<()> {
    let args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    if args.iter().any(|arg| arg == "--list-cameras") {
        print!("{}", camera::report_cameras());
        return Ok(());
    }
    let simulate = args.iter().any(|arg| arg == "--simulate");
    let settings_path = args.iter().position(|arg| arg == "--settings").map(|i| {
        args.get(i + 1)
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                eprintln!("--settings requires a TOML file path");
                std::process::exit(2)
            })
    });

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([760.0, 480.0])
            .with_title("Radiate Photobooth"),
        ..Default::default()
    };

    eframe::run_native(
        "Radiate Photobooth",
        options,
        Box::new(move |cc| Ok(Box::new(SparkboothApp::new(cc, simulate, settings_path)))),
    )
}

/// End-to-end check of the core flow without a window or a real camera:
/// countdown -> capture -> compose -> save.
#[cfg(test)]
mod session_tests {
    use crate::booth::{Booth, BoothEvent, Phase};
    use crate::camera::CameraClient;
    use crate::layout::{compose, LayoutKind, LayoutOptions};
    use crate::storage::save_strip;
    use image::DynamicImage;
    use std::time::Duration;

    #[test]
    fn full_simulated_session_writes_a_strip() {
        let mut camera = CameraClient::start(0, true);
        let mut booth = Booth::new(2, 1);

        let mut shots = Vec::new();
        for index in 0..2 {
            assert_eq!(booth.update(60_000), BoothEvent::None);
            assert_eq!(booth.shots_taken(), index);
            booth.request_shot();
            loop {
                match booth.update(50) {
                    BoothEvent::ShotDue { .. } => break,
                    _ => assert_eq!(booth.phase(), Phase::Countdown),
                }
            }
            let rx = camera.request_capture().expect("capture request");
            let frame = rx
                .recv_timeout(Duration::from_secs(3))
                .expect("capture should arrive")
                .expect("capture should succeed");
            shots.push(DynamicImage::ImageRgba8(frame));
            booth.deliver_shot();
        }
        assert_eq!(booth.phase(), Phase::Review);

        let opts = LayoutOptions {
            cell: (160, 120),
            margin: 8,
            gap: 4,
            background: [255, 255, 255, 255],
        };
        let strip = compose(&shots, LayoutKind::Strip, &opts).expect("compose strip");
        let dir =
            std::env::temp_dir().join(format!("sparkbooth-rust-session-{}", std::process::id()));
        let path = save_strip(&strip, &dir).expect("save strip");
        assert!(path.exists());
        let _ = std::fs::remove_dir_all(&dir);

        camera.shutdown();
    }
}
