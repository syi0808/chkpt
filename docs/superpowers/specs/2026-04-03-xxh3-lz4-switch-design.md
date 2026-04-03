# Design: Switch Hash & Compression to XXH3 + LZ4

**Date**: 2026-04-03
**Status**: Approved

## Motivation

chkpt의 해싱(BLAKE3)과 압축(zstd) 단계는 save 파이프라인에서 상당한 비중을 차지한다. XXH3와 LZ4는 각각 비암호학적 해시와 경량 압축으로, 순수 throughput에서 우위가 예상된다. chkpt는 로컬 전용 도구이므로 암호학적 충돌 저항성은 불필요하다.

## Constraints

- 하위호환성 불필요 — 기존 스냅샷은 무효화됨
- 비암호학적 해시 허용 — 로컬 도구, 악의적 공격 시나리오 무시
- 벤치마크 기반 의사결정 — 유의미한 개선(>10%)이 없으면 해당 전환 보류

## Approach: C → A (Benchmark-Driven Full Switch)

### Phase C: 독립 벤치마크 비교

기존 코드를 변경하지 않고, 벤치마크 크레이트에서 XXH3/LZ4를 직접 비교한다.

**측정 대상:**

| 비교 | 항목 |
|------|------|
| BLAKE3 vs XXH3-128 | 해싱 throughput (MB/s), 소형(<1KB) / 중형(1-256KB) / 대형(≥256KB) 파일별 |
| zstd(1) vs LZ4 | 압축 throughput, 압축률, 소스코드 워크로드 |
| zstd(1) vs LZ4 | 해제 throughput |
| 파이프라인 전체 | read→hash→compress 통합 시간, 기존 벤치마크와 동일 워크로드 |

**크레이트:**
- `xxhash-rust` (feature `xxh3`) — 순수 Rust XXH3 구현
- `lz4_flex` — 순수 Rust LZ4 (frame format)

**판단 기준:**
- 해싱 또는 압축 중 하나라도 >10% 개선이 없으면 해당 전환 보류
- 압축률이 zstd 대비 >30% 떨어지면 트레이드오프 재평가

### Phase A: 전면 교체

벤치마크에서 유의미한 개선이 확인된 항목에 대해 전면 전환한다.

#### 해시 변경 (BLAKE3 → XXH3-128)

| 영역 | 변경 내용 |
|------|----------|
| 타입 | `[u8; 32]` → `[u8; 16]` 전체 코드베이스 |
| hex 표현 | 64자 → 32자 |
| 팩 네이밍 | `pack-{hash}.dat` — 앞 16자 short form 유지 |
| SQLite | BLOB 컬럼 32B → 16B (자동 적용) |
| ShardedSeenHashes | `HashSet<[u8; 32]>` → `HashSet<[u8; 16]>` — 메모리 절반 |
| mmap 해싱 | `blake3::Hasher` → `xxh3_128` |
| 트리 해시 | `blake3::hash(&encoded)` → `xxh3_128(&encoded)` |

#### 압축 변경 (zstd → LZ4)

| 영역 | 변경 내용 |
|------|----------|
| 압축 | `zstd::encode_all(content, 1)` → `lz4_flex::compress_prepend_size(content)` |
| 해제 | `zstd::stream::copy_decode()` → `lz4_flex::decompress_size_prepended()` |
| 스킵 로직 | 유지 — 미디어/아카이브는 무압축 저장 |
| bulk Compressor | zstd stateful compressor → lz4_flex stateless 호출로 단순화 |

#### 팩 포맷 변경

```
기존: CHKP | VERSION | COUNT | [hash(32B) | comp_len(8B) | zstd_data]*
신규: CHKL | COUNT | [hash(16B) | comp_len(8B) | lz4_data]*
```

- 매직 바이트 `CHKP` → `CHKL`로 변경
- 트리 팩도 동일 적용: `CKTR` → `CKTL`, hash 32B → 16B, zstd → LZ4
- VERSION 필드 제거
- 기존 스냅샷은 무효화

#### 영향받는 파일

- `store/blob.rs` — 해시 함수, hex 변환, 타입
- `store/pack.rs` — 압축/해제, 팩 포맷, PackWriter
- `store/tree.rs` — 트리 해시
- `store/catalog.rs` — BLOB 크기 (자동 적용)
- `ops/save.rs` — 파이프라인 전체 (해시+압축)
- `ops/restore.rs` — 해제
- `ops/delete.rs` — 해시 참조
- `config.rs` — 프로젝트 ID 해시
- `index.rs` — FileEntry 해시 타입
- 벤치마크, 테스트 전부

## Testing

- 기존 테스트 스위트 전체 통과 (해시/압축 타입 변경 반영)
- save→restore round-trip 무결성 검증
- 벤치마크 결과를 `docs/benchmarks/`에 기록
