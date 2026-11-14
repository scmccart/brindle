# Design

**Proposal-stage notes.** Nothing here has been tried on hardware yet.

## Goal

A pocket device that attaches to Brindle sessions on the user's hosts. It is mainly for attaching and glancing at running sessions, with some longer sessions. It connects over Wi-Fi first (a phone hotspot when away), with LTE optional later. E-ink is acceptable (the user already uses a Boox Palma).

## Hardware

- **LilyGO T-Deck MAX** (the device):
  - ESP32-S3: dual-core Xtensa LX7, 16 MB flash, 8 MB PSRAM.
  - 3.1" GDEQ031T10 e-paper at 320×240, with CST3530 touch. At a 6×8 font that is about 53×30 cells.
  - TCA8418 keyboard controller, XL9555 IO expander, DRV2605 haptics.
  - 1500 mAh battery, SY6970 charger, BQ27220 fuel gauge.
  - A7682E LTE Cat 1, ES8311 audio, SX1262 LoRa (915 MHz), GPS, TF card, USB-C. Brindle uses none of the LoRa, audio or GPS.
  - LilyGO's wiki notes ghosting after about 5 consecutive fast or partial refreshes, so the renderer refreshes changed rows partially and does a full refresh periodically.
- **LilyGO T-Display-S3** (the dev board): ESP32-S3R8, 16 MB flash, 8 MB PSRAM, 1.9" 170×320 ST7789 LCD. It brings up networking without the e-paper. On battery, GPIO15 must be driven high or the backlight stays off.
- Both boards use the ESP32-S3's built-in USB Serial/JTAG: flashing, logs and debugging (probe-rs or OpenOCD) all go over USB-C, with no external probe.

## Transport

```
   pocket client --ssh--> brindle-remote --attach --unix socket--> host core
                          (terminal-server-remote's relay, protocol 1)
```

- **The relay** (`terminal-server-remote`, phase 4) relays SSH stdio to the host core's socket and speaks client protocol 1. That gives `LIST` and `ATTACH` with screen replay (`REPLAY_START`/`DATA`/`REPLAY_END`), so sessions survive the device dropping off Wi-Fi or LTE, and reconnecting restores the screen. The pocket client is the relay's second client after the desktop app.
- **A possible stopgap** is the existing stdio `brindle-remote --server` protocol (`SPAWN`/`DATA`/`RESIZE` plus heartbeats). It has one session per SSH connection, ends after 60 s without a frame (`DEFAULT_IDLE_LIMIT_SECS`), and has no persistence or replay. It doesn't fit "attach and glance", so build the relay first.

## Firmware stack

Rust on ESP-IDF with std (esp-rs, target `xtensa-esp32s3-espidf`, toolchain from `espup`).

- **SSH:** a C library built as an ESP-IDF component and wrapped: libssh2 with mbedTLS, or libssh through `david-cermak/libssh`. This is lower risk than pure-Rust russh on the MCU. Host keys are pinned on first use (TOFU); keys and `known_hosts` live on the TF card.
- **Tailscale:** there is no official port. Community ESP-IDF clients exist:
  - MicroLink (`CamM2325/microlink`): Wi-Fi and 4G, ESP32-S3 with PSRAM recommended.
  - ciniml's from-scratch client (WireGuard, DERP fallback, DISCO direct paths), used in his Tab5 Portable Terminal. That project, an ESP32-P4 VT100 terminal with libssh2 and Tailscale, is prior art for the whole device.

  Expect immaturity. Use a tagged auth key with key expiry disabled.
- **LTE (later):** ESP-IDF's `esp_modem` (PPPoS) with the A7682E. Behind carrier-grade NAT, expect DERP-relayed paths rather than direct ones.
- **Protocol:** reuse `brindle-remote-protocol`. Its framing (`wire.rs`), `client.rs` and `version.rs` are plain std plus `serde`/`serde_json`, which work on ESP-IDF std. The platform code is `scope.rs` (`systemd-run`, `std::process`, Unix permissions) and `server_dir.rs` (the runtime directory). Note that `espidf` targets are `target_family = "unix"`, so the existing `#[cfg(unix)]` on `scope` does not exclude them: gate these behind a feature (default on) or `not(target_os = "espidf")`. The Windows helpers (`src/win.rs`) are in the app crate and don't affect the protocol crate.
- **Emulator:** the device needs one, because a replay is raw bytes. Preferred: reuse `brindle-term-core` (`alacritty_terminal` 0.26) for the same OSC 133 and program-status handling as the desktop. The risk is that `alacritty_terminal`'s tty code (rustix, signal-hook, polling) may not compile for `espidf`. Fallbacks: a fork with the tty module behind a feature flag, or a lighter emulator on `vte`. Not tried yet.
- **Memory:** PSRAM is required to hold WireGuard, Tailscale control, the SSH session and the emulator grid at once.

## Development flow

1. **Desktop:** a std core crate (protocol client, emulator, screen model) runs on Linux against a real `brindle-remote` through an `ssh` subprocess. It renders to a 320×240 `embedded-graphics-simulator` window with SDL keyboard input, drawing through `embedded-graphics` so the same code drives the e-paper.
2. **T-Display-S3:** bring up Wi-Fi, Tailscale and SSH, and measure memory. Input is bytes from `espflash monitor` (USB serial); output is logs plus a text dump of the grid, or the LCD.
3. **T-Deck MAX:** add the TCA8418 keyboard, the e-paper driver with partial refresh, haptics and the BQ27220 battery gauge.

The seam is a pair of `Input` and `Display` traits, so every stage runs the same core: SDL and the simulator, then serial or the LCD, then the TCA8418 and the e-paper.

## Points to settle

- **Where the firmware lives:** a workspace member or a separate repository. The Xtensa toolchain (`espup`, a non-rustup-standard toolchain) and the `Cargo.lock` constraints in CLAUDE.md (seeded from gpui's lockfile, no broad `cargo update`) argue for separate, with the shared crates taken by path or git.
- **Does `brindle-term-core` build for `xtensa-esp32s3-espidf`?** Try it first, before choosing the emulator.
- **The Tailscale client:** MicroLink or ciniml's.
- **Auth and key storage on the device:** SSH key and Tailscale auth key on the TF card, in flash (NVS, possibly encrypted), or both; what happens when the device is lost.
- **Liveness and heartbeat over the relay:** shared with `terminal-server-remote`'s open point. A device on a flaky network or asleep needs the connection marked lost while the session keeps running.
- **Screen size:** a 53×30 client attaching to a session sized for a desktop window. Decide whether attaching resizes the session, or the device views a part of it.
