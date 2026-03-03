import { describe, it, expect, beforeEach } from "vitest";
import { save, list, restore, deleteSnapshot } from "../index.js";
import {
  mkdtempSync,
  writeFileSync,
  mkdirSync,
  readFileSync,
  existsSync,
} from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

describe("operations", () => {
  let workspace: string;

  beforeEach(() => {
    workspace = mkdtempSync(join(tmpdir(), "chkpt-ops-"));
    writeFileSync(join(workspace, "hello.txt"), "hello world");
    mkdirSync(join(workspace, "src"));
    writeFileSync(join(workspace, "src", "main.rs"), "fn main() {}");
  });

  it("save creates a checkpoint", async () => {
    const result = await save(workspace, "test save");
    expect(result.snapshotId).toBeTruthy();
    expect(result.totalFiles).toBe(2);
    expect(result.totalBytes).toBeGreaterThan(0);
    expect(result.newObjects).toBe(2);
  });

  it("list returns saved checkpoints", async () => {
    await save(workspace, "first");
    await save(workspace, "second");
    const snapshots = await list(workspace);
    expect(snapshots).toHaveLength(2);
    expect(snapshots[0].message).toBe("second");
    expect(snapshots[1].message).toBe("first");
  });

  it("list with limit", async () => {
    await save(workspace, "a");
    await save(workspace, "b");
    await save(workspace, "c");
    const snapshots = await list(workspace, 2);
    expect(snapshots).toHaveLength(2);
  });

  it("restore with dry-run shows changes", async () => {
    const { snapshotId } = await save(workspace, "before");
    writeFileSync(join(workspace, "hello.txt"), "modified");
    writeFileSync(join(workspace, "new.txt"), "new file");

    const result = await restore(workspace, snapshotId, true);
    expect(result.filesChanged).toBe(1);
    expect(result.filesRemoved).toBe(1);

    // Verify workspace unchanged (dry-run)
    expect(readFileSync(join(workspace, "hello.txt"), "utf-8")).toBe(
      "modified",
    );
  });

  it("restore actually restores files", async () => {
    const { snapshotId } = await save(workspace, "original");
    writeFileSync(join(workspace, "hello.txt"), "changed");
    writeFileSync(join(workspace, "extra.txt"), "extra");

    const result = await restore(workspace, snapshotId, false);
    expect(result.filesChanged).toBe(1);
    expect(result.filesRemoved).toBe(1);

    expect(readFileSync(join(workspace, "hello.txt"), "utf-8")).toBe(
      "hello world",
    );
    expect(existsSync(join(workspace, "extra.txt"))).toBe(false);
  });

  it('restore "latest" works', async () => {
    await save(workspace, "snap1");
    writeFileSync(join(workspace, "hello.txt"), "v2");
    await save(workspace, "snap2");
    writeFileSync(join(workspace, "hello.txt"), "v3");

    await restore(workspace, "latest", false);
    expect(readFileSync(join(workspace, "hello.txt"), "utf-8")).toBe("v2");
  });

  it("deleteSnapshot removes a checkpoint", async () => {
    const { snapshotId } = await save(workspace, "to-delete");
    let snapshots = await list(workspace);
    expect(snapshots).toHaveLength(1);

    await deleteSnapshot(workspace, snapshotId);
    snapshots = await list(workspace);
    expect(snapshots).toHaveLength(0);
  });
});
