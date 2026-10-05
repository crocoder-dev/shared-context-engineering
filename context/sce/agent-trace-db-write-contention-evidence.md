# Agent Trace DB write-contention evidence

This doc records the measured behavior of the Agent Trace DB contention contract and the test suite that measures it. The contract is: Turso `busy_timeout`, then at most one jittered outer retry, cut off by `contention_deadline_ms`. The contract itself is defined in [shared-turso-db.md](shared-turso-db.md). The repository adapter is described in [agent-trace-db.md](agent-trace-db.md).

This doc keeps **Contract** (what tests assert) apart from **Observed on reference host** (what one campaign measured on one machine). Nothing under "Observed" is a guarantee.

This is the canonical detailed measurement source. Plans and other context docs summarize it and link here.

## Policy history

| Policy (`busy_timeout_ms` / `contention_deadline_ms` / max attempts / backoff cap) | Status | Evidence |
| --- | --- | --- |
| 1000 / 2250 / 2 / 100 ms | **current defaults** | [Current campaign](#current-campaign-1000--2250--2--100) |
| 500 / 1250 / 2 / 100 ms | superseded | [Supported-load failure](#supported-load-failure-under-the-500--1250-policy) and [historical campaign](#historical-campaign-500--1250--2--100) |

Why the defaults were tuned:

- Under 500 / 1250, a final validation run of the strict N=2–4 suite failed. A supported-load writer held the Agent Trace writer lock for about 1.6–1.7 s, and the waiting writers exhausted the policy after about 1.0–1.1 s. One distinct event was lost.
- The 500 / 1250 policy cannot cover a lock hold above about 1.05 s. Its held-lock transition sat between a holder release of 1026 ms (succeeded) and 1503 ms (exhausted).
- 1000 / 2250 keeps the same two-attempt architecture and raises only the time budget. It was measured first; a third attempt was not tried, because this candidate passed.
- Only the default time budget changed. Retry classification (typed `Busy`/`BusySnapshot`), whole-transaction retry units, the 2-attempt cap, the full-jitter algorithm, the post-backoff admission re-check, deterministic-error behavior, hook fail-open behavior, the metadata read-only fast path and exhaustion observability are unchanged.

## Contractual assertions (current)

These are the only assertions the suite makes.

- Held-lock boundary (`lock_budget_boundary_characterizes_single_writer_blocked_by_begin_immediate_holder`), holds of 100, 250, 500, 750, 1000, 1500, 1750, 2000, 2250, 2500 and 3000 ms:
  - holds ≤ 1000 ms must succeed with 0 exhaustions;
  - holds ≥ 3000 ms must fail with the `under write contention` error and exactly 1 exhaustion;
  - every failure leaves 0 message and 0 part rows;
  - every sample makes 1–2 attempts, and `outer_retries + 1 == attempts`.
  - 1500–2500 ms is characterization only.
  - Margin basis: an exhausting writer cannot give up before about 2000 ms (two 1000 ms busy waits), so a ≤ 1000 ms hold has about 1000 ms of slack. The latest exhaustion observed in 110 samples was 2094 ms, so a ≥ 3000 ms hold has about 900 ms of slack. The earlier contract (≤ 250 ms succeeds, ≥ 2000 ms exhausts) used comparable slack against the 500 / 1250 policy.
- Every in-process round:
  - `messages == parts`;
  - the `Ok(true)` count equals the persisted rows;
  - no writer exceeds 2 attempts;
  - every duplicate round inserts at most once.
- `SCE_LOCK_CONTENTION_STRICT=1` adds, per level:
  - 0 lock errors, 0 other errors, 0 lost events and 0 exhaustions;
  - for hook processes: every expected message and part persisted, and 0 non-zero exits.
- Strict supported levels (unchanged):

  | Suite | Levels (N × rounds) |
  | --- | --- |
  | distinct events | 2×1000, 3×1000, 4×500 |
  | duplicate delivery | 2×500, 3×500, 4×500 |
  | hook processes | 2×500, 3×500, 4×200 |

  N=8 is stress characterization, not a supported requirement.

## Current campaign: 1000 / 2250 / 2 / 100

### Environment and artifacts

- Same reference host as the historical campaign (see [Test environment](#test-environment)): bare-metal AMD Ryzen 9 5900X, 24 threads, 46 GiB RAM, Linux 6.18.37, `/tmp` on ext4 over LUKS dm-crypt on NVMe, Turso crate 0.8.1, rustc 1.95.0. The same io_uring PSI/iowait caveat applies.
- Campaign date 2026-10-05, Unix ms 1791206344424–1791209382718 (50.6 minutes of measured runs). Load average (1 min) before each run ranged 0.77–5.78.
- Commit: `HEAD` `e72116239fa8f05befe9fb8e1220c79100914490` plus the working-tree policy change. The `cli/` + `config/` diff at build time had sha256 `3f78fe790fb2e9d10e070e2ab903be256634e8461baf8a23e0515b1f74498c91`.
  - `AGENT_TRACE_DB_BUSY_TIMEOUT_MS` 500 → 1000 and `AGENT_TRACE_DB_CONTENTION_DEADLINE_MS` 1250 → 2250 in `cli/src/services/db/mod.rs`, with matching Pkl schema defaults.
  - The boundary hold set was extended to 100–3000 ms, with a provisional exhaustion threshold of 3000 ms and the old success threshold of 250 ms.
  - After the campaign, the success threshold was raised to 1000 ms from this data. That is a test-only change; the measured binaries predate it.
- The policy ran with defaults, with no `policies.database_retry.agent_trace_db` override.
- Frozen release artifacts, built once before the first run:
  - test binary sha256 `d257dda55923bec809953e84489c927e2a410fb24d15119c1419eca03f917cf1`;
  - `sce` sha256 `c90b428162f60ef374374d3dcc5272a72bec837b0f079c3d9c482e0e5e2f526e`.
- Methodology and run order are identical to the historical campaign ([Measurement methodology](#measurement-methodology)): A = 10 boundary runs, B = 5 strict matrices, C = 5 N=8 stress runs, D = 5 strict real-hook matrices. Every level ran as its own strict-mode process with `uptime`, `/proc/pressure/*`, `vmstat 1 5` and `iostat -xz 1 5` captured before it, and the external host monitor ran for the whole campaign.
- 65 runs, all exit 0. No post-failure snapshot was needed. Nothing was rerun, replaced or dropped. The raw `SCE_MEAS` logs, host snapshots and monitor JSONL were kept in the session scratchpad and are not committed.

### Held-lock results

Contract: ≤ 1000 ms succeeds, ≥ 3000 ms exhausts, failure leaves 0/0 rows, ≤ 2 attempts. All 10 runs satisfied it: 110/110 samples, 0 invariant violations.

Observed on reference host (10 runs × 11 holds):

- **The success/exhaust transition lies between 2000 and 2250 ms of requested hold.** Actual holder release was 2004–2013 ms for the 2000 ms hold (10/10 succeeded) versus 2254–2265 ms for the 2250 ms hold (10/10 exhausted).
- **Exhausted writers gave up at 2001–2094 ms**: a `1000.1–1000.2 ms` busy wait, the drawn backoff (≤ 0.1 ms oversleep), and a second `1000.1–1000.2 ms` busy wait. No admission rejection occurred; every exhaustion made both attempts.
- **Holds of 1500–2000 ms succeeded on attempt 2** (30/30). This covers the 1.6–1.7 s holder that failed the 500 / 1250 validation run. The 2000 ms hold is borderline: it succeeded because the second busy wait ended a few ms after the release, and a smaller backoff draw could tip it to exhaustion. It is characterization only.
- **Holds up to 1000 ms succeeded on attempt 1** (50/50), including 750 ms holds that needed an outer retry under 500 / 1250.
- One 2500 ms sample released at 2977 ms, a delayed holder release. It exhausted at about 2.0 s as specified.

#### Held-lock raw samples

| run | hold ms | holder released ms | writer elapsed ms | result | attempts | outer retries | exhaustions | messages | parts | attempt1 ms | backoff req/actual ms | attempt2 ms |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 100 | 104.1 | 115.9 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 115.9 | - | - |
| 1 | 250 | 254.3 | 341.8 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 341.8 | - | - |
| 1 | 500 | 512.1 | 541.8 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 541.8 | - | - |
| 1 | 750 | 754.4 | 841.4 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 841.4 | - | - |
| 1 | 1000 | 1004.5 | 1018.0 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 1018.0 | - | - |
| 1 | 1500 | 1504.5 | 1583.6 | Ok(true) | 2 | 1 | 0 | 1 | 1 | 1000.2 | 34/34.1 | 549.3 |
| 1 | 1750 | 1762.5 | 1862.5 | Ok(true) | 2 | 1 | 0 | 1 | 1 | 1000.2 | 14/14.1 | 848.3 |
| 1 | 2000 | 2004.4 | 2085.2 | Ok(true) | 2 | 1 | 0 | 1 | 1 | 1000.2 | 81/81.1 | 1004.0 |
| 1 | 2250 | 2254.4 | 2056.5 | database is locked | 2 | 1 | 1 | 0 | 0 | 1000.2 | 56/56.1 | 1000.2 |
| 1 | 2500 | 2976.9 | 2015.5 | database is locked | 2 | 1 | 1 | 0 | 0 | 1000.2 | 15/15.1 | 1000.2 |
| 1 | 3000 | 3004.4 | 2008.5 | database is locked | 2 | 1 | 1 | 0 | 0 | 1000.2 | 8/8.1 | 1000.2 |
| 2 | 100 | 103.9 | 115.6 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 115.5 | - | - |
| 2 | 250 | 262.2 | 340.7 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 340.7 | - | - |
| 2 | 500 | 515.5 | 540.8 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 540.8 | - | - |
| 2 | 750 | 762.6 | 840.7 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 840.7 | - | - |
| 2 | 1000 | 1012.7 | 1023.3 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 1023.3 | - | - |
| 2 | 1500 | 1512.7 | 1562.1 | Ok(true) | 2 | 1 | 0 | 1 | 1 | 1000.2 | 20/20.1 | 541.8 |
| 2 | 1750 | 1762.6 | 1848.5 | Ok(true) | 2 | 1 | 0 | 1 | 1 | 1000.2 | 7/7.1 | 841.3 |
| 2 | 2000 | 2012.5 | 2052.4 | Ok(true) | 2 | 1 | 0 | 1 | 1 | 1000.1 | 39/39.1 | 1013.2 |
| 2 | 2250 | 2262.6 | 2042.5 | database is locked | 2 | 1 | 1 | 0 | 0 | 1000.2 | 42/42.1 | 1000.2 |
| 2 | 2500 | 2509.8 | 2003.5 | database is locked | 2 | 1 | 1 | 0 | 0 | 1000.2 | 3/3.1 | 1000.2 |
| 2 | 3000 | 3005.3 | 2014.5 | database is locked | 2 | 1 | 1 | 0 | 0 | 1000.2 | 14/14.1 | 1000.2 |
| 3 | 100 | 104.2 | 116.7 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 116.7 | - | - |
| 3 | 250 | 253.9 | 341.7 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 341.7 | - | - |
| 3 | 500 | 504.3 | 541.2 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 541.2 | - | - |
| 3 | 750 | 754.4 | 842.3 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 842.3 | - | - |
| 3 | 1000 | 1004.4 | 1038.4 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 1038.4 | - | - |
| 3 | 1500 | 1504.6 | 1523.4 | Ok(true) | 2 | 1 | 0 | 1 | 1 | 1000.2 | 75/75.1 | 448.1 |
| 3 | 1750 | 1754.7 | 1771.6 | Ok(true) | 2 | 1 | 0 | 1 | 1 | 1000.1 | 26/26.1 | 745.4 |
| 3 | 2000 | 2004.2 | 2065.6 | Ok(true) | 2 | 1 | 0 | 1 | 1 | 1000.2 | 53/53.0 | 1012.4 |
| 3 | 2250 | 2254.3 | 2070.4 | database is locked | 2 | 1 | 1 | 0 | 0 | 1000.2 | 70/70.1 | 1000.2 |
| 3 | 2500 | 2513.1 | 2010.5 | database is locked | 2 | 1 | 1 | 0 | 0 | 1000.2 | 10/10.1 | 1000.2 |
| 3 | 3000 | 3012.4 | 2008.5 | database is locked | 2 | 1 | 1 | 0 | 0 | 1000.2 | 8/8.1 | 1000.2 |
| 4 | 100 | 103.9 | 115.2 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 115.1 | - | - |
| 4 | 250 | 269.8 | 341.5 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 341.5 | - | - |
| 4 | 500 | 512.0 | 540.1 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 540.1 | - | - |
| 4 | 750 | 762.5 | 841.9 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 841.9 | - | - |
| 4 | 1000 | 1012.5 | 1020.4 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 1020.4 | - | - |
| 4 | 1500 | 1512.2 | 1550.8 | Ok(true) | 2 | 1 | 0 | 1 | 1 | 1000.2 | 9/9.1 | 541.6 |
| 4 | 1750 | 1765.8 | 1844.6 | Ok(true) | 2 | 1 | 0 | 1 | 1 | 1000.2 | 3/3.1 | 841.4 |
| 4 | 2000 | 2004.1 | 2017.2 | Ok(true) | 2 | 1 | 0 | 1 | 1 | 1000.1 | 78/78.1 | 939.0 |
| 4 | 2250 | 2256.5 | 2043.5 | database is locked | 2 | 1 | 1 | 0 | 0 | 1000.2 | 43/43.1 | 1000.2 |
| 4 | 2500 | 2504.1 | 2030.4 | database is locked | 2 | 1 | 1 | 0 | 0 | 1000.2 | 30/30.1 | 1000.2 |
| 4 | 3000 | 3008.3 | 2086.5 | database is locked | 2 | 1 | 1 | 0 | 0 | 1000.2 | 86/86.0 | 1000.2 |
| 5 | 100 | 104.2 | 134.6 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 134.5 | - | - |
| 5 | 250 | 258.0 | 346.7 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 346.7 | - | - |
| 5 | 500 | 505.3 | 547.5 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 547.5 | - | - |
| 5 | 750 | 754.3 | 847.0 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 847.0 | - | - |
| 5 | 1000 | 1004.4 | 1022.1 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 1022.1 | - | - |
| 5 | 1500 | 1511.9 | 1590.6 | Ok(true) | 2 | 1 | 0 | 1 | 1 | 1000.2 | 41/41.1 | 549.4 |
| 5 | 1750 | 1754.1 | 1828.7 | Ok(true) | 2 | 1 | 0 | 1 | 1 | 1000.2 | 73/73.1 | 755.5 |
| 5 | 2000 | 2004.3 | 2059.4 | Ok(true) | 2 | 1 | 0 | 1 | 1 | 1000.2 | 41/41.1 | 1018.2 |
| 5 | 2250 | 2264.9 | 2073.5 | database is locked | 2 | 1 | 1 | 0 | 0 | 1000.2 | 73/73.1 | 1000.3 |
| 5 | 2500 | 2512.4 | 2053.6 | database is locked | 2 | 1 | 1 | 0 | 0 | 1000.2 | 53/53.1 | 1000.3 |
| 5 | 3000 | 3012.5 | 2038.5 | database is locked | 2 | 1 | 1 | 0 | 0 | 1000.2 | 38/38.1 | 1000.2 |
| 6 | 100 | 104.0 | 115.9 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 115.9 | - | - |
| 6 | 250 | 270.3 | 341.6 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 341.6 | - | - |
| 6 | 500 | 514.9 | 540.6 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 540.5 | - | - |
| 6 | 750 | 762.2 | 841.4 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 841.4 | - | - |
| 6 | 1000 | 1012.5 | 1020.6 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 1020.6 | - | - |
| 6 | 1500 | 1515.1 | 1523.3 | Ok(true) | 2 | 1 | 0 | 1 | 1 | 1000.2 | 79/79.1 | 444.1 |
| 6 | 1750 | 1762.5 | 1779.5 | Ok(true) | 2 | 1 | 0 | 1 | 1 | 1000.2 | 38/38.1 | 741.3 |
| 6 | 2000 | 2012.2 | 2060.9 | Ok(true) | 2 | 1 | 0 | 1 | 1 | 1000.2 | 44/44.1 | 1016.7 |
| 6 | 2250 | 2253.9 | 2036.5 | database is locked | 2 | 1 | 1 | 0 | 0 | 1000.2 | 36/36.1 | 1000.2 |
| 6 | 2500 | 2509.1 | 2054.4 | database is locked | 2 | 1 | 1 | 0 | 0 | 1000.2 | 54/54.1 | 1000.2 |
| 6 | 3000 | 3013.8 | 2053.5 | database is locked | 2 | 1 | 1 | 0 | 0 | 1000.2 | 53/53.1 | 1000.2 |
| 7 | 100 | 104.3 | 116.3 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 116.3 | - | - |
| 7 | 250 | 262.4 | 340.6 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 340.6 | - | - |
| 7 | 500 | 512.1 | 540.9 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 540.9 | - | - |
| 7 | 750 | 761.9 | 840.2 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 840.2 | - | - |
| 7 | 1000 | 1004.5 | 1016.2 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 1016.2 | - | - |
| 7 | 1500 | 1504.5 | 1568.8 | Ok(true) | 2 | 1 | 0 | 1 | 1 | 1000.2 | 27/27.1 | 541.6 |
| 7 | 1750 | 1754.6 | 1797.5 | Ok(true) | 2 | 1 | 0 | 1 | 1 | 1000.2 | 57/57.1 | 740.3 |
| 7 | 2000 | 2004.2 | 2024.0 | Ok(true) | 2 | 1 | 0 | 1 | 1 | 1000.2 | 83/83.1 | 940.7 |
| 7 | 2250 | 2254.2 | 2027.5 | database is locked | 2 | 1 | 1 | 0 | 0 | 1000.2 | 27/27.1 | 1000.3 |
| 7 | 2500 | 2513.5 | 2020.7 | database is locked | 2 | 1 | 1 | 0 | 0 | 1000.3 | 20/20.1 | 1000.3 |
| 7 | 3000 | 3004.3 | 2086.6 | database is locked | 2 | 1 | 1 | 0 | 0 | 1000.2 | 86/86.1 | 1000.3 |
| 8 | 100 | 104.0 | 115.9 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 115.8 | - | - |
| 8 | 250 | 271.1 | 340.6 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 340.6 | - | - |
| 8 | 500 | 512.3 | 540.2 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 540.2 | - | - |
| 8 | 750 | 762.1 | 840.3 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 840.3 | - | - |
| 8 | 1000 | 1019.0 | 1027.6 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 1027.6 | - | - |
| 8 | 1500 | 1512.0 | 1570.4 | Ok(true) | 2 | 1 | 0 | 1 | 1 | 1000.2 | 30/30.1 | 540.2 |
| 8 | 1750 | 1763.4 | 1775.0 | Ok(true) | 2 | 1 | 0 | 1 | 1 | 1000.2 | 31/31.1 | 743.8 |
| 8 | 2000 | 2012.4 | 2023.7 | Ok(true) | 2 | 1 | 0 | 1 | 1 | 1000.2 | 83/83.1 | 940.5 |
| 8 | 2250 | 2262.2 | 2094.5 | database is locked | 2 | 1 | 1 | 0 | 0 | 1000.2 | 94/94.0 | 1000.2 |
| 8 | 2500 | 2512.9 | 2076.5 | database is locked | 2 | 1 | 1 | 0 | 0 | 1000.2 | 76/76.1 | 1000.2 |
| 8 | 3000 | 3013.0 | 2018.5 | database is locked | 2 | 1 | 1 | 0 | 0 | 1000.2 | 18/18.1 | 1000.2 |
| 9 | 100 | 103.8 | 115.0 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 115.0 | - | - |
| 9 | 250 | 254.3 | 340.9 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 340.9 | - | - |
| 9 | 500 | 512.3 | 540.6 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 540.6 | - | - |
| 9 | 750 | 762.4 | 841.1 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 841.1 | - | - |
| 9 | 1000 | 1012.5 | 1024.4 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 1024.3 | - | - |
| 9 | 1500 | 1512.6 | 1593.6 | Ok(true) | 2 | 1 | 0 | 1 | 1 | 1000.2 | 52/52.1 | 541.4 |
| 9 | 1750 | 1762.4 | 1805.6 | Ok(true) | 2 | 1 | 0 | 1 | 1 | 1000.2 | 63/63.1 | 742.4 |
| 9 | 2000 | 2012.7 | 2021.5 | Ok(true) | 2 | 1 | 0 | 1 | 1 | 1000.2 | 5/5.1 | 1016.2 |
| 9 | 2250 | 2262.0 | 2055.4 | database is locked | 2 | 1 | 1 | 0 | 0 | 1000.2 | 55/55.1 | 1000.2 |
| 9 | 2500 | 2505.0 | 2001.4 | database is locked | 2 | 1 | 1 | 0 | 0 | 1000.2 | 1/1.0 | 1000.2 |
| 9 | 3000 | 3012.8 | 2073.5 | database is locked | 2 | 1 | 1 | 0 | 0 | 1000.2 | 73/73.1 | 1000.2 |
| 10 | 100 | 104.2 | 116.5 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 116.5 | - | - |
| 10 | 250 | 262.4 | 341.8 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 341.8 | - | - |
| 10 | 500 | 512.1 | 541.2 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 541.2 | - | - |
| 10 | 750 | 767.0 | 841.1 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 841.1 | - | - |
| 10 | 1000 | 1012.3 | 1020.2 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 1020.2 | - | - |
| 10 | 1500 | 1512.0 | 1557.5 | Ok(true) | 2 | 1 | 0 | 1 | 1 | 1000.2 | 17/17.1 | 540.3 |
| 10 | 1750 | 1762.1 | 1775.0 | Ok(true) | 2 | 1 | 0 | 1 | 1 | 1000.2 | 34/34.1 | 740.8 |
| 10 | 2000 | 2012.6 | 2027.2 | Ok(true) | 2 | 1 | 0 | 1 | 1 | 1000.2 | 11/11.1 | 1016.0 |
| 10 | 2250 | 2262.1 | 2067.4 | database is locked | 2 | 1 | 1 | 0 | 0 | 1000.2 | 67/67.1 | 1000.2 |
| 10 | 2500 | 2512.3 | 2030.4 | database is locked | 2 | 1 | 1 | 0 | 0 | 1000.2 | 30/30.1 | 1000.2 |
| 10 | 3000 | 3016.1 | 2039.5 | database is locked | 2 | 1 | 1 | 0 | 0 | 1000.2 | 39/39.1 | 1000.2 |

#### Held-lock per-hold aggregate (10 runs)

| hold ms | success | exhausted | attempt-1 success | attempt-2 success | writer p50 ms | writer p95 ms | writer max ms | holder release p50 ms | holder release max ms |
|---|---|---|---|---|---|---|---|---|---|
| 100 | 10/10 | 0/10 | 10 | 0 | 115.9 | 134.6 | 134.6 | 104.0 | 104.3 |
| 250 | 10/10 | 0/10 | 10 | 0 | 341.5 | 346.7 | 346.7 | 262.2 | 271.1 |
| 500 | 10/10 | 0/10 | 10 | 0 | 540.8 | 547.5 | 547.5 | 512.1 | 515.5 |
| 750 | 10/10 | 0/10 | 10 | 0 | 841.1 | 847.0 | 847.0 | 762.1 | 767.0 |
| 1000 | 10/10 | 0/10 | 10 | 0 | 1020.6 | 1038.4 | 1038.4 | 1012.3 | 1019.0 |
| 1500 | 10/10 | 0/10 | 0 | 10 | 1562.1 | 1593.6 | 1593.6 | 1512.0 | 1515.1 |
| 1750 | 10/10 | 0/10 | 0 | 10 | 1797.5 | 1862.5 | 1862.5 | 1762.4 | 1765.8 |
| 2000 | 10/10 | 0/10 | 0 | 10 | 2027.2 | 2085.2 | 2085.2 | 2004.4 | 2012.7 |
| 2250 | 0/10 | 10/10 | 0 | 0 | 2055.4 | 2094.5 | 2094.5 | 2256.5 | 2264.9 |
| 2500 | 0/10 | 10/10 | 0 | 0 | 2020.7 | 2076.5 | 2076.5 | 2512.3 | 2976.9 |
| 3000 | 0/10 | 10/10 | 0 | 0 | 2038.5 | 2086.6 | 2086.6 | 3012.4 | 3016.1 |

### Strict N=2–4 results

| run | mode | N | rounds | total writes | expected rows | persisted rows | lost | lock errors | other errors | orphan rows | duplicate rows | attempts | outer retries | exhaustions | p50 ms | p95 ms | p99 ms | max ms | ops ≥500 ms | in-proc stall gaps ≥50 ms (max) | exit |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | distinct | 2 | 1000 | 2000 | 2000 | 2000 | 0 | 0 | 0 | 0 | 0 | 2000 | 0 | 0 | 30.1 | 93.0 | 128.9 | 180.2 | 0 | 0 (0) | 0 |
| 2 | distinct | 2 | 1000 | 2000 | 2000 | 2000 | 0 | 0 | 0 | 0 | 0 | 2000 | 0 | 0 | 29.1 | 61.6 | 110.7 | 254.7 | 0 | 0 (0) | 0 |
| 3 | distinct | 2 | 1000 | 2000 | 2000 | 2000 | 0 | 0 | 0 | 0 | 0 | 2000 | 0 | 0 | 29.6 | 63.6 | 101.8 | 241.6 | 0 | 0 (0) | 0 |
| 4 | distinct | 2 | 1000 | 2000 | 2000 | 2000 | 0 | 0 | 0 | 0 | 0 | 2000 | 0 | 0 | 21.7 | 60.7 | 113.5 | 189.1 | 0 | 0 (0) | 0 |
| 5 | distinct | 2 | 1000 | 2000 | 2000 | 2000 | 0 | 0 | 0 | 0 | 0 | 2000 | 0 | 0 | 30.1 | 92.3 | 135.3 | 292.9 | 0 | 0 (0) | 0 |
| 1 | distinct | 3 | 1000 | 3000 | 3000 | 3000 | 0 | 0 | 0 | 0 | 0 | 3000 | 0 | 0 | 50.2 | 126.8 | 201.8 | 303.7 | 0 | 0 (0) | 0 |
| 2 | distinct | 3 | 1000 | 3000 | 3000 | 3000 | 0 | 0 | 0 | 0 | 0 | 3000 | 0 | 0 | 36.1 | 111.1 | 171.3 | 292.8 | 0 | 0 (0) | 0 |
| 3 | distinct | 3 | 1000 | 3000 | 3000 | 3000 | 0 | 0 | 0 | 0 | 0 | 3000 | 0 | 0 | 41.8 | 102.6 | 147.6 | 212.7 | 0 | 0 (0) | 0 |
| 4 | distinct | 3 | 1000 | 3000 | 3000 | 3000 | 0 | 0 | 0 | 0 | 0 | 3000 | 0 | 0 | 31.4 | 67.2 | 134.2 | 236.2 | 0 | 0 (0) | 0 |
| 5 | distinct | 3 | 1000 | 3000 | 3000 | 3000 | 0 | 0 | 0 | 0 | 0 | 3000 | 0 | 0 | 34.4 | 106.2 | 148.6 | 230.1 | 0 | 0 (0) | 0 |
| 1 | distinct | 4 | 500 | 2000 | 2000 | 2000 | 0 | 0 | 0 | 0 | 0 | 2000 | 0 | 0 | 45.2 | 87.4 | 132.3 | 250.8 | 0 | 0 (0) | 0 |
| 2 | distinct | 4 | 500 | 2000 | 2000 | 2000 | 0 | 0 | 0 | 0 | 0 | 2000 | 0 | 0 | 31.3 | 65.8 | 98.9 | 221.4 | 0 | 0 (0) | 0 |
| 3 | distinct | 4 | 500 | 2000 | 2000 | 2000 | 0 | 0 | 0 | 0 | 0 | 2000 | 0 | 0 | 32.1 | 97.1 | 144.4 | 399.4 | 0 | 0 (0) | 0 |
| 4 | distinct | 4 | 500 | 2000 | 2000 | 2000 | 0 | 0 | 0 | 0 | 0 | 2000 | 0 | 0 | 33.5 | 84.3 | 167.2 | 230.9 | 0 | 0 (0) | 0 |
| 5 | distinct | 4 | 500 | 2000 | 2000 | 2000 | 0 | 0 | 0 | 0 | 0 | 2000 | 0 | 0 | 45.3 | 99.4 | 164.5 | 273.4 | 0 | 0 (0) | 0 |
| 1 | duplicate | 2 | 500 | 1000 | 500 | 500 | 0 | 0 | 0 | 0 | 0 | 1000 | 0 | 0 | 29.7 | 66.3 | 130.7 | 181.4 | 0 | 0 (0) | 0 |
| 2 | duplicate | 2 | 500 | 1000 | 500 | 500 | 0 | 0 | 0 | 0 | 0 | 1000 | 0 | 0 | 17.0 | 26.9 | 28.1 | 32.8 | 0 | 0 (0) | 0 |
| 3 | duplicate | 2 | 500 | 1000 | 500 | 500 | 0 | 0 | 0 | 0 | 0 | 1000 | 0 | 0 | 25.9 | 37.0 | 54.5 | 99.0 | 0 | 0 (0) | 0 |
| 4 | duplicate | 2 | 500 | 1000 | 500 | 500 | 0 | 0 | 0 | 0 | 0 | 1000 | 0 | 0 | 25.5 | 43.2 | 52.9 | 117.7 | 0 | 0 (0) | 0 |
| 5 | duplicate | 2 | 500 | 1000 | 500 | 500 | 0 | 0 | 0 | 0 | 0 | 1000 | 0 | 0 | 23.2 | 42.5 | 76.5 | 147.4 | 0 | 0 (0) | 0 |
| 1 | duplicate | 3 | 500 | 1500 | 500 | 500 | 0 | 0 | 0 | 0 | 0 | 1500 | 0 | 0 | 16.7 | 40.1 | 41.4 | 42.0 | 0 | 0 (0) | 0 |
| 2 | duplicate | 3 | 500 | 1500 | 500 | 500 | 0 | 0 | 0 | 0 | 0 | 1500 | 0 | 0 | 26.2 | 41.8 | 45.1 | 74.1 | 0 | 0 (0) | 0 |
| 3 | duplicate | 3 | 500 | 1500 | 500 | 500 | 0 | 0 | 0 | 0 | 0 | 1500 | 0 | 0 | 26.3 | 41.7 | 42.8 | 46.3 | 0 | 0 (0) | 0 |
| 4 | duplicate | 3 | 500 | 1500 | 500 | 500 | 0 | 0 | 0 | 0 | 0 | 1500 | 0 | 0 | 26.0 | 41.5 | 41.9 | 44.9 | 0 | 0 (0) | 0 |
| 5 | duplicate | 3 | 500 | 1500 | 500 | 500 | 0 | 0 | 0 | 0 | 0 | 1500 | 0 | 0 | 22.2 | 41.3 | 41.7 | 42.4 | 0 | 0 (0) | 0 |
| 1 | duplicate | 4 | 500 | 2000 | 500 | 500 | 0 | 0 | 0 | 0 | 0 | 2000 | 0 | 0 | 26.1 | 61.3 | 61.7 | 65.5 | 0 | 0 (0) | 0 |
| 2 | duplicate | 4 | 500 | 2000 | 500 | 500 | 0 | 0 | 0 | 0 | 0 | 2000 | 0 | 0 | 40.2 | 69.3 | 93.7 | 258.0 | 0 | 0 (0) | 0 |
| 3 | duplicate | 4 | 500 | 2000 | 500 | 500 | 0 | 0 | 0 | 0 | 0 | 2000 | 0 | 0 | 40.7 | 63.9 | 136.9 | 205.8 | 0 | 0 (0) | 0 |
| 4 | duplicate | 4 | 500 | 2000 | 500 | 500 | 0 | 0 | 0 | 0 | 0 | 2000 | 0 | 0 | 26.2 | 61.4 | 61.6 | 86.3 | 0 | 0 (0) | 0 |
| 5 | duplicate | 4 | 500 | 2000 | 500 | 500 | 0 | 0 | 0 | 0 | 0 | 2000 | 0 | 0 | 39.6 | 62.5 | 105.3 | 168.8 | 0 | 0 (0) | 0 |

**Aggregate across runs** (latency percentiles are of per-run values; pooled raw latencies are not retained, so the pooled column is the worst per-run value)

| mode | N | runs passing strict | total writes | expected rows | persisted | lost | lock errors | other errors | exhaustions | exhaustion rate | outer retries | attempts | p50 range ms | p95 range ms | p99 range ms | worst max ms |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| distinct | 2 | 5/5 | 10000 | 10000 | 10000 | 0 | 0 | 0 | 0 | 0 / 10000 = 0.000% | 0 | 10000 | 21.7–30.1 | 60.7–93.0 | 101.8–135.3 | 292.9 |
| distinct | 3 | 5/5 | 15000 | 15000 | 15000 | 0 | 0 | 0 | 0 | 0 / 15000 = 0.000% | 0 | 15000 | 31.4–50.2 | 67.2–126.8 | 134.2–201.8 | 303.7 |
| distinct | 4 | 5/5 | 10000 | 10000 | 10000 | 0 | 0 | 0 | 0 | 0 / 10000 = 0.000% | 0 | 10000 | 31.3–45.3 | 65.8–99.4 | 98.9–167.2 | 399.4 |
| duplicate | 2 | 5/5 | 5000 | 2500 | 2500 | 0 | 0 | 0 | 0 | 0 / 5000 = 0.000% | 0 | 5000 | 17.0–29.7 | 26.9–66.3 | 28.1–130.7 | 181.4 |
| duplicate | 3 | 5/5 | 7500 | 2500 | 2500 | 0 | 0 | 0 | 0 | 0 / 7500 = 0.000% | 0 | 7500 | 16.7–26.3 | 40.1–41.8 | 41.4–45.1 | 74.1 |
| duplicate | 4 | 5/5 | 10000 | 2500 | 2500 | 0 | 0 | 0 | 0 | 0 / 10000 = 0.000% | 0 | 10000 | 26.1–40.7 | 61.3–69.3 | 61.6–136.9 | 258.0 |

Strict matrices passing: 5/5. Writes: 57500. Distinct events requested: 35000, distinct events lost: 0. Exhaustions: 0. Lock errors: 0.

### N=8 stress results

**Stress characterization — not a supported requirement**

| run | mode | N | rounds | total writes | expected rows | persisted rows | lost | lock errors | other errors | orphan rows | duplicate rows | attempts | outer retries | exhaustions | p50 ms | p95 ms | p99 ms | max ms | ops ≥500 ms | in-proc stall gaps ≥50 ms (max) | exit |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | distinct | 8 | 500 | 4000 | 4000 | 4000 | 0 | 0 | 0 | 0 | 0 | 4000 | 0 | 0 | 90.4 | 251.9 | 454.1 | 780.9 | 27 | 0 (0) | 0 |
| 2 | distinct | 8 | 500 | 4000 | 4000 | 4000 | 0 | 0 | 0 | 0 | 0 | 4000 | 0 | 0 | 89.6 | 241.2 | 443.6 | 672.7 | 14 | 0 (0) | 0 |
| 3 | distinct | 8 | 500 | 4000 | 4000 | 4000 | 0 | 0 | 0 | 0 | 0 | 4000 | 0 | 0 | 79.6 | 192.7 | 274.9 | 573.2 | 3 | 0 (0) | 0 |
| 4 | distinct | 8 | 500 | 4000 | 4000 | 4000 | 0 | 0 | 0 | 0 | 0 | 4000 | 0 | 0 | 96.5 | 260.8 | 546.4 | 859.1 | 47 | 0 (0) | 0 |
| 5 | distinct | 8 | 500 | 4000 | 4000 | 4000 | 0 | 0 | 0 | 0 | 0 | 4000 | 0 | 0 | 91.0 | 243.9 | 440.8 | 745.9 | 23 | 0 (0) | 0 |
| 1 | duplicate | 8 | 500 | 4000 | 500 | 500 | 0 | 0 | 0 | 0 | 0 | 4000 | 0 | 0 | 86.6 | 218.4 | 349.0 | 841.2 | 13 | 0 (0) | 0 |
| 2 | duplicate | 8 | 500 | 4000 | 500 | 500 | 0 | 0 | 0 | 0 | 0 | 4000 | 0 | 0 | 86.3 | 239.5 | 351.1 | 661.4 | 9 | 0 (0) | 0 |
| 3 | duplicate | 8 | 500 | 4000 | 500 | 500 | 0 | 0 | 0 | 0 | 0 | 4000 | 0 | 0 | 86.3 | 237.0 | 436.8 | 847.9 | 28 | 0 (0) | 0 |
| 4 | duplicate | 8 | 500 | 4000 | 500 | 500 | 0 | 0 | 0 | 0 | 0 | 4000 | 0 | 0 | 86.4 | 196.9 | 352.0 | 683.1 | 13 | 0 (0) | 0 |
| 5 | duplicate | 8 | 500 | 4000 | 500 | 500 | 0 | 0 | 0 | 0 | 0 | 4000 | 0 | 0 | 86.1 | 187.5 | 295.9 | 637.0 | 4 | 0 (0) | 0 |

**Aggregate across runs** (latency percentiles are of per-run values; pooled raw latencies are not retained, so the pooled column is the worst per-run value)

| mode | N | runs passing strict | total writes | expected rows | persisted | lost | lock errors | other errors | exhaustions | exhaustion rate | outer retries | attempts | p50 range ms | p95 range ms | p99 range ms | worst max ms |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| distinct | 8 | 5/5 | 20000 | 20000 | 20000 | 0 | 0 | 0 | 0 | 0 / 20000 = 0.000% | 0 | 20000 | 79.6–96.5 | 192.7–260.8 | 274.9–546.4 | 859.1 |
| duplicate | 8 | 5/5 | 20000 | 2500 | 2500 | 0 | 0 | 0 | 0 | 0 / 20000 = 0.000% | 0 | 20000 | 86.1–86.6 | 187.5–239.5 | 295.9–436.8 | 847.9 |

N=8 writes: 40000. Distinct events requested: 20000, lost: 0. Exhaustions: 0. Lock errors: 0.

### Real-hook results

| run | N | rounds | expected | persisted msgs | persisted parts | lost | fail-open lost | non-zero exits | stderr outputs | stderr lines | p50 ms | p95 ms | p99 ms | max ms | procs ≥500 ms | in-proc stall gaps ≥50 ms (max) | exit |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 2 | 500 | 1000 | 1000 | 1000 | 0 | 0 | 0 | 0 | 0 | 25.2 | 37.0 | 37.5 | 41.6 | 0 | 0 (0) | 0 |
| 2 | 2 | 500 | 1000 | 1000 | 1000 | 0 | 0 | 0 | 0 | 0 | 25.7 | 38.0 | 51.9 | 104.3 | 0 | 0 (0) | 0 |
| 3 | 2 | 500 | 1000 | 1000 | 1000 | 0 | 0 | 0 | 0 | 0 | 29.8 | 74.0 | 127.2 | 221.1 | 0 | 0 (0) | 0 |
| 4 | 2 | 500 | 1000 | 1000 | 1000 | 0 | 0 | 0 | 0 | 0 | 27.5 | 37.5 | 40.1 | 44.5 | 0 | 0 (0) | 0 |
| 5 | 2 | 500 | 1000 | 1000 | 1000 | 0 | 0 | 0 | 0 | 0 | 26.8 | 37.4 | 41.1 | 44.6 | 0 | 0 (0) | 0 |
| 1 | 3 | 500 | 1500 | 1500 | 1500 | 0 | 0 | 0 | 0 | 0 | 36.6 | 60.8 | 135.1 | 191.7 | 0 | 0 (0) | 0 |
| 2 | 3 | 500 | 1500 | 1500 | 1500 | 0 | 0 | 0 | 0 | 0 | 36.6 | 69.6 | 98.5 | 211.6 | 0 | 0 (0) | 0 |
| 3 | 3 | 500 | 1500 | 1500 | 1500 | 0 | 0 | 0 | 0 | 0 | 36.9 | 53.9 | 77.6 | 185.7 | 0 | 0 (0) | 0 |
| 4 | 3 | 500 | 1500 | 1500 | 1500 | 0 | 0 | 0 | 0 | 0 | 37.7 | 80.6 | 114.7 | 235.8 | 0 | 0 (0) | 0 |
| 5 | 3 | 500 | 1500 | 1500 | 1500 | 0 | 0 | 0 | 0 | 0 | 36.8 | 58.1 | 96.7 | 225.4 | 0 | 0 (0) | 0 |
| 1 | 4 | 200 | 800 | 800 | 800 | 0 | 0 | 0 | 0 | 0 | 57.4 | 127.3 | 193.3 | 262.9 | 0 | 0 (0) | 0 |
| 2 | 4 | 200 | 800 | 800 | 800 | 0 | 0 | 0 | 0 | 0 | 63.9 | 154.3 | 227.7 | 388.3 | 0 | 0 (0) | 0 |
| 3 | 4 | 200 | 800 | 800 | 800 | 0 | 0 | 0 | 0 | 0 | 51.6 | 97.1 | 107.5 | 260.7 | 0 | 0 (0) | 0 |
| 4 | 4 | 200 | 800 | 800 | 800 | 0 | 0 | 0 | 0 | 0 | 51.0 | 87.4 | 114.3 | 247.4 | 0 | 0 (0) | 0 |
| 5 | 4 | 200 | 800 | 800 | 800 | 0 | 0 | 0 | 0 | 0 | 50.7 | 78.3 | 103.1 | 196.6 | 0 | 0 (0) | 0 |

**Aggregate**

| N | runs passing strict | expected | persisted | lost | fail-open lost | non-zero exits | stderr lines | p50 range ms | p95 range ms | p99 range ms | worst max ms |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 2 | 5/5 | 5000 | 5000 | 0 | 0 | 0 | 0 | 25.2–29.8 | 37.0–74.0 | 37.5–127.2 | 221.1 |
| 3 | 5/5 | 7500 | 7500 | 0 | 0 | 0 | 0 | 36.6–37.7 | 53.9–80.6 | 77.6–135.1 | 235.8 |
| 4 | 5/5 | 4000 | 4000 | 0 | 0 | 0 | 0 | 50.7–63.9 | 78.3–154.3 | 103.1–227.7 | 388.3 |

### Slow operations (≥ 500 ms)

Attempt duration is the time inside one Turso attempt (busy-handler wait included). Backoff actual vs requested shows scheduler oversleep. 'Outside' is time not inside an attempt or backoff.


#### B (strict N=2–4): 0 slow ops


#### C (N=8 stress): 181 slow ops

- Ok(false), 1 attempt(s): 67
- Ok(true), 1 attempt(s): 114
- longest single attempt: 859.1 ms; worst backoff oversleep: 0.0 ms; worst time outside attempts/backoff: 0.02 ms

### Every exhausted operation

None. No operation exhausted the policy in this campaign.

### Host monitor summary (whole campaign)

- campaign window: 1791206344424–1791209382718 (50.6 min), 3037 1 s samples
- external monitor sleep-gap events ≥50 ms: 0; max 0.0 ms
- psi_cpu_some_ms: p50 1.1, p99 2.9, max 5.7
- psi_mem_some_ms: p50 0.0, p99 0.0, max 0.0
- psi_io_full_ms: p50 398.2, p99 945.6, max 974.4 — io_uring idle-wait artifact, not usable (see Test environment)
- cpu_iowait_pct: p50 9.8, p99 18.6, max 20.0 — same artifact
- disk busy ms per 1 s (max of nvme0n1/dm-0): p50 593, p99 950, max 1004
- procs_blocked: p50 3, max 7

### Current-campaign conclusions

Correctness: no correctness violation in any of the 114,110 measured write operations (A 110 boundary samples, B 57,500, C 40,000, D 16,500 hook events). There were 0 orphan rows and 0 duplicate logical events across all 10,000 duplicate rounds (7,500 at N=2–4, 2,500 at N=8). Every failure left 0/0 rows, and no writer exceeded 2 attempts.

Availability (observed on reference host):

| Scope | Exhaustions | Lost distinct events |
| --- | --- | --- |
| Strict N=2–4 in-process | 0 / 57,500 writes; 5/5 strict matrices passed | 0 / 35,000 |
| Real hooks N=2–4 | not countable (no cross-process counters) | 0 / 16,500 events; 0 fail-open losses; 0 non-zero exits; 0 stderr records; 15/15 level-runs passed |
| N=8 stress, distinct | 0 / 20,000 | 0 / 20,000 |
| N=8 stress, duplicate | 0 / 20,000 | 0 (no logical-event loss) |
| Held lock, actual release ≤ 2013 ms | 0 / 80 samples | n/a |
| Held lock, actual release ≥ 2254 ms | 30 / 30 samples (by design) | n/a |

Supported-load slow operations: none. No strict N=2–4 operation took ≥ 500 ms, so this campaign did not reproduce a supported-load lock holder of 1.6–1.7 s. Coverage of such a holder rests on the held-lock experiment (1500–2000 ms holds succeeded 30/30), not on a reproduced concurrent incident.

N=8 stress: the former exhaustion mode did not appear. There were 181 operations ≥ 500 ms, all of them single-attempt (`Ok(true)` 114, `Ok(false)` 67). The longest single attempt was 859 ms; backoff oversleep was 0.0 ms and time outside attempts/backoff was ≤ 0.02 ms.

Host: the external monitor recorded 0 sleep-gap events ≥ 50 ms in 50.6 minutes. CPU PSI stayed at p99 2.9 ms/s. Device busy time had the same profile as the historical campaign (p50 593, max 1004 ms/s). As before, IO PSI and iowait are not usable on this host.

## Supported-load failure under the 500 / 1250 policy

During final validation of the 500 / 1250 policy (2026-10-05, release build from `HEAD` `e72116239fa8f05befe9fb8e1220c79100914490`, run through `nix develop -c ./scripts/run-cli-cargo.sh test --release`), the strict AC8 suite failed at supported levels. The run was a single pass, not part of the protocol above. **No host IO evidence (`vmstat`, `iostat`, monitor) was captured for it**, so the cause of the long holder transactions is not known.

| Level | Rounds | Result | Lock errors | Exhaustions | Lost distinct events | p50 / p95 / p99 / max ms |
| --- | --- | --- | --- | --- | --- | --- |
| distinct N=2 | 1000 | pass | 0 | 0 | 0 | 30 / 93 / 135 / 1138 |
| distinct N=3 | 1000 | pass | 0 | 0 | 0 | 45 / 105 / 151 / 236 |
| distinct N=4 | 500 | **fail** | 1 | 1 | **1** | 45 / 173 / 254 / 1666 |
| duplicate N=2 | 500 | pass | 0 | 0 | 0 | 18 / 26 / 26 / 34 |
| duplicate N=3 | 500 | **fail** | 2 | 2 | 0 (another writer persisted the event) | 40 / 119 / 168 / 1616 |
| duplicate N=4 | 500 | pass | 0 | 0 | 0 | 62 / 123 / 177 / 400 |
| distinct N=8 (stress) | 500 | non-strict | 2 | 2 | 2 | 105 / 360 / 530 / 2134 |
| duplicate N=8 (stress) | 500 | non-strict | 0 | 0 | 0 | 86 / 245 / 451 / 650 |

Retry timelines of the failing supported rounds (test-only instrumentation):

| Level | Round | Operation | Result | Elapsed ms | Attempt durations ms | Backoff requested / actual ms | Outside attempts + backoff ms |
| --- | --- | --- | --- | --- | --- | --- | --- |
| distinct N=4 | 384 | holder | `Ok(true)` | 1666.8 | 1666.8 | - | 0.00 |
| distinct N=4 | 384 | waiter | `database is locked` | 1006.5 | 500.2 + 500.2 | 6 / 6.1 | 0.01 |
| duplicate N=3 | 338 | holder | `Ok(true)` | 1616.1 | 1616.1 | - | 0.00 |
| duplicate N=3 | 338 | waiter | `database is locked` | 1089.5 | 500.2 + 500.2 | 89 / 89.1 | 0.01 |
| duplicate N=3 | 338 | waiter | `database is locked` | 1064.5 | 500.2 + 500.2 | 64 / 64.1 | 0.02 |

Why the waiters exhausted:

- **The holder kept the writer lock beyond the waiter's budget.** The holding transaction's single attempt took 1616–1667 ms; the waiters gave up at 1006–1090 ms.
- **Retry admission did not reject attempt 2.** Every waiter made both attempts.
- **Turso did not return `Busy` early.** Each busy wait lasted the full 500.2 ms.
- **No scheduler delay was observed.** Backoff matched the request within 0.1 ms, time outside attempts and backoff was ≤ 0.02 ms, and the in-process gap monitor recorded 0 gaps ≥ 50 ms.
- Why the holder transaction took 1.6–1.7 s is not established; no host evidence was captured.

This failure stays in the dataset. It is the reason the defaults were tuned.

## Policy comparison: 500 / 1250 / 2 vs 1000 / 2250 / 2

All figures are observed on the reference host, not guarantees. The 500 / 1250 column combines the historical campaign and the failed validation run.

| Measure | 500 / 1250 / 2 | 1000 / 2250 / 2 |
| --- | --- | --- |
| Supported N=2–4 in-process loss | 0 / 35,000 distinct (campaign); **1 / 7,000** distinct (validation run) | 0 / 35,000 distinct |
| Supported N=2–4 in-process exhaustions | 0 / 57,500 (campaign); **3 / 11,500** (validation run) | 0 / 57,500 |
| Supported strict matrices passed | 5/5 (campaign); validation run failed | 5/5 |
| Real hooks N=2–4 | 0 / 16,500 lost | 0 / 16,500 lost |
| In-process p50 range (distinct N=2 / 3 / 4) | 22.5–42.6 / 36.7–69.0 / 43.6–54.4 ms | 21.7–30.1 / 31.4–50.2 / 31.3–45.3 ms |
| In-process p95 range (distinct N=2 / 3 / 4) | 44–95 / 92–141 / 69–145 ms | 61–93 / 67–127 / 66–99 ms |
| In-process p99 range (distinct N=2 / 3 / 4) | 87–137 / 138–216 / 156–220 ms | 102–135 / 134–202 / 99–167 ms |
| In-process worst max, supported | 404 ms (campaign); 1666 ms (validation run) | 399 ms |
| Real-hook worst max (N=2 / 3 / 4) | 223 / 105 / 543 ms | 221 / 236 / 388 ms |
| Held-lock transition (actual holder release) | succeeded ≤ 1026 ms; exhausted ≥ 1503 ms | succeeded ≤ 2013 ms; exhausted ≥ 2254 ms |
| Exhausting writer latency (blocked write gives up) | 1.0–1.1 s (observed 1003–1099 ms) | 2.0–2.1 s (observed 2001–2094 ms) |
| N=8 stress exhaustions | 16 / 40,000 (campaign, 1 of 5 duplicate runs); 2 / 8,000 (validation run) | 0 / 40,000 |
| N=8 stress worst max | 1834 ms (campaign); 2134 ms (validation run) | 859 ms |

Latency cost of the larger budget:

- **Worst blocked-hook latency roughly doubles: about +1.0 s.** A write that exhausts now blocks its hook for about 2.0–2.1 s instead of 1.0–1.1 s. The bounded-wait property is unchanged: no hook waits indefinitely, and no wait exceeds two busy timeouts plus at most 100 ms backoff.
- **Writes blocked 0.5–1.0 s no longer pay for an outer retry.** A 750 ms hold completed at 841–847 ms on attempt 1, versus 771–860 ms through a retry before. The cost is similar.
- **The uncontended and normally contended path is unchanged.** Neither policy produced supported-load slow operations in its campaign, and the per-run percentile ranges overlap. The busy timeout only adds latency when a lock is held longer than the old 500 ms.

### Policy decision

Adopt 1000 / 2250 / 2 / 100 ms as the defaults.

- All 5 supported strict in-process matrices and all 5 strict real-hook matrices passed, with 0 lock errors, 0 exhaustions and 0 lost events.
- The policy covers the 1.6–1.7 s lock holder that failed the 500 / 1250 validation run, with about 300 ms of slack before the observed 2.0–2.1 s exhaustion point.
- The former N=8 exhaustion mode did not occur (0 / 40,000).
- The price is about +1 s of worst-case blocked-hook latency on exhausting writes.
- Limit: a supported-load holder above about 2 s would still exhaust this policy. If that is observed, the next step is not further timeout tuning. Bounded synchronous hook latency, no durable queue or spool, and unbounded writer-lock duration together mean zero-loss ingestion cannot be guaranteed. That would need a product/architecture decision between accepting occasional fail-open ingestion loss and introducing eventual persistence (spool or queue).

## Historical campaign: 500 / 1250 / 2 / 100

**Superseded.** The sections below are the full measurement campaign for the earlier 500 / 1250 defaults, kept unchanged as evidence. Their contract, conclusions and policy decision describe that policy, not the current defaults.

### Test environment

| Item | Value |
| --- | --- |
| Host | bare metal (`systemd-detect-virt` → `none`, no `hypervisor` CPU flag), not CI (`CI` unset) |
| Kernel | `Linux nixos 6.18.37 #1-NixOS SMP PREEMPT_DYNAMIC Sat Jun 27 10:06:50 UTC 2026 x86_64 GNU/Linux` |
| CPU | AMD Ryzen 9 5900X, 12 cores / 24 threads, 1 socket |
| RAM | 46 GiB total, about 41 GiB available at start, swap unused |
| Test workspace | `std::env::temp_dir()` = `/tmp`, ext4 on LUKS dm-crypt (`/dev/mapper/cryptroot`, `dm-0`) on NVMe (`nvme0n1`); `/tmp` is not tmpfs |
| Turso | `turso` crate `0.8.1` (`cli/Cargo.lock`), linked into both test and `sce` binaries. The repo-pinned Turso CLI (0.7.0) was not used. |
| Rust | `rustc 1.95.0 (59807616e 2026-04-14)`, `cargo 1.95.0`, via `nix develop` |
| Other load | Desktop session left running: Brave, Discord, Ghostty, and an idle Claude Code session. No builds, package managers or other benchmarks ran during the campaign. |
| Campaign date | 2026-10-05, about 12:29–13:18 local; 49.0 minutes of measured runs |

Host-health caveat: on this host `/proc/pressure/io` reads about 40–95% "full" even when idle. The disk had 0 requests in flight and flat `io_ms` counters at the same time. The cause is Ghostty's renderer and IO threads parked in `io_cqring_wait`: the kernel accounts io_uring CQ waits as iowait. Two "blocked" tasks therefore appear in `vmstat`'s `b` column, and about 8–9% shows as `wa`, permanently. IO PSI, `vmstat wa`/`b` and `cpu_iowait` are **not** usable as host-stall evidence here. This doc uses scheduler-gap monitors, CPU PSI, per-device busy time, and the operation timelines.

### Exact commit/config

- `git rev-parse HEAD` → `a33797054ae774783124c4b312be209dd94e6e36` (branch `agent-trace-db-write-contention`); `git status --short` was clean before the campaign.
- Measurement build = HEAD + a **test-only** instrumentation diff, sha256 `61e92a853fc9d6e03703bbcc0e8bc82c93dbabf620d43b1bad445a0caaf241a9`:
  - `cli/src/services/db/mod.rs`: `#[cfg(test)]` `record_write_contention_timeline` with `Instant` stamps for attempt start/end, backoff requested, and backoff slept.
  - `cli/src/services/agent_trace_db/lock_contention_tests.rs`: JSON `SCE_MEAS` records, per-operation timelines, slow-operation (≥ 500 ms) records, and an in-process scheduler-gap monitor (5 ms sleep loop; reports wake-ups ≥ 50 ms late).
  - No non-test code path changed.
  - After the campaign, the working tree took three clippy-only refactors: `RefCell::take` method reference, a `HookRound` struct for the round return type, and `#[allow(clippy::too_many_lines)]` on two test functions. None of them changes behavior. The measured binaries predate them.
  - The raw `SCE_MEAS` logs, host snapshots and monitor JSONL were kept in the session scratchpad and are not committed. Every per-run figure below is copied from them.
- The policy ran with defaults, with no `policies.database_retry.agent_trace_db` override:

  | Setting | Value |
  | --- | --- |
  | `busy_timeout_ms` | 500 |
  | `contention_deadline_ms` | 1250 |
  | max attempts | 2 |
  | full-jitter backoff | `0..=100` ms |
- All measurements used one frozen pair of release artifacts, built once before the first run. Build time is not in any latency.
  - test binary sha256 `3d5445513b25accd34473898864f556d6c4f61c34a64ffdbc5d5a7cc9f90a312`;
  - `sce` sha256 `c044fd62cc0d8baae60fcb07734d3c1d8850eb34ecddf8fc832104a0339ce29a`.

### Contractual assertions

These are the only assertions the suite makes.

- Held-lock boundary (`lock_budget_boundary_characterizes_single_writer_blocked_by_begin_immediate_holder`):
  - holds ≤ 250 ms must succeed with 0 exhaustions;
  - holds ≥ 2000 ms must fail with the `under write contention` error and exactly 1 exhaustion;
  - every failure leaves 0 message and 0 part rows;
  - every sample makes 1–2 attempts, and `outer_retries + 1 == attempts`.
  - 500–1500 ms is characterization only.
- Every in-process round:
  - `messages == parts`;
  - the `Ok(true)` count equals the persisted rows;
  - no writer exceeds 2 attempts;
  - every duplicate round inserts at most once.
- `SCE_LOCK_CONTENTION_STRICT=1` adds, per level:
  - 0 lock errors, 0 other errors, 0 lost events and 0 exhaustions;
  - for hook processes: every expected message and part persisted, and 0 non-zero exits.
- Strict supported levels:

  | Suite | Levels (N × rounds) |
  | --- | --- |
  | distinct events | 2×1000, 3×1000, 4×500 |
  | duplicate delivery | 2×500, 3×500, 4×500 |
  | hook processes | 2×500, 3×500, 4×200 |

  N=8 is stress characterization, not a supported requirement.

### Measurement methodology

Artifacts were built once and frozen:

```sh
nix develop -c ./scripts/run-cli-cargo.sh test --release --manifest-path cli/Cargo.toml --no-run
nix develop -c ./scripts/run-cli-cargo.sh build --release --manifest-path cli/Cargo.toml
cp cli/target/release/deps/sce-<hash> $S/bin/sce-tests && cp cli/target/release/sce $S/bin/sce
```

Each measured run was a separate process of the frozen test binary. Test path prefix: `services::agent_trace_db::lock_contention_tests::`.

```sh
# A (×10)
sce-tests --exact <prefix>lock_budget_boundary_characterizes_single_writer_blocked_by_begin_immediate_holder --nocapture --test-threads=1
# B/C: one process per level; D adds SCE_BIN=$S/bin/sce
SCE_LOCK_CONTENTION_STRICT=1 SCE_LOCK_CONTENTION_WRITERS=<N> SCE_LOCK_CONTENTION_ROUNDS=<R> \
  sce-tests --exact <prefix><test> --ignored --nocapture --test-threads=1
```

- `<test>` is one of:
  - `concurrent_distinct_events_persist_every_event_under_write_contention`;
  - `concurrent_duplicate_delivery_persists_each_event_once_under_write_contention`;
  - `concurrent_real_codex_hook_processes_persist_every_distinct_event`.
- Run order:
  1. A: 10 runs;
  2. B: matrices 1–5, each in the order d2, d3, d4, dup2, dup3, dup4;
  3. C: runs 1–5, each distinct then duplicate N=8;
  4. D: matrices 1–5, each in the order N=2, N=3, N=4.
- Every level ran in strict mode as its own process, so a strict failure could not stop later levels.
- Before every run: `uptime`, `/proc/pressure/*`, `vmstat 1 5`, and `iostat -xz 1 5` (sysstat 12.7.7 via a pre-resolved Nix store path).
- After every non-zero exit: `uptime`, `/proc/pressure/*`, `vmstat 1 10`, and `iostat -xz 1 10`.
- For the whole campaign, an external monitor process (`hostmon.py`) logged two things:
  - per second: CPU, memory and IO PSI deltas, iowait and steal, per-device busy ms and in-flight IO for `nvme0n1`/`dm-0`, `procs_running` and `procs_blocked`;
  - every wake-up of a 5 ms sleep loop that was ≥ 50 ms late, with Unix ms.
- Latency:
  - in-process: a monotonic clock around `insert_conversation_text_event`, from barrier release;
  - hooks: from the payload write to the child's exit (includes process startup).
  - Percentiles are nearest-rank per level-run. Raw per-write latencies were not retained, so cross-run aggregates report the range of per-run percentiles and the worst max.
- Two smoke runs preceded the protocol and are not in the dataset: one boundary run, which passed, and hook N=2×5, which passed. Nothing was rerun or replaced, and no run was dropped.

### Held-lock results

Contract: ≤ 250 ms succeeds, ≥ 2000 ms exhausts, failure leaves 0/0 rows, ≤ 2 attempts. All 10 runs satisfied it: 70/70 samples, 10/10 test passes.

Observed on reference host (10 runs × 7 holds):

| hold ms | success | exhausted | attempt-1 success | attempt-2 success | writer p50 ms | writer p95 ms | writer max ms | holder release p50 ms | holder release max ms |
|---|---|---|---|---|---|---|---|---|---|
| 100 | 10/10 | 0/10 | 10 | 0 | 116.5 | 155.4 | 155.4 | 104.2 | 114.5 |
| 250 | 10/10 | 0/10 | 10 | 0 | 341.2 | 369.5 | 369.5 | 262.3 | 262.4 |
| 500 | 10/10 | 0/10 | 9 | 1 | 520.8 | 597.5 | 597.5 | 512.3 | 515.1 |
| 750 | 10/10 | 0/10 | 0 | 10 | 799.2 | 859.5 | 859.5 | 762.3 | 766.4 |
| 1000 | 10/10 | 0/10 | 0 | 10 | 1035.9 | 1102.0 | 1102.0 | 1012.2 | 1025.7 |
| 1500 | 0/10 | 10/10 | 0 | 0 | 1052.4 | 1098.4 | 1098.4 | 1511.6 | 1515.9 |
| 2000 | 0/10 | 10/10 | 0 | 0 | 1021.5 | 1099.4 | 1099.4 | 2012.3 | 2035.5 |

What the timelines show:

- **Attempt 1 always lasts about 500.1–500.2 ms when blocked.** That is the busy timeout.
- **Backoff sleeps matched the requested jitter within ≤ 0.1 ms** in every sample.
- **The empirical success/exhaust transition lies between 1000 and 1500 ms of requested hold.** In this run set, actual holder release was 1012–1026 ms (all succeeded) versus 1503–1516 ms (all exhausted).
  - A blocked writer exhausts at about 1.0–1.1 s: 500 + jitter + 500.
  - A write succeeds only if the holder releases before attempt 2's busy wait ends, at about 1.0–1.1 s after start, depending on the drawn backoff.
  - Requested holds tracked actual release closely (+2–36 ms), so no host-delayed releases occurred in this campaign.
- **500 ms is not a first-attempt guarantee.** 9/10 succeeded on attempt 1. In 1/10 the holder released 5–15 ms after the 500 ms busy timeout expired, which forced an outer retry.
- **Turso's busy wait polls; it is not woken on release.**
  - 250 ms holds completed at 341 ms (p50), about 80 ms after release.
  - 100 ms holds completed at 116 ms (p50).

Raw samples:

| run | hold ms | holder released ms | writer elapsed ms | result | attempts | outer retries | exhaustions | messages | parts | attempt1 ms | backoff req/actual ms | attempt2 ms |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 100 | 104.2 | 117.1 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 117.1 | - | - |
| 1 | 250 | 262.3 | 342.0 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 342.0 | - | - |
| 1 | 500 | 512.7 | 520.8 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 520.8 | - | - |
| 1 | 750 | 762.4 | 853.5 | Ok(true) | 2 | 1 | 0 | 1 | 1 | 500.2 | 12/12.1 | 341.3 |
| 1 | 1000 | 1012.2 | 1041.5 | Ok(true) | 2 | 1 | 0 | 1 | 1 | 500.2 | 28/28.1 | 513.2 |
| 1 | 1500 | 1513.0 | 1055.4 | database is locked | 2 | 1 | 1 | 0 | 0 | 500.2 | 55/55.1 | 500.2 |
| 1 | 2000 | 2013.1 | 1084.4 | database is locked | 2 | 1 | 1 | 0 | 0 | 500.2 | 84/84.0 | 500.2 |
| 2 | 100 | 104.0 | 115.5 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 115.5 | - | - |
| 2 | 250 | 262.2 | 340.9 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 340.9 | - | - |
| 2 | 500 | 512.6 | 520.8 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 520.8 | - | - |
| 2 | 750 | 762.1 | 793.9 | Ok(true) | 2 | 1 | 0 | 1 | 1 | 500.2 | 53/53.0 | 240.7 |
| 2 | 1000 | 1012.3 | 1024.8 | Ok(true) | 2 | 1 | 0 | 1 | 1 | 500.2 | 11/11.1 | 513.6 |
| 2 | 1500 | 1513.3 | 1095.4 | database is locked | 2 | 1 | 1 | 0 | 0 | 500.1 | 95/95.0 | 500.2 |
| 2 | 2000 | 2012.3 | 1015.4 | database is locked | 2 | 1 | 1 | 0 | 0 | 500.2 | 15/15.1 | 500.1 |
| 3 | 100 | 104.0 | 115.5 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 115.5 | - | - |
| 3 | 250 | 262.4 | 341.1 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 341.1 | - | - |
| 3 | 500 | 512.3 | 520.8 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 520.8 | - | - |
| 3 | 750 | 762.1 | 854.6 | Ok(true) | 2 | 1 | 0 | 1 | 1 | 500.2 | 13/13.1 | 341.3 |
| 3 | 1000 | 1012.5 | 1027.3 | Ok(true) | 2 | 1 | 0 | 1 | 1 | 500.2 | 86/86.0 | 441.1 |
| 3 | 1500 | 1511.6 | 1052.4 | database is locked | 2 | 1 | 1 | 0 | 0 | 500.2 | 52/52.1 | 500.2 |
| 3 | 2000 | 2012.2 | 1032.4 | database is locked | 2 | 1 | 1 | 0 | 0 | 500.2 | 32/32.1 | 500.2 |
| 4 | 100 | 104.4 | 116.8 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 116.7 | - | - |
| 4 | 250 | 262.4 | 341.2 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 341.2 | - | - |
| 4 | 500 | 512.2 | 521.5 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 521.5 | - | - |
| 4 | 750 | 763.0 | 778.0 | Ok(true) | 2 | 1 | 0 | 1 | 1 | 500.2 | 87/87.1 | 190.8 |
| 4 | 1000 | 1012.5 | 1035.9 | Ok(true) | 2 | 1 | 0 | 1 | 1 | 500.2 | 95/95.1 | 440.7 |
| 4 | 1500 | 1512.4 | 1048.3 | database is locked | 2 | 1 | 1 | 0 | 0 | 500.1 | 48/48.1 | 500.2 |
| 4 | 2000 | 2012.9 | 1099.4 | database is locked | 2 | 1 | 1 | 0 | 0 | 500.2 | 99/99.0 | 500.2 |
| 5 | 100 | 104.4 | 116.5 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 116.5 | - | - |
| 5 | 250 | 262.4 | 340.8 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 340.8 | - | - |
| 5 | 500 | 512.6 | 520.4 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 520.4 | - | - |
| 5 | 750 | 762.4 | 859.5 | Ok(true) | 2 | 1 | 0 | 1 | 1 | 500.2 | 18/18.1 | 341.3 |
| 5 | 1000 | 1012.2 | 1066.1 | Ok(true) | 2 | 1 | 0 | 1 | 1 | 500.1 | 53/53.1 | 512.9 |
| 5 | 1500 | 1515.9 | 1048.4 | database is locked | 2 | 1 | 1 | 0 | 0 | 500.2 | 48/48.1 | 500.2 |
| 5 | 2000 | 2012.4 | 1029.4 | database is locked | 2 | 1 | 1 | 0 | 0 | 500.2 | 29/29.1 | 500.2 |
| 6 | 100 | 103.9 | 115.1 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 115.1 | - | - |
| 6 | 250 | 262.4 | 340.2 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 340.2 | - | - |
| 6 | 500 | 515.1 | 522.8 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 522.8 | - | - |
| 6 | 750 | 762.3 | 771.4 | Ok(true) | 2 | 1 | 0 | 1 | 1 | 500.2 | 78/78.1 | 193.2 |
| 6 | 1000 | 1012.0 | 1023.4 | Ok(true) | 2 | 1 | 0 | 1 | 1 | 500.2 | 11/11.1 | 512.2 |
| 6 | 1500 | 1512.5 | 1043.3 | database is locked | 2 | 1 | 1 | 0 | 0 | 500.1 | 43/43.1 | 500.1 |
| 6 | 2000 | 2012.7 | 1020.4 | database is locked | 2 | 1 | 1 | 0 | 0 | 500.2 | 20/20.1 | 500.2 |
| 7 | 100 | 103.9 | 115.6 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 115.6 | - | - |
| 7 | 250 | 262.3 | 341.4 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 341.4 | - | - |
| 7 | 500 | 512.5 | 520.7 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 520.7 | - | - |
| 7 | 750 | 766.4 | 799.2 | Ok(true) | 2 | 1 | 0 | 1 | 1 | 500.1 | 59/59.1 | 240.0 |
| 7 | 1000 | 1004.9 | 1020.0 | Ok(true) | 2 | 1 | 0 | 1 | 1 | 500.1 | 8/8.1 | 511.8 |
| 7 | 1500 | 1506.6 | 1090.4 | database is locked | 2 | 1 | 1 | 0 | 0 | 500.2 | 90/90.0 | 500.2 |
| 7 | 2000 | 2004.2 | 1048.4 | database is locked | 2 | 1 | 1 | 0 | 0 | 500.2 | 48/48.1 | 500.1 |
| 8 | 100 | 105.2 | 138.7 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 138.7 | - | - |
| 8 | 250 | 254.6 | 346.1 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 346.1 | - | - |
| 8 | 500 | 504.7 | 597.5 | Ok(true) | 2 | 1 | 0 | 1 | 1 | 500.2 | 76/76.1 | 21.3 |
| 8 | 750 | 754.5 | 787.1 | Ok(true) | 2 | 1 | 0 | 1 | 1 | 500.2 | 72/72.0 | 214.9 |
| 8 | 1000 | 1008.3 | 1089.7 | Ok(true) | 2 | 1 | 0 | 1 | 1 | 500.2 | 59/59.1 | 530.4 |
| 8 | 1500 | 1505.0 | 1066.4 | database is locked | 2 | 1 | 1 | 0 | 0 | 500.1 | 66/66.1 | 500.2 |
| 8 | 2000 | 2005.2 | 1009.4 | database is locked | 2 | 1 | 1 | 0 | 0 | 500.2 | 9/9.1 | 500.2 |
| 9 | 100 | 104.2 | 132.0 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 132.0 | - | - |
| 9 | 250 | 254.1 | 348.6 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 348.6 | - | - |
| 9 | 500 | 504.4 | 541.5 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 541.5 | - | - |
| 9 | 750 | 754.6 | 801.8 | Ok(true) | 2 | 1 | 0 | 1 | 1 | 500.2 | 56/56.0 | 245.6 |
| 9 | 1000 | 1014.7 | 1102.0 | Ok(true) | 2 | 1 | 0 | 1 | 1 | 500.1 | 71/71.1 | 530.8 |
| 9 | 1500 | 1505.2 | 1005.4 | database is locked | 2 | 1 | 1 | 0 | 0 | 500.1 | 5/5.1 | 500.2 |
| 9 | 2000 | 2004.4 | 1003.4 | database is locked | 2 | 1 | 1 | 0 | 0 | 500.2 | 3/3.1 | 500.2 |
| 10 | 100 | 114.5 | 155.4 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 155.4 | - | - |
| 10 | 250 | 254.7 | 369.5 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 369.4 | - | - |
| 10 | 500 | 504.6 | 541.5 | Ok(true) | 1 | 0 | 0 | 1 | 1 | 541.5 | - | - |
| 10 | 750 | 764.6 | 802.7 | Ok(true) | 2 | 1 | 0 | 1 | 1 | 500.2 | 26/26.1 | 276.5 |
| 10 | 1000 | 1025.7 | 1085.1 | Ok(true) | 2 | 1 | 0 | 1 | 1 | 500.1 | 33/33.1 | 551.9 |
| 10 | 1500 | 1505.9 | 1098.4 | database is locked | 2 | 1 | 1 | 0 | 0 | 500.1 | 98/98.0 | 500.2 |
| 10 | 2000 | 2035.5 | 1021.5 | database is locked | 2 | 1 | 1 | 0 | 0 | 500.2 | 21/21.1 | 500.2 |

### Strict N=2–4 results

| run | mode | N | rounds | total writes | expected rows | persisted rows | lost | lock errors | other errors | orphan rows | duplicate rows | attempts | outer retries | exhaustions | p50 ms | p95 ms | p99 ms | max ms | ops ≥500 ms | in-proc stall gaps ≥50 ms (max) | exit |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | distinct | 2 | 1000 | 2000 | 2000 | 2000 | 0 | 0 | 0 | 0 | 0 | 2000 | 0 | 0 | 42.6 | 92.7 | 136.5 | 186.4 | 0 | 0 (0) | 0 |
| 2 | distinct | 2 | 1000 | 2000 | 2000 | 2000 | 0 | 0 | 0 | 0 | 0 | 2000 | 0 | 0 | 29.7 | 70.8 | 100.2 | 241.2 | 0 | 0 (0) | 0 |
| 3 | distinct | 2 | 1000 | 2000 | 2000 | 2000 | 0 | 0 | 0 | 0 | 0 | 2000 | 0 | 0 | 29.7 | 94.7 | 136.0 | 192.1 | 0 | 0 (0) | 0 |
| 4 | distinct | 2 | 1000 | 2000 | 2000 | 2000 | 0 | 0 | 0 | 0 | 0 | 2000 | 0 | 0 | 24.3 | 70.9 | 102.0 | 151.1 | 0 | 0 (0) | 0 |
| 5 | distinct | 2 | 1000 | 2000 | 2000 | 2000 | 0 | 0 | 0 | 0 | 0 | 2000 | 0 | 0 | 22.5 | 44.4 | 87.3 | 150.1 | 0 | 0 (0) | 0 |
| 1 | distinct | 3 | 1000 | 3000 | 3000 | 3000 | 0 | 0 | 0 | 0 | 0 | 3000 | 0 | 0 | 69.0 | 140.7 | 215.6 | 315.7 | 0 | 0 (0) | 0 |
| 2 | distinct | 3 | 1000 | 3000 | 3000 | 3000 | 0 | 0 | 0 | 0 | 0 | 3000 | 0 | 0 | 44.4 | 114.5 | 167.0 | 264.3 | 0 | 0 (0) | 0 |
| 3 | distinct | 3 | 1000 | 3000 | 3000 | 3000 | 0 | 0 | 0 | 0 | 0 | 3000 | 0 | 0 | 38.0 | 100.2 | 137.8 | 261.9 | 0 | 0 (0) | 0 |
| 4 | distinct | 3 | 1000 | 3000 | 3000 | 3000 | 0 | 0 | 0 | 0 | 0 | 3000 | 0 | 0 | 36.7 | 92.3 | 151.6 | 236.2 | 0 | 0 (0) | 0 |
| 5 | distinct | 3 | 1000 | 3000 | 3000 | 3000 | 0 | 0 | 0 | 0 | 0 | 3000 | 0 | 0 | 44.9 | 119.1 | 184.0 | 304.5 | 0 | 0 (0) | 0 |
| 1 | distinct | 4 | 500 | 2000 | 2000 | 2000 | 0 | 0 | 0 | 0 | 0 | 2000 | 0 | 0 | 46.1 | 127.8 | 175.4 | 252.6 | 0 | 0 (0) | 0 |
| 2 | distinct | 4 | 500 | 2000 | 2000 | 2000 | 0 | 0 | 0 | 0 | 0 | 2000 | 0 | 0 | 54.4 | 139.6 | 220.1 | 404.2 | 0 | 0 (0) | 0 |
| 3 | distinct | 4 | 500 | 2000 | 2000 | 2000 | 0 | 0 | 0 | 0 | 0 | 2000 | 0 | 0 | 45.1 | 144.5 | 210.9 | 360.7 | 0 | 0 (0) | 0 |
| 4 | distinct | 4 | 500 | 2000 | 2000 | 2000 | 0 | 0 | 0 | 0 | 0 | 2000 | 0 | 0 | 44.4 | 69.2 | 156.0 | 239.9 | 0 | 0 (0) | 0 |
| 5 | distinct | 4 | 500 | 2000 | 2000 | 2000 | 0 | 0 | 0 | 0 | 0 | 2000 | 0 | 0 | 43.6 | 97.5 | 178.4 | 341.3 | 0 | 0 (0) | 0 |
| 1 | duplicate | 2 | 500 | 1000 | 500 | 500 | 0 | 0 | 0 | 0 | 0 | 1000 | 0 | 0 | 16.9 | 32.0 | 40.1 | 91.5 | 0 | 0 (0) | 0 |
| 2 | duplicate | 2 | 500 | 1000 | 500 | 500 | 0 | 0 | 0 | 0 | 0 | 1000 | 0 | 0 | 18.4 | 26.7 | 45.5 | 123.0 | 0 | 0 (0) | 0 |
| 3 | duplicate | 2 | 500 | 1000 | 500 | 500 | 0 | 0 | 0 | 0 | 0 | 1000 | 0 | 0 | 27.0 | 45.7 | 93.7 | 150.7 | 0 | 0 (0) | 0 |
| 4 | duplicate | 2 | 500 | 1000 | 500 | 500 | 0 | 0 | 0 | 0 | 0 | 1000 | 0 | 0 | 18.5 | 27.2 | 31.1 | 222.9 | 0 | 0 (0) | 0 |
| 5 | duplicate | 2 | 500 | 1000 | 500 | 500 | 0 | 0 | 0 | 0 | 0 | 1000 | 0 | 0 | 15.4 | 26.5 | 26.8 | 32.7 | 0 | 0 (0) | 0 |
| 1 | duplicate | 3 | 500 | 1500 | 500 | 500 | 0 | 0 | 0 | 0 | 0 | 1500 | 0 | 0 | 22.4 | 41.3 | 41.6 | 45.9 | 0 | 0 (0) | 0 |
| 2 | duplicate | 3 | 500 | 1500 | 500 | 500 | 0 | 0 | 0 | 0 | 0 | 1500 | 0 | 0 | 26.1 | 41.5 | 41.7 | 42.0 | 0 | 0 (0) | 0 |
| 3 | duplicate | 3 | 500 | 1500 | 500 | 500 | 0 | 0 | 0 | 0 | 0 | 1500 | 0 | 0 | 17.6 | 40.0 | 41.0 | 41.7 | 0 | 0 (0) | 0 |
| 4 | duplicate | 3 | 500 | 1500 | 500 | 500 | 0 | 0 | 0 | 0 | 0 | 1500 | 0 | 0 | 19.1 | 41.2 | 41.5 | 44.9 | 0 | 0 (0) | 0 |
| 5 | duplicate | 3 | 500 | 1500 | 500 | 500 | 0 | 0 | 0 | 0 | 0 | 1500 | 0 | 0 | 24.4 | 41.3 | 41.5 | 45.4 | 0 | 0 (0) | 0 |
| 1 | duplicate | 4 | 500 | 2000 | 500 | 500 | 0 | 0 | 0 | 0 | 0 | 2000 | 0 | 0 | 26.9 | 61.8 | 66.7 | 111.7 | 0 | 0 (0) | 0 |
| 2 | duplicate | 4 | 500 | 2000 | 500 | 500 | 0 | 0 | 0 | 0 | 0 | 2000 | 0 | 0 | 26.4 | 61.4 | 61.7 | 65.4 | 0 | 0 (0) | 0 |
| 3 | duplicate | 4 | 500 | 2000 | 500 | 500 | 0 | 0 | 0 | 0 | 0 | 2000 | 0 | 0 | 26.3 | 61.5 | 68.8 | 111.4 | 0 | 0 (0) | 0 |
| 4 | duplicate | 4 | 500 | 2000 | 500 | 500 | 0 | 0 | 0 | 0 | 0 | 2000 | 0 | 0 | 38.7 | 86.5 | 127.9 | 237.3 | 0 | 0 (0) | 0 |
| 5 | duplicate | 4 | 500 | 2000 | 500 | 500 | 0 | 0 | 0 | 0 | 0 | 2000 | 0 | 0 | 26.6 | 61.6 | 62.2 | 87.1 | 0 | 0 (0) | 0 |

Aggregate across runs: (latency percentiles are of per-run values; pooled raw latencies are not retained, so the pooled column is the worst per-run value)

| mode | N | runs passing strict | total writes | expected rows | persisted | lost | lock errors | other errors | exhaustions | exhaustion rate | outer retries | attempts | p50 range ms | p95 range ms | p99 range ms | worst max ms |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| distinct | 2 | 5/5 | 10000 | 10000 | 10000 | 0 | 0 | 0 | 0 | 0 / 10000 = 0.000% | 0 | 10000 | 22.5–42.6 | 44.4–94.7 | 87.3–136.5 | 241.2 |
| distinct | 3 | 5/5 | 15000 | 15000 | 15000 | 0 | 0 | 0 | 0 | 0 / 15000 = 0.000% | 0 | 15000 | 36.7–69.0 | 92.3–140.7 | 137.8–215.6 | 315.7 |
| distinct | 4 | 5/5 | 10000 | 10000 | 10000 | 0 | 0 | 0 | 0 | 0 / 10000 = 0.000% | 0 | 10000 | 43.6–54.4 | 69.2–144.5 | 156.0–220.1 | 404.2 |
| duplicate | 2 | 5/5 | 5000 | 2500 | 2500 | 0 | 0 | 0 | 0 | 0 / 5000 = 0.000% | 0 | 5000 | 15.4–27.0 | 26.5–45.7 | 26.8–93.7 | 222.9 |
| duplicate | 3 | 5/5 | 7500 | 2500 | 2500 | 0 | 0 | 0 | 0 | 0 / 7500 = 0.000% | 0 | 7500 | 17.6–26.1 | 40.0–41.5 | 41.0–41.7 | 45.9 |
| duplicate | 4 | 5/5 | 10000 | 2500 | 2500 | 0 | 0 | 0 | 0 | 0 / 10000 = 0.000% | 0 | 10000 | 26.3–38.7 | 61.4–86.5 | 61.7–127.9 | 237.3 |

Strict matrices passing: 5/5. Writes: 57500. Distinct events requested: 35000, distinct events lost: 0. Exhaustions: 0. Lock errors: 0.

Every strict write in B committed on its first attempt: 57,500 writes, 0 outer retries, and no operation ≥ 500 ms. The in-process scheduler-gap monitor recorded 0 gaps ≥ 50 ms in all 30 level-runs.

### N=8 stress results

**Stress characterization — not a supported requirement.**

| run | mode | N | rounds | total writes | expected rows | persisted rows | lost | lock errors | other errors | orphan rows | duplicate rows | attempts | outer retries | exhaustions | p50 ms | p95 ms | p99 ms | max ms | ops ≥500 ms | in-proc stall gaps ≥50 ms (max) | exit |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | distinct | 8 | 500 | 4000 | 4000 | 4000 | 0 | 0 | 0 | 0 | 0 | 4001 | 1 | 0 | 89.7 | 206.6 | 365.5 | 575.7 | 8 | 0 (0) | 0 |
| 2 | distinct | 8 | 500 | 4000 | 4000 | 4000 | 0 | 0 | 0 | 0 | 0 | 4000 | 0 | 0 | 89.8 | 191.5 | 280.1 | 576.9 | 6 | 0 (0) | 0 |
| 3 | distinct | 8 | 500 | 4000 | 4000 | 4000 | 0 | 0 | 0 | 0 | 0 | 4002 | 2 | 0 | 89.3 | 193.9 | 354.0 | 662.5 | 9 | 0 (0) | 0 |
| 4 | distinct | 8 | 500 | 4000 | 4000 | 4000 | 0 | 0 | 0 | 0 | 0 | 4001 | 1 | 0 | 87.8 | 192.0 | 256.6 | 577.8 | 2 | 0 (0) | 0 |
| 5 | distinct | 8 | 500 | 4000 | 4000 | 4000 | 0 | 0 | 0 | 0 | 0 | 4007 | 7 | 0 | 89.7 | 193.1 | 354.0 | 677.6 | 18 | 0 (0) | 0 |
| 1 | duplicate | 8 | 500 | 4000 | 500 | 500 | 0 | 0 | 0 | 0 | 0 | 4000 | 0 | 0 | 86.1 | 235.9 | 346.7 | 522.4 | 4 | 0 (0) | 0 |
| 2 | duplicate | 8 | 500 | 4000 | 500 | 500 | 0 | 16 | 0 | 0 | 0 | 4022 | 22 | 16 | 86.4 | 229.2 | 445.0 | 1834.4 | 34 | 0 (0) | 101 |
| 3 | duplicate | 8 | 500 | 4000 | 500 | 500 | 0 | 0 | 0 | 0 | 0 | 4001 | 1 | 0 | 86.1 | 188.7 | 337.8 | 559.4 | 3 | 0 (0) | 0 |
| 4 | duplicate | 8 | 500 | 4000 | 500 | 500 | 0 | 0 | 0 | 0 | 0 | 4003 | 3 | 0 | 86.1 | 192.2 | 346.1 | 615.1 | 12 | 0 (0) | 0 |
| 5 | duplicate | 8 | 500 | 4000 | 500 | 500 | 0 | 0 | 0 | 0 | 0 | 4003 | 3 | 0 | 85.3 | 194.0 | 350.6 | 602.2 | 11 | 0 (0) | 0 |

Aggregate across runs: (latency percentiles are of per-run values; pooled raw latencies are not retained, so the pooled column is the worst per-run value)

| mode | N | runs passing strict | total writes | expected rows | persisted | lost | lock errors | other errors | exhaustions | exhaustion rate | outer retries | attempts | p50 range ms | p95 range ms | p99 range ms | worst max ms |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| distinct | 8 | 5/5 | 20000 | 20000 | 20000 | 0 | 0 | 0 | 0 | 0 / 20000 = 0.000% | 11 | 20011 | 87.8–89.8 | 191.5–206.6 | 256.6–365.5 | 677.6 |
| duplicate | 8 | 4/5 | 20000 | 2500 | 2500 | 0 | 16 | 0 | 16 | 16 / 20000 = 0.080% | 29 | 20029 | 85.3–86.4 | 188.7–235.9 | 337.8–445.0 | 1834.4 |

N=8 writes: 40000. Distinct events requested: 20000, lost: 0. Exhaustions: 16. Lock errors: 16.

### Real-hook results

| run | N | rounds | expected | persisted msgs | persisted parts | lost | fail-open lost | non-zero exits | stderr outputs | stderr lines | p50 ms | p95 ms | p99 ms | max ms | procs ≥500 ms | in-proc stall gaps ≥50 ms (max) | exit |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 2 | 500 | 1000 | 1000 | 1000 | 0 | 0 | 0 | 0 | 0 | 36.2 | 77.0 | 123.9 | 223.1 | 0 | 0 (0) | 0 |
| 2 | 2 | 500 | 1000 | 1000 | 1000 | 0 | 0 | 0 | 0 | 0 | 27.5 | 38.0 | 45.2 | 64.1 | 0 | 0 (0) | 0 |
| 3 | 2 | 500 | 1000 | 1000 | 1000 | 0 | 0 | 0 | 0 | 0 | 29.6 | 51.4 | 72.3 | 134.9 | 0 | 0 (0) | 0 |
| 4 | 2 | 500 | 1000 | 1000 | 1000 | 0 | 0 | 0 | 0 | 0 | 28.7 | 42.6 | 75.7 | 173.3 | 0 | 0 (0) | 0 |
| 5 | 2 | 500 | 1000 | 1000 | 1000 | 0 | 0 | 0 | 0 | 0 | 24.5 | 37.1 | 38.3 | 46.9 | 0 | 0 (0) | 0 |
| 1 | 3 | 500 | 1500 | 1500 | 1500 | 0 | 0 | 0 | 0 | 0 | 34.2 | 52.2 | 53.1 | 60.2 | 0 | 0 (0) | 0 |
| 2 | 3 | 500 | 1500 | 1500 | 1500 | 0 | 0 | 0 | 0 | 0 | 30.7 | 52.0 | 52.8 | 59.2 | 0 | 0 (0) | 0 |
| 3 | 3 | 500 | 1500 | 1500 | 1500 | 0 | 0 | 0 | 0 | 0 | 29.1 | 52.1 | 52.7 | 72.2 | 0 | 0 (0) | 0 |
| 4 | 3 | 500 | 1500 | 1500 | 1500 | 0 | 0 | 0 | 0 | 0 | 36.5 | 52.6 | 54.1 | 72.5 | 0 | 0 (0) | 0 |
| 5 | 3 | 500 | 1500 | 1500 | 1500 | 0 | 0 | 0 | 0 | 0 | 36.1 | 52.5 | 58.0 | 105.2 | 0 | 0 (0) | 0 |
| 1 | 4 | 200 | 800 | 800 | 800 | 0 | 0 | 0 | 0 | 0 | 37.3 | 72.5 | 73.5 | 79.4 | 0 | 0 (0) | 0 |
| 2 | 4 | 200 | 800 | 800 | 800 | 0 | 0 | 0 | 0 | 0 | 51.3 | 97.5 | 129.1 | 410.9 | 0 | 0 (0) | 0 |
| 3 | 4 | 200 | 800 | 800 | 800 | 0 | 0 | 0 | 0 | 0 | 51.4 | 99.8 | 160.8 | 281.1 | 0 | 0 (0) | 0 |
| 4 | 4 | 200 | 800 | 800 | 800 | 0 | 0 | 0 | 0 | 0 | 61.8 | 139.8 | 203.0 | 542.5 | 1 | 0 (0) | 0 |
| 5 | 4 | 200 | 800 | 800 | 800 | 0 | 0 | 0 | 0 | 0 | 51.1 | 77.2 | 109.3 | 160.9 | 0 | 0 (0) | 0 |

Aggregate:

| N | runs passing strict | expected | persisted | lost | fail-open lost | non-zero exits | stderr lines | p50 range ms | p95 range ms | p99 range ms | worst max ms |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 2 | 5/5 | 5000 | 5000 | 0 | 0 | 0 | 0 | 24.5–36.2 | 37.1–77.0 | 38.3–123.9 | 223.1 |
| 3 | 5/5 | 7500 | 7500 | 0 | 0 | 0 | 0 | 29.1–36.5 | 52.0–52.6 | 52.7–58.0 | 105.2 |
| 4 | 5/5 | 4000 | 4000 | 0 | 0 | 0 | 0 | 37.3–61.8 | 72.5–139.8 | 73.5–203.0 | 542.5 |

There was no fail-open persistence loss: in no round was an event lost while the hook exited 0. One hook process took ≥ 500 ms: 542.5 ms, D-m4 N=4. Process-local contention counters do not exist across processes, so the table shows no attempt or exhaustion figures for hooks.

### Observed failures

Only one run exited non-zero: **`C-r2-duplicate-n8`** (N=8 stress, duplicate delivery, run 2 of 5, exit 101). The strict assertion failed with `8 concurrent writers produced 16 lock error(s) and 0 other error(s)`.

- 16 contention exhaustions out of 4,000 writes (0.40% of that run). They fell in 4 of 500 rounds: 272 (5), 273 (6), 274 (4) and 453 (1).
- 0 lost logical events. In every affected round at least one writer persisted or found the event (`Ok(true)`/`Ok(false)`), so `persisted rows = 500/500`. In **distinct** mode, the same exhaustions would have lost events.
- 0 orphan rows, 0 duplicate rows, no writer above 2 attempts.
- The same level ran 3 more times afterwards (C-r3, C-r4, C-r5). Each run had 0 exhaustions. The failure was not reproduced in those 3 runs, and it stays in the dataset.
- Totals for all strict-mode runs across A–D: 1 failing level-run out of 55 contention level-runs, and 0 out of 45 at supported N=2–4.

Anatomy of the failure (from the test-only timelines):

| run | round | writer | started (unix ms) | elapsed ms | attempt durations ms | backoff req/actual ms | in-proc gaps in window | host-monitor gaps in window | host psi_cpu some ms/s (max) | disk busy ms/s (max) |
|---|---|---|---|---|---|---|---|---|---|---|
| C-r2-duplicate-n8 | 272 | 2 | 1791197989232 | 1052.4 | 500.2 + 500.1 | 52/52.1 | 0 | 0 | 1.1 | 773 |
| C-r2-duplicate-n8 | 272 | 4 | 1791197989232 | 1085.4 | 500.2 + 500.2 | 85/85.1 | 0 | 0 | 1.1 | 773 |
| C-r2-duplicate-n8 | 272 | 5 | 1791197989232 | 1014.4 | 500.2 + 500.2 | 14/14.1 | 0 | 0 | 1.1 | 773 |
| C-r2-duplicate-n8 | 272 | 6 | 1791197989232 | 1039.4 | 500.2 + 500.1 | 39/39.1 | 0 | 0 | 1.1 | 773 |
| C-r2-duplicate-n8 | 272 | 7 | 1791197989232 | 1024.4 | 500.2 + 500.2 | 24/24.1 | 0 | 0 | 1.1 | 773 |
| C-r2-duplicate-n8 | 273 | 0 | 1791197991003 | 1099.5 | 500.2 + 500.1 | 99/99.1 | 0 | 0 | 0.4 | 995 |
| C-r2-duplicate-n8 | 273 | 2 | 1791197991003 | 1025.5 | 500.2 + 500.2 | 25/25.1 | 0 | 0 | 0.4 | 995 |
| C-r2-duplicate-n8 | 273 | 3 | 1791197991003 | 1062.5 | 500.2 + 500.2 | 62/62.1 | 0 | 0 | 0.4 | 995 |
| C-r2-duplicate-n8 | 273 | 4 | 1791197991003 | 1024.5 | 500.2 + 500.2 | 24/24.1 | 0 | 0 | 0.4 | 995 |
| C-r2-duplicate-n8 | 273 | 6 | 1791197991003 | 1089.5 | 500.2 + 500.2 | 89/89.0 | 0 | 0 | 0.4 | 995 |
| C-r2-duplicate-n8 | 273 | 7 | 1791197991003 | 1018.5 | 500.2 + 500.2 | 18/18.1 | 0 | 0 | 0.4 | 995 |
| C-r2-duplicate-n8 | 274 | 2 | 1791197992535 | 1050.4 | 500.2 + 500.2 | 50/50.1 | 0 | 0 | 0.4 | 993 |
| C-r2-duplicate-n8 | 274 | 3 | 1791197992535 | 1038.4 | 500.2 + 500.2 | 38/38.1 | 0 | 0 | 0.4 | 993 |
| C-r2-duplicate-n8 | 274 | 5 | 1791197992535 | 1081.4 | 500.2 + 500.2 | 81/81.0 | 0 | 0 | 0.4 | 993 |
| C-r2-duplicate-n8 | 274 | 6 | 1791197992535 | 1000.4 | 500.2 + 500.2 | 0/0.0 | 0 | 0 | 0.4 | 993 |
| C-r2-duplicate-n8 | 453 | 2 | 1791198031559 | 1093.4 | 500.2 + 500.2 | 93/93.0 | 0 | 0 | 1.4 | 515 |

- Every exhausted writer's time breaks down as `500.1–500.2 ms` attempt 1, backoff equal to the drawn jitter (≤ 0.1 ms oversleep), and `500.1–500.2 ms` attempt 2. The time outside attempts and backoff was ≤ 0.05 ms. This is Turso busy wait plus one retry. It is **not** a thread that was not scheduled.
- The writers holding the lock in those rounds spent **far longer than the busy timeout inside a single attempt** (transaction body plus commit):

  | Round | Writer | Result | Attempt durations |
  | --- | --- | --- | --- |
  | 272 | 0 | `Ok(false)` | 500.2 + 1208.5 ms |
  | 273 | 1 | `Ok(true)` | 827.6 ms |
  | 273 | 5 | `Ok(false)` | 1527.3 ms |
  | 274 | 1 | `Ok(false)` | 500.2 + 1312.2 ms |
  | 453 | 1 | `Ok(false)` | 1156.6 ms |
  | 453 | 3 | `Ok(false)` | 500.2 + 930.5 ms |

  Normal attempts in the same run were tens of ms. The waiters exhausted because some connection held the write lock (or the commit path) for more than about 1.05 s.

### Host correlation

- campaign window (Unix ms): 1791196162002–1791199102331 (49.0 min), 2938 1 s samples
- external monitor sleep-gap events ≥50 ms: 0; max 0.0 ms
- psi_cpu_some_ms: p50 0.9, p99 2.4, max 4.4
- psi_mem_some_ms: p50 0.0, p99 0.0, max 0.0
- psi_io_full_ms: p50 529.0, p99 971.3, max 976.0 — io_uring idle-wait artifact, not usable (see Test environment)
- cpu_iowait_pct: p50 9.9, p99 12.8, max 13.5 — same artifact
- disk busy ms per 1 s (max of nvme0n1/dm-0): p50 600, p99 952, max 1001
- procs_blocked: p50 3, max 5 — baseline 2 are the parked Ghostty io_uring threads

Slow-operation anatomy (all operations ≥ 500 ms):

Attempt duration is the time inside one Turso attempt (busy-handler wait included). Backoff actual vs requested shows scheduler oversleep. 'Outside' is time not inside an attempt or backoff.


##### B (strict N=2–4): 0 slow ops


##### C (N=8 stress): 107 slow ops

- Ok(false), 1 attempt(s): 34
- Ok(false), 2 attempt(s): 13
- Ok(true), 1 attempt(s): 33
- Ok(true), 2 attempt(s): 11
- database is locked, 2 attempt(s): 16
- longest single attempt: 1527.3 ms; worst backoff oversleep: 0.1 ms; worst time outside attempts/backoff: 0.05 ms

Around the failure:

- **No scheduler stall was observed.**
  - The in-process gap monitor: 0 gaps ≥ 50 ms in `C-r2-duplicate-n8`.
  - The external gap monitor: 0 events ≥ 50 ms in the failure windows, and 0 across the whole 49-minute campaign.
  - CPU PSI stayed at 0.3–1.5 ms/s, no higher than baseline.
- **Disk busy time rose sharply.** `dm-0` busy time went from about 350–390 ms/s, the run's baseline, to 773, 1000, 995, 993 and 998 ms/s. That lasted about 1791197989.9–1791197993.9 (about 4–5 s), exactly covering rounds 272–274. It then fell back to about 400 ms/s.
  - `nvme0n1` showed the same shape: 606 → 986 → 840 → 981 → 724 ms/s.
  - Round 453 coincided with a smaller rise to 515 ms/s.
- Busy ms/s on NVMe behind dm-crypt is a weak saturation signal. B-m1 distinct runs sat at a 915 ms/s median without failing, because commit fsyncs at N=2–3 keep the device permanently busy.
- The since-boot `iostat` averages show `dm-0` with `w_await ≈ 203 ms` and `aqu-sz ≈ 32`, versus `nvme0n1` with `w_await ≈ 41 ms`. This host's dm-crypt write path has heavy write-latency tails.
- Per-process IO attribution was not captured. It is therefore unknown whether the burst came from this test's own WAL/checkpoint/fsync traffic or from another writer on the same device.

Classification of `C-r2-duplicate-n8`:

- **Confirmed DB-policy exhaustion.** The policy behaved as specified, exhausting after 2 attempts at about 1.0–1.1 s against a lock held more than 1 s by a slow in-transaction attempt.
- **Correlated with a device-level IO burst.** `dm-0`/`nvme0n1` were near 100% busy for about 4–5 s.
- **No host-wide scheduler stall observed.**
- **Insufficient evidence** about the IO burst's source.

None of these waives the strict failure. The failure is outside the supported N=2–4 range.

The previous version of this doc said that 1–2 s whole-host stalls had made strict N=2–4 runs fail, and that such failures should be treated as host noise. Neither claim is supported by this campaign. No strict N=2–4 run failed, and no scheduler stall was observed. That guidance is withdrawn.

### Conclusions

#### Correctness

No correctness violation in any of the 114,070 measured write operations (A 70 boundary samples, B 57,500, C 40,000, D 16,500 hook events).

- 0 orphan message/part rows (`messages == parts` in every round and every hook round).
- 0 duplicate logical events (duplicate delivery inserted at most once in all 10,000 duplicate rounds: 7,500 at N=2–4 and 2,500 at N=8).
- 0 partial commits (every failed write left 0/0 rows).
- 0 writers above 2 attempts.

#### Availability (observed on reference host)

| Scope | Exhaustions | Lost distinct events |
| --- | --- | --- |
| Strict N=2–4 in-process | 0 / 57,500 writes (0 / 35,000 distinct events lost); 5/5 strict matrices passed | 0 |
| Real hooks N=2–4 | not countable (no cross-process counters) | 0 / 16,500 events; 0 non-zero exits; 0 fail-open losses; 15/15 level-runs passed |
| N=8 stress, distinct | 0 / 20,000 | 0 / 20,000 |
| N=8 stress, duplicate | 16 / 20,000 = 0.080%, all in 1 of 5 runs (4/5 runs passed strict) | 0 (another writer persisted each affected event) |
| Held lock, actual release ≤ 1026 ms | 0 / 50 samples | n/a |
| Held lock, actual release ≥ 1503 ms | 20 / 20 samples (by design) | n/a |

#### Latency (observed on reference host)

| Load | p50 | p95 | p99 | Worst max |
| --- | --- | --- | --- | --- |
| N=2 distinct / duplicate | 22.5–42.6 / 15.4–27.0 ms | 44–95 / 27–46 ms | 87–137 / 27–94 ms | 241 / 223 ms |
| N=3 distinct / duplicate | 36.7–69.0 / 17.6–26.1 ms | 92–141 / 40–42 ms | 138–216 / 41–42 ms | 316 / 46 ms |
| N=4 distinct / duplicate | 43.6–54.4 / 26.3–38.7 ms | 69–145 / 61–87 ms | 156–220 / 62–128 ms | 404 / 237 ms |
| N=8 stress distinct / duplicate | 87.8–89.8 / 85.3–86.4 ms | 192–207 / 189–236 ms | 257–366 / 338–445 ms | 678 / 1834 ms |
| Real hooks N=2 / 3 / 4 (incl. process start) | 24–36 / 29–37 / 37–62 ms | 37–77 / 52–53 / 73–140 ms | 38–124 / 53–58 / 74–203 ms | 223 / 105 / 543 ms |

Ranges are across the 5 runs. The worst max is the largest single write across all runs.

#### Policy decision

Overall verdict: current defaults (500 ms busy timeout / 1250 ms deadline / 2 attempts / 100 ms backoff cap) are **adequate** for the supported N=2–4 range, and **borderline at N=8 stress**.

- **Supported range.** Across 57,500 strict in-process writes and 16,500 real hook events, there was no loss and no exhaustion. B needed no outer retries at all. Tails stayed below about 0.55 s.
- **N=8 stress.** 16 exhaustions in 40,000 writes were concentrated in one about 4–5 s episode. During it, lock-holding transactions themselves took 0.8–1.5 s, coincident with a device-level IO burst.
- **No policy change is recommended from this dataset.**
  - The only failure mode observed is a holder whose transaction or commit runs more than about 1.05 s.
  - Raising `contention_deadline_ms` alone would not fix it. The exhausted writers stopped at the 2-attempt cap, not at the deadline.
  - Fixing it would need, for example, `busy_timeout_ms` ≈ 1000 with `contention_deadline_ms` ≥ about 2100, or a third attempt.
  - Either would roughly double worst-case blocked-hook latency, from about 1.1 s to about 2.1 s, for every exhausting write. The benefit would be on a load level that is not supported.
  - Whether to pay that cost is a product decision that this evidence does not force.

## Reproducing

The suite lives in `cli/src/services/agent_trace_db/lock_contention_tests.rs`, which is `#[cfg(test)]`. It drives the real production API (`insert_conversation_text_event` on hook-runtime connections) and the real release `sce hooks codex` binary against a temporary repository Agent Trace DB.

- Configuration:
  - The ignored tests take `SCE_LOCK_CONTENTION_WRITERS` (comma-separated) and `SCE_LOCK_CONTENTION_ROUNDS`.
  - The hook test also needs `SCE_BIN`.
  - `SCE_LOCK_CONTENTION_STRICT=1` enables the strict gates.
- How to run:
  - Run with `--release` and `-- --ignored --nocapture` through `nix develop -c ./scripts/run-cli-cargo.sh test`.
  - Or build once with `--no-run` and invoke the test binary directly, as above.
- Output:
  - Each test prints a human table and machine-readable `SCE_MEAS {json}` lines: `boundary_sample`, `concurrent_level`, `slow_operation`, `hook_level`, `slow_hook_process`, `hook_round_loss` and `hook_stderr`.
  - Those lines carry per-operation timelines (`record_write_contention_timeline`, test-only) and scheduler-gap monitor results.
- `count_write_contention` and the timelines are thread-local test instrumentation. They are unavailable for the hook-process test.
