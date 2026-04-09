import { describe, it, expect } from "vitest";
import { getProjectId, getStoreLayout } from "../index.js";
import { mkdtempSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

describe("config", () => {
  it("getProjectId returns a 16-char hex string", () => {
    const dir = mkdtempSync(join(tmpdir(), "chkpt-test-"));
    const id = getProjectId(dir);
    expect(id).toMatch(/^[0-9a-f]{16}$/);
  });

  it("getProjectId is deterministic for same path", () => {
    const dir = mkdtempSync(join(tmpdir(), "chkpt-test-"));
    expect(getProjectId(dir)).toBe(getProjectId(dir));
  });

  it("getStoreLayout returns all required paths", () => {
    const dir = mkdtempSync(join(tmpdir(), "chkpt-test-"));
    const layout = getStoreLayout(dir);
    expect(layout.root).toContain(".chkpt/stores/");
    expect(layout.treesDir).toContain("trees");
    expect(layout.indexPath).toContain("index.bin");
    expect(layout.locksDir).toContain("locks");
    expect(layout.packsDir).toContain("packs");
  });
});
