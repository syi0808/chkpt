import { describe, it, expect, beforeEach } from "vitest";
import {
  blobHash,
  treeBuild,
  treeLoad,
} from "../index.js";
import { mkdtempSync, mkdirSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

describe("blob store", () => {
  it("blobHash returns 64-char hex", () => {
    const hash = blobHash(Buffer.from("hello world"));
    expect(hash).toMatch(/^[0-9a-f]{64}$/);
  });

  it("blobHash is deterministic", () => {
    const buf = Buffer.from("test content");
    expect(blobHash(buf)).toBe(blobHash(buf));
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
