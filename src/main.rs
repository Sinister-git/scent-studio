#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
mod accessories;
mod ble;
mod cloud;
mod gui;
mod model;
mod protocol;
use eframe::egui;
fn main() -> eframe::Result<()> {
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1060., 760.])
            .with_min_inner_size([800., 580.]),
        ..Default::default()
    };
    eframe::run_native(
        "Scent Studio",
        opts,
        Box::new(|cc| Ok(Box::new(gui::Studio::new(&cc.egui_ctx)?))),
    )
}
