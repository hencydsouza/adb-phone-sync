import { AppShell } from "@astryxdesign/core/AppShell";
import { EmptyState } from "@astryxdesign/core/EmptyState";

function App() {
  return (
    <AppShell contentPadding={4}>
      <EmptyState
        description='Build the first screen with `bunx astryx build "<idea>"`.'
        title="ADB Phone Sync"
      />
    </AppShell>
  );
}

export default App;
