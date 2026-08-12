import { Badge } from "@astryxdesign/core/Badge";
import { Banner } from "@astryxdesign/core/Banner";
import { Button } from "@astryxdesign/core/Button";
import { Heading } from "@astryxdesign/core/Heading";
import { List, ListItem } from "@astryxdesign/core/List";
import { ProgressBar } from "@astryxdesign/core/ProgressBar";
import {
  SegmentedControl,
  SegmentedControlItem,
} from "@astryxdesign/core/SegmentedControl";
import { StatusDot } from "@astryxdesign/core/StatusDot";
import { Text } from "@astryxdesign/core/Text";
import { TextInput } from "@astryxdesign/core/TextInput";
import { VStack } from "@astryxdesign/core/VStack";
import { invoke } from "@tauri-apps/api/core";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";
import { db } from "../db/client";
import { devices, runItems, runs } from "../db/schema";

/** Mirrors `space::SpaceCheckResult` (`src-tauri/src/space.rs`). */
interface SpaceCheckResult {
  estimated_bytes: number;
  free_bytes: number;
  has_enough_space: boolean;
  is_cloud_synced: boolean;
}

/** Mirrors `sync::orchestration::BatchOutcome` (`src-tauri/src/sync/orchestration.rs`). */
interface BatchOutcome {
  completed: string[];
  failed_at: [folder: string, message: string] | null;
}

/**
 * Mirrors `sync::progress_parser::ProgressEvent`
 * (`src-tauri/src/sync/progress_parser.rs`). Serde's default representation
 * for an internally-tagged enum (`#[serde(tag = "type")]`) is `{ type:
 * "<Variant>", ...fields }`.
 */
type ProgressEvent =
  | { type: "Copying"; path: string }
  | { type: "Fatal"; message: string }
  | { type: "Error"; message: string };

interface SyncFolderStartPayload {
  folder: string;
}
interface SyncProgressPayload {
  event: ProgressEvent;
  folder: string;
}
interface SyncFolderSuccessPayload {
  folder: string;
}
interface SyncFolderFailurePayload {
  error: string;
  folder: string;
}

type Direction = "backup" | "restore";
type FolderStatus = "pending" | "running" | "success" | "error";

interface RunScreenProps {
  dest?: string;
  direction?: Direction;
  /** Full ANDROID-side paths (e.g. `/storage/emulated/0/DCIM`), matching
   * exactly what `run_backup`/`run_restore`/`space_check` expect — the same
   * shape `sync::orchestration`'s `included_paths` param takes. */
  includedPaths?: string[];
  /**
   * All of these are optional and pre-fill the screen's own local input
   * fields rather than being required — no routing exists between screens
   * yet (consistent with Task 11/12), so `RunScreen` needs its own way to
   * get a device serial, destination path, and included ANDROID-side paths
   * for manual verification. A future task that wires real navigation can
   * pass these in from the Device/Classification screens' state instead of
   * (or in addition to) leaving the fields editable.
   */
  serial?: string;
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) {
    return `${bytes} B`;
  }
  const units = ["KB", "MB", "GB", "TB"];
  let value = bytes / 1024;
  let unitIndex = 0;
  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024;
    unitIndex += 1;
  }
  return `${value.toFixed(1)} ${units[unitIndex]}`;
}

const INCLUDED_PATHS_SEPARATOR = /[,\n]/;

function parseIncludedPathsText(text: string): string[] {
  return text
    .split(INCLUDED_PATHS_SEPARATOR)
    .map((p) => p.trim())
    .filter((p) => p.length > 0);
}

function statusVariant(
  status: FolderStatus
): "success" | "warning" | "error" | "accent" | "neutral" {
  switch (status) {
    case "success":
      return "success";
    case "error":
      return "error";
    case "running":
      return "accent";
    default:
      return "neutral";
  }
}

function badgeVariant(status: FolderStatus): "error" | "success" | "neutral" {
  if (status === "error") {
    return "error";
  }
  if (status === "success") {
    return "success";
  }
  return "neutral";
}

/** Description shown below a folder's row: the file currently in transfer
 * while running, or the error text once failed. `undefined` otherwise so
 * `ListItem` renders no description line. */
function folderDescription(
  status: FolderStatus,
  currentFile: string | undefined,
  errorMessage: string | undefined
): string | undefined {
  if (status === "running" && currentFile) {
    return `Copying: ${currentFile}`;
  }
  if (status === "error" && errorMessage) {
    return errorMessage;
  }
}

function statusLabel(status: FolderStatus): string {
  switch (status) {
    case "success":
      return "Synced";
    case "error":
      return "Failed";
    case "running":
      return "In progress";
    default:
      return "Pending";
  }
}

/**
 * Writes the `runs`/`run_items` rows for a finished batch (Task 10's
 * deliberate deferral — Rust only emits events, this is "the later task" its
 * own doc comment points to). Called once, from the resolved
 * `run_backup`/`run_restore` promise (see `handleStart` below for why that
 * trigger was chosen over the `sync-batch-complete` event).
 *
 * `runs.device_serial` is a foreign key into `devices`, but no earlier task
 * (Device screen included — see that screen's own comments) ever inserts a
 * `devices` row. Rather than let this insert fail against a schema
 * constraint, this upserts a minimal `devices` row first — real
 * `displayName` data isn't available here, so it falls back to the serial
 * itself; a later profile-settings task (Task 15) owns giving devices a real
 * friendly name.
 */
async function persistRunResult(params: {
  serial: string;
  direction: Direction;
  startedAt: Date;
  finishedAt: Date;
  outcome: BatchOutcome;
}): Promise<void> {
  const { serial, direction, startedAt, finishedAt, outcome } = params;
  const status = outcome.failed_at ? "failed" : "completed";

  // All three writes share one transaction so a failure partway through
  // (e.g. after the `runs` insert but before `run_items`) can never leave an
  // orphaned `runs` row with no `run_items` -- drizzle-orm's sqlite-proxy
  // driver issues real `begin`/`commit`/`rollback` statements over the same
  // proxied connection (see `SQLiteRemoteSession.transaction` in
  // `drizzle-orm/sqlite-proxy`), so this is a genuine atomic commit, not
  // just an API nicety.
  await db.transaction(async (tx) => {
    await tx
      .insert(devices)
      .values({
        displayName: serial,
        firstSeen: startedAt,
        lastSeen: finishedAt,
        serial,
      })
      .onConflictDoUpdate({
        set: { lastSeen: finishedAt },
        target: devices.serial,
      });

    const [insertedRun] = await tx
      .insert(runs)
      .values({
        deviceSerial: serial,
        finishedAt,
        startedAt,
        status,
        type: direction,
      })
      .returning({ id: runs.id });

    const itemRows: (typeof runItems.$inferInsert)[] = outcome.completed.map(
      (path) => ({
        finishedAt,
        path,
        runId: insertedRun.id,
        status: "synced",
      })
    );
    if (outcome.failed_at) {
      const [failedPath, message] = outcome.failed_at;
      itemRows.push({
        errorMessage: message,
        finishedAt,
        path: failedPath,
        runId: insertedRun.id,
        status: "error",
      });
    }

    if (itemRows.length > 0) {
      await tx.insert(runItems).values(itemRows);
    }
  });
}

/**
 * Registers the 4 per-run progress event listeners and returns however many
 * of them succeeded, plus any failure messages. Pulled out of `handleStart`
 * both to keep that callback's cognitive complexity within lint limits and
 * so the "some listeners failed" bookkeeping (§ below) lives in one place.
 *
 * Deliberately uses `Promise.allSettled` rather than `Promise.all`:
 * `Promise.all` fails fast on the first rejected `listen()` call and
 * discards the settled results of the others -- but those other `listen()`
 * calls still resolve independently in the background regardless, so their
 * unlisten functions would be silently lost (a real listener leak) while the
 * caller never even learns setup failed. `Promise.allSettled` never rejects,
 * so every outcome (registered listener OR failure) is inspectable, letting
 * the caller clean up whatever DID register and surface a real error instead
 * of leaving the screen stuck (e.g. `isRunning` never reset).
 */
async function registerProgressListeners(setters: {
  setFolderStatuses: (
    update: (prev: Record<string, FolderStatus>) => Record<string, FolderStatus>
  ) => void;
  setFolderCurrentFile: (
    update: (prev: Record<string, string>) => Record<string, string>
  ) => void;
  setFolderErrors: (
    update: (prev: Record<string, string>) => Record<string, string>
  ) => void;
}): Promise<{ unlistenFns: UnlistenFn[]; errors: string[] }> {
  const { setFolderStatuses, setFolderCurrentFile, setFolderErrors } = setters;

  const listenerResults = await Promise.allSettled([
    listen<SyncFolderStartPayload>("sync-folder-start", (event) => {
      setFolderStatuses((prev) => ({
        ...prev,
        [event.payload.folder]: "running",
      }));
    }),
    listen<SyncProgressPayload>("sync-progress", (event) => {
      const { event: progressEvent, folder } = event.payload;
      if (progressEvent.type === "Copying") {
        setFolderCurrentFile((prev) => ({
          ...prev,
          [folder]: progressEvent.path,
        }));
      }
    }),
    listen<SyncFolderSuccessPayload>("sync-folder-success", (event) => {
      setFolderStatuses((prev) => ({
        ...prev,
        [event.payload.folder]: "success",
      }));
    }),
    listen<SyncFolderFailurePayload>("sync-folder-failure", (event) => {
      setFolderStatuses((prev) => ({
        ...prev,
        [event.payload.folder]: "error",
      }));
      setFolderErrors((prev) => ({
        ...prev,
        [event.payload.folder]: event.payload.error,
      }));
    }),
  ]);

  const unlistenFns: UnlistenFn[] = [];
  const errors: string[] = [];
  for (const result of listenerResults) {
    if (result.status === "fulfilled") {
      unlistenFns.push(result.value);
    } else {
      errors.push(
        result.reason instanceof Error
          ? result.reason.message
          : String(result.reason)
      );
    }
  }
  return { errors, unlistenFns };
}

/**
 * Invokes `run_backup`/`run_restore`, records the resolved `BatchOutcome`,
 * and (best-effort) persists it. Pulled out of `handleStart` alongside
 * `registerProgressListeners` above to keep that callback's cognitive
 * complexity within lint limits -- the listener-setup-failure and
 * batch-run-failure paths are independent concerns, so splitting them into
 * separate functions mirrors that.
 */
async function runSyncBatchAndPersist(params: {
  command: "run_backup" | "run_restore";
  dest: string;
  includedPaths: string[];
  serial: string;
  direction: Direction;
  startedAt: Date;
  setBatchOutcome: (outcome: BatchOutcome) => void;
  setBatchError: (message: string) => void;
  setPersistError: (message: string) => void;
}): Promise<void> {
  const {
    command,
    dest,
    includedPaths,
    serial,
    direction,
    startedAt,
    setBatchOutcome,
    setBatchError,
    setPersistError,
  } = params;

  try {
    // `run_backup`/`run_restore` (Task 10) return a `Promise<BatchOutcome>`
    // that only resolves once the WHOLE batch is done -- live updates come
    // from the event listeners `registerProgressListeners` sets up, which
    // fire WHILE this await is still pending. The DB write below is
    // deliberately anchored to THIS resolved promise rather than the
    // `sync-batch-complete` event: both carry the identical `BatchOutcome`
    // payload (Rust emits the event immediately before returning
    // `Ok(outcome)` -- see `run_sync_batch` in `sync/orchestration.rs`), so
    // listening to both would just be two races to do the same write. The
    // promise resolution is the simpler, single-source trigger: it's
    // guaranteed to fire exactly once, and it keeps the "kick off the run"
    // and "record what happened" logic in the same function instead of
    // splitting it across an event-callback boundary.
    const outcome = await invoke<BatchOutcome>(command, {
      dest,
      includedPaths,
      serial,
    });
    setBatchOutcome(outcome);
    const finishedAt = new Date();
    try {
      await persistRunResult({
        direction,
        finishedAt,
        outcome,
        serial,
        startedAt,
      });
    } catch (err) {
      setPersistError(err instanceof Error ? err.message : String(err));
    }
  } catch (err) {
    setBatchError(err instanceof Error ? err.message : String(err));
  }
}

export function RunScreen({
  serial: serialProp = "",
  dest: destProp = "",
  includedPaths: includedPathsProp,
  direction: directionProp = "backup",
}: RunScreenProps = {}) {
  const [serial, setSerial] = useState(serialProp);
  const [dest, setDest] = useState(destProp);
  const [includedPathsText, setIncludedPathsText] = useState(
    (includedPathsProp ?? []).join("\n")
  );
  const [direction, setDirection] = useState<Direction>(directionProp);

  const includedPaths = parseIncludedPathsText(includedPathsText);

  // --- Preflight (space_check) ---
  const [spaceCheck, setSpaceCheck] = useState<SpaceCheckResult | null>(null);
  const [isCheckingSpace, setIsCheckingSpace] = useState(false);
  const [spaceCheckError, setSpaceCheckError] = useState<string | null>(null);
  const [isCloudWarningDismissed, setIsCloudWarningDismissed] = useState(false);

  // Same generation-counter guard Task 12's classification screen uses,
  // covering every trigger (not just an initial mount fetch) so a stale
  // in-flight `space_check` call from an earlier click can never clobber
  // state set by a newer one.
  const spaceCheckGenerationRef = useRef(0);

  const invalidatePreflight = useCallback(() => {
    spaceCheckGenerationRef.current += 1;
    setSpaceCheck(null);
    setSpaceCheckError(null);
  }, []);

  const runSpaceCheck = useCallback(() => {
    spaceCheckGenerationRef.current += 1;
    const generation = spaceCheckGenerationRef.current;
    setIsCheckingSpace(true);
    setSpaceCheckError(null);
    setIsCloudWarningDismissed(false);

    invoke<SpaceCheckResult>("space_check", { dest, includedPaths, serial })
      .then((result) => {
        if (spaceCheckGenerationRef.current !== generation) {
          return;
        }
        setSpaceCheck(result);
      })
      .catch((err: unknown) => {
        if (spaceCheckGenerationRef.current !== generation) {
          return;
        }
        setSpaceCheck(null);
        setSpaceCheckError(err instanceof Error ? err.message : String(err));
      })
      .finally(() => {
        if (spaceCheckGenerationRef.current !== generation) {
          return;
        }
        setIsCheckingSpace(false);
      });
  }, [serial, dest, includedPaths]);

  // --- Run (backup/restore) ---
  const [isRunning, setIsRunning] = useState(false);
  const [folderStatuses, setFolderStatuses] = useState<
    Record<string, FolderStatus>
  >({});
  const [folderCurrentFile, setFolderCurrentFile] = useState<
    Record<string, string>
  >({});
  const [folderErrors, setFolderErrors] = useState<Record<string, string>>({});
  const [batchOutcome, setBatchOutcome] = useState<BatchOutcome | null>(null);
  const [batchError, setBatchError] = useState<string | null>(null);
  const [persistError, setPersistError] = useState<string | null>(null);

  const unlistenFnsRef = useRef<UnlistenFn[]>([]);
  useEffect(
    () => () => {
      // Unlisten everything on unmount, in case a run is still in progress.
      for (const unlisten of unlistenFnsRef.current) {
        unlisten();
      }
    },
    []
  );

  const canStart =
    !(isRunning || isCheckingSpace) &&
    serial.trim() !== "" &&
    dest.trim() !== "" &&
    includedPaths.length > 0 &&
    spaceCheck?.has_enough_space === true;

  const handleStart = useCallback(async () => {
    if (!canStart) {
      return;
    }
    setIsRunning(true);
    setBatchOutcome(null);
    setBatchError(null);
    setPersistError(null);

    const initialStatuses: Record<string, FolderStatus> = {};
    for (const path of includedPaths) {
      initialStatuses[path] = "pending";
    }
    setFolderStatuses(initialStatuses);
    setFolderCurrentFile({});
    setFolderErrors({});

    const startedAt = new Date();

    const { unlistenFns, errors: listenerErrors } =
      await registerProgressListeners({
        setFolderCurrentFile,
        setFolderErrors,
        setFolderStatuses,
      });

    if (listenerErrors.length > 0) {
      // Clean up whatever DID register before bailing out so nothing leaks.
      for (const unlisten of unlistenFns) {
        unlisten();
      }
      setBatchError(
        `Failed to set up progress listeners: ${listenerErrors.join("; ")}`
      );
      setIsRunning(false);
      return;
    }

    unlistenFnsRef.current = unlistenFns;

    try {
      await runSyncBatchAndPersist({
        command: direction === "backup" ? "run_backup" : "run_restore",
        dest,
        direction,
        includedPaths,
        serial,
        setBatchError,
        setBatchOutcome,
        setPersistError,
        startedAt,
      });
    } finally {
      for (const unlisten of unlistenFns) {
        unlisten();
      }
      unlistenFnsRef.current = [];
      setIsRunning(false);
    }
  }, [canStart, includedPaths, serial, dest, direction]);

  const handleDirectionChange = useCallback(
    (value: string) => {
      setDirection(value as Direction);
      invalidatePreflight();
    },
    [invalidatePreflight]
  );
  const handleSerialChange = useCallback(
    (value: string) => {
      setSerial(value);
      invalidatePreflight();
    },
    [invalidatePreflight]
  );
  const handleDestChange = useCallback(
    (value: string) => {
      setDest(value);
      invalidatePreflight();
    },
    [invalidatePreflight]
  );
  const handleIncludedPathsTextChange = useCallback(
    (value: string) => {
      setIncludedPathsText(value);
      invalidatePreflight();
    },
    [invalidatePreflight]
  );
  const handleDismissCloudWarning = useCallback(() => {
    setIsCloudWarningDismissed(true);
  }, []);

  return (
    <VStack gap={4}>
      <Heading level={1}>Run backup or restore</Heading>
      <Text color="secondary">
        No device/classification routing is wired up yet, so fill these in
        manually to exercise a run.
      </Text>

      <VStack gap={3}>
        <SegmentedControl
          isDisabled={isRunning}
          label="Direction"
          onChange={handleDirectionChange}
          value={direction}
        >
          <SegmentedControlItem label="Backup" value="backup" />
          <SegmentedControlItem label="Restore" value="restore" />
        </SegmentedControl>
        <TextInput
          isDisabled={isRunning}
          label="Device serial"
          onChange={handleSerialChange}
          value={serial}
        />
        <TextInput
          description={
            direction === "backup"
              ? "Local folder to back up into"
              : "Local folder to restore from"
          }
          isDisabled={isRunning}
          label="Destination path"
          onChange={handleDestChange}
          value={dest}
        />
        <TextInput
          description="Full ANDROID-side paths, comma- or newline-separated (e.g. /storage/emulated/0/DCIM)"
          isDisabled={isRunning}
          label="Included paths"
          onChange={handleIncludedPathsTextChange}
          value={includedPathsText}
        />
      </VStack>

      <VStack gap={2}>
        <Button
          isDisabled={
            isRunning ||
            isCheckingSpace ||
            serial.trim() === "" ||
            dest.trim() === "" ||
            includedPaths.length === 0
          }
          isLoading={isCheckingSpace}
          label="Check space"
          onClick={runSpaceCheck}
          variant="secondary"
        />

        {spaceCheckError ? (
          <Banner
            description={spaceCheckError}
            status="error"
            title="Preflight check failed"
          />
        ) : null}

        {spaceCheck ? (
          <VStack gap={2}>
            <Text color="secondary">
              Estimated transfer: {formatBytes(spaceCheck.estimated_bytes)} —
              free space: {formatBytes(spaceCheck.free_bytes)}
            </Text>
            {spaceCheck.has_enough_space ? null : (
              <Banner
                description="Free up space on the destination, or choose a different destination, before starting."
                status="error"
                title="Not enough free space"
              />
            )}
            {spaceCheck.is_cloud_synced && !isCloudWarningDismissed ? (
              <Banner
                description="This destination is inside a cloud-sync folder (OneDrive/Dropbox/Google Drive/iCloud Drive). Large transfers here can trigger slow hydration or sync churn. You can continue, but a local-only folder is recommended."
                isDismissable
                onDismiss={handleDismissCloudWarning}
                status="warning"
                title="Destination is inside a cloud-synced folder"
              />
            ) : null}
          </VStack>
        ) : null}

        <Button
          isDisabled={!canStart}
          isLoading={isRunning}
          label={direction === "backup" ? "Start backup" : "Start restore"}
          onClick={handleStart}
          tooltip={
            spaceCheck === null ? "Run the space check first" : undefined
          }
          variant="primary"
        />
      </VStack>

      {batchError ? (
        <Banner
          description={batchError}
          status="error"
          title="Run failed to start"
        />
      ) : null}
      {persistError ? (
        <Banner
          description={persistError}
          status="warning"
          title="Run finished, but saving its history failed"
        />
      ) : null}
      {batchOutcome && !batchOutcome.failed_at ? (
        <Banner
          description="All included folders finished syncing."
          status="success"
          title="Run complete"
        />
      ) : null}

      {includedPaths.length > 0 ? (
        <List hasDividers header={<Text type="label">Folders</Text>}>
          {includedPaths.map((path) => {
            const status = folderStatuses[path] ?? "pending";
            return (
              <ListItem
                description={folderDescription(
                  status,
                  folderCurrentFile[path],
                  folderErrors[path]
                )}
                endContent={
                  status === "running" ? (
                    <ProgressBar
                      isIndeterminate
                      isLabelHidden
                      label={`${path} progress`}
                    />
                  ) : (
                    <Badge
                      label={statusLabel(status)}
                      variant={badgeVariant(status)}
                    />
                  )
                }
                key={path}
                label={path}
                startContent={
                  <StatusDot
                    label={statusLabel(status)}
                    variant={statusVariant(status)}
                  />
                }
              />
            );
          })}
        </List>
      ) : null}
    </VStack>
  );
}
