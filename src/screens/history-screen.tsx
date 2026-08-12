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
import { and, desc, eq, inArray } from "drizzle-orm";
import { useCallback, useEffect, useRef, useState } from "react";
import { db } from "../db/client";
import { folderRules, runItems, runs } from "../db/schema";

type Run = typeof runs.$inferSelect;
type RunItem = typeof runItems.$inferSelect;
type FolderRule = typeof folderRules.$inferSelect;

interface HistoryScreenProps {
  /**
   * Scopes both queries below to one device. Omit to show history across
   * every device that has ever recorded a run/folder rule -- there's no
   * cross-screen routing yet (same as Tasks 11-13), so a device serial isn't
   * reliably available here.
   */
  serial?: string;
}

/** A folder_rules row paired with the most recent successful sync for its
 * (device, path), derived client-side from `run_items` (see `deriveFolderSyncStatus`
 * below) rather than as a second SQL aggregate query. */
interface FolderSyncStatus {
  lastSyncedAt: Date | null;
  rule: FolderRule;
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

function runStatusVariant(
  status: Run["status"]
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

function runStatusLabel(status: Run["status"]): string {
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

/**
 * Covers every `run_items.status` enum value from the schema, not just the
 * two ("synced"/"error") that `run-screen.tsx`'s `persistRunResult` actually
 * writes today. "outdated"/"broken"/"skipped" are schema-valid but nothing in
 * the codebase writes them yet (per the design doc §4, "outdated" is meant to
 * be recorded as an `outdated -> synced` transition, i.e. it should show up
 * as a `synced` row, not a distinct stored status) -- keeping this mapping
 * total rather than partial means this screen won't need a follow-up edit the
 * day a future task starts writing one of those values.
 */
function runItemStatusVariant(
  status: RunItem["status"]
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

function runItemStatusLabel(status: RunItem["status"]): string {
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

/**
 * Derives the two display-only facts the design doc (§4) says should never
 * need their own Rust command: "last synced" (`MAX(finished_at)` per path)
 * and "not-synced" (an `include`d `folder_rules` path with no matching
 * `run_items` row). Both are computed here from data the caller already
 * fetched, rather than as a second SQL `GROUP BY`/`MAX()` round trip -- one
 * fewer statement means one fewer chance to interleave with a concurrent
 * writer (see the module-level comment on `loadHistory`).
 *
 * "Last synced" only counts `status = "synced"` items: an `error` row's
 * `finished_at` marks when the attempt failed, not when the path was last
 * successfully synced, so folding it into the MAX would misreport a broken
 * path as freshly synced.
 */
function deriveFolderSyncStatus(
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
 * Fetches everything this screen displays: the `runs` list, the `run_items`
 * belonging to those runs, and the `include`d `folder_rules` rows used to
 * derive "not-synced". Three independent statements, each a separate Tauri
 * IPC call/pool checkout (see `src/db/client.ts`) -- deliberately NOT wrapped
 * in a `db.transaction()`, since this is a pure read with no invariant across
 * the three that needs atomicity, and per Task 13's review, this is exactly
 * the kind of concurrent DB access that could interleave with an in-progress
 * run's `persistRunResult` transaction. Callers of this function are
 * responsible for not polling it aggressively (see `HistoryScreen` below).
 */
async function loadHistory(serial: string | undefined): Promise<{
  runsList: Run[];
  itemsByRun: Map<number, RunItem[]>;
  folderSyncRows: FolderSyncStatus[];
}> {
  const runsList = await db
    .select()
    .from(runs)
    .where(serial ? eq(runs.deviceSerial, serial) : undefined)
    .orderBy(desc(runs.startedAt));

  const runIds = runsList.map((run) => run.id);
  const items =
    runIds.length > 0
      ? await db.select().from(runItems).where(inArray(runItems.runId, runIds))
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
  const folderSyncRows = deriveFolderSyncStatus(includedRules, items, runById);

  return { folderSyncRows, itemsByRun, runsList };
}

function runBadgeVariant(
  status: Run["status"]
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

function RunTrigger({ run, items }: { run: Run; items: RunItem[] }) {
  const errorCount = items.filter(
    (item) => item.status === "error" || item.status === "broken"
  ).length;

  return (
    <HStack gap={3} vAlign="center" wrap="wrap">
      {/* StatusDot's `label` is aria-only; the Badge next to it is the
       * visible status text, so status is never color-only (StatusDot best
       * practice) -- same pairing `run-screen.tsx` uses for folder rows. */}
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

/** Combines file count + transferred bytes into one line, e.g. "12 files —
 * 4.2 MB". Either half may be absent (not every `run_items` row has both set
 * -- e.g. an `error` row's outcome fields are typically null). */
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

function RunItemDetail({ items }: { items: RunItem[] }) {
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
              {/* Only badge the exceptional statuses -- "synced" is the
               * common/healthy case, and Badge best practice is to reserve
               * badges for states that need the user's attention rather than
               * repeating the same badge on every row. */}
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
  const [runsList, setRunsList] = useState<Run[]>([]);
  const [itemsByRun, setItemsByRun] = useState<Map<number, RunItem[]>>(
    new Map()
  );
  const [folderSyncRows, setFolderSyncRows] = useState<FolderSyncStatus[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // Same generation-counter guard Tasks 12/13 use, so a stale in-flight fetch
  // (e.g. from a fast double-click on Refresh) can never clobber state set by
  // a newer one.
  const fetchGenerationRef = useRef(0);

  // Deliberately NO polling interval here. This screen's queries run
  // alongside `run-screen.tsx`'s `persistRunResult`, which (per Task 13's
  // review) relies on every statement in its `db.transaction()` landing on
  // the same physical SQLite connection -- something `tauri-plugin-sql`'s
  // connection pool doesn't actually guarantee across independent IPC calls.
  // A live/frequent-polling history view would raise the odds of a fetch
  // here checking out a pool connection mid-transaction. Fetching only on
  // mount and on an explicit "Refresh" click keeps this screen's DB access
  // rare and user-triggered instead of a background timer competing for the
  // pool. (There's no routing yet that lets `HistoryScreen` and `RunScreen`
  // be mounted at the same time anyway -- this is forward-looking, not a
  // live bug.)
  const load = useCallback(() => {
    fetchGenerationRef.current += 1;
    const generation = fetchGenerationRef.current;
    setIsLoading(true);
    setError(null);

    loadHistory(serial)
      .then((result) => {
        if (fetchGenerationRef.current !== generation) {
          return;
        }
        setRunsList(result.runsList);
        setItemsByRun(result.itemsByRun);
        setFolderSyncRows(result.folderSyncRows);
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
  }, [serial]);

  useEffect(() => {
    load();
  }, [load]);

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
          onClick={load}
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
          endContent={<Button label="Retry" onClick={load} />}
          status="error"
          title="Failed to load history"
        />
      ) : null}

      <VStack gap={2}>
        <Heading level={2}>Folder sync status</Heading>
        {folderSyncRows.length === 0 ? (
          <Text color="secondary" type="supporting">
            No saved folder selections yet — this is expected until a
            Classification screen save actually persists to `folder_rules` (Task
            12 left that persistence step as a placeholder).
          </Text>
        ) : (
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
        )}
      </VStack>

      <VStack gap={2}>
        <Heading level={2}>Past runs</Heading>
        {isLoading && runsList.length === 0 ? (
          <Text color="secondary">Loading…</Text>
        ) : null}
        {!isLoading && runsList.length === 0 ? (
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
