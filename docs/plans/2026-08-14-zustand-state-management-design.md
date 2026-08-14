# Zustand state management — design

## Context

The app currently lifts 3 values (`activeScreen`, `selectedSerial`, `includedPaths`) in
`src/app.tsx` and prop-drills callbacks down to 5 screens. Each screen separately
re-implements its own "fetch on mount, guard against a stale response with a generation
counter ref" pattern for its own local server-state (connected devices, folder
suggestions, run history, device profiles). Domain data that's already persisted via
Drizzle/`tauri-plugin-sql` (`devices`, `runs`, `run_items`) is queried inline in each
screen's effect, with no shared caching or loading layer. A `folder_rules` table exists
in the schema but nothing has ever written to it — `ClassificationScreen`'s "Save" button
is a local-state-only placeholder that only carries the selection forward via a prop
callback, never persists it, and `HistoryScreen` shows an explicit "no saved folder
selections yet" empty state because of this gap.

As the app grows past its current 5 screens, this prop-drilling and duplicated
fetch-boilerplate will keep getting worse. This design adopts Zustand for state
management now, before more screens/features are added.

## Decisions made during brainstorming

1. **Scope**: not just navigation/cross-screen state — also move each screen's
   Drizzle-backed domain data into stores, and use this refactor to finally wire real
   `folder_rules` persistence (closing the gap above).
2. **Store granularity**: one store per domain (session/navigation, devices,
   classification, run), not one giant store or a UI-state/data-state split.
3. **Persistence mechanism**: load-on-init + write-through actions, NOT Zustand's
   `persist` middleware. Drizzle/SQLite remains the actual source of truth; each store is
   a thin reactive cache in front of it. `persist` was considered and rejected because it
   would serialize a whole store to a single JSON blob, throwing away the relational
   structure (`folder_rules`/`run_items` foreign keys and indexes) that queries like
   `deriveFolderSyncStatus`'s `inArray` lookups depend on.

## Architecture

Add the `zustand` package — no extra middleware (no `persist`, no `immer`). One store
file per domain in `src/stores/`, each a plain `create<T>()` hook. Screens read state and
call actions directly from stores instead of receiving them as props from `app.tsx`.
`app.tsx` shrinks to reading `activeScreen` from the session store and rendering the
matching screen; it no longer owns `handleDeviceSelected`/`handleClassificationSaved`-style
wiring. Existing optional-prop seams (e.g. `DeviceScreen`'s `onDeviceSelected?`) stay in
place for standalone/test usage, but the store becomes the primary data-flow path.

## Stores

- **`useSessionStore`** — `activeScreen`, `selectedSerial`, `includedPaths`. In-memory
  only (resets on app restart, same as today's behavior). Actions: `goTo(screen)`,
  `selectDevice(serial)` (also navigates to Classify, replacing today's
  `handleDeviceSelected`), `setIncludedPaths(paths)` (replacing
  `handleClassificationSaved`).
- **`useDevicesStore`** — two slices in one store, since they're one domain from the UI's
  perspective: `connectedDevices` (live `adb`-visible devices from the `list_devices`
  command, ephemeral, never persisted) and `savedProfiles` (the persisted `devices` table
  — display name, destination path, first/last seen). Actions: `loadConnectedDevices()`,
  `loadProfiles()`, `updateDraft(serial, field, value)`, `saveProfile(serial)` (Drizzle
  `update`, same statement `saveDeviceProfile` runs today).
- **`useClassificationStore`** — `suggestions` (from `classify_suggest`, ephemeral per
  device) and `folderRules` (the persisted `folder_rules` table). Actions:
  `loadSuggestions(serial)`, `toggleFolder(name)`, and the new `saveFolderRules(serial)`
  — the first real write to `folder_rules`. Converts folder names to full Android paths
  (same `STORAGE_ROOT` prefix `handleSave` already uses today) and upserts rows via a
  `db.transaction()`, since `folder_rules.path` must store full paths to correctly join
  against `run_items.path` in `deriveFolderSyncStatus`.
- **`useRunStore`** — preflight (`spaceCheck`, `isCheckingSpace`, cloud-warning
  dismissal), in-progress run state (`folderStatuses`, `folderCurrentFile`,
  `folderErrors`, `batchOutcome`/`batchError`/`persistError`), and history (`runsList`,
  `itemsByRun`, `folderSyncRows`). Actions: `runSpaceCheck`, `startRun`,
  `persistRunResult` (today's transaction, now a store action), `loadHistory(serial)`
  (today's `loadHistory` function becomes a store action; `deriveFolderSyncStatus` stays
  a pure standalone helper).

## Data flow & persistence mechanics

Each domain store's `load*()` action runs the existing Drizzle query (or Tauri `invoke`)
and sets state directly. Drizzle/SQLite stays the real source of truth; the store is a
reactive cache in front of it, not a parallel copy. Writes flow: component calls a store
action → action runs the Drizzle write (wrapped in `db.transaction()` where multiple
statements must land atomically, same pattern `persistRunResult` already uses) → on
success, the action updates in-memory state to match (or re-runs `load*()`). The
generation-counter-ref guard against stale in-flight fetches — duplicated today across
`device-screen.tsx`, `classification-screen.tsx`, `history-screen.tsx`, and
`profile-settings-screen.tsx` — moves inside each store's `load*()` action instead of
being re-implemented per screen.

## Error handling

Unchanged in substance. `describeError()`'s `.cause`-chain walking (today only in
`run-screen.tsx`) moves to a shared module since multiple stores now need it. Per-store
`error`/`isLoading` fields replace today's per-screen `useState` equivalents with the
same shape and behavior.

## Testing

Pure logic already pulled into standalone functions (`deriveFolderSyncStatus`,
`describeError`, `parseIncludedPathsText`) stays exported and independently testable,
same as today — moving into a store doesn't change their signatures or testability.

## Migration approach

Screen-by-screen replacement, each swapped and manually verified before moving to the
next, rather than one big-bang rewrite — mirrors how the original 16-task
backup/restore build was sequenced:

1. Add `zustand` dependency, create `src/stores/` with all 4 store files.
2. `useSessionStore` + `app.tsx` (navigation/cross-screen state).
3. `useDevicesStore` + `DeviceScreen` + `ProfileSettingsScreen`.
4. `useClassificationStore` + `ClassificationScreen`, including the new
   `saveFolderRules` persistence.
5. `useRunStore` + `RunScreen` + `HistoryScreen`.
6. Remove now-dead prop-drilling plumbing from `app.tsx` and the screens.
