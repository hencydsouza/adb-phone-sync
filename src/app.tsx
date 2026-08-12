import { AppShell } from "@astryxdesign/core/AppShell";
import { useState } from "react";
import { DeviceScreen } from "./screens/device-screen";

function App() {
  // Lifted so future screens (Task 12/13) can react to the selected device.
  // Not read yet — the setter is the seam Task 12 wires into; the value
  // itself becomes live once a screen needs it.
  const [_selectedSerial, setSelectedSerial] = useState<string | null>(null);

  return (
    <AppShell contentPadding={4}>
      <DeviceScreen onDeviceSelected={setSelectedSerial} />
    </AppShell>
  );
}

export default App;
