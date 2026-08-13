import { AppShell } from "@astryxdesign/core/AppShell";
import { Banner } from "@astryxdesign/core/Banner";
import { Button } from "@astryxdesign/core/Button";
import {
  SideNav,
  SideNavHeading,
  SideNavItem,
} from "@astryxdesign/core/SideNav";
import { VStack } from "@astryxdesign/core/VStack";
import { type ReactNode, useCallback, useState } from "react";
import { ClassificationScreen } from "./screens/classification-screen";
import { DeviceScreen } from "./screens/device-screen";
import { HistoryScreen } from "./screens/history-screen";
import { ProfileSettingsScreen } from "./screens/profile-settings-screen";
import { RunScreen } from "./screens/run-screen";

type Screen = "classify" | "device" | "history" | "profile" | "run";

/**
 * Minimal navigation to make all 5 screens reachable and let the device
 * serial / classification selection actually flow between them, per the
 * manual QA checklist's P0 item
 * (`docs/plans/2026-08-12-android-backup-restore-sync-manual-qa.md`).
 * Deliberately not a polished router -- just enough for a real click-through.
 */
function NeedsDeviceNotice({ onGoToDevices }: { onGoToDevices: () => void }) {
  return (
    <VStack gap={4}>
      <Banner
        description="Pick a device on the Device screen first."
        endContent={<Button label="Go to Devices" onClick={onGoToDevices} />}
        status="info"
        title="No device selected"
      />
    </VStack>
  );
}

function App() {
  const [activeScreen, setActiveScreen] = useState<Screen>("device");
  const [selectedSerial, setSelectedSerial] = useState<string | null>(null);
  const [includedPaths, setIncludedPaths] = useState<string[]>([]);

  const goToDevices = useCallback(() => setActiveScreen("device"), []);
  const goToClassify = useCallback(() => setActiveScreen("classify"), []);
  const goToRun = useCallback(() => setActiveScreen("run"), []);
  const goToHistory = useCallback(() => setActiveScreen("history"), []);
  const goToProfile = useCallback(() => setActiveScreen("profile"), []);

  const handleDeviceSelected = useCallback((serial: string) => {
    setSelectedSerial(serial);
    setActiveScreen("classify");
  }, []);

  const handleClassificationSaved = useCallback((paths: string[]) => {
    setIncludedPaths(paths);
    setActiveScreen("run");
  }, []);

  let content: ReactNode;
  if (activeScreen === "device") {
    content = <DeviceScreen onDeviceSelected={handleDeviceSelected} />;
  } else if (activeScreen === "classify") {
    content = selectedSerial ? (
      <ClassificationScreen
        onSaved={handleClassificationSaved}
        serial={selectedSerial}
      />
    ) : (
      <NeedsDeviceNotice onGoToDevices={goToDevices} />
    );
  } else if (activeScreen === "run") {
    content = selectedSerial ? (
      <RunScreen includedPaths={includedPaths} serial={selectedSerial} />
    ) : (
      <NeedsDeviceNotice onGoToDevices={goToDevices} />
    );
  } else if (activeScreen === "history") {
    content = <HistoryScreen serial={selectedSerial ?? undefined} />;
  } else {
    content = <ProfileSettingsScreen />;
  }

  return (
    <AppShell
      contentPadding={4}
      sideNav={
        <SideNav header={<SideNavHeading heading="ADB Phone Sync" />}>
          <SideNavItem
            isSelected={activeScreen === "device"}
            label="Device"
            onClick={goToDevices}
          />
          <SideNavItem
            isSelected={activeScreen === "classify"}
            label="Classify"
            onClick={goToClassify}
          />
          <SideNavItem
            isSelected={activeScreen === "run"}
            label="Run"
            onClick={goToRun}
          />
          <SideNavItem
            isSelected={activeScreen === "history"}
            label="History"
            onClick={goToHistory}
          />
          <SideNavItem
            isSelected={activeScreen === "profile"}
            label="Profile Settings"
            onClick={goToProfile}
          />
        </SideNav>
      }
    >
      {content}
    </AppShell>
  );
}

export default App;
