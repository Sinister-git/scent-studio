# Scent Studio

A native Rust desktop application for Scent Master Pro-compatible diffusers, developed for the **NOBL Grand 300 / Crearoma PRO301**. Choose **Bluetooth** or **Wi-Fi cloud** in the window. The GUI calls the device backends directly; it does not launch a CLI, require a browser, or require the Android APK at runtime.

## Open the app

- **Windows:** Double-click **`target\release\scent-studio.exe`** in this project. This is the new GUI, not the old `scentctl.exe`.
- **macOS:** Open **Scent Studio.app** from the macOS build. Under *System Settings → Privacy & Security → Bluetooth*, allow the app to use Bluetooth. The local bundle is ad-hoc signed, not notarized.
- **Linux:** Extract the matching build archive and open `scent-studio` from a desktop session with an OpenGL driver, BlueZ and D-Bus. A BLE adapter is needed only for Bluetooth mode.

The Windows GUI was built and launch-tested locally. The included CI workflow builds macOS, Linux and Windows artifacts; the presence of the workflow is not a claim those jobs have run. There has been **no live test with a Grand 300 or vendor account**.

## First use

1. **Nearby control:** Close the phone app's BLE connection. Choose **Bluetooth**, enter your device PIN (the field starts at the factory `9999`), and click **Scan for diffusers**. Select the result matching your unit. Scanning alone doesn't connect or change anything. BLE operations check your PIN and disconnect when done.
2. **Remote control:** Choose **Wi-Fi cloud**. Enter your **Scent Master Pro account** and password, not the BLE PIN. Sign in and select a linked device. The diffuser must already be provisioned on 2.4 GHz Wi-Fi in the vendor app. Your computer only needs Internet access. There is no direct connection to a local IP address.
3. Selecting a device **reads** its settings. Compare the five slots to the phone app. Click a slot number to load it into the editor. Changes to fields and selections are drafts. **Apply change** in the confirmation dialog is what sends the command.
4. **Refresh settings** after any change, particularly after an error or timeout. A cloud acceptance isn't proof the hardware has acted, and telemetry can lag. The app never retries a device write automatically.

## Controls

Controls depend on the selected device, its protocol variant and the status fields it reports. The tabs are not two names for the same transport.

| Control | Bluetooth | Wi-Fi cloud |
|---|---|---|
| Five schedule slots: create/replace, enable/disable, clear | Supported profiles | Yes |
| Days, start/end, work seconds, pause seconds | Firmware-specific options | Firmware-specific options |
| Master power | BLEWIFI profile; OFF inferred from shared cloud framing, not hardware-verified | Yes; displays last reported state |
| Fan / gear 0–2 | Firmware-dependent | Compatible devices reporting fan status |
| Clock sync to computer | Yes | — |
| Physical keypad lock / unlock | — | When lock status is reported |
| Device name | Writable profiles | Owner account only |
| Device note / change device PIN | Writable profiles | — |
| Fragrance name, bottle capacity, remaining oil and consumption estimate | — | When bottle telemetry is available |
| Ambient light, rhythm, speaker transport/volume/EQ/stereo pairing, motion timer & duty cycle | Matching accessory firmware and replies only | Music-box firmware and matching telemetry only |
| Running mode / ionizer | Compatible TJ-family variants only | — |

**Schedule enable is not master power.** Saving a window doesn't enable its slot. An already enabled slot can start using changed settings immediately. The editor warns about overlapping enabled windows, rejects overnight windows, and will not clear the last known configured slot. Schedule writes are blocked while a reported motion timer is active. BLE power is not inferred from the battery field: `Not reported` is intentional. BLE names/notes support up to 16 printable ASCII characters. Fragrance estimates are device configuration, not direct measurements.

PIN, password and session token are not saved to disk. Sign out to discard the cloud session. Do not share passwords, tokens or raw advertisements when reporting an issue. Wi-Fi provisioning, account administration, payment, firmware updates and device binding/unbinding remain in the official app.

## Build from source

Use current stable Rust. `cargo run --release --locked` opens the GUI. For a release build and tests:

```sh
cargo build --release --locked
cargo test --release --locked
```

- Windows: MSVC toolchain with Visual Studio C++ Build Tools.
- macOS: install Xcode Command Line Tools, then `sh packaging/macos/bundle.sh` to build **`target/release/Scent Studio.app`** with a Bluetooth usage description. Launch the bundle for BLE permissions.
- Debian/Ubuntu: install `build-essential pkg-config libdbus-1-dev libxkbcommon-dev libwayland-dev libgl1-mesa-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev`. Enable Bluetooth and allow the desktop session normal BlueZ access. X11 and Wayland backends are enabled.

## Source map

`src/gui.rs` — native egui/eframe workspace and background operation handling; `src/model.rs` — schedules, capabilities and validated controls; `src/accessories.rs` — variant-specific options; `src/ble.rs`, `src/protocol.rs`, `src/cloud.rs` — transports and framing. See [protocol evidence](docs/protocol.md) for the APK and Ghidra findings and remaining hardware assumptions.
