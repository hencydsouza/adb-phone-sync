use tauri_plugin_shell::ShellExt;

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
            let state = parts.next()?;
            Some(Device {
                serial: serial.to_string(),
                state: state.to_string(),
            })
        })
        .collect()
}

#[tauri::command]
pub async fn list_devices(app: tauri::AppHandle) -> Result<Vec<Device>, String> {
    let (mut rx, _child) = app
        .shell()
        .sidecar("adb")
        .map_err(|e| e.to_string())?
        .args(["devices", "-l"])
        .spawn()
        .map_err(|e| e.to_string())?;

    let mut output = String::new();
    while let Some(event) = rx.recv().await {
        if let tauri_plugin_shell::process::CommandEvent::Stdout(bytes) = event {
            output.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    Ok(parse_adb_devices_output(&output))
}

#[cfg(test)]
mod tests {
    use super::*;

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
