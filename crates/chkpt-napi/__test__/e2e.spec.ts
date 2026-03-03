import { describe, it, expect } from "vitest";
import {
  save,
  list,
  restore,
  deleteSnapshot,
  getProjectId,
  scanWorkspace,
  blobHash,
} from "../index.js";
import {
  mkdtempSync,
  writeFileSync,
  readFileSync,
  existsSync,
  mkdirSync,
} from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

describe("end-to-end", () => {
  it("full lifecycle: save → modify → save → list → restore → delete", async () => {
    const workspace = mkdtempSync(join(tmpdir(), "chkpt-e2e-"));

    // Create initial files
    writeFileSync(join(workspace, "README.md"), "# Hello");
    mkdirSync(join(workspace, "src"));
    writeFileSync(join(workspace, "src", "index.ts"), 'console.log("v1")');

    // Verify project ID is deterministic
    const id1 = getProjectId(workspace);
    const id2 = getProjectId(workspace);
    expect(id1).toBe(id2);

    // Save checkpoint 1
    const save1 = await save(workspace, "initial version");
    expect(save1.snapshotId).toBeTruthy();
    expect(save1.totalFiles).toBe(2);

    // Modify files
    writeFileSync(join(workspace, "src", "index.ts"), 'console.log("v2")');
    writeFileSync(join(workspace, "src", "utils.ts"), "export const x = 1");

    // Save checkpoint 2
    const save2 = await save(workspace, "added utils");
    expect(save2.totalFiles).toBe(3);

    // List should show 2 snapshots (newest first)
    const snapshots = await list(workspace);
    expect(snapshots).toHaveLength(2);
    expect(snapshots[0].message).toBe("added utils");
    expect(snapshots[1].message).toBe("initial version");

    // Dry-run restore to checkpoint 1
    const dryRun = await restore(workspace, save1.snapshotId, true);
    expect(dryRun.filesChanged).toBe(1); // index.ts changed
    expect(dryRun.filesRemoved).toBe(1); // utils.ts to be removed
    // Verify no actual changes
    expect(readFileSync(join(workspace, "src", "index.ts"), "utf-8")).toBe(
      'console.log("v2")',
    );

    // Actual restore to checkpoint 1
    const restoreResult = await restore(workspace, save1.snapshotId, false);
    expect(restoreResult.filesChanged).toBe(1);
    expect(restoreResult.filesRemoved).toBe(1);
    expect(readFileSync(join(workspace, "src", "index.ts"), "utf-8")).toBe(
      'console.log("v1")',
    );
    expect(existsSync(join(workspace, "src", "utils.ts"))).toBe(false);

    // Delete checkpoint 2
    await deleteSnapshot(workspace, save2.snapshotId);
    const remaining = await list(workspace);
    expect(remaining).toHaveLength(1);
    expect(remaining[0].id).toBe(save1.snapshotId);

    // Low-level APIs also work
    const files = await scanWorkspace(workspace);
    expect(files).toHaveLength(2);
    const hash = blobHash(Buffer.from("# Hello"));
    expect(hash).toMatch(/^[0-9a-f]{64}$/);
  });
});
