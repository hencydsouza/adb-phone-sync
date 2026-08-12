import { Badge } from "@astryxdesign/core/Badge";
import { Banner } from "@astryxdesign/core/Banner";
import { Button } from "@astryxdesign/core/Button";
import { EmptyState } from "@astryxdesign/core/EmptyState";
import { Heading } from "@astryxdesign/core/Heading";
import { List, ListItem } from "@astryxdesign/core/List";
import { StatusDot } from "@astryxdesign/core/StatusDot";
import { Text } from "@astryxdesign/core/Text";
import { VStack } from "@astryxdesign/core/VStack";
import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useRef, useState } from "react";

/** Mirrors the `Device` struct serialized by `src-tauri/src/devices.rs`. */
interface Device {
  serial: string;
  state: string;
}

/** adb reports "device" for a fully authorized, ready-to-use connection. */
const READY_STATE = "device";

function statusVariantForState(state: string): "success" | "warning" {
  return state === READY_STATE ? "success" : "warning";
}

interface DeviceScreenProps {
  /**
   * Called when the user picks a device, in addition to the screen's own
   * local selection state. Lets a parent (e.g. App) lift the selection for
   * later screens to react to. Optional so the screen stays usable
   * standalone/in tests.
   */
  onDeviceSelected?: (serial: string) => void;
}

export function DeviceScreen({ onDeviceSelected }: DeviceScreenProps = {}) {
  const [devices, setDevices] = useState<Device[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [selectedSerial, setSelectedSerial] = useState<string | null>(null);

  // Every fetch (mount AND retry) bumps a shared generation counter and
  // checks against it before touching state, so a stale in-flight request
  // from an earlier click/mount can never clobber state set by a newer one.
  // Ported from `classification-screen.tsx`, which fixed this same gap after
  // Task 11's review noted this screen's original closure-based `cancelled`
  // flag only guarded the mount effect's own fetch -- the "Retry" buttons
  // called `loadDevices` directly without capturing/using its returned
  // cancel closure, so rapidly clicking Retry could let a stale response
  // clobber a newer one.
  const fetchGenerationRef = useRef(0);

  const loadDevices = useCallback(() => {
    fetchGenerationRef.current += 1;
    const generation = fetchGenerationRef.current;
    setIsLoading(true);
    setError(null);

    invoke<Device[]>("list_devices")
      .then((result) => {
        if (fetchGenerationRef.current !== generation) {
          return;
        }
        setDevices(result);
        // Drop the selection if the previously-selected device is no longer
        // present in the refreshed list.
        setSelectedSerial((prev) =>
          prev !== null && !result.some((device) => device.serial === prev)
            ? null
            : prev
        );
      })
      .catch((err: unknown) => {
        if (fetchGenerationRef.current !== generation) {
          return;
        }
        setError(err instanceof Error ? err.message : String(err));
      })
      .finally(() => {
        if (fetchGenerationRef.current !== generation) {
          return;
        }
        setIsLoading(false);
      });
  }, []);

  useEffect(() => {
    loadDevices();
  }, [loadDevices]);

  const handleSelect = useCallback(
    (serial: string) => {
      setSelectedSerial(serial);
      onDeviceSelected?.(serial);
    },
    [onDeviceSelected]
  );

  if (error) {
    return (
      <VStack gap={4}>
        <Heading level={1}>Select a device</Heading>
        <Banner
          description={error}
          endContent={<Button label="Retry" onClick={loadDevices} />}
          status="error"
          title="Failed to list devices"
        />
      </VStack>
    );
  }

  if (isLoading) {
    return (
      <VStack gap={4}>
        <Heading level={1}>Select a device</Heading>
        <Text color="secondary">Looking for connected devices…</Text>
      </VStack>
    );
  }

  if (devices.length === 0) {
    return (
      <EmptyState
        actions={<Button label="Retry" onClick={loadDevices} />}
        description="Connect an Android device over USB and make sure USB debugging is enabled, then retry."
        title="No devices connected"
      />
    );
  }

  return (
    <VStack gap={4}>
      <Heading level={1}>Select a device</Heading>
      <List hasDividers header={<Text type="label">Connected devices</Text>}>
        {devices.map((device) => (
          <DeviceListItem
            device={device}
            isSelected={device.serial === selectedSerial}
            key={device.serial}
            onSelect={handleSelect}
          />
        ))}
      </List>
    </VStack>
  );
}

function DeviceListItem({
  device,
  isSelected,
  onSelect,
}: {
  device: Device;
  isSelected: boolean;
  onSelect: (serial: string) => void;
}) {
  const handleClick = useCallback(() => {
    onSelect(device.serial);
  }, [device.serial, onSelect]);

  return (
    <ListItem
      description={`Status: ${device.state}`}
      endContent={
        isSelected ? <Badge label="Selected" variant="info" /> : undefined
      }
      isSelected={isSelected}
      label={device.serial}
      onClick={handleClick}
      startContent={
        <StatusDot
          label={device.state}
          variant={statusVariantForState(device.state)}
        />
      }
    />
  );
}
