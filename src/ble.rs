use crate::{
    model::{flag, Action, Cycle, Device, Schedule, Snapshot},
    protocol::{self, Layout, Profile},
};
use anyhow::{bail, ensure, Context, Result};
use btleplug::{
    api::{
        Central, CharPropFlags as Flags, Characteristic, Manager as _, Peripheral as _,
        PeripheralProperties, ValueNotification, WriteType,
    },
    platform::{Manager, Peripheral},
};
use futures_util::{stream::BoxStream, StreamExt};
use std::time::Duration;
use tokio::time::{sleep, timeout};
use uuid::Uuid;
fn uuid16(n: u16) -> Uuid {
    Uuid::from_u128(((n as u128) << 96) | 0x0000_1000_8000_0080_5f9b_34fb)
}
struct Session<'a> {
    dev: &'a Peripheral,
    profile: Profile,
    tx: Characteristic,
    rx: Uuid,
    stream: BoxStream<'static, ValueNotification>,
}
impl<'a> Session<'a> {
    async fn open(dev: &'a Peripheral, profile: Profile) -> Result<Self> {
        dev.connect()
            .await
            .context("Connection failed; disconnect Scent Master Pro first")?;
        dev.discover_services().await?;
        let (service, tx, rx) = if profile.wifi {
            (0xfeb3, 0xfed5, 0xfed6)
        } else {
            (0xffe0, 0xffe2, 0xffe1)
        };
        let find = |n, flags| {
            dev.characteristics()
                .into_iter()
                .find(|c| {
                    c.service_uuid == uuid16(service)
                        && c.uuid == uuid16(n)
                        && c.properties.intersects(flags)
                })
                .with_context(|| format!("GATT characteristic {n:04X} missing or incompatible"))
        };
        let tx = find(tx, Flags::WRITE_WITHOUT_RESPONSE)?;
        let rx = find(rx, Flags::NOTIFY | Flags::INDICATE)?;
        dev.subscribe(&rx).await?;
        let stream = dev.notifications().await?;
        Ok(Self {
            dev,
            profile,
            tx,
            rx: rx.uuid,
            stream,
        })
    }
    async fn exchange(&mut self, op: u8, data: Option<&[u8]>) -> Result<Vec<u8>> {
        self.dev
            .write(
                &self.tx,
                &self.profile.request(op, data),
                WriteType::WithoutResponse,
            )
            .await?;
        timeout(Duration::from_secs(5), async {
            while let Some(n) = self.stream.next().await {
                if n.uuid != self.rx || n.value.len() < 3 || n.value[..2] != [85, 17] {
                    continue;
                }
                let r = n.value[2];
                if r == op
                    || matches!((op, r), (195, 192) | (178, 176))
                    || (self.profile.layout == Layout::Bulk && op == 0xd1 && r == 0xf2)
                {
                    return Ok(self.profile.response(r, &n.value)?.to_vec());
                }
            }
            bail!("Notification stream ended")
        })
        .await
        .with_context(|| {
            format!("No reply to command {op:02X}. Refresh before retrying a change.")
        })?
    }
    async fn snapshot(&mut self) -> Result<Snapshot> {
        let p = &self.profile;
        let mut s = Snapshot {
            ble: true,
            online: true,
            cycle: Some(p.cycle),
            master: p.wifi,
            levels: p.levels,
            labels: p.layout != Layout::Individual,
            name: p.name.clone(),
            ..Default::default()
        };
        match p.layout {
            Layout::Split => {
                let d = self.exchange(0xf1, None).await?;
                s.block(0, 1, &d)?;
                s.fan = d.get(11).copied();
                if self.profile.name.contains("-DC-") && s.fan == Some(2) {
                    s.cycle = Some(Cycle::Dc(5))
                }
                s.block(1, 4, &self.exchange(0xf2, None).await?)?
            }
            Layout::Bulk => s.block(0, 5, &self.exchange(0xd1, None).await?)?,
            Layout::Individual => {
                for (i, slot) in s.slots.iter_mut().enumerate() {
                    slot.settings = Schedule::decode(&self.exchange(i as u8 + 1, None).await?);
                    slot.enabled = self
                        .exchange(i as u8 + 17, None)
                        .await?
                        .first()
                        .and_then(|v| flag(*v));
                }
            }
        }
        // Optional modules are never probed by name alone on an unrelated variant.
        let music = self.profile.name.contains("-BLEWIFI-M");
        if music {
            s.accessories.light = self.exchange(176, None).await.ok();
            s.accessories.music = self.exchange(192, None).await.ok();
        }
        if self.profile.name.contains("-DCTJ-") || self.profile.name.contains("-TJ-") {
            s.accessories.run_mode = self
                .exchange(210, None)
                .await
                .ok()
                .and_then(|v| v.first().copied())
                .filter(|n| matches!(n,1..=3|161..=163));
        }
        if music || s.accessories.run_mode.is_some_and(|n| n >= 161) {
            s.accessories.motion = self
                .exchange(177, None)
                .await
                .ok()
                .and_then(|v| crate::accessories::motion(&v));
        }
        if self.profile.name.contains("-TJUN-") {
            s.accessories.ion = self
                .exchange(6, None)
                .await
                .ok()
                .and_then(|v| v.first().and_then(|v| flag(*v)));
        }
        Ok(s)
    }
}
async fn scan() -> Result<Vec<(Peripheral, PeripheralProperties)>> {
    timeout(Duration::from_secs(45), scan_inner())
        .await
        .context("Bluetooth scan timed out; check adapter permissions")?
}
async fn scan_inner() -> Result<Vec<(Peripheral, PeripheralProperties)>> {
    let adapters = Manager::new().await?.adapters().await?;
    ensure!(
        !adapters.is_empty(),
        "No Bluetooth LE adapter available. Enable Bluetooth or use Wi-Fi."
    );
    let mut list = vec![];
    for a in adapters {
        a.start_scan(Default::default())
            .await
            .context("Cannot scan. Check Bluetooth permissions.")?;
        sleep(Duration::from_secs(5)).await;
        a.stop_scan().await?;
        for d in a.peripherals().await? {
            if let Some(p) = d.properties().await? {
                if p.local_name.as_deref().is_some_and(protocol::candidate) {
                    list.push((d, p))
                }
            }
        }
    }
    Ok(list)
}
pub async fn list() -> Result<Vec<Device>> {
    Ok(scan()
        .await?
        .into_iter()
        .map(|(d, p)| Device {
            id: d.id().to_string(),
            name: p.local_name.unwrap_or_default(),
            status: "Nearby".into(),
        })
        .collect())
}
pub async fn operate(id: String, pin: String, action: Option<Action>) -> Result<Option<Snapshot>> {
    let mut found = scan()
        .await?
        .into_iter()
        .filter(|(d, _)| d.id().to_string() == id);
    let (dev, p) = found
        .next()
        .context("Diffuser no longer visible. Scan again or close the mobile app.")?;
    ensure!(
        found.next().is_none(),
        "Ambiguous device ID across adapters"
    );
    ensure!(
        p.manufacturer_data.len() == 1,
        "Missing or ambiguous device advertisement"
    );
    let (company, body) = p.manufacturer_data.iter().next().unwrap();
    let mut ad = company.to_le_bytes().to_vec();
    ad.extend(body);
    let profile = protocol::verify(&p.local_name.unwrap_or_default(), &ad, pin.as_bytes())?;
    let r = timeout(Duration::from_secs(75), async {
        let mut session = Session::open(&dev, profile).await?;
        let s = session.snapshot().await?;
        if let Some(a) = action {
            let (op, payload) = a.packet(&s)?;
            let reply = session.exchange(op, Some(&payload)).await?;
            if !matches!(
                a,
                Action::Clock
                    | Action::Name(_)
                    | Action::Pin(_)
                    | Action::Accessory(
                        crate::accessories::Setting::Music(_)
                            | crate::accessories::Setting::Rhythm(_)
                    )
            ) {
                ensure!(
                    reply == payload,
                    "Unexpected acknowledgement. Refresh before retrying."
                )
            }
            Ok(None)
        } else {
            Ok(Some(s))
        }
    })
    .await;
    let _ = timeout(Duration::from_secs(3), dev.disconnect()).await;
    r.context("BLE operation timed out. Refresh before retrying.")?
}
