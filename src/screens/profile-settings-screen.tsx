import { Banner } from "@astryxdesign/core/Banner";
import { Button } from "@astryxdesign/core/Button";
import { Card } from "@astryxdesign/core/Card";
import { EmptyState } from "@astryxdesign/core/EmptyState";
import { Heading } from "@astryxdesign/core/Heading";
import { HStack } from "@astryxdesign/core/HStack";
import { Text } from "@astryxdesign/core/Text";
import { TextInput } from "@astryxdesign/core/TextInput";
import { VStack } from "@astryxdesign/core/VStack";
import { eq } from "drizzle-orm";
import { useCallback, useEffect, useRef, useState } from "react";
import { db } from "../db/client";
import { devices } from "../db/schema";

type Device = typeof devices.$inferSelect;

/** Per-device editable draft, seeded from the loaded row (see `seedDraft`)
 * and reseeded whenever `load` replaces `devicesList`. Kept as its own field
 * (rather than editing `devicesList` in place) so a save failure doesn't
 * leave the list showing an unsaved value as if it were persisted. */
interface DeviceDraft {
  destinationPath: string;
  displayName: string;
}

function seedDraft(device: Device): DeviceDraft {
  return {
    destinationPath: device.destinationPath ?? "",
    displayName: device.displayName,
  };
}

function formatDateTime(date: Date): string {
  return date.toLocaleString();
}

/**
 * Persists one device's edited `displayName`/`destinationPath`. A single
 * `UPDATE ... WHERE serial = ?` statement -- no transaction needed (per
 * Task 13's review note on `persistRunResult`, transactions matter when
 * multiple statements must land atomically; this is one statement touching
 * one row).
 */
async function saveDeviceProfile(
  serial: string,
  draft: DeviceDraft
): Promise<void> {
  await db
    .update(devices)
    .set({
      destinationPath: draft.destinationPath.trim() || null,
      displayName: draft.displayName.trim(),
    })
    .where(eq(devices.serial, serial));
}

export function ProfileSettingsScreen() {
  const [devicesList, setDevicesList] = useState<Device[]>([]);
  const [drafts, setDrafts] = useState<Record<string, DeviceDraft>>({});
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  // A set (not a single scalar) so saving one device's card doesn't clear
  // another's in-flight "Saving…" state -- e.g. clicking Save on device B
  // while device A's save is still in flight must leave A's button spinning.
  const [savingSerials, setSavingSerials] = useState<ReadonlySet<string>>(
    new Set()
  );
  const [saveErrorBySerial, setSaveErrorBySerial] = useState<
    Record<string, string>
  >({});
  const [savedSerial, setSavedSerial] = useState<string | null>(null);

  // Same generation-counter guard as `history-screen.tsx`/`classification-screen.tsx`:
  // a stale in-flight fetch (e.g. a fast double-click on Refresh) can never
  // clobber state set by a newer one.
  const fetchGenerationRef = useRef(0);

  const load = useCallback(() => {
    fetchGenerationRef.current += 1;
    const generation = fetchGenerationRef.current;
    setIsLoading(true);
    setError(null);

    db.select()
      .from(devices)
      .orderBy(devices.displayName)
      .then((result) => {
        if (fetchGenerationRef.current !== generation) {
          return;
        }
        setDevicesList(result);
        setDrafts(
          Object.fromEntries(
            result.map((device) => [device.serial, seedDraft(device)])
          )
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
    load();
  }, [load]);

  const handleDraftChange = useCallback(
    (serial: string, field: keyof DeviceDraft, value: string) => {
      setDrafts((prev) => ({
        ...prev,
        [serial]: { ...prev[serial], [field]: value },
      }));
      // Editing after a save invalidates that row's "Saved" confirmation and
      // any earlier error for it -- both would otherwise describe a value
      // that's no longer what's on screen.
      setSavedSerial((prev) => (prev === serial ? null : prev));
      setSaveErrorBySerial((prev) => {
        if (!(serial in prev)) {
          return prev;
        }
        const next = { ...prev };
        delete next[serial];
        return next;
      });
    },
    []
  );

  const handleSave = useCallback(
    (serial: string) => {
      const draft = drafts[serial];
      if (!draft) {
        return;
      }
      setSavingSerials((prev) => new Set(prev).add(serial));
      setSavedSerial(null);
      setSaveErrorBySerial((prev) => {
        if (!(serial in prev)) {
          return prev;
        }
        const next = { ...prev };
        delete next[serial];
        return next;
      });

      saveDeviceProfile(serial, draft)
        .then(() => {
          setDevicesList((prev) =>
            prev.map((device) =>
              device.serial === serial
                ? {
                    ...device,
                    destinationPath: draft.destinationPath.trim() || null,
                    displayName: draft.displayName.trim(),
                  }
                : device
            )
          );
          setSavedSerial(serial);
        })
        .catch((err: unknown) => {
          setSaveErrorBySerial((prev) => ({
            ...prev,
            [serial]: err instanceof Error ? err.message : String(err),
          }));
        })
        .finally(() => {
          setSavingSerials((prev) => {
            if (!prev.has(serial)) {
              return prev;
            }
            const next = new Set(prev);
            next.delete(serial);
            return next;
          });
        });
    },
    [drafts]
  );

  return (
    <VStack gap={4}>
      <HStack gap={3} justify="between" vAlign="center">
        <Heading level={1}>Profile settings</Heading>
        <Button
          isLoading={isLoading}
          label="Refresh"
          onClick={load}
          variant="secondary"
        />
      </HStack>
      <Text color="secondary">
        Devices seen by a previous backup or restore run. Edit a device's
        display name or backup destination and save.
      </Text>
      <Text color="secondary" type="supporting">
        Per-folder classification isn't editable here yet -- no run has ever
        saved a folder selection to the database (see History), so there's
        nothing to edit yet.
      </Text>

      {error ? (
        <Banner
          description={error}
          endContent={<Button label="Retry" onClick={load} />}
          status="error"
          title="Failed to load devices"
        />
      ) : null}

      {isLoading && devicesList.length === 0 ? (
        <Text color="secondary">Loading…</Text>
      ) : null}

      {!(isLoading || error) && devicesList.length === 0 ? (
        <EmptyState
          description="A device profile is created automatically the first time you run a backup or restore from the Run screen."
          title="No saved devices yet"
        />
      ) : null}

      {devicesList.length > 0 ? (
        <VStack gap={3}>
          {devicesList.map((device) => (
            <DeviceProfileCard
              device={device}
              draft={drafts[device.serial] ?? seedDraft(device)}
              isSaved={savedSerial === device.serial}
              isSaving={savingSerials.has(device.serial)}
              key={device.serial}
              onDraftChange={handleDraftChange}
              onSave={handleSave}
              saveError={saveErrorBySerial[device.serial]}
            />
          ))}
        </VStack>
      ) : null}
    </VStack>
  );
}

function DeviceProfileCard({
  device,
  draft,
  isSaving,
  isSaved,
  saveError,
  onDraftChange,
  onSave,
}: {
  device: Device;
  draft: DeviceDraft;
  isSaving: boolean;
  isSaved: boolean;
  saveError: string | undefined;
  onDraftChange: (
    serial: string,
    field: keyof DeviceDraft,
    value: string
  ) => void;
  onSave: (serial: string) => void;
}) {
  const { serial } = device;

  const handleDisplayNameChange = useCallback(
    (value: string) => onDraftChange(serial, "displayName", value),
    [onDraftChange, serial]
  );
  const handleDestinationPathChange = useCallback(
    (value: string) => onDraftChange(serial, "destinationPath", value),
    [onDraftChange, serial]
  );
  const handleSaveClick = useCallback(() => onSave(serial), [onSave, serial]);

  const isDirty =
    draft.displayName.trim() !== device.displayName ||
    draft.destinationPath.trim() !== (device.destinationPath ?? "");
  const isSaveDisabled = isSaving || draft.displayName.trim().length === 0;

  return (
    <Card padding={4}>
      <VStack gap={3}>
        <VStack gap={0.5}>
          <Text type="label">{device.serial}</Text>
          <Text color="secondary" type="supporting">
            First seen {formatDateTime(device.firstSeen)} — last seen{" "}
            {formatDateTime(device.lastSeen)}
          </Text>
        </VStack>

        <TextInput
          label="Display name"
          onChange={handleDisplayNameChange}
          value={draft.displayName}
        />
        <TextInput
          description="Local folder this device backs up to. Leave blank to choose one on each run."
          isOptional
          label="Backup destination"
          onChange={handleDestinationPathChange}
          placeholder="Not set"
          value={draft.destinationPath}
        />

        {saveError ? (
          <Banner description={saveError} status="error" title="Save failed" />
        ) : null}
        {isSaved && !isDirty ? <Banner status="success" title="Saved" /> : null}

        <HStack gap={2}>
          <Button
            isDisabled={isSaveDisabled}
            isLoading={isSaving}
            label="Save"
            onClick={handleSaveClick}
          />
        </HStack>
      </VStack>
    </Card>
  );
}
