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
import { useCallback, useEffect, useState } from "react";

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

export function DeviceScreen() {
  const [devices, setDevices] = useState<Device[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [selectedSerial, setSelectedSerial] = useState<string | null>(null);

  const loadDevices = useCallback(() => {
    setIsLoading(true);
    setError(null);
    invoke<Device[]>("list_devices")
      .then((result) => {
        setDevices(result);
      })
      .catch((err: unknown) => {
        setError(err instanceof Error ? err.message : String(err));
      })
      .finally(() => {
        setIsLoading(false);
      });
  }, []);

  useEffect(() => {
    loadDevices();
  }, [loadDevices]);

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
            onSelect={setSelectedSerial}
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
