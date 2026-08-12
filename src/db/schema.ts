import {
  index,
  integer,
  sqliteTable,
  text,
  uniqueIndex,
} from "drizzle-orm/sqlite-core";

export const devices = sqliteTable("devices", {
  // Nullable: no task before Task 15 (Profile settings) ever writes this --
  // `run-screen.tsx`'s `persistRunResult` only upserts `displayName`/
  // `lastSeen`, and its `dest` field is a per-run form value, never
  // persisted. This column lets a device's chosen backup destination survive
  // across runs once a user sets it from the Profile settings screen; it's
  // NULL for every device until they do.
  destinationPath: text("destination_path"),
  displayName: text("display_name").notNull(),
  firstSeen: integer("first_seen", { mode: "timestamp" }).notNull(),
  lastSeen: integer("last_seen", { mode: "timestamp" }).notNull(),
  serial: text("serial").primaryKey(),
});

export const folderRules = sqliteTable(
  "folder_rules",
  {
    decision: text("decision", { enum: ["include", "skip"] }).notNull(),
    deviceSerial: text("device_serial")
      .notNull()
      .references(() => devices.serial),
    id: integer("id").primaryKey({ autoIncrement: true }),
    path: text("path").notNull(),
    source: text("source", { enum: ["heuristic", "manual"] }).notNull(),
    updatedAt: integer("updated_at", { mode: "timestamp" }).notNull(),
  },
  (table) => [
    index("folder_rules_device_serial_idx").on(table.deviceSerial),
    index("folder_rules_path_idx").on(table.path),
    uniqueIndex("folder_rules_device_serial_path_unique").on(
      table.deviceSerial,
      table.path
    ),
  ]
);

export const runs = sqliteTable(
  "runs",
  {
    deviceSerial: text("device_serial")
      .notNull()
      .references(() => devices.serial),
    finishedAt: integer("finished_at", { mode: "timestamp" }),
    id: integer("id").primaryKey({ autoIncrement: true }),
    startedAt: integer("started_at", { mode: "timestamp" }).notNull(),
    status: text("status", {
      enum: ["running", "completed", "failed", "cancelled"],
    }).notNull(),
    type: text("type", { enum: ["backup", "restore"] }).notNull(),
  },
  (table) => [index("runs_device_serial_idx").on(table.deviceSerial)]
);

export const runItems = sqliteTable(
  "run_items",
  {
    bytesTransferred: integer("bytes_transferred"),
    errorMessage: text("error_message"),
    fileCount: integer("file_count"),
    finishedAt: integer("finished_at", { mode: "timestamp" }),
    id: integer("id").primaryKey({ autoIncrement: true }),
    path: text("path").notNull(),
    runId: integer("run_id")
      .notNull()
      .references(() => runs.id),
    status: text("status", {
      enum: ["synced", "outdated", "broken", "skipped", "error"],
    }).notNull(),
  },
  (table) => [
    index("run_items_run_id_idx").on(table.runId),
    index("run_items_path_idx").on(table.path),
  ]
);
