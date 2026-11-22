# Scent Master Pro 1.0.43: protocol evidence

These are static findings from the supplied release, not a capture from a PRO301. The model-to-retail-brand equivalence is supplied by the owner. Protocol selection in the app is by advertised name and flags, not the string `PRO301`.

## What was examined

- XAPK SHA-256: `7ddd3147a070aadcc05277ff804803af75126d94062a1036bde0ef67c6bfec06`. Base APK: `com.xxiot.bx.scent.apk`, SHA-256 `38bb43f89c1aba4287c63216ee63b44b1b1bcf1b809060b10b605103fbaaeef7`.
- REA bound the XAPK as a ZIP. Under `assets/SaaA_embed/` in the base APK are three wxapkg archives for the main app, `page_ble` and `page_network`. Their file tables contain a UTF-8 path, byte offset and size per member. The extracted `app-service.js` files were formatted for inspection, not executed. The app logic is in these bundles; a full native decompilation isn't necessary to recover these commands.
- REA static JS analysis parent: `ev_038b93218e5f2b33f50f0daafaf1ea58eaac08187ac70e3187df347ef328cad4`, focused on the untouched `page_ble/chunk_0.appservice.js`. The graph is partial; source locations below are separate direct readings. Local session checkpoint: `.analysis/rea-evidence.json`.
- This Windows REA Ghidra MCP provider rejected the ARM64 ELF `libapp.so` as an unsupported target. As a supplement, local Ghidra 12.2 headless imported `classes2.dex` (`9f2ebc49ee779555b220b657d4d685e82aa7350e81aff01443e5e39f4217a693`) and decoded the launcher invocations: `com.xxiot.bx.scent.ui.MainActivity.onCreate` (DEX code item `0x4203cc`) has a call to `RurkI` (`0x42034c`), which calls `SaaAApi.launchAppModule`. This corroborates the embedded mini-app route, not execution or a live BLE connection. See `.analysis/ScentEntry.java` and `.analysis/ghidra.log` locally.

## BLE observations

Source paths here are relative to `.analysis/js/`.

| Question | Observed source | Result |
|---|---|---|
| How does it select devices? | `page_ble/app-service.js` → `searchBleDevices`, `submitPsw`; root `app-service.js` → `onBluetoothDeviceFound`, `setBLEFoundDevice`, `connectBLE` | Name contains `YGZK-XXJ` or `BXXXJ-`. The PIN is checked locally before opening the selected peripheral; there is no PIN write in this connection path. Scent Studio keeps this check and never displays an advertised PIN. |
| Which GATT endpoints? | `submitPsw`, then `connectBLE`, `notifyBLECharacteristicValue`, `writeBLECharacteristicValue` | If name contains `-BLEWIFI-`: service **FEB3**, notify **FED6**, write **FED5**. Otherwise: service **FFE0**, notify **FFE1**, write **FFE2**. All use the Bluetooth base UUID. The command queue requests write-without-response; device replies arrive as notifications. |
| What is a command? | `page_ble/app-service.js` → `control`, `queueSendTask`, `bleNotifyCallback` | Request starts `55 10`, then opcode. Query has `00`; setter has `05`, one length byte and data. Append the low byte of the sum starting at `10` (exclude `55`). Some legacy names add a trailer **after** this checksum. Notifications begin `55 11` plus the opcode. |
| How are schedules read? | `initSendCMD`, `syncF1DataToDeviceInfo`, `syncF2DataToDeviceInfo`, `syncDataToDeviceInfo`, `modelTime`, `workTime`, `pauseTime` | BLEWIFI and related models query F1 (slot 1 plus other settings) and F2 (slots 2–5). Other supported branches use D1 for five slots, or separate reads. Each combined record is an enable byte plus nine settings bytes. Times and days refer to the device clock. |
| How are schedules changed? | `modelSwitchChange`, `saveEdit`, `weeks`, `onLoad` | `10h + slot` changes only the schedule enable byte. Opcodes `01`–`05` set the corresponding schedule: weekday mask (Mon=bit 0), start hour/minute, end hour/minute, work and pause seconds (two big-endian u16s). The app's allowed durations vary by firmware. Scent Studio uses the duration tables associated with the selected device profile and keeps enable/disable separate from saving. |

**Implementation differences:** Scent Studio uses an explicit device selection, never connects to the first discovery result, doesn't resend writes automatically, and doesn't reproduce app initialization side effects such as setting the clock or opening the master switch. It requires a usable reply with a valid frame and checksum, even on legacy models where the app accepts some checksum anomalies. It relies on the OS for MTU negotiation; combined schedule responses need more than a default 20-byte ATT payload. Mapping desktop manufacturer data to the app's advertising buffer and variant-specific reply layouts still need a physical-device check.

## Wi-Fi / cloud observations

`page_network/app-service.js` → `onLogin`, `getDeviceList`, `getDeviceInfo`, `totalSwitchChange`, `control`, `queueSendTask`; root `app-service.js` → `UserService`, `APP_USER_API`, request interceptor, `buf2hexBlank`:

1. The network screen uses the Scent Master Pro **account**, not the diffuser's four-digit code. Login returns a session token. Authenticated requests carry a `token` header. The root service config points at `https://api.bx.ai-aroma.tech/`.
2. The app lists and reads account devices using `/xxiot/api/device/list` and `/xxiot/api/device/detail`. Device detail carries last-known telemetry. The UI compares the `SWITCH` telemetry with known master-switch reply frames; the GUI reports `Not reported` rather than inferring off from missing telemetry.
3. The network power switch sends opcode `07`, value `01` or `00`. It serializes the same base command to lowercase, space-separated hex and POSTs a single selected ID with a `TX` parameter to `/xxiot/api/device/control`. A successful service response is not a device acknowledgement.
4. The app also sets up MQTT from account-provided device details for updates. The GUI does not require those credentials, subscribe to MQTT, or print the full device detail. **Refresh settings** reads HTTPS detail instead.

## Additional desktop controls: source and limits

- In `page_ble/app-service.js`, `setTimeNow`, `saveLevel`, `submitEditName`, `submitPsw`, `deleteModel`, and `totalSwitchOpen` establish the clock, fan/gear, name/note, PIN, empty-slot, and master-on commands. The BLE screen sends **on** for opcode 07. **Off on BLEWIFI** is inferred from the network handler for the same opcode; a BLE acknowledgement is required, and it has not been tested on a PRO301. F1's battery reading is not a master-switch reading.
- Network `lockSwitchChange`, `confirmFan`, `modelSwitchChange`, `saveEdit`, `save`, `goUpdate` and `submitEditName` establish the lock, fan, schedule, oil-estimate and owner rename paths. `MODE{n}_SET`, `MODE{n}_SWITCH`, `LOCK`, `FAN`, `SWITCH`, and `REMAIN_INFO` feed the GUI. Invalid frames are not interpreted as a reported value.
- `saveSenser`, `saveLevel`, `lightMoveChange`, `changeMusicMode`, `musicPlaySwitch`, `twsSwitchChange`, `bleNotifyCallback` and conditional rendering in the BLE/network pages describe the accessory controls. Music-box models can report optional speaker, lighting and sensor state; the BLE TJ-family may report a running mode or ionizer instead. These are **not** evidence that the purchased diffuser has those accessories. Missing compatible status hides the option. The GUI does not stream audio, automatically pair a speaker, or send accessory commands on ordinary Grand 300 profiles.

No direct peer-to-peer LAN control endpoint was identified in this path. That is not evidence that no firmware supports one. Wi-Fi provisioning, account registration and firmware updates were outside the implemented command set. No vendor endpoints were contacted during development.
