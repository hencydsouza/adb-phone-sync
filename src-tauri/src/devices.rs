use std::time::Duration;
use tauri_plugin_shell::process::CommandEvent;
use tauri_plugin_shell::ShellExt;

/// Bound on how long we wait for `adb devices -l` to finish. Device listing
/// is a cheap, local operation, so a few seconds is generous; this exists
/// purely to guarantee the awaited Tauri command always resolves instead of
/// hanging forever if the sidecar process ever gets stuck.
const LIST_DEVICES_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(serde::Serialize, Debug, PartialEq, Eq)]
pub struct Device {
    pub serial: String,
    pub state: String,
}

pub fn parse_adb_devices_output(raw: &str) -> Vec<Device> {
    raw.lines()
        .skip(1) // "List of devices attached"
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let serial = parts.next()?;
            // NOTE: this only takes the second whitespace-separated token as
            // the state, so multi-word adb states (e.g. "no permissions
            // (missing udev rules?)") get truncated to just "no". Not
            // handled today; revisit if we need to surface those states.
            let state = parts.next()?;
            Some(Device {
                serial: serial.to_string(),
                state: state.to_string(),
            })
        })
        .collect()
}

/// How many times to retry `adb devices -l` if it fails specifically
/// because the adb server was mid-restart, and how long to wait between
/// attempts. Found via manual QA: when another adb installation (e.g.
/// Android Studio's bundled adb) is also running on this machine, our
/// bundled `adb.exe` can detect a server-version mismatch on startup, kill
/// the existing daemon, and restart its own — a real, normally-brief race
/// during which `adb devices` fails with `"daemon not running... could not
/// read ok from ADB Server... failed to start daemon... cannot connect to
/// daemon"`. This is not a bug in our command construction (confirmed: the
/// error text is `adb`'s own, verbatim) and not something a fixed timeout
/// bound can rule out ahead of time -- it's inherently racy, so retrying
/// after a short delay (rather than surfacing the error to the user
/// immediately) mirrors what re-plugging the device was doing anyway: just
/// giving the daemon restart enough time to finish.
const DAEMON_RESTART_RETRY_ATTEMPTS: u32 = 3;
const DAEMON_RESTART_RETRY_DELAY: Duration = Duration::from_millis(1500);

/// Whether an error looks like the daemon-restart race described above,
/// rather than some other failure (missing sidecar, genuinely no device,
/// unrelated adb error) that retrying the exact same command wouldn't fix.
fn is_daemon_restart_race(error: &str) -> bool {
    let lower = error.to_lowercase();
    lower.contains("cannot connect to daemon")
        || lower.contains("failed to start daemon")
        || lower.contains("daemon not running")
}

#[tauri::command]
pub async fn list_devices(app: tauri::AppHandle) -> Result<Vec<Device>, String> {
    let mut last_error = String::new();
    for attempt in 0..DAEMON_RESTART_RETRY_ATTEMPTS {
        if attempt > 0 {
            tokio::time::sleep(DAEMON_RESTART_RETRY_DELAY).await;
        }
        match run_adb_devices_once(&app).await {
            Ok(devices) => return Ok(devices),
            Err(err) if is_daemon_restart_race(&err) => last_error = err,
            Err(err) => return Err(err),
        }
    }
    Err(last_error)
}

async fn run_adb_devices_once(app: &tauri::AppHandle) -> Result<Vec<Device>, String> {
    // NOTE: the `shell:allow-execute` scope entry in capabilities/default.json
    // only gates shell invocations initiated from frontend JS through the
    // shell plugin's JS API. It does not gate this Rust-side
    // `Shell::sidecar()` call — removing that capability entry would not
    // change anything for this command. Its `adb` entry uses `"args": true`
    // (unrestricted) rather than pinning specific args like `["devices",
    // "-l"]`: `space.rs` and `sync::orchestration` both invoke the same
    // `adb` sidecar with different args (`shell du ...`, `shell -s <serial>
    // ...`), and since this scope isn't load-bearing for any Rust-side call
    // anyway, pinning it to one command's args would only create the false
    // impression that `adb` is restricted to a harmless read-only
    // invocation when it isn't.
    let (mut rx, child) = app
        .shell()
        .sidecar("adb")
        .map_err(|e| e.to_string())?
        .args(["devices", "-l"])
        .spawn()
        .map_err(|e| e.to_string())?;

    let mut stdout = String::new();
    let mut stderr = String::new();

    let receive_result = tokio::time::timeout(LIST_DEVICES_TIMEOUT, async {
        // `Terminated` is emitted by a separate OS thread (the blocking
        // `child.wait()`) racing against the stdout/stderr pipe-reader
        // threads onto the same channel — there's no guarantee all buffered
        // Stdout chunks have been delivered by the time `Terminated` shows
        // up. So we only *record* the exit code here and keep draining the
        // loop; it only ends naturally once `rx.recv()` returns `None`,
        // i.e. once every sender (pipe readers included) has been dropped
        // and all output has actually been received. This mirrors
        // `Command::output()`'s own pattern in tauri-plugin-shell.
        let mut exit_code: Option<Option<i32>> = None;
        while let Some(event) = rx.recv().await {
            match event {
                CommandEvent::Stdout(bytes) => {
                    stdout.push_str(&String::from_utf8_lossy(&bytes));
                }
                CommandEvent::Stderr(bytes) => {
                    stderr.push_str(&String::from_utf8_lossy(&bytes));
                }
                CommandEvent::Error(err) => {
                    stderr.push_str(&err);
                }
                CommandEvent::Terminated(payload) => {
                    exit_code = Some(payload.code);
                }
                _ => {}
            }
        }
        // Channel closed without ever seeing a Terminated event — treat as
        // failure rather than silently returning an empty device list.
        exit_code
    })
    .await;

    match receive_result {
        Ok(Some(Some(0))) => Ok(parse_adb_devices_output(&stdout)),
        Ok(Some(code)) => Err(format!(
            "adb devices exited with code {code:?}: {}",
            stderr.trim()
        )),
        Ok(None) => Err(format!(
            "adb devices process ended unexpectedly: {}",
            stderr.trim()
        )),
        Err(_) => {
            // Timed out waiting for the process to finish; kill it so it
            // doesn't linger, and surface the hang as an error instead of
            // leaving the frontend awaiting forever.
            let _ = child.kill();
            Err("timed out waiting for `adb devices` to respond".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_daemon_restart_race_recognizes_the_real_error_text() {
        // Exact text from the manual QA report this retry was added for.
        let real_error = "adb devices exited with code Some(1): * daemon not running; \
            starting now at tcp:5037 could not read ok from ADB Server * failed to start \
            daemon adb.exe: failed to check server version: cannot connect to daemon";
        assert!(is_daemon_restart_race(real_error));
    }

    #[test]
    fn is_daemon_restart_race_does_not_match_unrelated_errors() {
        assert!(!is_daemon_restart_race("timed out waiting for `adb devices` to respond"));
        assert!(!is_daemon_restart_race(
            "adb devices exited with code Some(1): unauthorized"
        ));
    }

    #[test]
    fn parses_adb_devices_dash_l_output() {
        let raw = "List of devices attached\n\
                    00070344C000047        device usb:1-1 product:Nothing model:Phone_2a device:Spacewar transport_id:3\n\
                    emulator-5554           offline\n";
        let devices = parse_adb_devices_output(raw);
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].serial, "00070344C000047");
        assert_eq!(devices[0].state, "device");
        assert_eq!(devices[1].serial, "emulator-5554");
        assert_eq!(devices[1].state, "offline");
    }

    #[test]
    fn ignores_the_header_and_blank_lines() {
        let devices = parse_adb_devices_output("List of devices attached\n\n");
        assert!(devices.is_empty());
    }
}
