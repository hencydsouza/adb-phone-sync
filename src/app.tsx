import { AppShell } from "@astryxdesign/core/AppShell";
import { DeviceScreen } from "./screens/device-screen";

function App() {
  return (
    <AppShell contentPadding={4}>
      <DeviceScreen />
    </AppShell>
  );
}

export default App;
