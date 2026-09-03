//! Claiming the iPhone's PTP session.
//!
//! iOS speaks PTP, not USB mass storage, and PTP allows exactly one session at
//! a time. Desktop Linux races us for it: gvfs spawns a gphoto2 backend to
//! mount the phone the moment it is plugged in, and photo managers grab it on
//! the same signal. Whoever wins, everyone else gets "Could not claim the USB
//! device".
//!
//! Unlike the CLI version, we claim the session *once* and hold it for the
//! entire run instead of re-claiming it per file, so this eviction dance
//! happens once rather than a thousand times.

use anyhow::{bail, Result};
use gphoto2::{Camera, Context};
use std::process::Command;
use std::thread::sleep;
use std::time::Duration;

/// Processes that habitually squat on a freshly-attached camera.
const SQUATTERS: &[&str] = &[
    "gthumb",
    "shotwell",
    "gvfsd-gphoto2",
    "gvfs-gphoto2-volume-monitor",
];

pub fn evict_competitors() {
    for name in SQUATTERS {
        // Failure is the normal case (the process usually isn't running), so
        // the status is deliberately ignored. gvfsd will be respawned by D-Bus
        // later; we only need it gone long enough to claim the session.
        let _ = Command::new("pkill").arg("-f").arg(name).status();
    }
}

pub fn connect(attempts: u32, evict: bool) -> Result<Camera> {
    let mut last_err = None;

    for attempt in 1..=attempts {
        println!("Connection attempt {attempt}/{attempts}...");

        if evict {
            evict_competitors();
            // The sleeps are load-bearing: after a pkill the kernel needs a
            // moment to release the USB device, and iOS is slow to re-offer
            // the session afterwards.
            sleep(Duration::from_secs(2));
        }

        match Context::new().and_then(|ctx| ctx.autodetect_camera().wait()) {
            Ok(camera) => {
                println!("iPhone connection OK.");
                return Ok(camera);
            }
            Err(e) => {
                println!("Could not claim iPhone: {e}");
                last_err = Some(e);
                sleep(Duration::from_secs(3));
            }
        }
    }

    // Almost always one of: phone locked (a locked iPhone refuses PTP
    // outright), Trust not granted for this host, or cable/port trouble.
    bail!(
        "could not establish a connection to the iPhone after {attempts} attempts \
         (last error: {}). Make sure it is connected, unlocked, and trusts this computer.",
        last_err
            .map(|e| e.to_string())
            .unwrap_or_else(|| "none".into())
    )
}
