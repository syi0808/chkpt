import { describe, it, expect, beforeEach } from "vitest";
import {
  indexOpen,
  indexLookup,
  indexUpsert,
  indexAllEntries,
  indexClear,
} from "../index.js";
import { mkdtempSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

describe("index", () => {
  let dbPath: string;

  beforeEach(() => {
    const dir = mkdtempSync(join(tmpdir(), "chkpt-idx-"));
    dbPath = join(dir, "index.sqlite");
  });

  it("upsert and lookup roundtrip", async () => {
    await indexOpen(dbPath);
    const entries = [
      {
        path: "hello.txt",
        blobHash: "a".repeat(64),
        size: 11,
        mtimeSecs: 1000,
        mtimeNanos: 0,
        inode: 12345,
        mode: 0o100644,
      },
    ];
    await indexUpsert(dbPath, entries);
    const result = await indexLookup(dbPath, "hello.txt");
    expect(result).not.toBeNull();
    expect(result!.path).toBe("hello.txt");
    expect(result!.blobHash).toBe("a".repeat(64));
    expect(result!.size).toBe(11);
  });

  it("lookup returns null for missing path", async () => {
    await indexOpen(dbPath);
    const result = await indexLookup(dbPath, "missing.txt");
    expect(result).toBeNull();
  });

  it("allEntries returns all entries", async () => {
    await indexOpen(dbPath);
    await indexUpsert(dbPath, [
      {
        path: "a.txt",
        blobHash: "a".repeat(64),
        size: 1,
        mtimeSecs: 0,
        mtimeNanos: 0,
        inode: null,
        mode: 0o100644,
      },
      {
        path: "b.txt",
        blobHash: "b".repeat(64),
        size: 2,
        mtimeSecs: 0,
        mtimeNanos: 0,
        inode: null,
        mode: 0o100644,
      },
    ]);
    const all = await indexAllEntries(dbPath);
    expect(all).toHaveLength(2);
  });

  it("clear removes all entries", async () => {
    await indexOpen(dbPath);
    await indexUpsert(dbPath, [
      {
        path: "x.txt",
        blobHash: "c".repeat(64),
        size: 3,
        mtimeSecs: 0,
        mtimeNanos: 0,
        inode: null,
        mode: 0o100644,
      },
    ]);
    await indexClear(dbPath);
    const all = await indexAllEntries(dbPath);
    expect(all).toHaveLength(0);
  });
});
