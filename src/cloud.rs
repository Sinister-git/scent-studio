use crate::{
    model::{flag, Action, Cycle, Device, Schedule, Slot, Snapshot},
    protocol::{frame, response},
};
use anyhow::{ensure, Context, Result};
use reqwest::{header::HeaderValue, redirect::Policy, Client, RequestBuilder};
use serde_json::{json, Value};
use std::time::Duration;
const BASE: &str = "https://api.bx.ai-aroma.tech/xxiot/api";
#[derive(Clone)]
pub struct Account {
    owner_id: Option<String>,
    client: Client,
    token: HeaderValue,
}
fn text(v: &Value) -> String {
    v.as_str()
        .map(str::to_string)
        .or_else(|| v.as_number().map(ToString::to_string))
        .unwrap_or_default()
}
fn ok(body: Value) -> Result<Value> {
    let code = text(&body["code"]).parse::<i64>().ok();
    ensure!(
        code == Some(0),
        "Vendor rejected the request (code {}). Check sign-in and device permissions.",
        code.map(|n| n.to_string())
            .unwrap_or_else(|| "unknown".into())
    );
    Ok(body)
}
async fn send(r: RequestBuilder) -> Result<Value> {
    let resp = r
        .header("language", "en")
        .send()
        .await
        .context("Cannot reach Scent Master Pro")?
        .error_for_status()?;
    ok(resp.json().await.context("Invalid vendor response")?)
}
fn payload(t: &Value, key: &str, op: Option<u8>) -> Option<Vec<u8>> {
    let bytes: Vec<u8> = t[key]
        .as_str()?
        .split_ascii_whitespace()
        .map(|p| u8::from_str_radix(p, 16).ok())
        .collect::<Option<_>>()?;
    Some(
        response(op.unwrap_or(*bytes.get(2)?), &bytes)
            .ok()?
            .to_vec(),
    )
}
fn snapshot(d: &Value) -> Snapshot {
    let t = &d["latestSelfTelemetry"];
    let firmware = d["binVersion"].as_str().unwrap_or("");
    let ty = payload(t, "DEVICE_TYPE", None).and_then(|d| d.first().copied());
    let fan = payload(t, "FAN", Some(6)).and_then(|d| d.first().copied());
    let oil = payload(t, "REMAIN_INFO", None).and_then(|v| {
        let v = v.get(..8)?;
        Some(std::array::from_fn(|i| {
            u16::from_be_bytes([v[2 * i], v[2 * i + 1]])
        }))
    });
    let switch = |k: &str, op| {
        payload(t, k, Some(op))
            .and_then(|v| v.first().copied())
            .and_then(flag)
    };
    Snapshot {
        online: d["status"] == "ONLINE",
        master: true,
        accessories: if firmware.contains("BLEWIFI-BX-XXJ-MBOX-") {
            crate::accessories::Accessories {
                light: payload(t, "LIGHT_STATUS", Some(176)),
                music: payload(t, "MUSIC_STATUS", Some(192)),
                motion: payload(t, "BODY_SENSER_TIME", Some(177))
                    .and_then(|d| crate::accessories::motion(&d)),
                ..Default::default()
            }
        } else {
            Default::default()
        },
        power: switch("SWITCH", 7),
        lock: switch("LOCK", 9),
        levels: fan.is_some() && ty.unwrap_or(0) == 0 && !firmware.contains("MBOX"),
        fan,
        oil,
        oil_name: text(&d["oilName"]),
        name: display_name(d),
        cycle: Some(if ty == Some(1) && firmware.contains("BLEWIFI-BX-") {
            Cycle::Short
        } else {
            Cycle::Standard
        }),
        slots: std::array::from_fn(|i| {
            let n = i + 1;
            Slot {
                settings: payload(t, &format!("MODE{n}_SET"), Some(n as u8))
                    .and_then(|d| Schedule::decode(&d)),
                enabled: switch(&format!("MODE{n}_SWITCH"), 16 + n as u8),
            }
        }),
        ..Default::default()
    }
}
fn display_name(d: &Value) -> String {
    d["deviceName"]
        .as_str()
        .or(d["name"].as_str())
        .unwrap_or("Diffuser")
        .to_owned()
}
fn tx(id: &Value, op: u8, data: &[u8]) -> Value {
    let hex = frame(op, Some(data))
        .iter()
        .map(|v| format!("{v:02x}"))
        .collect::<Vec<_>>()
        .join(" ");
    json!({"id":id,"params":{"key":"TX","value":hex}})
}
impl Account {
    pub async fn login(account: String, password: String) -> Result<Self> {
        ensure!(
            !account.trim().is_empty() && !password.is_empty(),
            "Enter your account and password"
        );
        let client = Client::builder()
            .https_only(true)
            .redirect(Policy::none())
            .timeout(Duration::from_secs(15))
            .build()?;
        let login = send(
            client
                .post(format!("{BASE}/login"))
                .json(&json!({"account":account.trim(),"password":password})),
        )
        .await?;
        let mut token = HeaderValue::from_str(
            login["token"]
                .as_str()
                .filter(|s| !s.is_empty())
                .context("Login returned no session")?,
        )?;
        token.set_sensitive(true);
        // The mobile app only shows device administration to the account owner.
        // If this optional detail isn't available, other device controls still work.
        let owner_id = send(
            client
                .get(format!("{BASE}/customer/detail"))
                .header("token", &token),
        )
        .await
        .ok()
        .map(|v| text(&v["data"]["id"]))
        .filter(|s| !s.is_empty());
        Ok(Self {
            client,
            token,
            owner_id,
        })
    }
    pub async fn page(&self, page: u32) -> Result<(Vec<Device>, u32)> {
        let out = send(
            self.client
                .get(format!("{BASE}/device/list"))
                .header("token", &self.token)
                .query(&[
                    ("page", page.to_string().as_str()),
                    ("limit", "20"),
                    ("sim", "-1"),
                    ("surplus", "-1"),
                    ("deviceTypeId", ""),
                    ("status", ""),
                    ("keyword", ""),
                ]),
        )
        .await?;
        let records = out
            .pointer("/data/records")
            .and_then(Value::as_array)
            .context("Unexpected device list")?;
        let pages = text(&out["data"]["pages"])
            .parse()
            .unwrap_or(if records.len() == 20 {
                page.saturating_add(1)
            } else {
                page
            });
        let devices = records
            .iter()
            .filter(|v| !text(&v["id"]).is_empty())
            .map(|v| Device {
                id: text(&v["id"]),
                name: display_name(v),
                status: text(&v["status"]),
            })
            .collect();
        Ok((devices, pages))
    }
    pub async fn operate(&self, id: String, action: Option<Action>) -> Result<Option<Snapshot>> {
        let out = send(
            self.client
                .get(format!("{BASE}/device/detail"))
                .header("token", &self.token)
                .query(&[("id", &id)]),
        )
        .await?;
        let d = &out["data"];
        ensure!(
            !id.is_empty() && text(&d["id"]) == id,
            "Unexpected device identity"
        );
        let mut s = snapshot(d);
        s.cloud_rename = self
            .owner_id
            .as_ref()
            .is_some_and(|id| *id == text(&d["customerId"]));
        let Some(a) = action else { return Ok(Some(s)) };
        ensure!(s.online, "Device is offline; nothing sent");
        let (path, body) = if let Action::Name(name) = a {
            ensure!(
                s.cloud_rename && !name.trim().is_empty() && name.len() <= 16,
                "Owner access and a name of 1–16 bytes are required"
            );
            ("device/update", json!({"id":d["id"],"deviceName":name}))
        } else if let Action::OilName(name) = a {
            ensure!(
                name.len() <= 16 && !name.is_empty(),
                "Oil name must be 1–16 bytes"
            );
            (
                "device/update",
                json!({"id":d["id"],"oilName":name,"deviceName":s.name}),
            )
        } else {
            let (op, payload) = a.packet(&s)?;
            ("device/control", tx(&d["id"], op, &payload))
        };
        send(
            self.client
                .post(format!("{BASE}/{path}"))
                .header("token", &self.token)
                .json(&body),
        )
        .await?;
        Ok(None)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn telemetry() {
        let s = snapshot(
            &json!({"status":"ONLINE","latestSelfTelemetry":{"SWITCH":"55 11 07 05 01 01 1f","LOCK":"bad","MODE1_SWITCH":"55 11 11 05 01 01 29"}}),
        );
        assert_eq!(s.power, Some(true));
        assert_eq!(s.lock, None);
        assert_eq!(s.slots[0].enabled, Some(true));
        assert_eq!(s.slots[1].enabled, None);
    }
    #[test]
    fn request() {
        assert_eq!(
            tx(&json!("id"), 7, &[0])["params"]["value"],
            "55 10 07 05 01 00 1d"
        );
        assert!(ok(json!({"code":401})).is_err());
    }
}
