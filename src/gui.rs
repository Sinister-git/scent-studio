use crate::{
    ble,
    cloud::Account,
    model::{Action, Device, Schedule, Snapshot, DAYS},
};
use anyhow::Result;
use eframe::egui::{self, Color32, RichText, Ui};
use std::{
    future::Future,
    sync::mpsc::{Receiver, TryRecvError},
    time::Duration,
};
use tokio::runtime::Runtime;

const TEAL: Color32 = Color32::from_rgb(18, 104, 92);
const MUTED: Color32 = Color32::from_rgb(77, 101, 95);
#[derive(Clone, Copy, PartialEq, Eq)]
enum Link {
    Bluetooth,
    Cloud,
}
impl Link {
    fn title(self) -> &'static str {
        match self {
            Self::Bluetooth => "Bluetooth",
            Self::Cloud => "Wi-Fi cloud",
        }
    }
}
enum Event {
    Page {
        account: Option<Account>,
        devices: Vec<Device>,
        page: u32,
        pages: u32,
    },
    Loaded(Box<Snapshot>),
    Written(String),
}
struct Notice {
    message: String,
    error: bool,
}
pub struct Studio {
    rt: Runtime,
    pending: Option<Receiver<Result<Event>>>,
    busy: String,
    link: Link,
    account: Option<Account>,
    user: String,
    password: String,
    pin: String,
    devices: Vec<Device>,
    selected: Option<usize>,
    page: u32,
    pages: u32,
    snapshot: Option<Snapshot>,
    notice: Notice,
    confirm: Option<Action>,
    slot: usize,
    draft: Schedule,
    sensor_draft: [u16; 3],
    capacity: u16,
    remaining: u16,
    rate: u16,
    oil_name: String,
    new_name: String,
    note: String,
    new_pin: String,
    confirm_pin: String,
}
fn eyebrow(ui: &mut Ui, s: &str) {
    ui.label(RichText::new(s).size(11.).strong().color(TEAL));
}
fn state(v: Option<bool>) -> &'static str {
    match v {
        Some(true) => "On",
        Some(false) => "Off",
        None => "Not reported",
    }
}
impl Studio {
    pub fn new(ctx: &egui::Context) -> Result<Self> {
        ctx.set_visuals(egui::Visuals::light());
        ctx.style_mut(|s| {
            s.spacing.item_spacing = egui::vec2(9., 8.);
            s.spacing.button_padding = egui::vec2(10., 5.);
            s.visuals.panel_fill = Color32::from_rgb(246, 248, 245);
            s.visuals.selection.bg_fill = Color32::from_rgb(152, 215, 196);
            s.visuals.selection.stroke.color = Color32::from_rgb(22, 66, 58);
        });
        Ok(Self {
            rt: Runtime::new()?,
            pending: None,
            busy: String::new(),
            link: Link::Bluetooth,
            account: None,
            user: String::new(),
            password: String::new(),
            pin: "9999".into(),
            devices: vec![],
            selected: None,
            page: 1,
            pages: 1,
            snapshot: None,
            notice: Notice {
                message: "Ready. Choose a connection to find your diffuser.".into(),
                error: false,
            },
            confirm: None,
            slot: 0,
            draft: Schedule::default(),
            sensor_draft: [0, 5, 120],
            capacity: 0,
            remaining: 0,
            rate: 0,
            oil_name: String::new(),
            new_name: String::new(),
            note: String::new(),
            new_pin: String::new(),
            confirm_pin: String::new(),
        })
    }
    fn note(&mut self, msg: impl Into<String>, error: bool) {
        self.notice = Notice {
            message: msg.into(),
            error,
        }
    }
    fn reset_selection(&mut self) {
        self.selected = None;
        self.snapshot = None;
        self.confirm = None;
        self.slot = 0;
        self.draft = Schedule::default();
        self.new_name.clear();
        self.note.clear();
        self.new_pin.clear();
        self.confirm_pin.clear();
        self.oil_name.clear()
    }
    fn route(&mut self, link: Link) {
        self.reset_selection();
        self.devices.clear();
        self.page = 1;
        self.pages = 1;
        self.link = link;
        self.note(
            "Choose a device to read its settings. No changes are sent automatically.",
            false,
        )
    }
    fn task(
        &mut self,
        ctx: &egui::Context,
        label: &str,
        f: impl Future<Output = Result<Event>> + Send + 'static,
    ) {
        if self.pending.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        self.pending = Some(rx);
        self.busy = label.into();
        let ctx = ctx.clone();
        self.rt.spawn(async move {
            let result = f.await;
            let _ = tx.send(result);
            ctx.request_repaint();
        });
    }
    fn list(&mut self, ctx: &egui::Context, page: u32) {
        self.reset_selection();
        self.devices.clear();
        if self.link == Link::Bluetooth {
            self.task(ctx, "Scanning nearby diffusers…", async {
                Ok(Event::Page {
                    account: None,
                    devices: ble::list().await?,
                    page: 1,
                    pages: 1,
                })
            });
            return;
        }
        let auth = self.account.clone();
        let user = self.user.clone();
        let pw = std::mem::take(&mut self.password);
        self.task(ctx, "Loading account devices…", async move {
            let a = match auth {
                Some(a) => a,
                None => Account::login(user, pw).await?,
            };
            let (devices, pages) = a.page(page).await?;
            Ok(Event::Page {
                account: Some(a),
                devices,
                page,
                pages,
            })
        });
    }
    fn read_or_write(&mut self, ctx: &egui::Context, a: Option<Action>) {
        let Some(d) = self.selected.and_then(|i| self.devices.get(i)) else {
            return;
        };
        let id = d.id.clone();
        let pin = self.pin.clone();
        let auth = self.account.clone();
        let link = self.link;
        let writing = a.is_some();
        self.snapshot = None;
        self.task(ctx,if writing{"Sending one change…"}else{"Reading device settings…"},async move{let out=match link{Link::Bluetooth=>ble::operate(id,pin,a).await?,Link::Cloud=>auth.ok_or_else(||anyhow::anyhow!("Sign in first"))?.operate(id,a).await?};Ok(match out{Some(s)=>Event::Loaded(Box::new(s)),None=>Event::Written(if link==Link::Cloud{"Vendor accepted the request; telemetry may lag. Refresh to check the device."}else{"Device acknowledged the request. Refresh to read back its settings."}.into())})});
    }
    fn populate(&mut self) {
        if let Some(s) = &self.snapshot {
            self.draft = s.slots[self.slot]
                .settings
                .clone()
                .filter(Schedule::configured)
                .unwrap_or_default()
        }
    }
    fn poll(&mut self) {
        let Some(rx) = &self.pending else { return };
        let r = match rx.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => return,
            Err(_) => Err(anyhow::anyhow!(
                "Background operation stopped. Refresh before retrying."
            )),
        };
        self.pending = None;
        match r {
            Err(e) => self.note(format!("{e:#}"), true),
            Ok(Event::Written(s)) => self.note(s, false),
            Ok(Event::Page {
                account,
                devices,
                page,
                pages,
            }) => {
                if account.is_some() {
                    self.account = account
                }
                self.devices = devices;
                self.page = page;
                self.pages = pages.max(1);
                self.note(if self.devices.is_empty(){"No matching devices found. Check Bluetooth permissions or your linked account."}else{"Select a device to read its settings."},false)
            }
            Ok(Event::Loaded(s)) => {
                if let Some([cap, left, rate, _]) = s.oil {
                    self.capacity = cap / 10;
                    self.remaining = left / 10;
                    self.rate = rate;
                }
                self.oil_name = s.oil_name.clone();
                if s.cloud_rename {
                    self.new_name = s.name.clone();
                }
                self.sensor_draft = s.accessories.motion.unwrap_or([0, 5, 120]);
                if let Some(c) = s.cycle {
                    if !c.work().contains(&self.sensor_draft[1]) {
                        self.sensor_draft[1] = 5
                    }
                    if !c.pause().contains(&self.sensor_draft[2]) {
                        self.sensor_draft[2] = 120
                    }
                }
                self.snapshot = Some(*s);
                self.populate();
                self.note(
                    "Settings loaded. Edits stay local until you confirm a change.",
                    false,
                )
            }
        }
    }
    fn connect_ui(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        eyebrow(ui, "01   CONNECTION");
        ui.horizontal(|ui| {
            for l in [Link::Bluetooth, Link::Cloud] {
                if ui.selectable_label(l == self.link, l.title()).clicked() && l != self.link {
                    self.route(l)
                }
            }
        });
        ui.separator();
        match self.link {
            Link::Bluetooth => {
                ui.heading("Nearby devices");
                ui.label("Disconnect the mobile app before scanning.");
                ui.label("Device PIN");
                ui.add(
                    egui::TextEdit::singleline(&mut self.pin)
                        .password(true)
                        .desired_width(145.)
                        .char_limit(6),
                );
                if ui.button("Scan for diffusers").clicked() {
                    self.list(ctx, 1)
                }
            }
            Link::Cloud => {
                ui.heading("Linked devices");
                if self.account.is_none() {
                    ui.label("Scent Master Pro account");
                    ui.text_edit_singleline(&mut self.user);
                    ui.label("Account password");
                    ui.add(egui::TextEdit::singleline(&mut self.password).password(true));
                    if ui
                        .add_enabled(
                            !self.user.trim().is_empty() && !self.password.is_empty(),
                            egui::Button::new("Sign in and list devices"),
                        )
                        .clicked()
                    {
                        self.list(ctx, 1)
                    }
                    ui.small("Uses the vendor's Internet service, not a local IP address.");
                } else {
                    ui.label(&self.user);
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("Reload list").clicked() {
                            self.list(ctx, self.page)
                        }
                        if ui.button("Sign out").clicked() {
                            self.account = None;
                            self.password.clear();
                            self.route(Link::Cloud)
                        }
                    });
                    if self.pages > 1 {
                        ui.horizontal(|ui| {
                            if ui
                                .add_enabled(self.page > 1, egui::Button::new("‹"))
                                .clicked()
                            {
                                self.list(ctx, self.page - 1)
                            }
                            ui.label(format!("{} / {}", self.page, self.pages));
                            if ui
                                .add_enabled(self.page < self.pages, egui::Button::new("›"))
                                .clicked()
                            {
                                self.list(ctx, self.page + 1)
                            }
                        });
                    }
                }
            }
        }
        ui.add_space(10.);
        let mut select = None;
        for (i, d) in self.devices.iter().enumerate() {
            if ui
                .selectable_label(self.selected == Some(i), RichText::new(&d.name).strong())
                .clicked()
            {
                select = Some(i)
            }
            ui.small(format!("{} · {}", d.status, d.id));
            ui.separator();
        }
        if let Some(i) = select {
            self.reset_selection();
            self.selected = Some(i);
            self.read_or_write(ctx, None)
        }
    }
    fn device_ui(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let Some(i) = self.selected else {
            ui.add_space(38.);
            eyebrow(ui, "SCENT STUDIO   /   DEVICE WORKSPACE");
            ui.heading("Choose your diffuser");
            ui.label("Use Bluetooth for nearby control, or Wi-Fi for an already linked device.");
            ui.add_space(14.);
            ui.group(|ui|{ui.strong("Your five schedules");ui.label("Set active days, time windows, and work / pause intervals. Read the current settings before making changes.");});
            return;
        };
        eyebrow(
            ui,
            &format!(
                "DEVICE WORKSPACE   /   {}",
                self.link.title().to_uppercase()
            ),
        );
        ui.heading(&self.devices[i].name);
        ui.horizontal_wrapped(|ui| {
            ui.small(&self.devices[i].id);
            if ui.button("Refresh settings").clicked() {
                self.read_or_write(ctx, None)
            }
        });
        ui.separator();
        let Some(s) = self.snapshot.clone() else {
            ui.label("Refresh to load the current device controls.");
            return;
        };
        if !s.online {
            ui.colored_label(
                MUTED,
                "Offline · showing last reported settings. Writes are disabled.",
            );
        }
        ui.add_enabled_ui(s.online, |ui| self.controls(ui, &s));
    }
    fn controls(&mut self, ui: &mut Ui, s: &Snapshot) {
        let mut action = None;
        eyebrow(ui, "02   DEVICE CONTROLS");
        egui::Grid::new("switches").num_columns(2).show(ui, |ui| {
            if s.master {
                ui.label(format!("Master power · {}", state(s.power)));
                ui.horizontal(|ui| {
                    if ui.button("Power on").clicked() {
                        action = Some(Action::Power(true))
                    }
                    if ui.button("Power off").clicked() {
                        action = Some(Action::Power(false))
                    }
                });
                ui.end_row();
            }
            if let Some(v) = s.lock {
                ui.label(format!(
                    "Keypad · {}",
                    if v { "locked" } else { "unlocked" }
                ));
                ui.horizontal(|ui| {
                    if ui.button("Lock").clicked() {
                        action = Some(Action::Lock(true))
                    }
                    if ui.button("Unlock").clicked() {
                        action = Some(Action::Lock(false))
                    }
                });
                ui.end_row();
            }
            if s.levels {
                ui.label(format!(
                    "Fan / gear · {}",
                    s.fan
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "unknown".into())
                ));
                ui.horizontal(|ui| {
                    for v in 0..3 {
                        if ui.button(["Level 0", "Level 1", "Level 2"][v]).clicked() {
                            action = Some(Action::Fan(v as u8))
                        }
                    }
                });
                ui.end_row();
            }
            if s.ble {
                ui.label("Device clock");
                if ui.button("Sync local time").clicked() {
                    action = Some(Action::Clock)
                }
                ui.end_row();
            }
        });
        ui.add_space(10.);
        ui.separator();
        if let Some(v) =
            crate::accessories::show(ui, &s.accessories, s.cycle, &mut self.sensor_draft)
        {
            action = Some(Action::Accessory(v));
        }
        eyebrow(ui, "03   DIFFUSION SCHEDULES");
        egui::Grid::new("slot_table")
            .striped(true)
            .spacing([16., 4.])
            .show(ui, |ui| {
                for label in ["Slot", "State", "Weekdays", "Window", "Work / pause"] {
                    ui.strong(label);
                }
                ui.end_row();
                for (i, v) in s.slots.iter().enumerate() {
                    if ui
                        .selectable_label(self.slot == i, format!("{:02}", i + 1))
                        .clicked()
                    {
                        self.slot = i;
                        self.populate()
                    }
                    ui.label(state(v.enabled));
                    if let Some(sc) = v.settings.as_ref().filter(|a| a.configured()) {
                        ui.label(
                            DAYS.iter()
                                .enumerate()
                                .filter_map(|(i, d)| (sc.days & (1 << i) != 0).then_some(*d))
                                .collect::<Vec<_>>()
                                .join(" "),
                        );
                        ui.label(format!("{}–{}", sc.start, sc.end));
                        ui.label(format!("{}s / {}s", sc.work, sc.pause));
                    } else {
                        ui.label(if v.settings.is_some() {
                            "Empty"
                        } else {
                            "Not reported"
                        });
                        ui.label("—");
                        ui.label("—");
                    }
                    ui.end_row();
                }
            });
        ui.add_space(5.);
        ui.group(|ui| {
            ui.horizontal_wrapped(|ui| {
                ui.strong(format!("Schedule {:02}", self.slot + 1));
                if ui.button("Enable").clicked() {
                    action = Some(Action::Mode(self.slot, true))
                }
                if ui.button("Disable").clicked() {
                    action = Some(Action::Mode(self.slot, false))
                }
            });
            ui.horizontal_wrapped(|ui| {
                for (i, d) in DAYS.iter().enumerate() {
                    if ui
                        .selectable_label(self.draft.days & (1 << i) != 0, *d)
                        .clicked()
                    {
                        self.draft.days ^= 1 << i
                    }
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("Start");
                ui.add(
                    egui::TextEdit::singleline(&mut self.draft.start)
                        .desired_width(50.)
                        .char_limit(5),
                );
                ui.label("End");
                ui.add(
                    egui::TextEdit::singleline(&mut self.draft.end)
                        .desired_width(50.)
                        .char_limit(5),
                );
                if let Some(c) = s.cycle {
                    for (label, val, options) in [
                        ("Work", &mut self.draft.work, c.work()),
                        ("Pause", &mut self.draft.pause, c.pause()),
                    ] {
                        ui.label(label);
                        egui::ComboBox::from_id_salt(label)
                            .selected_text(format!("{val}s"))
                            .width(64.)
                            .show_ui(ui, |ui| {
                                for v in options {
                                    ui.selectable_value(val, v, format!("{v}s"));
                                }
                            });
                    }
                }
            });
            ui.horizontal_wrapped(|ui| {
                if ui.button("Save schedule…").clicked() {
                    action = Some(Action::Schedule(self.slot, self.draft.clone()))
                }
                if ui.button("Clear slot…").clicked() {
                    action = Some(Action::Clear(self.slot))
                }
            });
            ui.small(
                "Times use the diffuser's clock. Saving a window doesn't change its enable switch.",
            );
        });
        if s.cloud_rename {
            ui.collapsing("Device name", |ui| {
                ui.text_edit_singleline(&mut self.new_name);
                if ui
                    .add_enabled(
                        !self.new_name.trim().is_empty() && self.new_name.len() <= 16,
                        egui::Button::new("Rename device…"),
                    )
                    .clicked()
                {
                    action = Some(Action::Name(self.new_name.clone()))
                }
            });
        }
        if s.labels {
            ui.collapsing("Device name, note and PIN", |ui| {
                for (label, text) in [("Name", &mut self.new_name), ("Note", &mut self.note)] {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(label);
                        ui.add(
                            egui::TextEdit::singleline(text)
                                .desired_width(160.)
                                .char_limit(16),
                        );
                        if ui.button(format!("Set {label}…")).clicked() {
                            action = Some(if label == "Name" {
                                Action::Name(text.clone())
                            } else {
                                Action::Note(text.clone())
                            })
                        }
                    });
                }
                ui.label("Change device PIN (four digits). Enter it again to confirm.");
                ui.horizontal_wrapped(|ui| {
                    for txt in [&mut self.new_pin, &mut self.confirm_pin] {
                        ui.add(
                            egui::TextEdit::singleline(txt)
                                .password(true)
                                .char_limit(4)
                                .desired_width(68.),
                        );
                    }
                    if ui
                        .add_enabled(
                            self.new_pin.len() == 4 && self.new_pin == self.confirm_pin,
                            egui::Button::new("Set PIN…"),
                        )
                        .clicked()
                    {
                        action = Some(Action::Pin(self.new_pin.clone()))
                    }
                });
            });
        }
        if let Some(o) = s.oil {
            ui.collapsing("Fragrance & bottle estimates", |ui| {
                ui.label(format!(
                    "{:.1} / {:.1} mL · {:.1} mL/h · {} days estimated",
                    o[1] as f32 / 10.,
                    o[0] as f32 / 10.,
                    o[2] as f32 / 10.,
                    o[3]
                ));
                for (idx, label, val, max) in [
                    (0, "Capacity (mL)", &mut self.capacity, 5000),
                    (1, "Remaining (mL)", &mut self.remaining, o[0] / 10),
                    (2, "Consumption (tenths of mL/h)", &mut self.rate, 100),
                ] {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(label);
                        ui.add(egui::DragValue::new(val).range(0..=max));
                        if ui.button(format!("Apply {label}…")).clicked() {
                            action = Some(match idx {
                                0 => Action::Capacity(*val),
                                1 => Action::Remaining(*val),
                                _ => Action::Consumption(*val),
                            })
                        }
                    });
                }
                ui.horizontal_wrapped(|ui| {
                    ui.label("Oil name");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.oil_name)
                            .desired_width(130.)
                            .char_limit(16),
                    );
                    if ui.button("Save name…").clicked() {
                        action = Some(Action::OilName(self.oil_name.clone()))
                    }
                });
            });
        }
        if let Some(a) = action {
            if !(matches!(&a, Action::OilName(_))
                || s.cloud_rename && matches!(&a, Action::Name(_)))
            {
                if let Err(e) = a.packet(s) {
                    self.note(e.to_string(), true);
                    return;
                }
            }
            self.confirm = Some(a);
        }
    }
}
impl eframe::App for Studio {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.poll();
        let enabled = self.pending.is_none() && self.confirm.is_none();
        egui::TopBottomPanel::top("brand").show(ctx, |ui| {
            ui.add_space(4.);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("SCENT  /  STUDIO")
                        .size(20.)
                        .strong()
                        .color(TEAL),
                );
                ui.separator();
                ui.label("Diffuser control");
            });
            ui.add_space(4.);
        });
        egui::TopBottomPanel::bottom("notice").show(ctx, |ui| {
            if self.pending.is_some() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(&self.busy);
                });
                ctx.request_repaint_after(Duration::from_millis(150));
            } else {
                ui.colored_label(
                    if self.notice.error {
                        Color32::from_rgb(154, 48, 35)
                    } else {
                        MUTED
                    },
                    &self.notice.message,
                );
            }
        });
        egui::SidePanel::left("connection")
            .default_width(240.)
            .width_range(205.0..=300.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("side_scroll")
                    .show(ui, |ui| {
                        ui.add_space(10.);
                        ui.add_enabled_ui(enabled, |ui| self.connect_ui(ui, ctx));
                    });
            });
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::both()
                .id_salt("device_scroll")
                .show(ui, |ui| {
                    ui.add_space(10.);
                    ui.add_enabled_ui(enabled && self.pending.is_none(), |ui| {
                        self.device_ui(ui, ctx)
                    });
                });
        });
        if let Some(a) = self.confirm.clone() {
            egui::Window::new("Confirm device change")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0., 0.])
                .show(ctx, |ui| {
                    ui.strong(a.description());
                    if let Some(d) = self.selected.and_then(|i| self.devices.get(i)) {
                        ui.label(format!("{} · {} · {}", d.name, d.id, self.link.title()));
                    }
                    if let Action::Schedule(slot, s) = &a {
                        ui.label(
                            DAYS.iter()
                                .enumerate()
                                .filter_map(|(i, d)| (s.days & (1 << i) != 0).then_some(*d))
                                .collect::<Vec<_>>()
                                .join(", "),
                        );
                        if self.snapshot.as_ref().is_some_and(|snap| {
                            snap.slots.iter().enumerate().any(|(i, other)| {
                                i != *slot
                                    && other.enabled == Some(true)
                                    && other.settings.as_ref().is_some_and(|v| {
                                        v.days & s.days != 0 && v.start < s.end && s.start < v.end
                                    })
                            })
                        }) {
                            ui.colored_label(
                                Color32::DARK_RED,
                                "This window overlaps another enabled schedule.",
                            );
                        }
                        ui.label(format!(
                            "{}–{} · work {}s / pause {}s",
                            s.start, s.end, s.work, s.pause
                        ));
                    }
                    if matches!(a, Action::Pin(_)) {
                        ui.label("After applying, enter the new PIN in the connection panel.");
                    }
                    ui.small("This sends a real change to the selected diffuser.");
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            self.confirm = None
                        }
                        if ui.button("Apply change").clicked() {
                            self.confirm = None;
                            self.new_pin.clear();
                            self.confirm_pin.clear();
                            self.read_or_write(ctx, Some(a))
                        }
                    });
                });
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn loaded_device_renders_without_io_and_switching_clears_it() {
        let ctx = egui::Context::default();
        let mut app = Studio::new(&ctx).unwrap();
        app.devices.push(Device {
            id: "test".into(),
            name: "Fixture".into(),
            status: "Nearby".into(),
        });
        app.selected = Some(0);
        let mut s = Snapshot {
            online: true,
            ble: true,
            master: true,
            levels: true,
            cycle: Some(crate::model::Cycle::Standard),
            ..Default::default()
        };
        s.slots[0].settings = Some(Schedule {
            work: 15,
            ..Default::default()
        });
        app.snapshot = Some(s);
        app.populate();
        assert_eq!(app.draft.work, 15);
        let out = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| app.device_ui(ui, ctx));
        });
        assert!(!out.shapes.is_empty());
        assert!(app.pending.is_none() && app.confirm.is_none());
        app.route(Link::Cloud);
        assert!(app.devices.is_empty() && app.selected.is_none() && app.snapshot.is_none());
        assert_eq!(app.draft.work, 5);
    }
}
