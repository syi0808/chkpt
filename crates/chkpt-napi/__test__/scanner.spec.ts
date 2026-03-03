import { describe, it, expect } from "vitest";
import { scanWorkspace } from "../index.js";
import { mkdtempSync, writeFileSync, mkdirSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

describe("scanner", () => {
  it("scans files in workspace", async () => {
    const dir = mkdtempSync(join(tmpdir(), "chkpt-scan-"));
    writeFileSync(join(dir, "hello.txt"), "hello");
    mkdirSync(join(dir, "sub"));
    writeFileSync(join(dir, "sub", "world.txt"), "world");

    const files = await scanWorkspace(dir);
    expect(files).toHaveLength(2);

    const paths = files.map((f: any) => f.relativePath).sort();
    expect(paths).toEqual(["hello.txt", "sub/world.txt"]);

    const hello = files.find((f: any) => f.relativePath === "hello.txt");
    expect(hello.size).toBe(5);
    expect(hello.absolutePath).toContain("hello.txt");
    expect(hello.mode).toBeGreaterThan(0);
  });

  it("returns empty array for empty workspace", async () => {
    const dir = mkdtempSync(join(tmpdir(), "chkpt-scan-"));
    const files = await scanWorkspace(dir);
    expect(files).toHaveLength(0);
  });

  it("excludes .git directory", async () => {
    const dir = mkdtempSync(join(tmpdir(), "chkpt-scan-"));
    mkdirSync(join(dir, ".git"));
    writeFileSync(join(dir, ".git", "config"), "x");
    writeFileSync(join(dir, "file.txt"), "y");

    const files = await scanWorkspace(dir);
    expect(files).toHaveLength(1);
    expect(files[0].relativePath).toBe("file.txt");
  });
});
