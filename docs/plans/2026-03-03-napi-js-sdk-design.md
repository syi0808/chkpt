# Design: N-API JS SDK + npm CLI Package

## Overview

chkpt의 Rust 코어를 Node.js에서 사용할 수 있도록 N-API 바인딩을 제공하고, npm을 통해 CLI 바이너리와 SDK를 단일 패키지로 배포한다.

## 결정 사항

| 항목           | 결정                                                                                                       |
| -------------- | ---------------------------------------------------------------------------------------------------------- |
| N-API 도구     | napi-rs                                                                                                    |
| 패키지명       | `chkpt`                                                                                                    |
| 패키지 구조    | 단일 npm 패키지 (SDK + CLI)                                                                                |
| CLI 실행       | Rust 바이너리 직접 실행 (Node.js 오버헤드 없음)                                                            |
| SDK API 스타일 | Async (Promise) 기반                                                                                       |
| API 범위       | 저수준 API까지 전부 노출                                                                                   |
| CI/CD          | 로컬 빌드만 (MVP), 향후 GitHub Actions                                                                     |
| 타겟 플랫폼    | darwin-arm64, darwin-x64, linux-arm64-gnu, linux-x64-gnu, linux-arm64-musl, linux-x64-musl, win32-x64-msvc |

## 프로젝트 구조

```
crates/chkpt-napi/
├── Cargo.toml                      # napi-rs 의존성, chkpt-core 참조
├── build.rs                        # napi-build
├── src/
│   ├── lib.rs                      # 모듈 선언 + napi 초기화
│   ├── ops.rs                      # 고수준: save, list, restore, delete
│   ├── store.rs                    # 저수준: blob, tree, snapshot, pack
│   ├── scanner.rs                  # 저수준: scan_workspace
│   ├── index.rs                    # 저수준: FileIndex
│   ├── attachments.rs              # 저수준: deps, git attachment
│   ├── config.rs                   # StoreLayout, ProjectConfig
│   └── error.rs                    # ChkptError → napi::Error 변환
├── package.json                    # "chkpt" npm 패키지 (main + bin)
├── index.js                        # napi-rs 자동 생성 (네이티브 로더)
├── index.d.ts                      # napi-rs 자동 생성 (TypeScript 타입)
├── cli.mjs                         # Rust 바이너리를 exec하는 thin launcher
├── __test__/
│   ├── ops.spec.ts                 # 고수준 API 통합 테스트
│   ├── store.spec.ts               # 저수준 store API 테스트
│   ├── scanner.spec.ts             # scanner 테스트
│   ├── cli.spec.ts                 # CLI 바이너리 실행 테스트
│   └── e2e.spec.ts                 # 전체 라이프사이클 테스트
└── npm/                            # 플랫폼별 optional dependency 패키지
    ├── darwin-arm64/
    │   ├── package.json
    │   ├── chkpt.darwin-arm64.node  # N-API 모듈
    │   └── chkpt                    # Rust CLI 바이너리
    ├── darwin-x64/
    ├── linux-arm64-gnu/
    ├── linux-x64-gnu/
    ├── linux-arm64-musl/
    ├── linux-x64-musl/
    └── win32-x64-msvc/
```

## 실행 경로

### CLI (최소 오버헤드)

```
npx chkpt save -m "msg"
  → cli.mjs
  → execFileSync(platform_rust_binary, args)
  → Rust chkpt-cli 네이티브 실행
```

Node.js 런타임 부팅 후 즉시 Rust 바이너리로 위임. 실질 오버헤드 ~1-5ms.

### SDK (프로그래매틱)

```js
import { save, blobHash, scanWorkspace } from 'chkpt'
  → index.js (napi-rs 네이티브 로더)
  → .node 파일 로드
  → Rust N-API → chkpt-core 직접 호출
```

## Rust 바인딩 (chkpt-napi crate)

### Cargo.toml

```toml
[package]
name = "chkpt-napi"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
chkpt-core = { path = "../chkpt-core" }
napi = { version = "2", features = ["async", "serde-json", "napi9"] }
napi-derive = "2"
serde_json = "1"

[build-dependencies]
napi-build = "2"
```

### 바인딩 패턴

```rust
// #[napi] 매크로로 JS에 함수 노출
// napi-rs가 자동으로 index.d.ts 생성

#[napi(object)]
pub struct JsSaveResult {
    pub snapshot_id: String,
    pub total_files: u32,
    pub new_objects: u32,
    pub total_bytes: i64,
}

#[napi]
pub async fn save(workspace_path: String, message: Option<String>) -> napi::Result<JsSaveResult> {
    let result = chkpt_core::ops::save::save(&workspace_path, message.as_deref())
        .map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))?;
    Ok(result.into())
}
```

## JS SDK API

### 고수준 API (Operations)

```typescript
// chkpt-core ops 모듈 대응
export function save(
  workspacePath: string,
  message?: string,
): Promise<SaveResult>;
export function list(
  workspacePath: string,
  limit?: number,
): Promise<Snapshot[]>;
export function restore(
  workspacePath: string,
  snapshotId: string,
  dryRun?: boolean,
): Promise<RestoreResult>;
export function deleteSnapshot(
  workspacePath: string,
  snapshotId: string,
): Promise<void>;
```

### 저수준 API (Store)

```typescript
// Blob
export function blobHash(content: Buffer): string;
export function blobStore(
  storePath: string,
  hash: string,
  content: Buffer,
): Promise<void>;
export function blobLoad(storePath: string, hash: string): Promise<Buffer>;
export function blobExists(storePath: string, hash: string): boolean;

// Tree
export function treeBuild(entries: TreeEntry[]): Promise<TreeBuildResult>;
export function treeLoad(storePath: string, hash: string): Promise<TreeEntry[]>;

// Snapshot
export function snapshotSave(
  storePath: string,
  snapshot: SnapshotData,
): Promise<void>;
export function snapshotLoad(
  storePath: string,
  snapshotId: string,
): Promise<SnapshotData>;
export function snapshotList(storePath: string): Promise<SnapshotData[]>;
```

### 저수준 API (Scanner, Index, Config, Attachments)

```typescript
// Scanner
export function scanWorkspace(workspacePath: string): Promise<ScannedFile[]>;

// Index
export function indexOpen(storePath: string): Promise<FileIndex>;
export function indexLookup(
  index: FileIndex,
  path: string,
): Promise<IndexEntry | null>;
export function indexUpsert(
  index: FileIndex,
  entries: IndexEntry[],
): Promise<void>;

// Config
export function getStoreLayout(workspacePath: string): StoreLayout;
export function getProjectId(workspacePath: string): string;

// Attachments
export function depsArchive(
  storePath: string,
  depsDir: string,
  lockfilePath: string,
): Promise<string>;
export function depsRestore(
  storePath: string,
  depsKey: string,
  targetDir: string,
): Promise<void>;
export function gitBundleCreate(
  storePath: string,
  repoPath: string,
): Promise<string>;
export function gitBundleRestore(
  storePath: string,
  gitKey: string,
  targetPath: string,
): Promise<void>;
```

### TypeScript 타입

```typescript
interface SaveResult {
  snapshotId: string;
  totalFiles: number;
  newObjects: number;
  totalBytes: number;
}

interface RestoreResult {
  snapshotId: string;
  filesAdded: number;
  filesChanged: number;
  filesRemoved: number;
  filesUnchanged: number;
}

interface Snapshot {
  id: string;
  createdAt: string;
  message?: string;
  rootTreeHash: string;
  parentSnapshotId?: string;
  stats: { totalFiles: number; totalBytes: number };
}

interface TreeEntry {
  name: string;
  entryType: "file" | "directory" | "symlink";
  hash: string;
  size: number;
  mode: number;
}

interface ScannedFile {
  relativePath: string;
  absolutePath: string;
  size: number;
  mtimeSecs: number;
  mtimeNanos: number;
  inode?: number;
  mode: number;
}

interface StoreLayout {
  root: string;
  objectsDir: string;
  treesDir: string;
  snapshotsDir: string;
  indexPath: string;
  locksDir: string;
  attachmentsDir: string;
}
```

## npm 패키지 설정

### package.json (메인)

```json
{
  "name": "chkpt",
  "version": "0.1.0",
  "description": "Filesystem checkpoint engine - save, restore, and manage workspace snapshots",
  "main": "index.js",
  "types": "index.d.ts",
  "bin": {
    "chkpt": "./cli.mjs"
  },
  "napi": {
    "name": "chkpt",
    "triples": {
      "defaults": false,
      "additional": [
        "aarch64-apple-darwin",
        "x86_64-apple-darwin",
        "aarch64-unknown-linux-gnu",
        "x86_64-unknown-linux-gnu",
        "aarch64-unknown-linux-musl",
        "x86_64-unknown-linux-musl",
        "x86_64-pc-windows-msvc"
      ]
    }
  },
  "engines": {
    "node": ">= 18"
  },
  "files": ["index.js", "index.d.ts", "cli.mjs"]
}
```

### CLI launcher (cli.mjs)

```js
#!/usr/bin/env node
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { join, dirname } from "node:path";

// 플랫폼별 바이너리 경로 resolve
function getBinaryPath() {
  const platform = process.platform;
  const arch = process.arch;
  const platformMap = {
    "darwin-arm64": "@chkpt/cli-darwin-arm64",
    "darwin-x64": "@chkpt/cli-darwin-x64",
    "linux-arm64": "@chkpt/cli-linux-arm64-gnu",
    "linux-x64": "@chkpt/cli-linux-x64-gnu",
    "win32-x64": "@chkpt/cli-win32-x64-msvc",
  };
  const key = `${platform}-${arch}`;
  const pkg = platformMap[key];
  if (!pkg) throw new Error(`Unsupported platform: ${key}`);
  return join(dirname(fileURLToPath(import.meta.resolve(pkg))), "chkpt");
}

try {
  execFileSync(getBinaryPath(), process.argv.slice(2), { stdio: "inherit" });
} catch (e) {
  process.exit(e.status ?? 1);
}
```

### 에러 핸들링

```rust
// Rust ChkptError → JS Error 변환
impl From<ChkptError> for napi::Error {
    fn from(err: ChkptError) -> Self {
        napi::Error::new(napi::Status::GenericFailure, err.to_string())
    }
}
```

## 테스트 전략

1. **Rust 단위 테스트**: chkpt-napi crate 내 바인딩 로직
2. **JS 통합 테스트 (vitest)**:
   - 고수준 API: save → list → restore → delete 라이프사이클
   - 저수준 API: blob, tree, scanner, index 개별 검증
   - CLI: child_process로 Rust 바이너리 실행 테스트
3. **격리**: os.tmpdir()로 임시 디렉토리 사용

## 빌드 (MVP, 로컬)

```bash
cd crates/chkpt-napi
npm run build        # → napi build --release
npm test             # → vitest run
```

## 향후 확장

- GitHub Actions CI/CD: napi-rs 공식 CI 템플릿으로 플랫폼별 크로스컴파일 + npm publish
- AVX2 변형: BLAKE3가 런타임 CPU feature 자동 감지하므로 별도 빌드 불필요할 수 있음, 벤치마크 후 결정
