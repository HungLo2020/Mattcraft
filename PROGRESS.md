# Rendering migration handoff

Updated: 2026-09-10. Latest completed rendering checkpoint: **r605**. Next unused capture number at handoff: **r606** (check before reuse).

This file is a handoff, not a declaration of completion. It was written by inspecting the current source and retained progress records; no tests or game clients were launched for this documentation task. Test results below are from the recorded completed runs, not newly rerun results.

## Read this first

- **The overall goal is NOT complete.** Do not tell the user Vulkan renders the entire supported game, Iris, or DH based on the recent shield results.
- Current work is a narrow vanilla shield/item rendering slice. Static shield milestones have real paired-game evidence. Animated shields have useful phase-matched evidence but remain **private and unadmitted**.
- **Next concrete task:** investigate GUI mesh stream retirement bookkeeping under interleaved internal uploads and multiple in-flight submissions. There is a specific suspected predicted-submission-ID hazard; it is not yet a demonstrated corruption bug. Establish a regression before changing it.
- Then finish the animated shield slice, including required patterned/foil combinations and coherent admission. Do not move to an unrelated family while this prerequisite/slice remains unfinished.
- Main repository: `/home/matt/Documents/Repos/MattMC`.
- Frozen baseline: `/home/matt/Documents/Repos/MattMC_JavaPerfTesting/MattMC`, **Java OpenGL only**.
- Both repositories can legitimately be dirty. Preserve existing work. No commits, pushes, resets, or blanket reverts.
- Detailed newest-first working diary: `logs/graphics-audit/shield-model-progress.md`. Older “running” entries are superseded by the completed entries above them.
- At the last completed checkpoint, no clients/tests/recovery jobs were live. Recheck processes before launching new work; do not assume this remains true indefinitely.

## Goal and scope

The user's ultimate goal is complete rendering through semantic callsites → Rust-implemented explicit VulkanicGAL → Rust Vulkan, including vanilla, Distant Horizons, Iris shader packs, resource packs, and their interactions. There must be no Java Vulkan rendering, hidden fallback, borrowed Iris/DH GPU runtime state, shared native GPU handles, or second presenter.

The working sequence has been narrowed into phases:

1. **Vanilla Rust Vulkan:** terrain, fluids/translucency, GUI/HUD/text, entities and layers, block entities, items/hands, particles, sky/clouds/weather, world effects, post-processing, resource packs, menus/loading, and every remaining vanilla callsite. DH and Iris shader execution are outside this phase except isolation.
2. Distant Horizons through the same Rust-owned frame.
3. Rust-owned Iris shader-pack execution, without DH.
4. Correct combined Iris + DH operation.

`docs/RUST-MIGRATION-PROMPTS.md` contains the later Goal 2–4 prompts. Phase separation does not remove DH/Iris from the ultimate objective and does not mean vanilla is complete.

Completion requires actual supported gameplay, deterministic paired Frozen OpenGL parity, clean validation, bounded resources, and proof of semantic ownership/isolation—not merely compilation, source checks, synthetic GPU scenes, or a favorable screenshot.

## Non-negotiable architecture and workflow

- Callsites supply semantic data/commands. Transitional Java extraction may supply bounded immutable CPU model/resource data; it must not become a renderer/policy engine or copy Java-selected animation frames to simulate Rust ownership.
- VulkanicGAL exposes explicit backend-neutral resources, pipelines, passes, usages, synchronization, submission, completion, and presentation. Rust Vulkan consumes that explicit state directly.
- Only the Rust OpenGL backend may privately reconstruct OpenGL's implicit state machine. Do not solve Vulkan gaps by borrowing GL/Iris/DH runtime state.
- Keep unfinished capabilities unavailable/private, with no fallback. A private diagnostic passing is not normal capability admission.
- Prefer complete narrow vertical slices. Internal defects and missing prerequisites are implementation work, not excuses to stop at an audit.
- Frozen Java OpenGL is authoritative. Never use Frozen/current Java Vulkan for correctness, acceptance, or normal testing.
- Equivalent fixtures, worlds, cameras, settings, packs, timing, and capture provenance are mandatory. Do not loosen thresholds, fit reference pixels to the candidate, freeze clocks, or hide capture defects to get green results.
- **Stop for the user's decision before treating a new suspected Frozen behavior as a bug or changing its rendering behavior.** Permission for a previous specific fix is not blanket permission.
- Watch host memory and disk. There is **no arbitrary 4 GiB limit**: the user wants memory roughly comparable to Frozen and low enough not to crash the workstation. Avoid concurrent builds/test suites and game clients.
- Generated captures/audits belong under the ignored `logs/graphics-audit/`, never loose in the repository root. Do not recreate hundreds of gigabytes of disposable fixtures.

## What has actually improved

### Broader work and limits of this handoff

The worktree contains extensive Java/Rust/harness changes beyond shields: GUI/item extraction and raster contracts, item/foil resources, atlas animation, terrain/particle semantics, entity/world rendering, and deterministic parity instrumentation. Earlier work also addressed user-reported Rust OpenGL GUI/text/loading/hand issues and Frozen terrain issues; user acknowledgments are historical feedback, not a current whole-game regression certification.

Retained family diaries include `gui-item-animation-progress.md`, `gui-item-transform-progress.md`, `gui-item-foil-progress.md`, `standard-foil-default-progress.md`, `special-item-foil-progress.md`, `item-translucent-cutout-progress.md`, `gui-atlas-whole-transaction-progress.md`, and `creeper-resource-graph-progress.md`, under `logs/graphics-audit/`. Consult their final accepted evidence and the current admission code when expanding the inventory.

**No fresh exhaustive family-by-family readiness audit was performed for this document.** Do not turn the presence of code/tests or those diary filenames into a claim that an entire family is complete. The following recent milestones have much more specific evidence.

### Static shields and shared Rust-owned atlas

- Earlier static milestones covered plain shields, patterned shields, enchanted/foil variants, and reload. The diary records normal plain admission at r577, patterned reload at r580, patterned enchanted reload at r583, and plain enchanted control at r584.
- Subsequent work fixed opaque GUI alpha/depth semantics (r590/r591) and solid-model semantic declarations (r592/r593).
- r597/r598 established shared Rust-owned SHIELD atlas source/lifecycle/reload prerequisites for GUI and held consumers. Consumers use one native atlas incarnation, not independent standalone copies of a selected frame.
- Java declares immutable full-sheet animation metadata and semantic ticks/use; Rust owns playback, interpolation, retained pixels, and native resources.
- SHIELD texture ID is signed Java `-34138350`, unsigned Rust `4260828946` (`0xfdf71712`). Nonzero signed Java IDs are valid bit patterns; do not require positivity.

### User-approved Frozen fix: animated shield classification

The discovered baseline defect was specific: non-foil GUI shield icons could retain their first cached raster while the resource-pack texture and held shield animated. The user explicitly authorized fixing Frozen.

Both repositories now have:

- `SpecialModelRenderer.isAnimated(argument)`, default false.
- `ShieldSpecialRenderer.isAnimated`, examining actually selected plain/patterned base and visible pattern materials, respecting the 16-layer limit.
- `SpecialModelWrapper` propagating animation independently of foil.
- `ShieldGuiAnimationTest` regressions for selected/static/animated/reloaded materials, ignoring an unused plain material on colored shields, wrapper/foil behavior, and the visible-layer cutoff.

This did not alter animation clocks, lighting, or selected GPU frames. Frozen's full suite and the paired r600 run verified the fix. No subsequent Frozen renderer changes are implied or authorized by it.

### Explicit native GUI item cache and lifetime fixes

- Java supplies bounded semantic item identities (`GuiItemSemanticIdentities`, maximum 64 retained identities, monotonic IDs not reused by clear) and immutable model identity snapshots.
- `GuiItemCacheRecord(identity, isAnimated)` crosses the explicit bridge; ABI layout 97 has 40 GUI mesh fields, with identity/mode at fields 38/39. Validation rejects invalid identities and unsupported cache combinations.
- Rust cache keys include generation, semantic identity, and raster extent. Composite keys distinguish item identities. Animated items rerasterize; static items reuse their native raster with explicit attachment/read usages. Reload and identity changes retire native targets.
- Discarded preparation no longer publishes an attachment layout as though it had submitted. Layout evidence comes from the target's own accepted pass use or earlier use within the same command transaction.
- A prepare/drop/retry regression found missing vertex/index uploads on retry. Transaction-scoped geometry cache keys now preserve same-transaction deduplication without treating discarded preparation as uploaded geometry.
- An actual Vulkan pixel test found a predicted first-raster submission ID could be consumed by an internal upload before the real raster. The fix **removed `raster_submission`/`mark_rasterized` prediction**: static cache proof is the private target pass's actual accepted submission, queried through `render_pass_last_submission`.
- That same test exposed a three-resource leak during partial image replacement. Teardown now releases dependent mesh descriptors before shared image views/samplers/textures, matching full teardown dependency order.

These are real correctness/resource-lifetime improvements, not just additional logging. They do **not** establish all multi-in-flight stream allocation safety.

## Current private animation status

Both flags are required for the private animated shield route:

```text
-Dmattmc.dev.rustGalShieldAtlas=true
-Dmattmc.dev.rustGalShieldAtlasAnimation=true
```

In `DevUtils/Common/graphics_harness.py`, `model_item_foil_parity_report` intentionally returns `passed: false` for `shield-animation` and `shield-animation-interpolated`. Its `phase_diagnostic` can pass while `capability_admitted` remains false. **Do not simply flip this gate to true.** Finish the missing coverage and implement a coherent evidence-driven gate.

The fixture is a resource-only three-frame shield pack: red, green, blue sheets; playback `[(2,3), (0,5), (2,2), (1,7)]`, total 17 ticks. It exercises nonzero initial sheet, repeated sheet, unequal durations, and optional interpolation.

Matching evidence uses the **retained submitted source pixels**, not just the latest clock. An empty-use tick may advance the clock while the last visible pixels remain correct. Native source traces explicitly are not GPU readbacks; final paired screenshots provide separate output evidence. Presentation correlation, source/mip hashes, owner/sprite identity, and Frozen-derived probes are all necessary.

| Run | Verified result | Admission |
| --- | --- | --- |
| r600 | Discrete phase 4: GUI and held red frame match after Frozen classification fix | Private diagnostic only |
| r601 | Interpolated phase 4: all 7 probes exact | Private diagnostic only |
| r603 | Repeated-frame phase 9: correct retained interpolated cyan despite later empty-use clock tick | Private diagnostic only |
| r604 | Cycle boundary phase 0 after 8 cycles: correct initial-declared blue sheet; source proof requires tick ≥17 and modulo 17 =0 | Private diagnostic only |
| r605 | Ordinary static shield, both private flags, reload, latest cache/teardown fixes: full manifest success, 7 probes exact | Accepted static control, not animation admission |

r603/r604 had driver exit 1 because the animation admission gate remains closed. Never assume an exit 1 is harmless without inspecting all client exits, validation, reload, source/presentation evidence, and parity reports.

## Exact next work

### 1. Prove or eliminate the mesh stream retirement suspicion

`GuiMeshGeometryResidency.last_submission` still uses predicted `gal.next_submission_id()` before possible internal GUI asset-upload submissions. The first-raster test proved such uploads can consume IDs. The stream allocator may therefore associate a range with an earlier submission than its actual draw use.

**This remains a hypothesis, not a reproduced multi-in-flight corruption failure.** The new GPU cache test waits after every frame and cannot settle it.

Inspect `gui_frontend.rs`: `reclaim_completed_mesh_geometry`, `allocate_mesh_geometry`, `pending_submission`, residency updates, and transaction handling; inspect GAL's actual accepted resource usage/completion metadata.

Create a regression with interleaved upload/draw submissions and explicit completion, then fix any demonstrated defect using actual submitted usage or a coherent prepare/submit/cancel lifetime protocol. Protect unsubmitted reservations and multiple GUI preparations in one whole-frame submission. Do not “fix” it with unconditional wait-idle, larger capacity, a global latest-ID guess, or early reclamation of pending commands.

### 2. Finish this shield slice

Complete required animated patterned/foil combinations, relevant remaining endpoint/phase cases, reload and bounded lifetime checks (including the in-flight prerequisite above). Replace the hard-closed animation gate with a tested gate only when its actual supported scope is coherent. Do not claim all shield materials based on a plain shield fixture.

### 3. Resume remaining vanilla migration, then later phases

Reconcile every vanilla rendering callsite/capability with implementation, admission, actual gameplay parity, lifecycle, and ownership evidence. Remaining whole-goal work includes any gaps in terrain/fluids, GUI/text/menus/loading, entities/layers/block entities, items/hands, particles, sky/weather/clouds, world effects, post-processing, resource packs, resize/fullscreen/world transitions, and resource retirement. This is a completion checklist, not a claim all listed systems are currently broken.

Only after vanilla completion proceed to Rust-owned DH, Rust-owned Iris, and their combined integration. Keep unsupported routes unadmitted throughout.

## Latest verification and evidence

Recorded completed suites:

- `logs/graphics-audit/shield-cache-lifetime-full-tests-r605.log`: **1718 native passed / 2 ignored; 2902 Java passed / 2 skipped**.
- `shield-cycle-harness-tests-r604.log`: **549 Python harness tests passed**.
- `shield-cycle-reference-tests-r604.log`: **10 phase/reference/pack tests passed**.
- `frozen-shield-animation-final-tests-r600.log`: **919 Frozen tests passed / 1 skipped**. No later Frozen edits were recorded.

Important actual Vulkan test in `gui_frontend.rs`:
`vulkan_item_cache_retains_pixels_moves_composition_and_invalidates_identity_and_reload`.
It checks static versus animated behavior after in-place red→green source mutation, moved composition, identity replacement, blue reload, draw counts, pixels, and created==destroyed resource counts. It waits per frame, so it is not a multi-in-flight test.

Latest paired artifact root: `logs/graphics-audit/shield-first-raster-lifetime-r605`.
Both clients completed exit 0; cross-repository/visual/static shield parity and reload passed, Rust Vulkan attribution was present, validation clean, no crash/device loss, RSS guard false. Native peak RSS was 5,064,000 KiB. This is not a demonstrated Frozen-relative memory comparison; equivalent Frozen measurements were not available for that claim.

Both r605 images were visually inspected:

- Current: `current-rust-vulkan-shaders-off/capture/run-01/capture/deterministic_camera_capture_20260910_142750_527178/01_initial.png`.
- Frozen: `frozen-opengl-shaders-off/capture/run-01/capture/deterministic_camera_capture_20260910-142849/01_initial.png`.

Other retained roots under `logs/graphics-audit/`:
`shield-animation-fixed-phase4-r600`, `shield-animation-interpolated-phase4-r601`, `shield-static-cache-layout-r602`, `shield-animation-repeat-phase9-r603`, `shield-animation-cycle-boundary-r604`.

Capture pitfalls already encountered: desktop notifications invalidated r594/r595 captures; r596 exposed a **diagnostic BGRA normalization defect**, not a rendering color defect. Frozen internal screenshot normalization was corrected according to declared texture format without changing renderer behavior. Use the completed-main-target internal screenshot path, not contaminated desktop pixels.

## Source map

| Area | Main files |
| --- | --- |
| Native GUI/cache/stream allocation | `src/main/rust/render/vulkanic/gui_frontend.rs`, `gui_mesh_frontend.rs` |
| Accepted submission/resource lifetime | `src/main/rust/render/vulkanic/gal.rs` |
| Bridge ABI | `src/main/rust/render/vulkanic/ffi/{abi.rs,layout.rs,gui.rs,tests/mod.rs}`, `src/main/java/net/vulkanic/bridge/VulkanicGalBridge.java` |
| Semantic GUI item extraction/cache identity | `src/main/java/net/vulkanic/gui/{GuiShieldItemSemanticCollector.java,GuiItemSemanticIdentities.java,RustGalGuiRawImageAssets.java}` |
| Atlas consumers and lifecycle | `src/main/java/net/vulkanic/world/{RustGalWorldPrimitiveRenderer.java,AtlasAnimationResource.java}`, `TextureAtlas`, `RustGalFrameCoordinator` |
| Frozen/current classification fix | `src/main/java/net/minecraft/client/renderer/special/{SpecialModelRenderer.java,ShieldSpecialRenderer.java}`, `renderer/item/SpecialModelWrapper.java` |
| Shared parity | `DevUtils/Common/{graphics_harness.py,capture_runner.py,shield_item_reference.py,shield_animation_reference.py}` |
| Fixture recovery | `DevUtils/Common/canonical_recovery.py` |

## Commands for the next agent

These are instructions for future work, **not commands run for this handoff**. Use unique ignored log names and check memory/disk/processes first. Run GPU/native tests serially; driver-global EGL/Vulkan/OpenAL state matters. Poll the existing process rather than restarting a still-running test.

```bash
# Full current suite; no game clients running concurrently.
CARGO_INCREMENTAL=0 ./gradlew --no-daemon test > logs/graphics-audit/<unique>-tests.log 2>&1

# Targeted native regression (replace TEST_FILTER).
CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/home/matt/Documents/Repos/MattMC/build/rust/target ALSOFT_DRIVERS=null cargo test --manifest-path src/main/rust/Cargo.toml --locked TEST_FILTER -- --test-threads=1

python3 -m unittest discover -s DevUtils/Audit -p 'test_shield_animation*.py'
python3 -m unittest discover -s DevUtils/Audit -p 'test_graphics_harness.py'
```

Paired private animated shield capture template; replace `UNIQUE_RUN` and select the intended phase/scenario. Do not overwrite previous evidence:

```bash
CARGO_INCREMENTAL=0 MATTMC_ATLAS_TRACE_TEXTURE=4260828946 MATTMC_ATLAS_TRACE_SPRITE=1 \
python3 DevUtils/Common/graphics_harness.py capture --profile extended \
  --mode current-rust-vulkan-shaders-off --mode frozen-opengl-shaders-off \
  --frozen-repo /home/matt/Documents/Repos/MattMC_JavaPerfTesting/MattMC \
  --world Origin --world-static-terrain-scenario real-world \
  --hotbar-item-fixture shield --selected-hotbar-slot 1 \
  --gui-resource-pack-scenario shield-animation-interpolated --world-resource-reload \
  --jvm-arg=-Dmattmc.dev.rustGalShieldAtlas=true \
  --jvm-arg=-Dmattmc.dev.rustGalShieldAtlasAnimation=true \
  --jvm-arg=-Dmattmc.dev.graphicsAuditShieldAnimation=true \
  --jvm-arg=-Dmattmc.dev.graphicsAuditGuiItemCapturePhase=0 \
  --jvm-arg=-Dmattmc.dev.graphicsAuditSliceMetrics=true \
  --jvm-arg=-Dmattmc.dev.deterministicCameraCapture.frozenInternalScreenshots=true \
  --rust-full-gameplay-attachments --validation standard \
  --artifact-root logs/graphics-audit/UNIQUE_RUN --artifact-dir logs/graphics-audit/UNIQUE_RUN \
  --artifact-preserve-current-run > logs/graphics-audit/UNIQUE_RUN-driver.log 2>&1
```

For ordinary static control omit the animation pack/phase/trace diagnostics, retaining both private flags, reload, and Frozen internal screenshots as appropriate to the cache path being tested.

Inspect `graphics_audit_manifest.json`, each mode's `capture/run-01/graphics_audit_artifact.json`, `paired_visual_static_terrain/visual_parity_report.json`, deterministic capture receipts, and `capture_request_01_initial.ack.json`. Read selected JSON fields: metrics can be enormous. Actual client logs are under `capture/run-01/capture/runClient_*.log`. Use `rg --files --hidden --no-ignore` to find ignored evidence.

## Artifact retention and workstation safety

At the last checkpoint disk had approximately 397 GiB available; idle host memory approximately 24 GiB available out of 31 GiB. These are historical observations, not current limits. Monitor available RAM, swap pressure, process RSS, and disk during new runs. Do not use a 4 GiB cap or claim memory parity without paired measurements.

Generated per-run canonical fixture copies through r605 were removed **only after verified recovery**. Images, reports, logs, recovery overlays, and the immutable base were retained. Each disposable run copy was about 2.6 GiB.

Immutable recovery base (do not delete):

```text
logs/graphics-audit/dh-isolated-moon-r100/.canonical-fixtures/Origin-migration-gate-real-world-vanilla-hidden-hidden-hidden-hidden-hidden-auto-dh-none-mattmc-cross-repo-fixture-v2/run
```

After both clients and driver terminate, use `DevUtils/Common/canonical_recovery.py --base BASE --source EXACT_GENERATED_RUN --output ARTIFACT/recovery`, retain its log, and verify terminal success plus a receipt matching the exact source. Only then remove that explicitly validated generated `run` directory. Never delete the immutable base or entire evidence tree. Report what was removed and whether recovery exists.

## Handoff cautions

- This root document is portable context; detailed evidence lives in ignored local files and will not automatically accompany a clone. Copy required evidence separately if the next agent works elsewhere.
- Preserve dirty Frozen diagnostics/tests and approved fixes. A dirty baseline is not itself a blocker or permission to overwrite it.
- Do not mistake supporting phase diagnostics for complete admission, source traces for GPU readback, or one static fixture for full-game parity.
- Continue implementation once the user resumes the goal; this turn was documentation only.
