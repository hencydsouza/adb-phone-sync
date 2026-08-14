# Zustand State Management Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace app.tsx's prop-drilled cross-screen state and each screen's duplicated
fetch/race-guard boilerplate with 4 Zustand stores, and wire real `folder_rules`
persistence for the first time.

**Architecture:** One Zustand store per domain (`session`, `devices`, `classification`,
`run`) in `src/stores/`. Each domain store's `load*()` action runs the existing
Drizzle/Tauri query and sets state; write actions run the Drizzle write, then update
state to match. Drizzle/SQLite stays the real source of truth — stores are a reactive
cache, not `persist`-middleware-serialized blobs. See
`docs/plans/2026-08-14-zustand-state-management-design.md` for the approved design and
rationale.

**Tech Stack:** Zustand (new dependency), React 19, Drizzle ORM (`sqlite-proxy` over
`tauri-plugin-sql`), Tauri v2 `invoke`/`listen`.

**No automated frontend test runner exists in this repo** (no vitest/jest/testing-library
in `package.json` — only Rust has `cargo test`). Every task's verification step is
therefore: `bun run check` (Biome/Ultracite lint + type errors) plus a manual
`bun run tauri dev` click-through, matching how the rest of this frontend has been
verified throughout the project. Pure helper functions stay exported as standalone
functions so they remain independently testable if a test runner is ever added.

---

## Task 1: Add the zustand dependency

**Files:**
- Modify: `package.json`

**Step 1: Install**

Run: `cd "C:\Users\hency\OneDrive\Desktop\adb-phone-sync" && bun add zustand`

Expected: `package.json`'s `dependencies` gains `"zustand": "^5..."` (or whatever the
latest 5.x resolves to), `bun.lock` updates.

**Step 2: Verify it resolves**

Run: `bun run check`
Expected: passes with no new errors (nothing imports it yet).

**Step 3: Commit**

```bash
git add package.json bun.lock
git commit -m "chore: add zustand for frontend state management"
```

---

## Task 2: Shared error-description helper

Pulls `run-screen.tsx`'s `describeError`/`errorCause` out into a shared module, since
`devices-store.ts`, `classification-store.ts`, and `run-store.ts` (Tasks 5, 8, 10) all
need it, not just `run-screen.tsx`.

**Files:**
- Create: `src/stores/errors.ts`
- Modify: `src/screens/run-screen.tsx:84-114` (delete the two functions, import instead)

**Step 1: Create the shared module**

```typescript
// src/stores/errors.ts
const MAX_ERROR_CAUSE_DEPTH = 5;

/** `Error.cause` is ES2022+; this project's `tsconfig.json` targets ES2020, so read it
 * via an index-signature cast rather than bumping the global lib target for one call
 * site. */
function errorCause(err: Error): unknown {
  return (err as unknown as { cause?: unknown }).cause;
}

/**
 * `err.message` alone drops the real root cause for a `DrizzleQueryError` (e.g. from a
 * store's `db.transaction()` call): drizzle-orm sets `this.cause = <the original
 * proxy-callback error>` on top of its own generic `"Failed query: ..."` message (see
 * `node_modules/drizzle-orm/errors.js`). Walk the `.cause` chain so the underlying error
 * -- the one that actually explains *why* a query failed -- is never silently dropped
 * from what the user sees.
 */
export function describeError(err: unknown): string {
  const parts: string[] = [];
  let current: unknown = err;
  for (let depth = 0; depth < MAX_ERROR_CAUSE_DEPTH && current; depth += 1) {
    const message =
      current instanceof Error ? current.message : String(current);
    if (!parts.includes(message)) {
      parts.push(message);
    }
    current = current instanceof Error ? errorCause(current) : undefined;
  }
  return parts.join(" — caused by: ");
}
```

**Step 2: Remove the duplicate from run-screen.tsx and import instead**

In `src/screens/run-screen.tsx`, delete lines 84-114 (the `MAX_ERROR_CAUSE_DEPTH`
constant, `errorCause`, and `describeError`), and add to the top import block:

```typescript
import { describeError } from "../stores/errors";
```

**Step 3: Verify**

Run: `bun run check`
Expected: passes — `run-screen.tsx` still compiles, now importing `describeError`
instead of defining it.

**Step 4: Commit**

```bash
git add src/stores/errors.ts src/screens/run-screen.tsx
git commit -m "refactor: extract describeError into a shared module"
```

---

## Task 3: Session store

**Files:**
- Create: `src/stores/session-store.ts`

**Step 1: Write the store**

```typescript
// src/stores/session-store.ts
import { create } from "zustand";

export type Screen = "classify" | "device" | "history" | "profile" | "run";

interface SessionState {
  activeScreen: Screen;
  selectedSerial: string | null;
  includedPaths: string[];
  goTo: (screen: Screen) => void;
  /** Records the picked device and advances to Classify, replacing today's
   * `App`'s `handleDeviceSelected`. */
  selectDevice: (serial: string) => void;
  /** Records the saved folder selection and advances to Run, replacing today's
   * `App`'s `handleClassificationSaved`. */
  setIncludedPaths: (paths: string[]) => void;
}

export const useSessionStore = create<SessionState>((set) => ({
  activeScreen: "device",
  selectedSerial: null,
  includedPaths: [],
  goTo: (screen) => set({ activeScreen: screen }),
  selectDevice: (serial) =>
    set({ activeScreen: "classify", selectedSerial: serial }),
  setIncludedPaths: (paths) =>
    set({ activeScreen: "run", includedPaths: paths }),
}));
```

**Step 2: Verify**

Run: `bun run check`
Expected: passes (nothing imports it yet, so this just checks the file itself compiles).

**Step 3: Commit**

```bash
git add src/stores/session-store.ts
git commit -m "feat: add session store for cross-screen navigation state"
```

---

## Task 4: Migrate app.tsx to the session store

**Files:**
- Modify: `src/app.tsx` (full rewrite — it's short and every part changes)

**Step 1: Rewrite app.tsx**

```tsx
import { AppShell } from "@astryxdesign/core/AppShell";
import { Banner } from "@astryxdesign/core/Banner";
import { Button } from "@astryxdesign/core/Button";
import {
  SideNav,
  SideNavHeading,
  SideNavItem,
} from "@astryxdesign/core/SideNav";
import { VStack } from "@astryxdesign/core/VStack";
import type { ReactNode } from "react";
import { ClassificationScreen } from "./screens/classification-screen";
import { DeviceScreen } from "./screens/device-screen";
import { HistoryScreen } from "./screens/history-screen";
import { ProfileSettingsScreen } from "./screens/profile-settings-screen";
import { RunScreen } from "./screens/run-screen";
import { useSessionStore } from "./stores/session-store";

/**
 * Minimal navigation to make all 5 screens reachable and let the device
 * serial / classification selection actually flow between them, per the
 * manual QA checklist's P0 item
 * (`docs/plans/2026-08-12-android-backup-restore-sync-manual-qa.md`).
 * Deliberately not a polished router -- just enough for a real click-through.
 */
function NeedsDeviceNotice() {
  const goTo = useSessionStore((state) => state.goTo);
  return (
    <VStack gap={4}>
      <Banner
        description="Pick a device on the Device screen first."
        endContent={
          <Button label="Go to Devices" onClick={() => goTo("device")} />
        }
        status="info"
        title="No device selected"
      />
    </VStack>
  );
}

function App() {
  const activeScreen = useSessionStore((state) => state.activeScreen);
  const selectedSerial = useSessionStore((state) => state.selectedSerial);
  const includedPaths = useSessionStore((state) => state.includedPaths);
  const goTo = useSessionStore((state) => state.goTo);

  let content: ReactNode;
  if (activeScreen === "device") {
    content = <DeviceScreen />;
  } else if (activeScreen === "classify") {
    content = selectedSerial ? (
      <ClassificationScreen serial={selectedSerial} />
    ) : (
      <NeedsDeviceNotice />
    );
  } else if (activeScreen === "run") {
    content = selectedSerial ? (
      <RunScreen includedPaths={includedPaths} serial={selectedSerial} />
    ) : (
      <NeedsDeviceNotice />
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
            onClick={() => goTo("device")}
          />
          <SideNavItem
            isSelected={activeScreen === "classify"}
            label="Classify"
            onClick={() => goTo("classify")}
          />
          <SideNavItem
            isSelected={activeScreen === "run"}
            label="Run"
            onClick={() => goTo("run")}
          />
          <SideNavItem
            isSelected={activeScreen === "history"}
            label="History"
            onClick={() => goTo("history")}
          />
          <SideNavItem
            isSelected={activeScreen === "profile"}
            label="Profile Settings"
            onClick={() => goTo("profile")}
          />
        </SideNav>
      }
    >
      {content}
    </AppShell>
  );
}

export default App;
```

Note this removes the `onDeviceSelected`/`onSaved` props from `<DeviceScreen>` and
`<ClassificationScreen>` — Tasks 6 and 9 update those screens to call
`useSessionStore`'s actions directly instead of receiving them as props. Until those
tasks land, `DeviceScreen`/`ClassificationScreen` still declare those props (now unused
by `App`) — that's fine, TypeScript won't error on an unused optional prop not being
passed.

**Step 2: Verify**

Run: `bun run check`
Expected: passes.

Run: `bun run tauri dev`, click through Device → (device won't navigate yet, that's
Task 6) → manually click "Classify"/"Run"/"History"/"Profile Settings" in the side nav.
Expected: navigation between all 5 screens still works exactly as before.

**Step 3: Commit**

```bash
git add src/app.tsx
git commit -m "refactor: migrate app.tsx to useSessionStore"
```

---

## Task 5: Devices store

Holds both the live `adb`-connected device list and the persisted `devices` table
profiles — one domain from the UI's perspective (see design doc).

**Files:**
- Create: `src/stores/devices-store.ts`

**Step 1: Write the store**

```typescript
// src/stores/devices-store.ts
import { invoke } from "@tauri-apps/api/core";
import { eq } from "drizzle-orm";
import { create } from "zustand";
import { db } from "../db/client";
import { devices } from "../db/schema";
import { describeError } from "./errors";

/** Mirrors the `Device` struct serialized by `src-tauri/src/devices.rs`. */
export interface ConnectedDevice {
  serial: string;
  state: string;
}

type SavedProfile = typeof devices.$inferSelect;

/** Per-device editable draft, seeded from the loaded row and reseeded whenever
 * `loadProfiles` replaces `savedProfiles`. Kept as its own field (rather than editing
 * `savedProfiles` in place) so a save failure doesn't leave the list showing an
 * unsaved value as if it were persisted. */
export interface DeviceDraft {
  destinationPath: string;
  displayName: string;
}

function seedDraft(device: SavedProfile): DeviceDraft {
  return {
    destinationPath: device.destinationPath ?? "",
    displayName: device.displayName,
  };
}

function withoutKey<T>(record: Record<string, T>, key: string): Record<string, T> {
  if (!(key in record)) {
    return record;
  }
  const next = { ...record };
  delete next[key];
  return next;
}

interface DevicesState {
  connectedDevices: ConnectedDevice[];
  isLoadingConnected: boolean;
  connectedError: string | null;
  loadConnectedDevices: () => Promise<void>;

  savedProfiles: SavedProfile[];
  drafts: Record<string, DeviceDraft>;
  isLoadingProfiles: boolean;
  profilesError: string | null;
  savingSerials: ReadonlySet<string>;
  saveErrorBySerial: Record<string, string>;
  savedSerial: string | null;
  loadProfiles: () => Promise<void>;
  updateDraft: (
    serial: string,
    field: keyof DeviceDraft,
    value: string
  ) => void;
  saveProfile: (serial: string) => Promise<void>;
}

// Module-level generation counters (one per independently-triggerable fetch),
// same purpose as the `fetchGenerationRef`s each screen used to keep separately:
// guard against a stale in-flight request clobbering state set by a newer one.
let connectedGeneration = 0;
let profilesGeneration = 0;

export const useDevicesStore = create<DevicesState>((set, get) => ({
  connectedDevices: [],
  isLoadingConnected: true,
  connectedError: null,
  loadConnectedDevices: async () => {
    connectedGeneration += 1;
    const generation = connectedGeneration;
    set({ connectedError: null, isLoadingConnected: true });
    try {
      const result = await invoke<ConnectedDevice[]>("list_devices");
      if (connectedGeneration !== generation) {
        return;
      }
      set({ connectedDevices: result, isLoadingConnected: false });
    } catch (err) {
      if (connectedGeneration !== generation) {
        return;
      }
      set({ connectedError: describeError(err), isLoadingConnected: false });
    }
  },

  savedProfiles: [],
  drafts: {},
  isLoadingProfiles: true,
  profilesError: null,
  savingSerials: new Set(),
  saveErrorBySerial: {},
  savedSerial: null,

  loadProfiles: async () => {
    profilesGeneration += 1;
    const generation = profilesGeneration;
    set({ isLoadingProfiles: true, profilesError: null });
    try {
      const result = await db
        .select()
        .from(devices)
        .orderBy(devices.displayName);
      if (profilesGeneration !== generation) {
        return;
      }
      set({
        drafts: Object.fromEntries(
          result.map((device) => [device.serial, seedDraft(device)])
        ),
        isLoadingProfiles: false,
        savedProfiles: result,
      });
    } catch (err) {
      if (profilesGeneration !== generation) {
        return;
      }
      set({ isLoadingProfiles: false, profilesError: describeError(err) });
    }
  },

  updateDraft: (serial, field, value) => {
    set((state) => ({
      drafts: {
        ...state.drafts,
        [serial]: { ...state.drafts[serial], [field]: value },
      },
      // Editing after a save invalidates that row's "Saved" confirmation and
      // any earlier error for it -- both would otherwise describe a value
      // that's no longer what's on screen.
      savedSerial: state.savedSerial === serial ? null : state.savedSerial,
      saveErrorBySerial: withoutKey(state.saveErrorBySerial, serial),
    }));
  },

  saveProfile: async (serial) => {
    const draft = get().drafts[serial];
    if (!draft) {
      return;
    }
    set((state) => ({
      savedSerial: null,
      saveErrorBySerial: withoutKey(state.saveErrorBySerial, serial),
      savingSerials: new Set(state.savingSerials).add(serial),
    }));

    try {
      await db
        .update(devices)
        .set({
          destinationPath: draft.destinationPath.trim() || null,
          displayName: draft.displayName.trim(),
        })
        .where(eq(devices.serial, serial));

      set((state) => ({
        savedProfiles: state.savedProfiles.map((device) =>
          device.serial === serial
            ? {
                ...device,
                destinationPath: draft.destinationPath.trim() || null,
                displayName: draft.displayName.trim(),
              }
            : device
        ),
        savedSerial: serial,
      }));
    } catch (err) {
      set((state) => ({
        saveErrorBySerial: {
          ...state.saveErrorBySerial,
          [serial]: describeError(err),
        },
      }));
    } finally {
      set((state) => {
        if (!state.savingSerials.has(serial)) {
          return state;
        }
        const next = new Set(state.savingSerials);
        next.delete(serial);
        return { savingSerials: next };
      });
    }
  },
}));
```

**Step 2: Verify**

Run: `bun run check`
Expected: passes.

**Step 3: Commit**

```bash
git add src/stores/devices-store.ts
git commit -m "feat: add devices store for connected devices + saved profiles"
```

---

## Task 6: Migrate device-screen.tsx to the devices + session stores

**Files:**
- Modify: `src/screens/device-screen.tsx` (full rewrite — every piece of state moves)

**Step 1: Rewrite**

```tsx
import { Badge } from "@astryxdesign/core/Badge";
import { Banner } from "@astryxdesign/core/Banner";
import { Button } from "@astryxdesign/core/Button";
import { EmptyState } from "@astryxdesign/core/EmptyState";
import { Heading } from "@astryxdesign/core/Heading";
import { List, ListItem } from "@astryxdesign/core/List";
import { StatusDot } from "@astryxdesign/core/StatusDot";
import { Text } from "@astryxdesign/core/Text";
import { VStack } from "@astryxdesign/core/VStack";
import { useCallback, useEffect } from "react";
import { useDevicesStore } from "../stores/devices-store";
import { useSessionStore } from "../stores/session-store";

/** adb reports "device" for a fully authorized, ready-to-use connection. */
const READY_STATE = "device";

function statusVariantForState(state: string): "success" | "warning" {
  return state === READY_STATE ? "success" : "warning";
}

export function DeviceScreen() {
  const devices = useDevicesStore((state) => state.connectedDevices);
  const isLoading = useDevicesStore((state) => state.isLoadingConnected);
  const error = useDevicesStore((state) => state.connectedError);
  const loadDevices = useDevicesStore((state) => state.loadConnectedDevices);
  const selectedSerial = useSessionStore((state) => state.selectedSerial);
  const selectDevice = useSessionStore((state) => state.selectDevice);

  useEffect(() => {
    loadDevices();
  }, [loadDevices]);

  const handleSelect = useCallback(
    (serial: string) => {
      selectDevice(serial);
    },
    [selectDevice]
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
  device: { serial: string; state: string };
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
```

This drops the `parse_adb_devices_output`-adjacent `Device` interface (now
`ConnectedDevice`, defined in the store) and the `onDeviceSelected` prop — selection now
goes straight through `useSessionStore.selectDevice`, which also handles the
device→classify navigation `handleDeviceSelected` used to do in `app.tsx`. It also drops
the screen's own `selectedSerial` local state in favor of reading it from
`useSessionStore`, so the highlighted row stays correct even if this screen remounts.

**Step 2: Verify**

Run: `bun run check`
Expected: passes.

Run: `bun run tauri dev`, open Device screen, confirm the connected device list still
loads, click a device, confirm it navigates to Classify (same as before this refactor).

**Step 3: Commit**

```bash
git add src/screens/device-screen.tsx
git commit -m "refactor: migrate device-screen.tsx to devices + session stores"
```

---

## Task 7: Migrate profile-settings-screen.tsx to the devices store

**Files:**
- Modify: `src/screens/profile-settings-screen.tsx` (full rewrite)

**Step 1: Rewrite**

```tsx
import { Banner } from "@astryxdesign/core/Banner";
import { Button } from "@astryxdesign/core/Button";
import { Card } from "@astryxdesign/core/Card";
import { EmptyState } from "@astryxdesign/core/EmptyState";
import { Heading } from "@astryxdesign/core/Heading";
import { HStack } from "@astryxdesign/core/HStack";
import { Text } from "@astryxdesign/core/Text";
import { TextInput } from "@astryxdesign/core/TextInput";
import { VStack } from "@astryxdesign/core/VStack";
import { useCallback, useEffect } from "react";
import type { DeviceDraft } from "../stores/devices-store";
import { useDevicesStore } from "../stores/devices-store";

function formatDateTime(date: Date): string {
  return date.toLocaleString();
}

export function ProfileSettingsScreen() {
  const devicesList = useDevicesStore((state) => state.savedProfiles);
  const drafts = useDevicesStore((state) => state.drafts);
  const isLoading = useDevicesStore((state) => state.isLoadingProfiles);
  const error = useDevicesStore((state) => state.profilesError);
  const savingSerials = useDevicesStore((state) => state.savingSerials);
  const saveErrorBySerial = useDevicesStore((state) => state.saveErrorBySerial);
  const savedSerial = useDevicesStore((state) => state.savedSerial);
  const load = useDevicesStore((state) => state.loadProfiles);
  const updateDraft = useDevicesStore((state) => state.updateDraft);
  const saveProfile = useDevicesStore((state) => state.saveProfile);

  useEffect(() => {
    load();
  }, [load]);

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
        Per-folder classification isn't editable here yet -- see History for
        saved folder selections.
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
              draft={
                drafts[device.serial] ?? {
                  destinationPath: device.destinationPath ?? "",
                  displayName: device.displayName,
                }
              }
              isSaved={savedSerial === device.serial}
              isSaving={savingSerials.has(device.serial)}
              key={device.serial}
              onDraftChange={updateDraft}
              onSave={saveProfile}
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
  device: { serial: string; displayName: string; destinationPath: string | null; firstSeen: Date; lastSeen: Date };
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
  const isSaveDisabled =
    isSaving || draft.displayName.trim().length === 0 || !isDirty;

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
```

**Step 2: Verify**

Run: `bun run check`
Expected: passes.

Run: `bun run tauri dev`, open Profile Settings, edit a device's display name, save,
confirm the "Saved" banner appears and the value persists after Refresh.

**Step 3: Commit**

```bash
git add src/screens/profile-settings-screen.tsx
git commit -m "refactor: migrate profile-settings-screen.tsx to devices store"
```

---

## Task 8: Classification store, including real folder_rules persistence

This is the task that closes the actual functional gap: `saveFolderRules` is the first
code in the project that ever writes to `folder_rules`.

**Files:**
- Create: `src/stores/classification-store.ts`

**Step 1: Write the store**

```typescript
// src/stores/classification-store.ts
import { invoke } from "@tauri-apps/api/core";
import { create } from "zustand";
import { db } from "../db/client";
import { folderRules } from "../db/schema";
import { describeError } from "./errors";

/**
 * Mirrors `classify::Decision` (`src-tauri/src/classify.rs`), as serialized
 * by `device_scan::classify_suggest`. Serde's default representation for a
 * unit-only enum is a bare string of the variant name.
 */
type Decision = "Include" | "Skip" | "SkipStaleDuplicate";

/** Mirrors `device_scan::SuggestedFolder`. */
export interface SuggestedFolder {
  decision: Decision;
  name: string;
}

/**
 * Mirrors `device_scan::STORAGE_ROOT` (`src-tauri/src/device_scan.rs`).
 * `classify_suggest` returns bare top-level folder names; both `RunScreen`'s
 * `includedPaths` and `folder_rules.path` need the full ANDROID-side path.
 */
const STORAGE_ROOT = "/storage/emulated/0";

function isIncludedByDefault(decision: Decision): boolean {
  return decision === "Include";
}

function toAndroidPath(folderName: string): string {
  return `${STORAGE_ROOT}/${folderName}`;
}

interface ClassificationState {
  suggestions: SuggestedFolder[];
  included: Record<string, boolean>;
  isLoading: boolean;
  error: string | null;
  isSaved: boolean;
  saveError: string | null;
  loadSuggestions: (serial: string) => Promise<void>;
  toggleFolder: (name: string, checked: boolean) => void;
  /** Upserts one `folder_rules` row per suggestion and returns the full
   * ANDROID-side paths of every currently-included folder, for the caller to
   * hand to `useSessionStore().setIncludedPaths`. Every row is written with
   * `source: "manual"`: this only ever runs from an explicit "Save
   * selections" click, i.e. a user-reviewed decision, even for folders whose
   * checkbox the user left at its heuristic default. */
  saveFolderRules: (serial: string) => Promise<string[]>;
}

let suggestionsGeneration = 0;

export const useClassificationStore = create<ClassificationState>(
  (set, get) => ({
    suggestions: [],
    included: {},
    isLoading: true,
    error: null,
    isSaved: false,
    saveError: null,

    loadSuggestions: async (serial) => {
      suggestionsGeneration += 1;
      const generation = suggestionsGeneration;
      set({ error: null, isLoading: true, isSaved: false });
      try {
        const result = await invoke<SuggestedFolder[]>("classify_suggest", {
          serial,
        });
        if (suggestionsGeneration !== generation) {
          return;
        }
        set({
          included: Object.fromEntries(
            result.map((folder) => [
              folder.name,
              isIncludedByDefault(folder.decision),
            ])
          ),
          isLoading: false,
          suggestions: result,
        });
      } catch (err) {
        if (suggestionsGeneration !== generation) {
          return;
        }
        set({ error: describeError(err), isLoading: false });
      }
    },

    toggleFolder: (name, checked) => {
      set((state) => ({
        included: { ...state.included, [name]: checked },
        // A post-save edit invalidates the "Selections saved" banner -- it
        // would otherwise keep claiming the (now-stale) selection was saved.
        isSaved: false,
      }));
    },

    saveFolderRules: async (serial) => {
      const { suggestions, included } = get();
      const now = new Date();

      try {
        await db.transaction(async (tx) => {
          for (const folder of suggestions) {
            const path = toAndroidPath(folder.name);
            const decision = included[folder.name] ? "include" : "skip";
            await tx
              .insert(folderRules)
              .values({
                decision,
                deviceSerial: serial,
                path,
                source: "manual",
                updatedAt: now,
              })
              .onConflictDoUpdate({
                set: { decision, source: "manual", updatedAt: now },
                target: [folderRules.deviceSerial, folderRules.path],
              });
          }
        });
      } catch (err) {
        set({ saveError: describeError(err) });
        throw err;
      }

      set({ isSaved: true, saveError: null });
      return suggestions
        .filter((folder) => included[folder.name])
        .map((folder) => toAndroidPath(folder.name));
    },
  })
);
```

**Step 2: Verify**

Run: `bun run check`
Expected: passes. Pay attention to the `onConflictDoUpdate({ target: [...] })` call —
this must type-check against `folder_rules_device_serial_path_unique` in
`src/db/schema.ts:38-41`. If drizzle-orm's sqlite `onConflictDoUpdate` type only accepts
a single column (not a tuple) for `target` in this version, adjust to whatever the
installed `drizzle-orm` version's types require (check
`node_modules/drizzle-orm/sqlite-core/query-builders/insert.d.ts`) — the important
behavior to preserve is: this must upsert (insert-or-update) keyed on `(deviceSerial,
path)`, matching the schema's unique index.

**Step 3: Commit**

```bash
git add src/stores/classification-store.ts
git commit -m "feat: add classification store with real folder_rules persistence"
```

---

## Task 9: Migrate classification-screen.tsx to the classification + session stores

**Files:**
- Modify: `src/screens/classification-screen.tsx` (full rewrite)

**Step 1: Rewrite**

```tsx
import { Badge } from "@astryxdesign/core/Badge";
import { Banner } from "@astryxdesign/core/Banner";
import { Button } from "@astryxdesign/core/Button";
import { CheckboxInput } from "@astryxdesign/core/CheckboxInput";
import { EmptyState } from "@astryxdesign/core/EmptyState";
import { Heading } from "@astryxdesign/core/Heading";
import { List, ListItem } from "@astryxdesign/core/List";
import { Text } from "@astryxdesign/core/Text";
import { VStack } from "@astryxdesign/core/VStack";
import { useCallback, useEffect, useRef } from "react";
import { useClassificationStore } from "../stores/classification-store";
import { useSessionStore } from "../stores/session-store";

interface ClassificationScreenProps {
  /**
   * The device serial to scan, e.g. lifted from `DeviceScreen`'s
   * `onDeviceSelected` seam (see `src/app.tsx`).
   */
  serial: string;
}

function decisionSummary(
  decision: "Include" | "Skip" | "SkipStaleDuplicate"
): string {
  switch (decision) {
    case "Include":
      return "Suggested: include";
    case "Skip":
      return "Suggested: skip";
    case "SkipStaleDuplicate":
      return "Suggested: skip — superseded by a newer copy under Android/media";
    default:
      return decision;
  }
}

export function ClassificationScreen({ serial }: ClassificationScreenProps) {
  const suggestions = useClassificationStore((state) => state.suggestions);
  const included = useClassificationStore((state) => state.included);
  const isLoading = useClassificationStore((state) => state.isLoading);
  const error = useClassificationStore((state) => state.error);
  const isSaved = useClassificationStore((state) => state.isSaved);
  const saveError = useClassificationStore((state) => state.saveError);
  const loadSuggestions = useClassificationStore(
    (state) => state.loadSuggestions
  );
  const toggleFolder = useClassificationStore((state) => state.toggleFolder);
  const saveFolderRules = useClassificationStore(
    (state) => state.saveFolderRules
  );
  const setIncludedPaths = useSessionStore((state) => state.setIncludedPaths);

  useEffect(() => {
    loadSuggestions(serial);
  }, [serial, loadSuggestions]);

  const handleSave = useCallback(async () => {
    const includedPaths = await saveFolderRules(serial).catch(() => null);
    if (includedPaths) {
      setIncludedPaths(includedPaths);
    }
  }, [serial, saveFolderRules, setIncludedPaths]);

  const handleRetry = useCallback(() => {
    loadSuggestions(serial);
  }, [serial, loadSuggestions]);

  if (error) {
    return (
      <VStack gap={4}>
        <Heading level={1}>Review folders</Heading>
        <Banner
          description={error}
          endContent={<Button label="Retry" onClick={handleRetry} />}
          status="error"
          title="Failed to scan the device"
        />
      </VStack>
    );
  }

  if (isLoading) {
    return (
      <VStack gap={4}>
        <Heading level={1}>Review folders</Heading>
        <Text color="secondary">Scanning device storage…</Text>
      </VStack>
    );
  }

  if (suggestions.length === 0) {
    return (
      <EmptyState
        actions={<Button label="Retry" onClick={handleRetry} />}
        description="No top-level folders were found under the device's storage root."
        title="Nothing to classify"
      />
    );
  }

  const includedCount = Object.values(included).filter(Boolean).length;

  return (
    <VStack gap={4}>
      <Heading level={1}>Review folders</Heading>
      <Text color="secondary">
        {includedCount} of {suggestions.length} folders selected for backup.
        Suggestions are pre-checked based on the folder's contents — review and
        adjust before saving.
      </Text>
      {isSaved ? (
        <Banner
          status="success"
          title="Selections saved"
        />
      ) : null}
      {saveError ? (
        <Banner description={saveError} status="error" title="Save failed" />
      ) : null}
      <List hasDividers header={<Text type="label">Detected folders</Text>}>
        {suggestions.map((folder) => (
          <FolderListItem
            folder={folder}
            isIncluded={included[folder.name] ?? false}
            key={folder.name}
            onToggle={toggleFolder}
          />
        ))}
      </List>
      <Button label="Save selections" onClick={handleSave} />
    </VStack>
  );
}

function FolderListItem({
  folder,
  isIncluded,
  onToggle,
}: {
  folder: { name: string; decision: "Include" | "Skip" | "SkipStaleDuplicate" };
  isIncluded: boolean;
  onToggle: (name: string, checked: boolean) => void;
}) {
  const checkboxRef = useRef<HTMLInputElement>(null);

  const handleChange = useCallback(
    (checked: boolean) => {
      onToggle(folder.name, checked);
    },
    [folder.name, onToggle]
  );

  return (
    <ListItem
      description={decisionSummary(folder.decision)}
      endContent={
        folder.decision === "SkipStaleDuplicate" ? (
          <Badge label="Stale duplicate" variant="warning" />
        ) : undefined
      }
      interactiveRef={checkboxRef}
      label={folder.name}
      startContent={
        <CheckboxInput
          isLabelHidden
          label={`Include ${folder.name}`}
          onChange={handleChange}
          ref={checkboxRef}
          value={isIncluded}
        />
      }
    />
  );
}
```

This removes the `onSaved` prop (selection now flows through
`useSessionStore().setIncludedPaths`, called after `saveFolderRules` actually persists —
previously `onSaved` fired regardless of persistence since there was none) and the
now-stale "This is a local review only for now" banner copy, since selections really are
persisted now.

**Step 2: Verify**

Run: `bun run check`
Expected: passes.

Run: `bun run tauri dev`, pick a device, review folders, toggle a couple checkboxes,
click "Save selections". Confirm it navigates to Run with the right included paths, then
check History — the "Folder sync status" section should now show the saved rows instead
of the old "No saved folder selections yet" empty state.

**Step 3: Commit**

```bash
git add src/screens/classification-screen.tsx
git commit -m "refactor: migrate classification-screen.tsx, wire real folder_rules save"
```

---

## Task 10: Run store — preflight and active-run state

Split from history (Task 12's slice) into its own task since this half is large on its
own: preflight (`space_check`) and the live backup/restore run with its 4 event
listeners.

**Files:**
- Create: `src/stores/run-store.ts`

**Step 1: Write the preflight + active-run portion of the store**

```typescript
// src/stores/run-store.ts
import { invoke } from "@tauri-apps/api/core";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { listen } from "@tauri-apps/api/event";
import { and, desc, eq, inArray } from "drizzle-orm";
import { create } from "zustand";
import { db } from "../db/client";
import { devices, folderRules, runItems, runs } from "../db/schema";
import { describeError } from "./errors";

/** Mirrors `space::SpaceCheckResult` (`src-tauri/src/space.rs`). */
export interface SpaceCheckResult {
  estimated_bytes: number;
  free_bytes: number;
  has_enough_space: boolean;
  is_cloud_synced: boolean;
}

/** Mirrors `sync::orchestration::BatchOutcome` (`src-tauri/src/sync/orchestration.rs`). */
export interface BatchOutcome {
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

export type Direction = "backup" | "restore";
export type FolderStatus = "pending" | "running" | "success" | "error";

type Run = typeof runs.$inferSelect;
type RunItem = typeof runItems.$inferSelect;
type FolderRule = typeof folderRules.$inferSelect;

/** A folder_rules row paired with the most recent successful sync for its
 * (device, path), derived client-side from `run_items` rather than as a
 * second SQL aggregate query. */
export interface FolderSyncStatus {
  lastSyncedAt: Date | null;
  rule: FolderRule;
}

/**
 * Derives the two display-only facts that should never need their own Rust
 * command: "last synced" (`MAX(finished_at)` per path) and "not-synced" (an
 * `include`d `folder_rules` path with no matching `run_items` row). Kept as a
 * standalone exported function (not inlined into `loadHistory` below) so it
 * stays independently testable if a test runner is ever added to this repo.
 *
 * "Last synced" only counts `status = "synced"` items: an `error` row's
 * `finished_at` marks when the attempt failed, not when the path was last
 * successfully synced.
 */
export function deriveFolderSyncStatus(
  rules: FolderRule[],
  items: RunItem[],
  runById: Map<number, Run>
): FolderSyncStatus[] {
  const lastSyncedByKey = new Map<string, Date>();
  for (const item of items) {
    if (item.status !== "synced" || !item.finishedAt) {
      continue;
    }
    const run = runById.get(item.runId);
    if (!run) {
      continue;
    }
    const key = `${run.deviceSerial}::${item.path}`;
    const existing = lastSyncedByKey.get(key);
    if (!existing || item.finishedAt > existing) {
      lastSyncedByKey.set(key, item.finishedAt);
    }
  }

  return rules.map((rule) => ({
    lastSyncedAt:
      lastSyncedByKey.get(`${rule.deviceSerial}::${rule.path}`) ?? null,
    rule,
  }));
}

/**
 * Writes the `runs`/`run_items` rows for a finished batch. `runs.device_serial`
 * is a foreign key into `devices`, so this upserts a minimal `devices` row
 * first in case this is the first time this serial has ever run a sync.
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
  // can never leave an orphaned `runs` row with no `run_items`.
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
 * Registers the 4 per-run progress event listeners. Uses `Promise.allSettled`
 * rather than `Promise.all`: `Promise.all` fails fast on the first rejected
 * `listen()` call and discards the settled results of the others, but those
 * other `listen()` calls still resolve independently in the background
 * regardless, so their unlisten functions would be silently lost.
 */
async function registerProgressListeners(handlers: {
  onStart: (folder: string) => void;
  onCopying: (folder: string, path: string) => void;
  onSuccess: (folder: string) => void;
  onFailure: (folder: string, error: string) => void;
}): Promise<{ unlistenFns: UnlistenFn[]; errors: string[] }> {
  const listenerResults = await Promise.allSettled([
    listen<SyncFolderStartPayload>("sync-folder-start", (event) => {
      handlers.onStart(event.payload.folder);
    }),
    listen<SyncProgressPayload>("sync-progress", (event) => {
      const { event: progressEvent, folder } = event.payload;
      if (progressEvent.type === "Copying") {
        handlers.onCopying(folder, progressEvent.path);
      }
    }),
    listen<SyncFolderSuccessPayload>("sync-folder-success", (event) => {
      handlers.onSuccess(event.payload.folder);
    }),
    listen<SyncFolderFailurePayload>("sync-folder-failure", (event) => {
      handlers.onFailure(event.payload.folder, event.payload.error);
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

interface RunState {
  // --- Preflight ---
  spaceCheck: SpaceCheckResult | null;
  isCheckingSpace: boolean;
  spaceCheckError: string | null;
  isCloudWarningDismissed: boolean;
  invalidatePreflight: () => void;
  dismissCloudWarning: () => void;
  runSpaceCheck: (params: {
    serial: string;
    dest: string;
    includedPaths: string[];
  }) => Promise<void>;

  // --- Active run ---
  isRunning: boolean;
  folderStatuses: Record<string, FolderStatus>;
  folderCurrentFile: Record<string, string>;
  folderErrors: Record<string, string>;
  batchOutcome: BatchOutcome | null;
  batchError: string | null;
  persistError: string | null;
  startRun: (params: {
    command: "run_backup" | "run_restore";
    serial: string;
    dest: string;
    includedPaths: string[];
    direction: Direction;
  }) => Promise<void>;

  // --- History (Task 12 fills these in; declared here so the interface is
  // whole from the start) ---
  runsList: Run[];
  itemsByRun: Map<number, RunItem[]>;
  folderSyncRows: FolderSyncStatus[];
  isLoadingHistory: boolean;
  historyError: string | null;
  loadHistory: (serial?: string) => Promise<void>;
}

let spaceCheckGeneration = 0;
let historyGeneration = 0;

export const useRunStore = create<RunState>((set) => ({
  spaceCheck: null,
  isCheckingSpace: false,
  spaceCheckError: null,
  isCloudWarningDismissed: false,

  invalidatePreflight: () => {
    spaceCheckGeneration += 1;
    set({ spaceCheck: null, spaceCheckError: null });
  },
  dismissCloudWarning: () => set({ isCloudWarningDismissed: true }),

  runSpaceCheck: async ({ serial, dest, includedPaths }) => {
    spaceCheckGeneration += 1;
    const generation = spaceCheckGeneration;
    set({
      isCheckingSpace: true,
      isCloudWarningDismissed: false,
      spaceCheckError: null,
    });
    try {
      const result = await invoke<SpaceCheckResult>("space_check", {
        dest,
        includedPaths,
        serial,
      });
      if (spaceCheckGeneration !== generation) {
        return;
      }
      set({ isCheckingSpace: false, spaceCheck: result });
    } catch (err) {
      if (spaceCheckGeneration !== generation) {
        return;
      }
      set({
        isCheckingSpace: false,
        spaceCheck: null,
        spaceCheckError: describeError(err),
      });
    }
  },

  isRunning: false,
  folderStatuses: {},
  folderCurrentFile: {},
  folderErrors: {},
  batchOutcome: null,
  batchError: null,
  persistError: null,

  // NOTE (behavior change from the pre-store version): listeners used to be
  // torn down on RunScreen unmount via a ref+cleanup effect, since their
  // setters wrote into that component's own useState. Now that this state
  // lives in a store, listeners are registered/unlistened entirely within
  // this action's own lifetime instead of tied to any component's mount
  // state -- so navigating to another screen mid-run no longer stops
  // progress from being recorded, and returning to Run shows accurate,
  // up-to-date state instead of a reset screen. The backend sync itself was
  // never affected by frontend unmounting either way (it's a Tauri command
  // future that runs independently of the event channel).
  startRun: async ({ command, serial, dest, includedPaths, direction }) => {
    set({
      batchError: null,
      batchOutcome: null,
      folderCurrentFile: {},
      folderErrors: {},
      folderStatuses: Object.fromEntries(
        includedPaths.map((path) => [path, "pending" as FolderStatus])
      ),
      isRunning: true,
      persistError: null,
    });

    const startedAt = new Date();

    const { unlistenFns, errors: listenerErrors } =
      await registerProgressListeners({
        onCopying: (folder, path) =>
          set((state) => ({
            folderCurrentFile: { ...state.folderCurrentFile, [folder]: path },
          })),
        onFailure: (folder, error) =>
          set((state) => ({
            folderErrors: { ...state.folderErrors, [folder]: error },
            folderStatuses: { ...state.folderStatuses, [folder]: "error" },
          })),
        onStart: (folder) =>
          set((state) => ({
            folderStatuses: { ...state.folderStatuses, [folder]: "running" },
          })),
        onSuccess: (folder) =>
          set((state) => ({
            folderStatuses: { ...state.folderStatuses, [folder]: "success" },
          })),
      });

    if (listenerErrors.length > 0) {
      for (const unlisten of unlistenFns) {
        unlisten();
      }
      set({
        batchError: `Failed to set up progress listeners: ${listenerErrors.join("; ")}`,
        isRunning: false,
      });
      return;
    }

    try {
      const outcome = await invoke<BatchOutcome>(command, {
        dest,
        includedPaths,
        serial,
      });
      set({ batchOutcome: outcome });
      const finishedAt = new Date();
      try {
        await persistRunResult({ direction, finishedAt, outcome, serial, startedAt });
      } catch (err) {
        set({ persistError: describeError(err) });
      }
    } catch (err) {
      set({ batchError: describeError(err) });
    } finally {
      for (const unlisten of unlistenFns) {
        unlisten();
      }
      set({ isRunning: false });
    }
  },

  // --- History placeholders, implemented in Task 12 ---
  runsList: [],
  itemsByRun: new Map(),
  folderSyncRows: [],
  isLoadingHistory: true,
  historyError: null,
  loadHistory: async () => {
    // Filled in by Task 12.
  },
}));

export { historyGeneration as __historyGeneration };
```

The trailing `export { historyGeneration as __historyGeneration }` is a deliberate,
temporary marker so `bun run check` doesn't flag `historyGeneration` as an unused
module-level `let` between this task and Task 12 filling in the real `loadHistory` body.
**Task 12 removes this export** when it replaces the placeholder `loadHistory`.

**Step 2: Verify**

Run: `bun run check`
Expected: passes.

**Step 3: Commit**

```bash
git add src/stores/run-store.ts
git commit -m "feat: add run store (preflight + active run), history stubbed"
```

---

## Task 11: Migrate run-screen.tsx to the run store

**Files:**
- Modify: `src/screens/run-screen.tsx` (large rewrite — keeps the form-input local state,
  moves everything else to `useRunStore`)

**Step 1: Rewrite**

Delete everything from the top of the file through the end of
`runSyncBatchAndPersist` (originally lines 1-420: all the type mirrors, `describeError`
import is already fixed by Task 2, `persistRunResult`, `registerProgressListeners`,
`runSyncBatchAndPersist`) — all of that now lives in `run-store.ts`. Replace with:

```tsx
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
import { useCallback, useState } from "react";
import type { Direction, FolderStatus } from "../stores/run-store";
import { useRunStore } from "../stores/run-store";

interface RunScreenProps {
  dest?: string;
  direction?: Direction;
  /** Full ANDROID-side paths (e.g. `/storage/emulated/0/DCIM`), matching
   * exactly what `run_backup`/`run_restore`/`space_check` expect. */
  includedPaths?: string[];
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

  const spaceCheck = useRunStore((state) => state.spaceCheck);
  const isCheckingSpace = useRunStore((state) => state.isCheckingSpace);
  const spaceCheckError = useRunStore((state) => state.spaceCheckError);
  const isCloudWarningDismissed = useRunStore(
    (state) => state.isCloudWarningDismissed
  );
  const invalidatePreflight = useRunStore((state) => state.invalidatePreflight);
  const dismissCloudWarning = useRunStore((state) => state.dismissCloudWarning);
  const runSpaceCheckAction = useRunStore((state) => state.runSpaceCheck);

  const isRunning = useRunStore((state) => state.isRunning);
  const folderStatuses = useRunStore((state) => state.folderStatuses);
  const folderCurrentFile = useRunStore((state) => state.folderCurrentFile);
  const folderErrors = useRunStore((state) => state.folderErrors);
  const batchOutcome = useRunStore((state) => state.batchOutcome);
  const batchError = useRunStore((state) => state.batchError);
  const persistError = useRunStore((state) => state.persistError);
  const startRun = useRunStore((state) => state.startRun);

  const canStart =
    !(isRunning || isCheckingSpace) &&
    serial.trim() !== "" &&
    dest.trim() !== "" &&
    includedPaths.length > 0 &&
    spaceCheck?.has_enough_space === true;

  const runSpaceCheck = useCallback(() => {
    runSpaceCheckAction({ dest, includedPaths, serial });
  }, [runSpaceCheckAction, serial, dest, includedPaths]);

  const handleStart = useCallback(() => {
    if (!canStart) {
      return;
    }
    startRun({
      command: direction === "backup" ? "run_backup" : "run_restore",
      dest,
      direction,
      includedPaths,
      serial,
    });
  }, [canStart, startRun, direction, dest, includedPaths, serial]);

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

  return (
    <VStack gap={4}>
      <Heading level={1}>Run backup or restore</Heading>

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
                onDismiss={dismissCloudWarning}
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
```

Note the removed "No device/classification routing is wired up yet" `Text` line — that's
no longer true as of `app.tsx`'s original navigation wiring, and this task's job is
migrating state management, not scope-creeping into further copy cleanup, but that one
line was already stale before this task and directly contradicts the now-fully-wired
navigation, so it's removed here rather than left actively misleading.

**Step 2: Verify**

Run: `bun run check`
Expected: passes.

Run: `bun run tauri dev`, do a full backup run against a real (or test) device end to
end: check space, start backup, confirm live per-folder progress still updates exactly
as before, confirm the run completes and its history write succeeds.

Additionally verify the documented behavior change: start a run, immediately navigate to
another screen (e.g. History) while it's still in progress, then navigate back to Run.
Expected: the in-progress/completed state is still there (not reset), unlike before this
refactor.

**Step 3: Commit**

```bash
git add src/screens/run-screen.tsx
git commit -m "refactor: migrate run-screen.tsx to run store"
```

---

## Task 12: Run store — history slice, and migrate history-screen.tsx

**Files:**
- Modify: `src/stores/run-store.ts` (replace the `loadHistory` stub from Task 10)
- Modify: `src/screens/history-screen.tsx` (large rewrite — drops its own `loadHistory`/
  `deriveFolderSyncStatus` in favor of the store's)

**Step 1: Replace the `loadHistory` stub in run-store.ts**

Remove the trailing `export { historyGeneration as __historyGeneration };` line and the
placeholder `loadHistory: async () => { /* Filled in by Task 12. */ }`, replacing the
whole history section of the `create<RunState>` object with:

```typescript
  runsList: [],
  itemsByRun: new Map(),
  folderSyncRows: [],
  isLoadingHistory: true,
  historyError: null,

  loadHistory: async (serial) => {
    historyGeneration += 1;
    const generation = historyGeneration;
    set({ historyError: null, isLoadingHistory: true });
    try {
      const runsList = await db
        .select()
        .from(runs)
        .where(serial ? eq(runs.deviceSerial, serial) : undefined)
        .orderBy(desc(runs.startedAt));

      const runIds = runsList.map((run) => run.id);
      const items =
        runIds.length > 0
          ? await db
              .select()
              .from(runItems)
              .where(inArray(runItems.runId, runIds))
          : [];

      const itemsByRun = new Map<number, RunItem[]>();
      for (const item of items) {
        const existing = itemsByRun.get(item.runId);
        if (existing) {
          existing.push(item);
        } else {
          itemsByRun.set(item.runId, [item]);
        }
      }

      const includedRules = await db
        .select()
        .from(folderRules)
        .where(
          and(
            eq(folderRules.decision, "include"),
            serial ? eq(folderRules.deviceSerial, serial) : undefined
          )
        );

      const runById = new Map(runsList.map((run) => [run.id, run]));
      const folderSyncRows = deriveFolderSyncStatus(
        includedRules,
        items,
        runById
      );

      if (historyGeneration !== generation) {
        return;
      }
      set({
        folderSyncRows,
        isLoadingHistory: false,
        itemsByRun,
        runsList,
      });
    } catch (err) {
      if (historyGeneration !== generation) {
        return;
      }
      set({ historyError: describeError(err), isLoadingHistory: false });
    }
  },
```

**Step 2: Rewrite history-screen.tsx**

```tsx
import { Badge } from "@astryxdesign/core/Badge";
import { Banner } from "@astryxdesign/core/Banner";
import { Button } from "@astryxdesign/core/Button";
import { Collapsible, CollapsibleGroup } from "@astryxdesign/core/Collapsible";
import { EmptyState } from "@astryxdesign/core/EmptyState";
import { Heading } from "@astryxdesign/core/Heading";
import { HStack } from "@astryxdesign/core/HStack";
import { List, ListItem } from "@astryxdesign/core/List";
import { StatusDot } from "@astryxdesign/core/StatusDot";
import { Text } from "@astryxdesign/core/Text";
import { VStack } from "@astryxdesign/core/VStack";
import { useCallback, useEffect } from "react";
import type { FolderStatus } from "../stores/run-store";
import { useRunStore } from "../stores/run-store";

interface HistoryScreenProps {
  serial?: string;
}

function formatDateTime(date: Date): string {
  return date.toLocaleString();
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

type RunStatus = "running" | "completed" | "failed" | "cancelled";

function runStatusVariant(
  status: RunStatus
): "success" | "warning" | "error" | "accent" | "neutral" {
  switch (status) {
    case "completed":
      return "success";
    case "failed":
      return "error";
    case "running":
      return "accent";
    default:
      return "neutral";
  }
}

function runStatusLabel(status: RunStatus): string {
  switch (status) {
    case "completed":
      return "Completed";
    case "failed":
      return "Failed";
    case "running":
      return "Running";
    default:
      return "Cancelled";
  }
}

type RunItemStatus = "synced" | "outdated" | "broken" | "skipped" | "error";

function runItemStatusVariant(
  status: RunItemStatus
): "success" | "warning" | "error" | "neutral" {
  switch (status) {
    case "synced":
      return "success";
    case "broken":
    case "error":
      return "error";
    case "outdated":
      return "warning";
    default:
      return "neutral";
  }
}

function runItemStatusLabel(status: RunItemStatus): string {
  switch (status) {
    case "synced":
      return "Synced";
    case "broken":
      return "Broken";
    case "error":
      return "Error";
    case "outdated":
      return "Outdated";
    default:
      return "Skipped";
  }
}

function runBadgeVariant(
  status: RunStatus
): "error" | "success" | "info" | "neutral" {
  if (status === "failed") {
    return "error";
  }
  if (status === "completed") {
    return "success";
  }
  if (status === "running") {
    return "info";
  }
  return "neutral";
}

function RunTrigger({
  run,
  items,
}: {
  run: { id: number; deviceSerial: string; status: RunStatus; type: "backup" | "restore"; startedAt: Date; finishedAt: Date | null };
  items: { status: RunItemStatus }[];
}) {
  const errorCount = items.filter(
    (item) => item.status === "error" || item.status === "broken"
  ).length;

  return (
    <HStack gap={3} vAlign="center" wrap="wrap">
      <StatusDot
        label={runStatusLabel(run.status)}
        variant={runStatusVariant(run.status)}
      />
      <Badge
        label={runStatusLabel(run.status)}
        variant={runBadgeVariant(run.status)}
      />
      <VStack gap={0.5}>
        <Text type="label">
          {run.type === "backup" ? "Backup" : "Restore"} — {run.deviceSerial}
        </Text>
        <Text color="secondary" type="supporting">
          Started {formatDateTime(run.startedAt)}
          {run.finishedAt
            ? ` — finished ${formatDateTime(run.finishedAt)}`
            : ""}
        </Text>
      </VStack>
      <Text color="secondary" type="supporting">
        {items.length} folder{items.length === 1 ? "" : "s"}
      </Text>
      {errorCount > 0 ? (
        <Badge
          label={`${errorCount} error${errorCount === 1 ? "" : "s"}`}
          variant="error"
        />
      ) : null}
    </HStack>
  );
}

function transferSummary(
  fileCount: number | null,
  bytesTransferred: number | null
): string | undefined {
  const parts: string[] = [];
  if (fileCount !== null) {
    parts.push(`${fileCount} files`);
  }
  if (bytesTransferred !== null) {
    parts.push(formatBytes(bytesTransferred));
  }
  return parts.length > 0 ? parts.join(" — ") : undefined;
}

function RunItemDetail({
  items,
}: {
  items: {
    id: number;
    path: string;
    status: RunItemStatus;
    errorMessage: string | null;
    fileCount: number | null;
    bytesTransferred: number | null;
  }[];
}) {
  if (items.length === 0) {
    return (
      <Text color="secondary" type="supporting">
        No per-folder detail was recorded for this run.
      </Text>
    );
  }

  return (
    <List density="compact" hasDividers>
      {items.map((item) => (
        <ListItem
          description={item.errorMessage ?? undefined}
          endContent={
            <HStack gap={2} vAlign="center">
              {item.status === "synced" ? null : (
                <Badge
                  label={runItemStatusLabel(item.status)}
                  variant={runItemStatusVariant(item.status)}
                />
              )}
              <Text color="secondary" type="supporting">
                {transferSummary(item.fileCount, item.bytesTransferred)}
              </Text>
            </HStack>
          }
          key={item.id}
          label={item.path}
          startContent={
            <StatusDot
              label={runItemStatusLabel(item.status)}
              variant={runItemStatusVariant(item.status)}
            />
          }
        />
      ))}
    </List>
  );
}

export function HistoryScreen({ serial }: HistoryScreenProps = {}) {
  const runsList = useRunStore((state) => state.runsList);
  const itemsByRun = useRunStore((state) => state.itemsByRun);
  const folderSyncRows = useRunStore((state) => state.folderSyncRows);
  const isLoading = useRunStore((state) => state.isLoadingHistory);
  const error = useRunStore((state) => state.historyError);
  const load = useRunStore((state) => state.loadHistory);

  const loadForSerial = useCallback(() => {
    load(serial);
  }, [load, serial]);

  useEffect(() => {
    loadForSerial();
  }, [loadForSerial]);

  const notSyncedCount = folderSyncRows.filter(
    (row) => row.lastSyncedAt === null
  ).length;

  return (
    <VStack gap={4}>
      <HStack gap={3} justify="between" vAlign="center">
        <Heading level={1}>History</Heading>
        <Button
          isLoading={isLoading}
          label="Refresh"
          onClick={loadForSerial}
          variant="secondary"
        />
      </HStack>
      <Text color="secondary">
        Past backup/restore runs and which included folders have never been
        synced.
      </Text>

      {error ? (
        <Banner
          description={error}
          endContent={<Button label="Retry" onClick={loadForSerial} />}
          status="error"
          title="Failed to load history"
        />
      ) : null}

      <VStack gap={2}>
        <Heading level={2}>Folder sync status</Heading>
        {isLoading && folderSyncRows.length === 0 ? (
          <Text color="secondary">Loading…</Text>
        ) : null}
        {!(isLoading || error) && folderSyncRows.length === 0 ? (
          <Text color="secondary" type="supporting">
            No saved folder selections yet — save some from the Classification
            screen first.
          </Text>
        ) : null}
        {folderSyncRows.length > 0 ? (
          <>
            {notSyncedCount > 0 ? (
              <Banner
                description={`${notSyncedCount} included folder${notSyncedCount === 1 ? " has" : "s have"} never completed a sync.`}
                status="warning"
                title="Not-synced folders"
              />
            ) : null}
            <List
              hasDividers
              header={<Text type="label">Included folders</Text>}
            >
              {folderSyncRows.map(({ rule, lastSyncedAt }) => (
                <ListItem
                  description={rule.deviceSerial}
                  endContent={
                    lastSyncedAt ? (
                      <Text color="secondary" type="supporting">
                        Last synced {formatDateTime(lastSyncedAt)}
                      </Text>
                    ) : (
                      <Badge label="Not synced" variant="warning" />
                    )
                  }
                  key={`${rule.deviceSerial}::${rule.path}`}
                  label={rule.path}
                  startContent={
                    <StatusDot
                      label={lastSyncedAt ? "Synced" : "Not synced"}
                      variant={lastSyncedAt ? "success" : "warning"}
                    />
                  }
                />
              ))}
            </List>
          </>
        ) : null}
      </VStack>

      <VStack gap={2}>
        <Heading level={2}>Past runs</Heading>
        {isLoading && runsList.length === 0 ? (
          <Text color="secondary">Loading…</Text>
        ) : null}
        {!(isLoading || error) && runsList.length === 0 ? (
          <EmptyState
            description="Backup/restore runs will show up here once you start one from the Run screen."
            title="No runs yet"
          />
        ) : null}
        {runsList.length > 0 ? (
          <CollapsibleGroup hasDividers type="multiple">
            {runsList.map((run) => {
              const items = itemsByRun.get(run.id) ?? [];
              return (
                <Collapsible
                  defaultIsOpen={false}
                  key={run.id}
                  trigger={<RunTrigger items={items} run={run} />}
                  value={String(run.id)}
                >
                  <RunItemDetail items={items} />
                </Collapsible>
              );
            })}
          </CollapsibleGroup>
        ) : null}
      </VStack>
    </VStack>
  );
}
```

Note this also updates the "Folder sync status" empty-state copy, since it previously
explicitly said persistence didn't exist yet ("Task 12 left that persistence step as a
placeholder") — that's no longer true as of Task 8/9.

**Step 2: Verify**

Run: `bun run check`
Expected: passes.

Run: `bun run tauri dev`, open History. Confirm past runs and folder sync rows still
render identically to before this refactor, and that a freshly-saved Classification
selection (from Task 9's verification) shows up correctly.

**Step 3: Commit**

```bash
git add src/stores/run-store.ts src/screens/history-screen.tsx
git commit -m "feat: complete run store history slice, migrate history-screen.tsx"
```

---

## Task 13: Final cleanup and full click-through verification

**Files:**
- Review: `src/app.tsx`, all 5 screens, all 4 stores

**Step 1: Grep for dead code**

Run: `cd "C:\Users\hency\OneDrive\Desktop\adb-phone-sync" && grep -rn "useState\|useCallback\|useRef" src/screens/ src/app.tsx`

Expected: only the intentionally-kept local state remains — `RunScreen`'s form inputs
(`serial`, `dest`, `includedPathsText`, `direction`) and `ClassificationScreen`'s/
`DeviceScreen`'s `checkboxRef`. Every screen's old `fetchGenerationRef`, loading/error/
list `useState`, and prop-drilled callback should be gone.

**Step 2: Full lint + type check**

Run: `bun run check`
Expected: passes with zero errors.

**Step 3: Full manual click-through**

Run: `bun run tauri dev` and walk the entire flow once more end to end: Device → select
a device → Classify → toggle folders → Save (confirm folder_rules persists) → Run →
check space → start backup → watch live progress → confirm completion → History (confirm
the run and folder sync status show up correctly) → Profile Settings (confirm the device
profile from this run is editable and saves).

**Step 4: Commit**

If Step 1 found nothing to remove and Step 3 passes clean, there's nothing new to
commit — this task is a verification pass, not a code change. If Step 1 did find leftover
dead code, remove it and commit:

```bash
git add -A
git commit -m "chore: remove dead state left over from Zustand migration"
```
