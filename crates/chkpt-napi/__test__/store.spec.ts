import { describe, it, expect, beforeEach } from "vitest";
import {
  blobHash,
  blobStore,
  blobLoad,
  blobExists,
  treeBuild,
  treeLoad,
} from "../index.js";
import { mkdtempSync, mkdirSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

describe("blob store", () => {
  let storeDir: string;

  beforeEach(() => {
    storeDir = mkdtempSync(join(tmpdir(), "chkpt-blob-"));
    mkdirSync(join(storeDir, "objects"), { recursive: true });
  });

  it("blobHash returns 64-char hex", () => {
    const hash = blobHash(Buffer.from("hello world"));
    expect(hash).toMatch(/^[0-9a-f]{64}$/);
  });

  it("blobHash is deterministic", () => {
    const buf = Buffer.from("test content");
    expect(blobHash(buf)).toBe(blobHash(buf));
  });

  it("blobStore + blobLoad roundtrip", async () => {
    const content = Buffer.from("hello world");
    const hash = blobHash(content);
    const objectsDir = join(storeDir, "objects");
    await blobStore(objectsDir, hash, content);
    expect(blobExists(objectsDir, hash)).toBe(true);
    const loaded = await blobLoad(objectsDir, hash);
    expect(Buffer.from(loaded)).toEqual(content);
  });

  it("blobExists returns false for missing hash", () => {
    const objectsDir = join(storeDir, "objects");
    expect(blobExists(objectsDir, "a".repeat(64))).toBe(false);
  });
});

describe("tree store", () => {
  let storeDir: string;

  beforeEach(() => {
    storeDir = mkdtempSync(join(tmpdir(), "chkpt-tree-"));
    mkdirSync(join(storeDir, "trees"), { recursive: true });
  });

  it("treeBuild + treeLoad roundtrip", async () => {
    const treesDir = join(storeDir, "trees");
    const entries = [
      {
        name: "hello.txt",
        entryType: "file",
        hash: "a".repeat(64),
        size: 11,
        mode: 0o100644,
      },
    ];
    const result = await treeBuild(treesDir, entries);
    expect(result.hash).toMatch(/^[0-9a-f]{64}$/);

    const loaded = await treeLoad(treesDir, result.hash);
    expect(loaded).toHaveLength(1);
    expect(loaded[0].name).toBe("hello.txt");
    expect(loaded[0].entryType).toBe("file");
  });
});
