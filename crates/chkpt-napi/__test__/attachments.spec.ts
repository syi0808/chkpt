import { describe, it, expect, beforeEach } from "vitest";
import { depsArchive, depsRestore, computeDepsKey } from "../index.js";
import {
  mkdtempSync,
  writeFileSync,
  mkdirSync,
  readFileSync,
  existsSync,
} from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

describe("deps attachment", () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = mkdtempSync(join(tmpdir(), "chkpt-deps-"));
  });

  it("computeDepsKey returns 16-char hex", () => {
    const lockfile = join(tmpDir, "package-lock.json");
    writeFileSync(lockfile, '{"lockfileVersion":3}');
    const key = computeDepsKey(lockfile);
    expect(key).toMatch(/^[0-9a-f]{16}$/);
  });

  it("computeDepsKey is deterministic", () => {
    const lockfile = join(tmpDir, "package-lock.json");
    writeFileSync(lockfile, '{"lockfileVersion":3}');
    expect(computeDepsKey(lockfile)).toBe(computeDepsKey(lockfile));
  });

  it("depsArchive + depsRestore roundtrip", async () => {
    const depsDir = join(tmpDir, "node_modules");
    mkdirSync(join(depsDir, "lodash"), { recursive: true });
    writeFileSync(join(depsDir, "lodash", "index.js"), "module.exports = {}");

    const lockfile = join(tmpDir, "package-lock.json");
    writeFileSync(lockfile, '{"lockfileVersion":3}');

    const archiveDir = join(tmpDir, "archive");
    mkdirSync(archiveDir);

    const key = computeDepsKey(lockfile);
    await depsArchive(depsDir, archiveDir, key);
    expect(existsSync(join(archiveDir, `${key}.tar.zst`))).toBe(true);

    const restoreDir = join(tmpDir, "restored_modules");
    await depsRestore(restoreDir, archiveDir, key);
    expect(readFileSync(join(restoreDir, "lodash", "index.js"), "utf-8")).toBe(
      "module.exports = {}",
    );
  });
});
