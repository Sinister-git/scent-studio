use anyhow::{bail, ensure, Context, Result};
use chrono::{Datelike, Local, Timelike};

pub const DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Cycle {
    Standard,
    Short,
    Dc(u16),
}
impl Cycle {
    pub fn work(self) -> Vec<u16> {
        match self {
            Self::Standard => (5..=300).step_by(5).collect(),
            Self::Short => (3..=25).collect(),
            Self::Dc(max) => (3..=max).collect(),
        }
    }
    pub fn pause(self) -> Vec<u16> {
        match self {
            Self::Standard => (5..=300)
                .step_by(5)
                .chain((350..=500).step_by(50))
                .chain((600..=900).step_by(100))
                .collect(),
            Self::Short => (60..=105)
                .step_by(5)
                .chain((110..=200).step_by(10))
                .chain((250..=450).step_by(50))
                .chain((500..=900).step_by(100))
                .collect(),
            Self::Dc(_) => (120..=300)
                .step_by(20)
                .chain([350, 400, 450, 600, 700, 800, 900])
                .collect(),
        }
    }
}
#[derive(Clone, PartialEq, Eq)]
pub struct Schedule {
    pub days: u8,
    pub start: String,
    pub end: String,
    pub work: u16,
    pub pause: u16,
}
impl Default for Schedule {
    fn default() -> Self {
        Self {
            days: 31,
            start: "09:00".into(),
            end: "17:00".into(),
            work: 5,
            pause: 120,
        }
    }
}
impl Schedule {
    pub fn decode(d: &[u8]) -> Option<Self> {
        let [days, h, m, hh, mm, wh, wl, ph, pl] = *d else {
            return None;
        };
        Some(Self {
            days,
            start: format!("{h:02}:{m:02}"),
            end: format!("{hh:02}:{mm:02}"),
            work: u16::from_be_bytes([wh, wl]),
            pause: u16::from_be_bytes([ph, pl]),
        })
    }
    pub fn configured(&self) -> bool {
        self.days != 0 && self.work != 0 && self.pause != 0
    }
    pub fn encode(&self, cycle: Cycle) -> Result<Vec<u8>> {
        fn time(v: &str) -> Result<(u8, u8)> {
            let (h, m) = v.split_once(':').context("Use HH:MM (24-hour time)")?;
            ensure!(
                h.len() == 2
                    && m.len() == 2
                    && h.bytes().chain(m.bytes()).all(|b| b.is_ascii_digit()),
                "Use HH:MM (24-hour time)"
            );
            let (h, m) = (h.parse()?, m.parse()?);
            ensure!(h < 24 && m < 60, "Invalid time");
            Ok((h, m))
        }
        let (a, b) = (time(&self.start)?, time(&self.end)?);
        ensure!(a < b, "End must be after start on the same day");
        ensure!(
            (1..=127).contains(&self.days),
            "Select at least one weekday"
        );
        ensure!(
            cycle.work().contains(&self.work) && cycle.pause().contains(&self.pause),
            "Choose durations supported by this firmware"
        );
        let mut v = vec![self.days, a.0, a.1, b.0, b.1];
        v.extend(self.work.to_be_bytes());
        v.extend(self.pause.to_be_bytes());
        Ok(v)
    }
}
#[derive(Clone, Default)]
pub struct Slot {
    pub enabled: Option<bool>,
    pub settings: Option<Schedule>,
}
#[derive(Clone, Default)]
pub struct Snapshot {
    pub accessories: crate::accessories::Accessories,
    pub online: bool,
    pub slots: [Slot; 5],
    pub cycle: Option<Cycle>,
    pub master: bool,
    pub power: Option<bool>,
    pub lock: Option<bool>,
    pub fan: Option<u8>,
    pub levels: bool,
    pub oil: Option<[u16; 4]>,
    pub ble: bool,
    pub labels: bool,
    pub name: String,
    pub cloud_rename: bool,
    pub oil_name: String,
}
impl Snapshot {
    pub fn block(&mut self, first: usize, count: usize, data: &[u8]) -> Result<()> {
        let data = data
            .get(..10 * count)
            .context("Incomplete schedule data (check BLE MTU)")?;
        for (i, s) in data.chunks_exact(10).enumerate() {
            self.slots[first + i] = Slot {
                enabled: flag(s[0]),
                settings: Schedule::decode(&s[1..]),
            };
        }
        Ok(())
    }
}
pub fn flag(v: u8) -> Option<bool> {
    match v {
        0 => Some(false),
        1 => Some(true),
        _ => None,
    }
}
#[derive(Clone)]
pub enum Action {
    Accessory(crate::accessories::Setting),
    Mode(usize, bool),
    Schedule(usize, Schedule),
    Clear(usize),
    Power(bool),
    Lock(bool),
    Fan(u8),
    Clock,
    Name(String),
    Note(String),
    Pin(String),
    Capacity(u16),
    Remaining(u16),
    Consumption(u16),
    OilName(String),
}
impl Action {
    pub fn description(&self) -> String {
        match self {
            Self::Mode(n, v) => format!(
                "{} schedule {}",
                if *v { "Enable" } else { "Disable" },
                n + 1
            ),
            Self::Schedule(n, _) => format!("Replace schedule {}", n + 1),
            Self::Clear(n) => format!("Clear schedule {}", n + 1),
            Self::Power(v) => format!("Turn master power {}", if *v { "on" } else { "off" }),
            Self::Lock(v) => {
                if *v {
                    "Lock device keys".into()
                } else {
                    "Unlock device keys".into()
                }
            }
            Self::Fan(v) => format!("Set fan / gear to {v}"),
            Self::Clock => "Sync device clock to this computer".into(),
            Self::Accessory(v) => v.describe(),
            Self::Name(v) => format!("Set device name to {v:?}"),
            Self::Note(_) => "Change device note".into(),
            Self::Pin(_) => "Change Bluetooth PIN".into(),
            Self::Capacity(v) => format!("Set bottle capacity to {v} mL"),
            Self::Remaining(v) => format!("Set remaining oil to {v} mL"),
            Self::Consumption(v) => format!("Set consumption to {:.1} mL/h", *v as f32 / 10.),
            Self::OilName(v) => format!("Set oil name to {v:?}"),
        }
    }
    pub fn packet(&self, s: &Snapshot) -> Result<(u8, Vec<u8>)> {
        ensure!(s.online, "Device is offline");
        if matches!(self, Self::Mode(..) | Self::Schedule(..) | Self::Clear(..)) {
            ensure!(
                s.accessories.motion.is_none_or(|v| v[0] == 0),
                "Disable motion mode before editing schedules"
            );
        }
        match self {
            Self::Mode(n, v) if *n < 5 => Ok((0x11 + *n as u8, vec![*v as u8])),
            Self::Schedule(n, sc) if *n < 5 => Ok((
                1 + *n as u8,
                sc.encode(s.cycle.context("Unknown schedule profile")?)?,
            )),
            Self::Clear(n) if *n < 5 => {
                ensure!(
                    s.slots
                        .iter()
                        .enumerate()
                        .any(|(i, v)| i != *n
                            && v.settings.as_ref().is_some_and(Schedule::configured)),
                    "Cannot clear the last known schedule"
                );
                Ok((1 + *n as u8, vec![0; 9]))
            }
            Self::Accessory(v) => v.packet(&s.accessories, s.cycle),
            Self::Power(v) if s.master => Ok((7, vec![*v as u8])),
            Self::Lock(v) if s.lock.is_some() && !s.ble => Ok((9, vec![*v as u8])),
            Self::Fan(v) if s.levels && *v <= 2 => Ok((6, vec![*v])),
            Self::Clock if s.ble => {
                let n = Local::now();
                Ok((
                    0,
                    vec![
                        1 << n.weekday().num_days_from_monday(),
                        n.hour() as u8,
                        n.minute() as u8,
                        n.second() as u8,
                    ],
                ))
            }
            Self::Pin(p) if s.labels => {
                ensure!(
                    p.len() == 4 && p.bytes().all(|b| b.is_ascii_digit()),
                    "PIN must be exactly four digits"
                );
                let mut v = p.as_bytes().to_vec();
                v.extend([0, 0]);
                Ok((0xdc, v))
            }
            Self::Name(t) | Self::Note(t) if s.labels => {
                ensure!(
                    t.len() <= 16 && t.bytes().all(|b| b == 32 || b.is_ascii_graphic()),
                    "Use up to 16 printable ASCII characters"
                );
                let mut v = t.as_bytes().to_vec();
                v.resize(16, 0);
                Ok((
                    if matches!(self, Self::Name(_)) {
                        0xda
                    } else {
                        0xdb
                    },
                    v,
                ))
            }
            Self::Capacity(n) if s.oil.is_some() && (1..=5000).contains(n) => {
                Ok((33, (n * 10).to_be_bytes().to_vec()))
            }
            Self::Remaining(n) if s.oil.is_some_and(|o| *n <= o[0] / 10) => {
                Ok((34, (n * 10).to_be_bytes().to_vec()))
            }
            Self::Consumption(n) if s.oil.is_some() && *n <= 100 => {
                Ok((35, n.to_be_bytes().to_vec()))
            }
            _ => bail!("Setting unavailable or value out of range for this device"),
        }
    }
}
#[derive(Clone)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub status: String,
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn schedule_roundtrip() {
        let bytes = [65, 8, 30, 18, 5, 0, 5, 1, 44];
        let s = Schedule::decode(&bytes).unwrap();
        assert_eq!(s.encode(Cycle::Standard).unwrap(), bytes);
        assert!(Schedule {
            days: 0,
            ..s.clone()
        }
        .encode(Cycle::Standard)
        .is_err());
        assert!(Schedule {
            end: "05:00".into(),
            ..s
        }
        .encode(Cycle::Standard)
        .is_err());
    }
    #[test]
    fn bounds() {
        let s = Snapshot {
            online: true,
            ..Default::default()
        };
        assert!(Action::Clear(0).packet(&s).is_err());
        assert!(Action::Lock(false).packet(&s).is_err());
        assert!(Action::Mode(5, true).packet(&s).is_err());
        assert_eq!(Action::Mode(4, true).packet(&s).unwrap(), (21, vec![1]));
    }
}
