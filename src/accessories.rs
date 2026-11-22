//! Optional controls. Match the app's model gates AND require a compatible status reply.
use crate::model::Cycle;
use anyhow::{bail, ensure, Context, Result};
use eframe::egui::{self, Ui};

#[derive(Clone, Default)]
pub struct Accessories {
    pub light: Option<Vec<u8>>,
    pub music: Option<Vec<u8>>,
    pub motion: Option<[u16; 3]>,
    pub run_mode: Option<u8>,
    pub ion: Option<bool>,
}
pub fn motion(bytes: &[u8]) -> Option<[u16; 3]> {
    let v = bytes.get(..6)?;
    Some(std::array::from_fn(|i| {
        u16::from_be_bytes([v[i * 2], v[i * 2 + 1]])
    }))
}
const COLORS: [(&str, u8); 9] = [
    ("Off", 0),
    ("Gradient", 255),
    ("Purple", 1),
    ("Red", 2),
    ("Orange", 3),
    ("Yellow", 4),
    ("Green", 5),
    ("Cyan", 6),
    ("Blue", 7),
];
const MUSIC: [(&str, u8); 11] = [
    ("Play", 1),
    ("Pause", 0),
    ("Previous", 161),
    ("Next", 162),
    ("Volume +", 192),
    ("Volume −", 193),
    ("Vocal EQ", 176),
    ("Balanced EQ", 177),
    ("Bass EQ", 178),
    ("Stereo pair on", 209),
    ("Stereo pair off", 208),
];
#[derive(Clone)]
pub enum Setting {
    Color(u8),
    Rhythm(bool),
    Music(u8),
    Motion([u16; 3]),
    Run(u8),
    Ion(bool),
}
impl Setting {
    pub fn describe(&self) -> String {
        match self {
            Self::Color(c) => format!(
                "Light: {}",
                COLORS
                    .iter()
                    .find(|(_, v)| v == c)
                    .map_or("unknown", |p| p.0)
            ),
            Self::Music(m) => format!(
                "Speaker: {}",
                MUSIC
                    .iter()
                    .find(|(_, v)| v == m)
                    .map_or("unknown", |p| p.0)
            ),
            Self::Rhythm(v) => format!("Rhythmic lighting: {}", if *v { "on" } else { "off" }),
            Self::Motion([d, w, p]) => format!("Motion timer: {d} min, work {w}s / pause {p}s"),
            Self::Ion(v) => format!("Ionizer: {}", if *v { "on" } else { "off" }),
            Self::Run(v) => format!("Set running mode {v}"),
        }
    }
    pub fn packet(&self, a: &Accessories, c: Option<Cycle>) -> Result<(u8, Vec<u8>)> {
        match self {
            Self::Color(v) if COLORS.iter().any(|p| p.1 == *v) => {
                let l = a
                    .light
                    .as_ref()
                    .filter(|d| !d.is_empty())
                    .context("No lighting status")?;
                let mut d = vec![*v];
                if let Some(r) = l.get(1) {
                    d.push(*r)
                }
                Ok((176, d))
            }
            Self::Rhythm(v) if a.light.as_ref().is_some_and(|l| l.len() == 2) => {
                Ok((178, vec![*v as u8]))
            }
            Self::Music(v) => {
                let m = a
                    .music
                    .as_ref()
                    .filter(|m| m.len() >= 4 && m[0] != 0 && m[1] != 0)
                    .context("Connect and power on the speaker first")?;
                ensure!(
                    MUSIC.iter().any(|p| p.1 == *v)
                        && (!matches!(v, 176..=178) || m.len() >= 5)
                        && (!matches!(v, 208..=209) || m.len() >= 6),
                    "Speaker feature not available"
                );
                Ok((195, vec![*v]))
            }
            Self::Motion([t, w, p]) if a.motion.is_some() => {
                let c = c.context("Unknown duty cycle")?;
                ensure!(
                    (*t == 0 || *t == 15 || *t == 30 || (*t <= 1440 && *t % 60 == 0))
                        && c.work().contains(w)
                        && c.pause().contains(p),
                    "Unsupported motion-sensor timing"
                );
                let mut d = vec![];
                for n in [t, w, p] {
                    d.extend(n.to_be_bytes())
                }
                Ok((177, d))
            }
            Self::Run(v) if (1..=3).contains(v) => {
                let r = a.run_mode.context("Running mode not available")?;
                Ok((210, vec![(r & 0xa0) | v]))
            }
            Self::Ion(v) if a.ion.is_some() => Ok((6, vec![*v as u8])),
            _ => bail!("Unsupported accessory"),
        }
    }
}
pub fn show(
    ui: &mut Ui,
    a: &Accessories,
    c: Option<Cycle>,
    draft: &mut [u16; 3],
) -> Option<Setting> {
    let mut change = None;
    if let Some(r) = a.run_mode {
        ui.horizontal_wrapped(|ui| {
            ui.label("Running mode");
            for (i, label) in ["Performance", "Balanced", "Quiet"].iter().enumerate() {
                if ui
                    .selectable_label((r & 3) == 1 + i as u8, *label)
                    .clicked()
                {
                    change = Some(Setting::Run(1 + i as u8))
                }
            }
        });
    }
    if let Some(v) = a.ion {
        if ui
            .button(if v {
                "Turn ionizer off"
            } else {
                "Turn ionizer on"
            })
            .clicked()
        {
            change = Some(Setting::Ion(!v))
        }
    }
    if let Some(l) = a.light.as_ref().filter(|l| !l.is_empty()) {
        ui.collapsing("Ambient lighting", |ui| {
            ui.horizontal_wrapped(|ui| {
                for (label, v) in COLORS {
                    if ui.selectable_label(l[0] == v, label).clicked() {
                        change = Some(Setting::Color(v))
                    }
                }
            });
            if l.len() == 2
                && ui
                    .button(if l[1] == 1 {
                        "Disable rhythmic lighting"
                    } else {
                        "Enable rhythmic lighting"
                    })
                    .clicked()
            {
                change = Some(Setting::Rhythm(l[1] != 1))
            }
        });
    }
    if let Some(m) = a.music.as_ref().filter(|m| m.len() >= 4) {
        ui.collapsing("Speaker", |ui| {
            ui.label(format!(
                "{} · volume {}",
                if m[0] == 0 {
                    "Powered off"
                } else if m[1] == 0 {
                    "Not paired"
                } else if m[3] == 0 {
                    "Paused"
                } else {
                    "Playing"
                },
                m[2]
            ));
            ui.horizontal_wrapped(|ui| {
                for (label, v) in MUSIC {
                    let command = Setting::Music(v);
                    if ui
                        .add_enabled(command.packet(a, c).is_ok(), egui::Button::new(label))
                        .clicked()
                    {
                        change = Some(command)
                    }
                }
            });
        });
    }
    if let (Some(_), Some(c)) = (a.motion, c) {
        ui.collapsing("Motion-triggered diffusion", |ui| {
            ui.horizontal_wrapped(|ui| {
                for (i, label, choices) in [
                    (
                        0,
                        "Duration (min)",
                        vec![0, 15, 30]
                            .into_iter()
                            .chain((1..=24).map(|h| 60 * h))
                            .collect(),
                    ),
                    (1, "Sensor work (s)", c.work()),
                    (2, "Sensor pause (s)", c.pause()),
                ] {
                    ui.label(label);
                    egui::ComboBox::from_id_salt(label)
                        .selected_text(draft[i].to_string())
                        .show_ui(ui, |ui| {
                            for n in choices {
                                ui.selectable_value(&mut draft[i], n, n.to_string());
                            }
                        });
                }
            });
            ui.small("Choose duration 0 to disable motion mode and use schedules.");
            if ui.button("Save sensor settings…").clicked() {
                change = Some(Setting::Motion(*draft))
            }
        });
    }
    change
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserve_light_rhythm() {
        let a = Accessories {
            light: Some(vec![1, 1]),
            ..Default::default()
        };
        assert_eq!(
            Setting::Color(7).packet(&a, None).unwrap(),
            (176, vec![7, 1])
        );
        assert!(Setting::Music(1).packet(&a, None).is_err());
    }
}
