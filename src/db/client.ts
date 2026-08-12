import Database from "@tauri-apps/plugin-sql";
import { drizzle } from "drizzle-orm/sqlite-proxy";
// biome-ignore lint/performance/noNamespaceImport: drizzle needs the whole schema module as one object
import * as schema from "./schema";

const sqlite = await Database.load("sqlite:adb-phone-sync.db");

export const db = drizzle(
  async (sql, params, method) => {
    // `run` is used by drizzle for statements that don't need rows back
    // (INSERT/UPDATE/DELETE without a RETURNING clause).
    if (method === "run") {
      await sqlite.execute(sql, params);
      return { rows: [] };
    }

    // `all`, `get`, and `values` all read rows back from the database.
    // The Tauri SQL plugin returns rows as objects keyed by column name;
    // drizzle-orm's sqlite-proxy expects each row as an array of column
    // values in column order, so we convert with `Object.values`.
    const result = await sqlite.select<Record<string, unknown>[]>(sql, params);
    const rows = result.map((row) => Object.values(row));

    // `get` expects a single row (flattened, not wrapped in an outer array),
    // while `all`/`values` expect an array of rows.
    if (method === "get") {
      return { rows: rows[0] };
    }

    return { rows };
  },
  { schema }
);
