#!/usr/bin/env python3
"""Run a bounded MattMC dev-client capture with diagnostic artifacts."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import platform as py_platform
import re
import shlex
import shutil
import signal
import stat
import struct
import subprocess
import sys
import time
import zlib
from dataclasses import dataclass
from pathlib import Path

import artifact_retention
from capture_window import menu_capture_window


SHADER_EVENT_PATTERN = (
    r"Using shaderpack:|Loaded Shaderpack:|shaderPack=|enableShaders=|Profile:|"
    r"Reloading pipeline on dimension change|Creating pipeline for dimension|"
    r"Skipping compute shader|Missing program .*sodium:pipeline|"
    r"Sodium Vulkan chunk pipelines|raw GLSL imports|ShaderInputParity|"
    r"LightmapInfoParity|Type is (VERTEX|FRAGMENT|GEOMETRY|COMPUTE)|"
    r"bindVertexBuffer requires an active render pass|No active Vulkan render pass to end|"
    r"shader compose into swapchain|Unexpected error|DistantHorizons|\[DH-|"
    r"DH Ready|DH Iris events|Validation Error|Validation Warning|VUID-|"
    r"UNASSIGNED-|VK_LAYER_KHRONOS_validation|GL_INVALID|OpenGL debug"
)
VALIDATION_EVENT_PATTERN = r"VK_LAYER_KHRONOS_validation|Validation Error|Validation Warning|VUID-|UNASSIGNED-"
KEY_SUMMARY_PATTERN = (
    r"Using shaderpack:|Loaded Shaderpack:|Profile:|Reloading pipeline on dimension change|"
    r"Creating pipeline for dimension|Skipping compute shader|Missing program .*sodium:pipeline|"
    r"Sodium Vulkan chunk pipelines|raw GLSL imports|ShaderInputParity|LightmapInfoParity|"
    r"bindVertexBuffer requires an active render pass|No active Vulkan render pass to end|"
    r"Unexpected error|DistantHorizons|\[DH-|DH Ready|DH Iris events|Validation Error|"
    r"Validation Warning|VUID-|UNASSIGNED-"
)
CLIENT_PROCESS_PATTERN = re.compile(r"KnotClient|devlaunchinjector|MattMC-1\.21", re.IGNORECASE)
WATER_TERRAIN_TEXTURES = (
    "assets/minecraft/textures/block/water_still.png",
    "assets/minecraft/textures/block/water_flow.png",
    "assets/minecraft/textures/block/water_overlay.png",
)
WATER_ANIMATION_SCENARIOS = {
    "water-non-sequential",
    "water-unequal-duration",
    "water-interpolation-off",
    "water-interpolation-on",
    "water-pixel-replacement",
}

DEFAULT_STATIC_TERRAIN_PARITY_MAX_SAMPLES = 512
MAX_STATIC_TERRAIN_PARITY_MAX_SAMPLES = 2_048
DEFAULT_STATIC_TERRAIN_PARITY_MAX_EVENTS = 512
MAX_STATIC_TERRAIN_PARITY_MAX_EVENTS = 8
DEFAULT_STATIC_TERRAIN_PARITY_MAX_VISIBLE_LIST_EVENTS = 8
MAX_STATIC_TERRAIN_PARITY_MAX_VISIBLE_LIST_EVENTS = 32
DEFAULT_STATIC_TERRAIN_PARITY_MAX_COVERAGE_SAMPLES = 128
MAX_STATIC_TERRAIN_PARITY_MAX_COVERAGE_SAMPLES = 1_024
DEFAULT_STATIC_TERRAIN_PARITY_MAX_COVERAGE_EVENTS = 8
MAX_STATIC_TERRAIN_PARITY_MAX_COVERAGE_EVENTS = 32
DEFAULT_STATIC_TERRAIN_PARITY_MAX_WHOLE_FRAME_COVERAGE_EVENTS = 8
MAX_STATIC_TERRAIN_PARITY_MAX_WHOLE_FRAME_COVERAGE_EVENTS = 32
DEFAULT_STATIC_TERRAIN_PARITY_MAX_PORTAL_TRACE_EVENTS = 32
MAX_STATIC_TERRAIN_PARITY_MAX_PORTAL_TRACE_EVENTS = 128
DEFAULT_STATIC_TERRAIN_PARITY_MAX_FACE_CULL_TRACE_EVENTS = 32
MAX_STATIC_TERRAIN_PARITY_MAX_FACE_CULL_TRACE_EVENTS = 128


def static_terrain_parity_max_samples() -> int:
    """Return the explicitly requested, bounded terrain-source receipt size."""
    raw_value = os.environ.get(
        "MATTMC_STATIC_TERRAIN_PARITY_MAX_SAMPLES",
        str(DEFAULT_STATIC_TERRAIN_PARITY_MAX_SAMPLES),
    )
    try:
        requested = int(raw_value)
    except ValueError:
        return DEFAULT_STATIC_TERRAIN_PARITY_MAX_SAMPLES
    return max(0, min(requested, MAX_STATIC_TERRAIN_PARITY_MAX_SAMPLES))


def static_terrain_parity_max_events() -> int:
    """Bound repeated diagnostic receipts without changing the normal default."""
    raw_value = os.environ.get(
        "MATTMC_STATIC_TERRAIN_PARITY_MAX_EVENTS",
        str(DEFAULT_STATIC_TERRAIN_PARITY_MAX_EVENTS),
    )
    try:
        requested = int(raw_value)
    except ValueError:
        return DEFAULT_STATIC_TERRAIN_PARITY_MAX_EVENTS
    return max(1, min(requested, MAX_STATIC_TERRAIN_PARITY_MAX_EVENTS))


def static_terrain_parity_max_visible_list_events() -> int:
    return static_terrain_parity_diagnostic_limit(
        "MATTMC_STATIC_TERRAIN_PARITY_MAX_VISIBLE_LIST_EVENTS",
        DEFAULT_STATIC_TERRAIN_PARITY_MAX_VISIBLE_LIST_EVENTS,
        MAX_STATIC_TERRAIN_PARITY_MAX_VISIBLE_LIST_EVENTS,
    )


def static_terrain_parity_diagnostic_limit(name: str, default: int, maximum: int, minimum: int = 1) -> int:
    """Read a bounded optional diagnostic budget shared by paired captures."""
    try:
        requested = int(os.environ.get(name, str(default)))
    except ValueError:
        return default
    return max(minimum, min(requested, maximum))


def static_terrain_parity_max_coverage_samples() -> int:
    return static_terrain_parity_diagnostic_limit(
        "MATTMC_STATIC_TERRAIN_PARITY_MAX_COVERAGE_SAMPLES",
        DEFAULT_STATIC_TERRAIN_PARITY_MAX_COVERAGE_SAMPLES,
        MAX_STATIC_TERRAIN_PARITY_MAX_COVERAGE_SAMPLES,
        0,
    )


def static_terrain_parity_max_coverage_events() -> int:
    return static_terrain_parity_diagnostic_limit(
        "MATTMC_STATIC_TERRAIN_PARITY_MAX_COVERAGE_EVENTS",
        DEFAULT_STATIC_TERRAIN_PARITY_MAX_COVERAGE_EVENTS,
        MAX_STATIC_TERRAIN_PARITY_MAX_COVERAGE_EVENTS,
    )


def static_terrain_parity_max_whole_frame_coverage_events() -> int:
    return static_terrain_parity_diagnostic_limit(
        "MATTMC_STATIC_TERRAIN_PARITY_MAX_WHOLE_FRAME_COVERAGE_EVENTS",
        DEFAULT_STATIC_TERRAIN_PARITY_MAX_WHOLE_FRAME_COVERAGE_EVENTS,
        MAX_STATIC_TERRAIN_PARITY_MAX_WHOLE_FRAME_COVERAGE_EVENTS,
    )


def static_terrain_parity_max_portal_trace_events() -> int:
    return static_terrain_parity_diagnostic_limit(
        "MATTMC_STATIC_TERRAIN_PARITY_MAX_PORTAL_TRACE_EVENTS",
        DEFAULT_STATIC_TERRAIN_PARITY_MAX_PORTAL_TRACE_EVENTS,
        MAX_STATIC_TERRAIN_PARITY_MAX_PORTAL_TRACE_EVENTS,
    )


def static_terrain_parity_max_face_cull_trace_events() -> int:
    return static_terrain_parity_diagnostic_limit(
        "MATTMC_STATIC_TERRAIN_PARITY_MAX_FACE_CULL_TRACE_EVENTS",
        DEFAULT_STATIC_TERRAIN_PARITY_MAX_FACE_CULL_TRACE_EVENTS,
        MAX_STATIC_TERRAIN_PARITY_MAX_FACE_CULL_TRACE_EVENTS,
    )


def capture_client_heap_max_mb(rss_limit_mb: int) -> int:
    """Choose a generous client heap without making RSS protection optional.

    The heap is not the process's memory footprint: Vulkan allocations,
    driver mappings, JVM native regions, and Rust-owned resources all live
    outside it. Keep the normal client maximum at 8 GiB (matching interactive
    development), while reserving at least 25% of a configured RSS budget for
    those non-heap consumers. The RSS guard remains the authoritative safety
    boundary.
    """
    if rss_limit_mb <= 0:
        return 8192
    return min(8192, max(512, int(rss_limit_mb * 0.75)))


def capture_client_heap_initial_mb(heap_max_mb: int) -> int:
    """Start captures lean while retaining the full adaptive max heap.

    A large ``-Xms`` eagerly commits Java pages before Vulkan/native work has
    a chance to establish its real footprint.  That made the capture's RSS
    look like a rendering leak and could pressure the workstation.  The
    client may still grow to ``heap_max_mb`` as gameplay genuinely requires;
    only the initial reservation is kept small and deterministic.
    """
    return min(1024, max(512, heap_max_mb // 8))


def static_terrain_base_scenario(scenario: str) -> str:
    scenario = (scenario or "").strip().lower()
    if not scenario.startswith("translucent-"):
        return scenario
    suffix = scenario.removeprefix("translucent-")
    aliases = {
        "rapid-edit": "interior-edit",
        "quiescence-stationary-performance": "steady-state-performance",
        "moving-camera-performance": "steady-state-performance",
    }
    base_scenarios = {
        "interior-edit",
        "boundary-x-edit",
        "boundary-y-edit",
        "boundary-z-edit",
        "section-reentry",
        "resource-reload",
        "opaque-texture-replacement",
        "cutout-texture-replacement",
        "pack-priority-reversal",
        "missing-atlas-payload",
        "malformed-png-payload",
        "partial-texture-update",
        "model-resource-generation-change",
        "resize-cycle",
        "swapchain-recreate",
        "world-unload-reload",
        "world-different-reload",
        "view-distance-decrease",
        "view-distance-increase",
        "camera-relocation",
        "return-visited-terrain",
        "memory-cache-soak",
        "steady-state-performance",
    }
    if suffix in aliases:
        return aliases[suffix]
    return suffix if suffix in base_scenarios else scenario


def static_terrain_requires_translucent_camera_sort(scenario: str) -> bool:
    scenario = (scenario or "").strip().lower()
    return scenario in {"translucent-overlap", "translucent-moving-camera-performance"}


@dataclass
class CaptureConfig:
    backend: str
    shaders: str
    max_secs: int
    dump_secs: int
    client_rss_limit_mb: int
    screenshot_interval_secs: int
    screenshot_max_count: int
    screenshot_start_delay_secs: int
    validation_mode: str
    shader_input_parity: str
    shader_input_parity_max_logs: int
    lightmap_info_parity_max_logs: int
    skip_tests: bool
    client_args: str
    deterministic_camera_capture: bool
    deterministic_static_camera_capture: bool
    deterministic_pose_tolerance: float
    audio_validation: bool
    capture_meshing_corpus: bool
    meshing_corpus_output: str
    meshing_corpus_fixture: str
    meshing_corpus_warmup: int
    meshing_corpus_measure: int
    artifact_dir: str
    platform_name: str | None
    world: str
    game_dir: str
    jvm_args: list[str]
    gui_resource_pack_scenario: str
    world_static_terrain_scenario: str
    world_static_terrain_second_world: str
    world_static_terrain_fault: str
    world_static_terrain_resource_pack_scenario: str
    world_static_terrain_water_animation_capture: bool
    region_validation: bool
    region_validation_copy_world: bool
    poi_validation: bool
    deterministic_shutdown_grace_secs: int = 20
    title_screen_capture: bool = False
    # Capture the ordinary reload-overlay-to-title transition from launch. This
    # is diagnostic-only: it never affects client timing or rendering.
    title_screen_transition_capture: bool = False


class CaptureRunner:
    @staticmethod
    def resolve_artifact_dir(root: Path, configured: str | None) -> Path:
        """Resolve capture output inside a repository-local managed ignored subtree.

        The standalone runner defaults to ``artifacts/graphics-captures`` while
        the cross-repository graphics matrix supplies a retention-managed
        ``logs/graphics-audit`` child.  Both are intentionally ignored and
        bounded; accepting the latter keeps matrix artifacts associated with
        their run instead of silently redirecting them to the global capture
        directory.
        """
        artifact_root = root / "artifacts" / "graphics-captures"
        # The graphics matrix allocates a retention-managed run root beneath
        # ``artifacts/`` (for example ``artifacts/low-memory-smoke/...``).
        # That entire tree is repository-local and ignored; accepting it here
        # keeps the runner's metadata beside the benchmark it describes.  The
        # narrower graphics-captures and logs/graphics-audit roots remain the
        # documented standalone defaults.
        artifact_tree = root / "artifacts"
        graphics_audit_root = root / "logs" / "graphics-audit"
        artifact_dir = Path(configured) if configured else artifact_root / "auto-capture"
        if not artifact_dir.is_absolute():
            artifact_dir = root / artifact_dir
        resolved_root = root.resolve()
        resolved_artifact_dir = artifact_dir.resolve()
        managed_roots = (
            artifact_root.resolve(),
            graphics_audit_root.resolve(),
            artifact_tree.resolve(),
        )
        if not any(
            resolved_artifact_dir == managed_root
            or managed_root in resolved_artifact_dir.parents
            for managed_root in managed_roots
        ):
            # RenderDoc and screenshot helpers can emit sidecars beside the
            # requested path. Preserve only the requested leaf name while
            # preventing root, external, and legacy dot-capture paths from
            # escaping the ignored subtree.
            leaf = resolved_artifact_dir.name
            if not leaf or leaf.startswith(".capture") or resolved_artifact_dir == resolved_root:
                leaf = "configured"
            return artifact_root / leaf
        return artifact_dir

    def __init__(self, config: CaptureConfig) -> None:
        self.config = config
        self.platform_info, self.normalize_platform = load_platform_detection()
        self.platform_name = (
            self.normalize_platform(config.platform_name) if config.platform_name else self.platform_info.platform
        )
        self.root = repo_root()
        self.gradle = gradle_command(self.root, self.platform_name)
        self.run_dir = self.root / "run"
        if config.game_dir:
            configured_game_dir = Path(config.game_dir)
            self.run_dir = configured_game_dir if configured_game_dir.is_absolute() else self.root / configured_game_dir
        self.options_file = self.run_dir / "options.txt"
        # Keep every capture product inside the repository's single ignored
        # graphics-artifact subtree.  A repository-root default is unsafe:
        # RenderDoc and screenshot helpers can emit sidecars beside the
        # requested artifact, which previously left stray ``.capture`` files
        # contaminating the project root.
        self.artifact_dir = self.resolve_artifact_dir(self.root, config.artifact_dir)
        self.artifact_dir.mkdir(parents=True, exist_ok=True)
        # Keep the coordination lock an ordinary generated artifact.  A
        # dot-prefixed name is easily mistaken for a RenderDoc ``.capture``
        # product and previously left thousands of confusing files in old
        # capture trees.
        self.lock_file = self.artifact_dir / "capture_runner.lock"
        self.lock_handle = None

        # Test suites and bounded capture batches can construct multiple
        # runners within one wall-clock second.  A second-resolution id then
        # aliases their isolated game directories and causes later runs to be
        # rejected as unsafe reuse.  Keep the human-readable timestamp while
        # adding a process-local monotonic suffix for collision-free ownership
        # of every generated artifact directory.
        self.run_id = f"{time.strftime('%Y%m%d_%H%M%S')}_{time.time_ns() % 1_000_000:06d}"
        self.start_epoch = int(time.time())
        self.git_commit = command_text(["git", "rev-parse", "HEAD"], cwd=self.root).strip() or "unknown"

        self.run_log = self.artifact_dir / f"runClient_{self.run_id}.log"
        self.meta_log = self.artifact_dir / f"meta_{self.run_id}.txt"
        self.thread_dump = self.artifact_dir / f"thread_dump_{self.run_id}.txt"
        self.process_snapshot = self.artifact_dir / f"process_snapshot_{self.run_id}.txt"
        self.system_snapshot = self.artifact_dir / f"system_snapshot_{self.run_id}.txt"
        self.git_snapshot = self.artifact_dir / f"git_snapshot_{self.run_id}.txt"
        self.shaderpack_info = self.artifact_dir / f"shaderpack_{self.run_id}.txt"
        self.shader_summary = self.artifact_dir / f"shader_summary_{self.run_id}.txt"
        self.shader_event_log = self.artifact_dir / f"shader_events_{self.run_id}.log"
        self.validation_event_log = self.artifact_dir / f"validation_events_{self.run_id}.log"
        self.latest_log_copy = self.artifact_dir / f"latest_{self.run_id}.log"
        self.latest_tail = self.artifact_dir / f"latest_tail_{self.run_id}.log"
        self.hserr_list = self.artifact_dir / f"hs_err_{self.run_id}.txt"
        self.crash_report_list = self.artifact_dir / f"crash_reports_{self.run_id}.txt"
        self.crash_report_dir = self.artifact_dir / f"crash_reports_{self.run_id}"
        self.debug_file_list = self.artifact_dir / f"debug_files_{self.run_id}.txt"
        self.debug_snapshot_dir = self.artifact_dir / f"debug_{self.run_id}"
        self.config_before_dir = self.artifact_dir / f"config_before_{self.run_id}"
        self.config_after_dir = self.artifact_dir / f"config_after_{self.run_id}"
        self.patched_shaders_snapshot = self.artifact_dir / f"patched_shaders_{self.run_id}"
        self.patched_shaders_manifest = self.artifact_dir / f"patched_shaders_manifest_{self.run_id}.txt"
        self.window_tree = self.artifact_dir / f"window_tree_{self.run_id}.txt"
        self.window_tree_dump = self.artifact_dir / f"window_tree_dump_{self.run_id}.txt"
        self.deterministic_metadata = self.artifact_dir / f"deterministic_camera_capture_{self.run_id}.json"
        self.deterministic_screenshot_dir = self.artifact_dir / f"deterministic_camera_capture_{self.run_id}"
        self.title_presented_frame_dir = self.artifact_dir / f"title_presented_frame_{self.run_id}"
        self.audio_validation_status = self.artifact_dir / f"audio_validation_{self.run_id}.json"
        self.region_validation_status = self.artifact_dir / f"region_validation_{self.run_id}.json"
        self.poi_validation_status = self.artifact_dir / f"poi_validation_{self.run_id}.json"
        self.meshing_corpus_replay = self.artifact_dir / f"real_meshing_replay_{self.run_id}.json"
        self.subsystem_status_path = (
            Path(os.environ["MATTMC_GRAPHICS_SUBSYSTEM_STATUS"])
            if os.environ.get("MATTMC_GRAPHICS_SUBSYSTEM_STATUS")
            else None
        )
        self.frame_benchmark_status_path = (
            Path(os.environ["MATTMC_GRAPHICS_FRAME_BENCHMARK_STATUS"])
            if os.environ.get("MATTMC_GRAPHICS_FRAME_BENCHMARK_STATUS")
            else None
        )

        self.validation_layer_manifest = ""
        self.validation_layer_dir = ""
        self.validation_layer_available = False
        self.validation_enabled = False
        self.client_arg_shader_pack = ""
        self.client_arg_enable_shaders = ""
        self.iris_property_shader_pack = ""
        self.iris_property_enable_shaders = ""
        self.iris_property_color_space = ""
        self.effective_shader_pack = ""
        self.effective_enable_shaders = ""
        self.screenshot_count = 0
        self.screenshot_enabled = False
        self.gradle_process: subprocess.Popen[bytes] | None = None
        self.run_client_active = False
        self.dump_taken = False
        self.timed_out = False
        self.memory_guard_triggered = False
        self.memory_guard_peak_rss_kb = 0
        self.memory_guard_rss_kb = 0
        self.deterministic_completed = False
        self.intentional_deterministic_shutdown = False
        self.title_screen_capture_completed = False
        self.title_screen_receipt_observed = False
        self.title_screen_receipt_wait_reported = False
        self.intentional_title_screen_shutdown = False
        self.subsystem_benchmark_completed = False
        self.intentional_subsystem_shutdown = False
        self.awaiting_frame_benchmark_after_capture = False
        self.deterministic_validation_status = "not_requested"
        self.audio_validation_result = "not_requested"
        self.region_validation_result = "not_requested"
        self.poi_validation_result = "not_requested"
        self.env = os.environ.copy()
        self.isolated_game_source: Path | None = None
        self.isolated_game_dir: Path | None = None
        self.isolated_world_copy_meta: list[str] = []
        self.region_validation_game_dir: Path | None = None
        self.generated_gui_resource_packs: list[str] = []

    def run(self) -> int:
        self.acquire_lock()
        self.preflight_existing_clients()
        self.prepare_isolated_game_dir()
        self.prepare_region_validation_game_dir()
        self.write_initial_meta()
        self.configure_gui_resource_pack_scenario()
        self.configure_screenshots()
        self.configure_backend_and_validation()
        self.configure_client_args()
        self.prepare_snapshots()
        self.start_gradle()

        exit_code = self.monitor_gradle()
        self.run_client_active = False

        if self.config.deterministic_camera_capture and self.deterministic_capture_status() == "complete":
            self.deterministic_completed = True
            if exit_code != 0:
                self.intentional_deterministic_shutdown = True

        if self.intentional_deterministic_shutdown and exit_code != 0:
            self.append_meta(f"raw_exit_code={exit_code}")
            exit_code = 0
        if self.intentional_subsystem_shutdown and exit_code != 0:
            self.append_meta(f"raw_exit_code={exit_code}")
            exit_code = 0
        if self.intentional_title_screen_shutdown and exit_code != 0:
            self.append_meta(f"raw_exit_code={exit_code}")
            exit_code = 0

        self.append_final_meta(exit_code)
        self.collect_final_artifacts()
        self.validate_deterministic_capture()
        self.validate_audio_status()
        self.validate_region_status()
        self.validate_poi_status()
        self.cleanup_generated_game_dirs()
        self.print_summary(exit_code)
        self.release_lock()

        if self.memory_guard_triggered:
            return 124
        if self.config.deterministic_camera_capture and self.deterministic_validation_status != "ok":
            return 2
        if self.config.audio_validation and self.audio_validation_result != "ok":
            return 3
        if self.config.region_validation and self.region_validation_result != "ok":
            return 4
        if self.config.poi_validation and self.poi_validation_result != "ok":
            return 5
        return 0

    def acquire_lock(self) -> None:
        self.lock_file.parent.mkdir(parents=True, exist_ok=True)
        handle = self.lock_file.open("a+", encoding="utf-8")
        try:
            handle.seek(0)
            if os.name == "nt":
                import msvcrt

                try:
                    msvcrt.locking(handle.fileno(), msvcrt.LK_NBLCK, 1)
                except OSError as exc:
                    raise SystemExit(lock_refusal_message()) from exc
            else:
                import fcntl

                try:
                    fcntl.flock(handle.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
                except OSError as exc:
                    raise SystemExit(lock_refusal_message()) from exc

            handle.seek(0)
            handle.truncate()
            handle.write(f"pid={os.getpid()}\nstarted_epoch={int(time.time())}\n")
            handle.flush()
            self.lock_handle = handle
        except Exception:
            handle.close()
            raise

    def release_lock(self) -> None:
        if not self.lock_handle:
            return
        try:
            if os.name == "nt":
                import msvcrt

                self.lock_handle.seek(0)
                msvcrt.locking(self.lock_handle.fileno(), msvcrt.LK_UNLCK, 1)
            else:
                import fcntl

                fcntl.flock(self.lock_handle.fileno(), fcntl.LOCK_UN)
        finally:
            self.lock_handle.close()
            self.lock_handle = None

    def cleanup_generated_game_dirs(self) -> None:
        if os.environ.get("MATTMC_CAPTURE_PRESERVE_ISOLATED_GAME_DIR", "false").lower() == "true":
            self.append_meta("artifact_retention_game_dir_cleanup=preserved_by_explicit_diagnostic_request")
            return
        marker_root = artifact_retention.nearest_marked_root(self.artifact_dir)
        if marker_root is None:
            self.append_meta("artifact_retention_game_dir_cleanup=skipped_no_marker")
            return
        removed: list[Path] = []
        for path in (self.isolated_game_dir, self.region_validation_game_dir):
            if path is None or not path.exists():
                continue
            try:
                artifact_retention.remove_path(marker_root, path)
                removed.append(path)
                parent = path.parent
                if parent.name == self.run_id and parent.parent.name == ".tmp" and parent.exists() and not any(parent.iterdir()):
                    artifact_retention.remove_path(marker_root, parent)
            except Exception as exc:
                self.append_meta(f"artifact_retention_game_dir_cleanup_error={exc}")
        self.append_meta(f"artifact_retention_game_dirs_removed={len(removed)}")
        for path in removed:
            self.append_meta(f"artifact_retention_removed_game_dir={path}")

    def write_initial_meta(self) -> None:
        save_fingerprint = save_state_fingerprint(self.run_dir, self.config.world)
        lines = [
            f"run_id={self.run_id}",
            f"start_epoch={self.start_epoch}",
            f"backend={self.config.backend}",
            f"shaders={self.config.shaders}",
            f"world={self.config.world}",
            f"world_profile={os.environ.get('MATTMC_GRAPHICS_WORLD_PROFILE', 'migration-gate')}",
            f"world_profile_role={os.environ.get('MATTMC_GRAPHICS_WORLD_PROFILE_ROLE', '')}",
            f"migration_gate_blocking={os.environ.get('MATTMC_GRAPHICS_MIGRATION_GATE_BLOCKING', 'true')}",
            f"world_save_state_hash={save_fingerprint['hash']}",
            f"world_save_state_file_count={save_fingerprint['file_count']}",
            f"world_save_state_total_bytes={save_fingerprint['total_bytes']}",
            f"world_save_state_truncated={str(save_fingerprint['truncated']).lower()}",
            f"game_dir_initial={self.run_dir}",
            f"parity_fixture_schema={os.environ.get('MATTMC_PARITY_FIXTURE_SCHEMA', '')}",
            f"parity_fixture_id={os.environ.get('MATTMC_PARITY_FIXTURE_ID', '')}",
            f"parity_fixture_manifest={os.environ.get('MATTMC_PARITY_FIXTURE_MANIFEST', '')}",
            f"parity_fixture_source_save_hash={os.environ.get('MATTMC_PARITY_FIXTURE_SOURCE_SAVE_HASH', '')}",
            f"parity_camera_x={os.environ.get('MATTMC_PARITY_CAMERA_X', '')}",
            f"parity_camera_y={os.environ.get('MATTMC_PARITY_CAMERA_Y', '')}",
            f"parity_camera_z={os.environ.get('MATTMC_PARITY_CAMERA_Z', '')}",
            f"parity_camera_yaw={os.environ.get('MATTMC_PARITY_CAMERA_YAW', '')}",
            f"parity_camera_pitch={os.environ.get('MATTMC_PARITY_CAMERA_PITCH', '')}",
            f"parity_camera_pose_sequence={os.environ.get('MATTMC_PARITY_CAMERA_POSE_SEQUENCE', '')}",
            f"world_static_terrain_scenario={self.config.world_static_terrain_scenario}",
            f"world_static_terrain_second_world={self.config.world_static_terrain_second_world}",
            f"world_static_terrain_fault={self.config.world_static_terrain_fault}",
            f"world_static_terrain_resource_pack_scenario={self.config.world_static_terrain_resource_pack_scenario}",
            f"world_static_terrain_water_animation_capture={str(self.config.world_static_terrain_water_animation_capture).lower()}",
            f"region_validation={str(self.config.region_validation).lower()}",
            f"region_validation_copy_world={str(self.config.region_validation_copy_world).lower()}",
            f"poi_validation={str(self.config.poi_validation).lower()}",
            f"max_secs={self.config.max_secs}",
            f"dump_secs={self.config.dump_secs}",
            f"client_rss_limit_mb={self.config.client_rss_limit_mb}",
            f"validation_mode={self.config.validation_mode}",
            f"graphics_run_type={os.environ.get('MATTMC_GRAPHICS_RUN_TYPE', 'clean-performance')}",
            f"graphics_audit_enabled={os.environ.get('MATTMC_GRAPHICS_AUDIT', 'false')}",
            f"graphics_subsystem_benchmark={os.environ.get('MATTMC_GRAPHICS_SUBSYSTEM_BENCHMARK', 'false')}",
            f"graphics_subsystem_status={self.subsystem_status_path or ''}",
            f"validation_profile={os.environ.get('MATTMC_GRAPHICS_VALIDATION_PROFILE', self.config.validation_mode)}",
            f"validation_fail_severity={os.environ.get('MATTMC_GRAPHICS_VALIDATION_FAIL_SEVERITY', 'warning')}",
            f"renderdoc_capture={os.environ.get('MATTMC_RENDERDOC_CAPTURE', 'false')}",
            f"renderdoc_frame={os.environ.get('MATTMC_RENDERDOC_FRAME', '0')}",
            f"renderdoc_capture_path={os.environ.get('MATTMC_RENDERDOC_CAPTURE_PATH', '')}",
            f"renderdoc_vulkan_layer_manifest={os.environ.get('MATTMC_RENDERDOC_VULKAN_LAYER_MANIFEST', '')}",
            f"tracy_capture={os.environ.get('MATTMC_TRACY_CAPTURE', 'false')}",
            f"rust_tracy={os.environ.get('MATTMC_RUST_TRACY', 'false')}",
            f"tracy_duration_seconds={os.environ.get('MATTMC_TRACY_DURATION_SECONDS', '0')}",
            f"tracy_max_size_mb={os.environ.get('MATTMC_TRACY_MAX_SIZE_MB', '0')}",
            f"deterministic_shutdown_grace_secs={self.config.deterministic_shutdown_grace_secs}",
            f"shader_input_parity={self.config.shader_input_parity}",
            f"shader_input_parity_max_logs={self.config.shader_input_parity_max_logs}",
            f"lightmap_info_parity_max_logs={self.config.lightmap_info_parity_max_logs}",
            f"skip_tests={str(self.config.skip_tests).lower()}",
            f"deterministic_camera_capture={str(self.config.deterministic_camera_capture).lower()}",
            f"deterministic_pose_tolerance={self.config.deterministic_pose_tolerance}",
            f"capture_meshing_corpus={str(self.config.capture_meshing_corpus).lower()}",
            f"meshing_corpus_output={self.config.meshing_corpus_output or self.meshing_corpus_replay}",
            f"meshing_corpus_fixture={self.config.meshing_corpus_fixture or 'all'}",
            f"meshing_corpus_warmup={self.config.meshing_corpus_warmup}",
            f"meshing_corpus_measure={self.config.meshing_corpus_measure}",
            f"client_args_initial={self.config.client_args}",
        ]
        if self.isolated_game_source is not None and self.isolated_game_dir is not None:
            lines.extend(
                [
                    f"isolated_game_source={self.isolated_game_source}",
                    f"isolated_game_dir={self.isolated_game_dir}",
                    f"isolated_world_source={os.environ.get('MATTMC_CAPTURE_WORLD_SOURCE') or self.isolated_game_source / 'saves' / self.config.world}",
                    f"isolated_world_copy={self.isolated_game_dir / 'saves' / self.config.world}",
                ]
            )
            lines.extend(self.isolated_world_copy_meta)
        self.meta_log.write_text("\n".join(lines) + "\n", encoding="utf-8")

    def append_meta(self, text: str) -> None:
        with self.meta_log.open("a", encoding="utf-8") as handle:
            handle.write(text.rstrip("\n") + "\n")

    def append_isolated_world_copy_meta(self, text: str) -> None:
        self.isolated_world_copy_meta.append(text.rstrip("\n"))
        self.append_meta(text)

    def generated_game_temp_root(self) -> Path:
        marker_root = artifact_retention.nearest_marked_root(self.artifact_dir)
        if marker_root is None:
            return self.artifact_dir
        temp_root = marker_root / ".tmp" / self.run_id
        temp_root.mkdir(parents=True, exist_ok=True)
        return temp_root

    def static_terrain_second_world_name(self) -> str:
        configured = (self.config.world_static_terrain_second_world or "").strip()
        if not configured:
            configured = os.environ.get("MATTMC_CAPTURE_SECOND_WORLD", "").strip()
        scenario = static_terrain_base_scenario(self.config.world_static_terrain_scenario)
        if not configured and scenario == "world-different-reload":
            configured = f"{self.config.world}-different"
        return configured

    @staticmethod
    def validate_world_copy_name(name: str, label: str) -> None:
        if not name or Path(name).name != name or name in {".", ".."}:
            raise SystemExit(f"Invalid {label} copied-world destination name: {name!r}")

    @staticmethod
    def tree_file_count_and_bytes(path: Path) -> tuple[int, int]:
        file_count = 0
        total_bytes = 0
        for entry in path.rglob("*"):
            if entry.is_file():
                file_count += 1
                total_bytes += entry.stat().st_size
        return file_count, total_bytes

    @staticmethod
    def ignored_world_copy_name(directory: str, name: str) -> bool:
        """Exclude stale DH snapshots, or all DH state for explicit no-DH rows.

        Development captures sometimes leave multi-hundred-megabyte SQLite
        backups beside the active ``DistantHorizons.sqlite``.  They are not
        part of the world state and copying them needlessly delays quick-play
        startup (or exhausts the bounded capture budget).  The live database is
        excluded only when the caller explicitly disables DH for an ordinary
        source capture; dedicated DH rows retain it.
        """
        if Path(directory).name != "data":
            return False
        if (
            os.environ.get("MATTMC_CAPTURE_DISABLE_DH_FOR_ORDINARY_SOURCE", "false").lower() == "true"
            and name == "DistantHorizons.sqlite"
        ):
            return True
        return name.startswith("DistantHorizons.sqlite.") and (
            ".backup_" in name or ".generated_" in name or ".corrupt_" in name
        )

    @classmethod
    def tree_file_count_and_bytes_for_copy(cls, path: Path) -> tuple[int, int, int]:
        file_count = 0
        total_bytes = 0
        ignored_count = 0
        for entry in path.rglob("*"):
            if not entry.is_file():
                continue
            if cls.ignored_world_copy_name(str(entry.parent), entry.name):
                ignored_count += 1
            else:
                file_count += 1
                total_bytes += entry.stat().st_size
        return file_count, total_bytes, ignored_count

    def copy_isolated_world(self, source: Path, saves_dir: Path, destination_name: str, label: str) -> tuple[Path, int, int]:
        self.validate_world_copy_name(destination_name, label)
        if not source.is_dir():
            raise SystemExit(f"Cannot copy missing {label} benchmark world: {source}")
        destination = saves_dir / destination_name
        if destination.exists():
            raise SystemExit(f"Refusing to overwrite existing {label} copied world: {destination}")
        max_copy_bytes = int(os.environ.get("MATTMC_CAPTURE_WORLD_COPY_MAX_BYTES", str(8 * 1024 * 1024 * 1024)))
        source_file_count, source_bytes, ignored_file_count = self.tree_file_count_and_bytes_for_copy(source)
        if source_bytes > max_copy_bytes:
            raise SystemExit(
                f"Refusing to copy oversized {label} world: {source_bytes} bytes exceeds {max_copy_bytes} byte limit"
            )
        shutil.copytree(
            source,
            destination,
            ignore=lambda directory, names: [
                name for name in names if self.ignored_world_copy_name(directory, name)
            ],
        )
        copied_file_count, copied_bytes = self.tree_file_count_and_bytes(destination)
        if copied_file_count != source_file_count or copied_bytes != source_bytes:
            raise SystemExit(
                f"Incomplete {label} world copy: source files/bytes={source_file_count}/{source_bytes} "
                f"copied files/bytes={copied_file_count}/{copied_bytes}"
            )
        if ignored_file_count:
            self.append_isolated_world_copy_meta(f"isolated_{label}_world_ignored_stale_dh_files={ignored_file_count}")
        self.append_isolated_world_copy_meta(f"isolated_{label}_world_source={source}")
        self.append_isolated_world_copy_meta(f"isolated_{label}_world_name={destination_name}")
        self.append_isolated_world_copy_meta(f"isolated_{label}_world_copy={destination}")
        self.append_isolated_world_copy_meta(f"isolated_{label}_world_status=ok")
        self.append_isolated_world_copy_meta(f"isolated_{label}_world_files={copied_file_count}")
        self.append_isolated_world_copy_meta(f"isolated_{label}_world_bytes={copied_bytes}")
        return destination, copied_file_count, copied_bytes

    def prepare_isolated_game_dir(self) -> None:
        if self.config.game_dir or self.config.region_validation_copy_world:
            return
        source_run = Path(os.environ["MATTMC_CAPTURE_RUN_SOURCE"]) if os.environ.get("MATTMC_CAPTURE_RUN_SOURCE") else self.root / "run"
        source_world = Path(os.environ["MATTMC_CAPTURE_WORLD_SOURCE"]) if os.environ.get("MATTMC_CAPTURE_WORLD_SOURCE") else source_run / "saves" / self.config.world
        if not source_world.is_dir():
            raise SystemExit(f"Cannot copy missing benchmark world: {source_world}")
        isolated_game_dir = self.generated_game_temp_root() / f"game_dir_{self.run_id}"
        if isolated_game_dir.exists():
            raise SystemExit(f"Refusing to reuse existing isolated game dir: {isolated_game_dir}")

        isolated_game_dir.mkdir(parents=True)
        for name in (
            "options.txt",
            "config",
            "resourcepacks",
            "assets",
            "shaderpacks",
            "Distant_Horizons_server_data",
            "voxelmap",
        ):
            self.copy_optional_path(source_run / name, isolated_game_dir / name)
        if os.environ.get("MATTMC_CAPTURE_RESET_DH_DATABASE", "false").lower() == "true":
            server_data = isolated_game_dir / "Distant_Horizons_server_data"
            if server_data.exists():
                # The DH server-side cache can repopulate a freshly deleted
                # client DB with legacy columns. Remove it only from this
                # bounded isolated copy so both paired rows rebuild from the
                # canonical world fixture.
                shutil.rmtree(server_data)
                self.append_isolated_world_copy_meta("isolated_dh_server_data_reset=true")
        saves_dir = isolated_game_dir / "saves"
        saves_dir.mkdir(parents=True)
        self.copy_isolated_world(source_world, saves_dir, self.config.world, "primary")
        if os.environ.get("MATTMC_CAPTURE_RESET_DH_DATABASE", "false").lower() == "true":
            # Dedicated DH correctness rows must build their copied columns
            # from the fixture instead of silently reusing a legacy SQLite
            # snapshot that predates semantic contributor provenance.  Only
            # the isolated copy is touched; the source world and Frozen repo
            # remain unchanged.
            removed = 0
            for database in (saves_dir / self.config.world).rglob("DistantHorizons.sqlite"):
                database.unlink()
                removed += 1
            self.append_isolated_world_copy_meta(f"isolated_dh_databases_reset={removed}")
        second_world = self.static_terrain_second_world_name()
        if second_world:
            if second_world == self.config.world:
                raise SystemExit(
                    f"Secondary copied-world destination must differ from primary world: {second_world}"
                )
            second_source = Path(os.environ["MATTMC_CAPTURE_SECOND_WORLD_SOURCE"]) if os.environ.get("MATTMC_CAPTURE_SECOND_WORLD_SOURCE") else source_world
            self.copy_isolated_world(second_source, saves_dir, second_world, "secondary")
            if not (saves_dir / second_world).is_dir():
                raise SystemExit(f"Secondary copied world was requested but is absent before launch: {saves_dir / second_world}")
        elif static_terrain_base_scenario(self.config.world_static_terrain_scenario) == "world-different-reload":
            raise SystemExit("--world-static-terrain-scenario world-different-reload requires a secondary copied world")
        saves_listing = sorted(path.name for path in saves_dir.iterdir() if path.is_dir())
        self.append_isolated_world_copy_meta(f"isolated_saves_listing={json.dumps(saves_listing, separators=(',', ':'))}")

        self.run_dir = isolated_game_dir
        self.options_file = self.run_dir / "options.txt"
        self.isolated_game_source = source_run
        self.isolated_game_dir = isolated_game_dir

    def prepare_region_validation_game_dir(self) -> None:
        if not self.config.region_validation_copy_world:
            return
        if self.config.game_dir:
            raise SystemExit("--region-validation-copy-world cannot be combined with --game-dir")
        source_world = self.root / "run" / "saves" / self.config.world
        if not source_world.is_dir():
            raise SystemExit(f"Cannot copy missing validation world: {source_world}")
        validation_game_dir = self.generated_game_temp_root() / f"region_validation_game_{self.run_id}"
        if validation_game_dir.exists():
            raise SystemExit(f"Refusing to reuse existing validation game dir: {validation_game_dir}")

        validation_game_dir.mkdir(parents=True)
        self.copy_optional_path(self.root / "run" / "options.txt", validation_game_dir / "options.txt")
        self.copy_optional_path(self.root / "run" / "config", validation_game_dir / "config")
        self.copy_optional_path(self.root / "run" / "resourcepacks", validation_game_dir / "resourcepacks")
        (validation_game_dir / "saves").mkdir(parents=True)
        shutil.copytree(source_world, validation_game_dir / "saves" / self.config.world)
        if self.config.poi_validation:
            fixture_path = write_poi_validation_fixture(validation_game_dir / "saves" / self.config.world)
            self.append_meta(f"poi_validation_fixture={fixture_path}")
        self.run_dir = validation_game_dir
        self.region_validation_game_dir = validation_game_dir
        self.options_file = self.run_dir / "options.txt"
        self.append_meta(f"validation_world_source={source_world}")
        self.append_meta(f"validation_game_dir={self.run_dir}")
        self.append_meta(f"validation_world_copy={self.run_dir / 'saves' / self.config.world}")

    def configure_gui_resource_pack_scenario(self) -> None:
        gui_scenario = (self.config.gui_resource_pack_scenario or "").strip().lower()
        terrain_scenario = (self.config.world_static_terrain_resource_pack_scenario or "").strip().lower()
        if gui_scenario and terrain_scenario and gui_scenario != terrain_scenario:
            raise SystemExit(
                "--gui-resource-pack-scenario and --world-static-terrain-resource-pack-scenario "
                "must match when both are provided; static terrain uses the same generated block-texture packs"
            )
        scenario = terrain_scenario or gui_scenario
        if not scenario:
            return
        if self.config.game_dir:
            raise SystemExit("--gui-resource-pack-scenario requires the isolated game directory path")
        if scenario in {"none", "vanilla"}:
            upsert_option(self.options_file, "resourcePacks", "[]")
            upsert_option(self.options_file, "incompatibleResourcePacks", "[]")
            self.append_meta(f"gui_resource_pack_scenario={scenario}")
            self.append_meta(f"world_static_terrain_resource_pack_scenario={terrain_scenario or ''}")
            self.append_meta("gui_resource_pack_selected=[]")
            return

        pack_root = self.run_dir / "resourcepacks"
        pack_root.mkdir(parents=True, exist_ok=True)
        specs = gui_resource_pack_specs(scenario)
        selected: list[str] = []
        for spec in specs:
            pack_dir = pack_root / spec["name"]
            write_gui_resource_pack(pack_dir, spec)
            pack_id = f"file/{spec['name']}"
            selected.append(pack_id)
            self.generated_gui_resource_packs.append(pack_id)
            self.append_meta(f"gui_resource_pack_generated={pack_id}")
            self.append_meta(f"gui_resource_pack_{spec['name']}_sha256={directory_digest(pack_dir)}")
        upsert_option(self.options_file, "resourcePacks", json.dumps(selected, separators=(",", ":")))
        upsert_option(self.options_file, "incompatibleResourcePacks", "[]")
        self.append_meta(f"gui_resource_pack_scenario={scenario}")
        self.append_meta(f"world_static_terrain_resource_pack_scenario={terrain_scenario or ''}")
        self.append_meta(f"gui_resource_pack_selected={json.dumps(selected, separators=(',', ':'))}")

    def copy_optional_path(self, source: Path, target: Path) -> None:
        if not source.exists():
            return
        if source.is_dir():
            shutil.copytree(source, target)
        else:
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, target)

    def preflight_existing_clients(self) -> None:
        clients = find_existing_mattmc_clients()
        if not clients:
            return
        self.append_meta("preflight_existing_clients=true")
        for client in clients:
            self.append_meta(f"preflight_existing_client={client}")
        print("Refusing to start the graphics capture runner because a MattMC client is already running:", file=sys.stderr)
        print("\n".join(clients), file=sys.stderr)
        print("Stop the existing client first; concurrent runs can lock Origin and corrupt capture results.", file=sys.stderr)
        raise SystemExit(1)

    def configure_screenshots(self) -> None:
        self.screenshot_enabled = screenshot_backend_available(self.platform_name)
        if self.screenshot_enabled:
            self.append_meta("screenshot_enabled=true")
            self.append_meta(f"screenshot_interval_secs={self.config.screenshot_interval_secs}")
            self.append_meta(f"screenshot_max_count={self.config.screenshot_max_count}")
            self.append_meta(f"screenshot_start_delay_secs={self.config.screenshot_start_delay_secs}")
            self.append_meta(f"display={os.environ.get('DISPLAY', 'unset')}")
            if self.platform_name == "linux" and shutil.which("xwininfo"):
                write_command(self.window_tree, ["xwininfo", "-root", "-tree"], cwd=self.root)
        else:
            self.append_meta("screenshot_enabled=false")
            self.append_meta(f"display={os.environ.get('DISPLAY', 'unset')}")

        if self.config.deterministic_camera_capture and not self.screenshot_enabled:
            raise SystemExit(
                "--deterministic-camera-capture requires a supported screenshot tool "
                "for this platform"
            )
        if self.config.deterministic_camera_capture:
            self.config.screenshot_interval_secs = 0
            self.append_meta("deterministic_wall_clock_screenshots=false")
            self.append_meta("screenshot_interval_secs_effective=0")

    def configure_backend_and_validation(self) -> None:
        if self.config.backend == "rust-vulkan":
            launch_backend = "vulkan"
        elif self.config.backend == "rust-opengl":
            launch_backend = "opengl"
        else:
            launch_backend = self.config.backend
        upsert_property(self.options_file, "graphics_backend", launch_backend)
        gui_scale = os.environ.get("MATTMC_CAPTURE_GUI_SCALE", "3")
        if not gui_scale.isdigit() or int(gui_scale) <= 0:
            raise SystemExit(f"MATTMC_CAPTURE_GUI_SCALE must be a positive integer, got {gui_scale!r}")
        hide_gui = os.environ.get("MATTMC_CAPTURE_HIDE_GUI", "false").lower()
        if hide_gui not in {"true", "false"}:
            raise SystemExit(
                "MATTMC_CAPTURE_HIDE_GUI must be true or false, "
                f"got {hide_gui!r}"
            )
        max_fps = os.environ.get("MATTMC_CAPTURE_MAX_FPS", "120")
        if not max_fps.isdigit() or int(max_fps) <= 0:
            raise SystemExit(f"MATTMC_CAPTURE_MAX_FPS must be a positive integer, got {max_fps!r}")
        # `PanoramaTheme` is a serialized vanilla enum.  "default" is not an
        # enum element (the game's default is AQUATIC), so writing it makes a
        # capture depend on Options' error recovery and can select a stale
        # panorama from a copied fixture instead of the declared one.
        panorama_theme = os.environ.get("MATTMC_CAPTURE_PANORAMA_THEME", "aquatic")
        if panorama_theme not in {
            "aquatic", "caves", "copper_age", "nether", "release",
            "spring_to_life", "tricky_trials",
        }:
            raise SystemExit(f"MATTMC_CAPTURE_PANORAMA_THEME must be a vanilla theme, got {panorama_theme!r}")
        render_distance = os.environ.get("MATTMC_CAPTURE_RENDER_DISTANCE", "10")
        simulation_distance = os.environ.get("MATTMC_CAPTURE_SIMULATION_DISTANCE", "12")
        if not render_distance.isdigit() or int(render_distance) <= 0:
            raise SystemExit(f"MATTMC_CAPTURE_RENDER_DISTANCE must be a positive integer, got {render_distance!r}")
        if not simulation_distance.isdigit() or int(simulation_distance) <= 0:
            raise SystemExit(f"MATTMC_CAPTURE_SIMULATION_DISTANCE must be a positive integer, got {simulation_distance!r}")
        title_static_fixture = os.environ.get("MATTMC_CAPTURE_TITLE_STATIC_FIXTURE", "false").lower()
        if title_static_fixture not in {"true", "false"}:
            raise SystemExit(
                "MATTMC_CAPTURE_TITLE_STATIC_FIXTURE must be true or false, "
                f"got {title_static_fixture!r}"
            )
        forced_options = {
            "renderDistance": render_distance,
            "simulationDistance": simulation_distance,
            "guiScale": gui_scale,
            "fullscreen": "false",
            "hideGui": hide_gui,
            "maxFps": max_fps,
            "enableVsync": "false",
            "tutorialStep": "none",
            "panoramaTheme": f'"{panorama_theme}"',
        }
        mipmaps = os.environ.get("MATTMC_CAPTURE_MIPMAP_LEVELS")
        if mipmaps is not None:
            if mipmaps not in {"0", "1", "2", "3", "4"}:
                raise SystemExit("MATTMC_CAPTURE_MIPMAP_LEVELS must be 0 through 4")
            forced_options["mipmapLevels"] = mipmaps
        if self.config.title_screen_capture:
            menu_blur = os.environ.get("MATTMC_CAPTURE_MENU_BACKGROUND_BLUR", "5")
            if not menu_blur.isdigit() or not 0 <= int(menu_blur) <= 10:
                raise SystemExit("MATTMC_CAPTURE_MENU_BACKGROUND_BLUR must be an integer from 0 to 10")
            forced_options["menuBackgroundBlurriness"] = menu_blur
        if self.config.title_screen_capture and title_static_fixture == "true":
            # A title parity fixture needs a stable vanilla scene, not a
            # wall-clock-dependent spin or randomly chosen splash. This is
            # written only into the isolated capture copy for both routes.
            forced_options.update({
                "panoramaScrollSpeed": "0.0",
                "hideSplashTexts": "true",
            })
        for key, value in forced_options.items():
            upsert_option(self.options_file, key, value)
        dh_file = self.run_dir / "config" / "DistantHorizons.toml"
        disable_dh_for_perf = os.environ.get("MATTMC_CAPTURE_DISABLE_DH_FOR_PERF", "false").lower() == "true"
        disable_dh_for_ordinary_source = (
            os.environ.get("MATTMC_CAPTURE_DISABLE_DH_FOR_ORDINARY_SOURCE", "false").lower() == "true"
        )
        if disable_dh_for_perf or disable_dh_for_ordinary_source:
            if upsert_toml_value(dh_file, "enableDistantGeneration", "false"):
                self.append_meta(
                    "forced_dh_enableDistantGeneration=false"
                    + (" reason=ordinary-selected-source" if disable_dh_for_ordinary_source else "")
                )
            # Ordinary Rust-Vulkan captures validate near-world semantic
            # terrain; do not let DH's background sync queues consume that
            # capture's bounded heap. Dedicated DH rows do not set this flag
            # and retain their full synchronization/rendering configuration.
            if disable_dh_for_ordinary_source:
                for key, value in (
                    # Isolated vanilla producer fixtures do not exercise DH;
                    # disable its legacy draw pass in both Current and Frozen
                    # copied runs so their workload fingerprints describe the
                    # same world/camera fixture. Dedicated DH rows do not set
                    # this flag and retain the real DH renderer.
                    ("rendererMode", '"DISABLED"'),
                    ("enableRendering", "false"),
                    # DH's fade hooks are independently configured and can
                    # repaint vanilla terrain even when LOD rendering is off.
                    ("vanillaFadeMode", '"NONE"'),
                    ("lodOnlyMode", "false"),
                    # DH can retain its vanilla-settings override even with
                    # its renderer disabled.  That override suppresses or
                    # changes vanilla clouds, which invalidates a vanilla
                    # Current/Frozen parity fixture.  Disable it only in the
                    # isolated ordinary-vanilla capture copy.
                    ("overrideVanillaGraphicsSettings", "false"),
                    # DH normally suppresses vanilla fog when it owns the
                    # far-world compositor.  That compositor is excluded
                    # from ordinary vanilla parity rows, so retain Frozen's
                    # vanilla fog instead of comparing its suppression
                    # against Rust's vanilla semantic fog.
                    ("enableVanillaFog", "true"),
                    ("enableDhFog", "false"),
                    ("synchronizeOnLoad", "false"),
                    ("enableRealTimeUpdates", "false"),
                    ("maxSyncOnLoadRequestDistance", "0"),
                    ("maxGenerationRequestDistance", "0"),
                    ("numberOfThreads", "1"),
                    ("threadRunTimeRatio", '"0.0"'),
                    ("lodChunkRenderDistanceRadius", "2"),
                ):
                    if upsert_toml_value(dh_file, key, value):
                        self.append_meta(f"forced_dh_ordinary_{key}={value}")
        dh_opaque_only = os.environ.get("MATTMC_CAPTURE_DH_RUST_OPAQUE_ONLY", "false").lower() == "true"
        dh_non_water = os.environ.get("MATTMC_CAPTURE_DH_RUST_NON_WATER", "false").lower() == "true"
        dh_water = os.environ.get("MATTMC_CAPTURE_DH_RUST_WATER", "false").lower() == "true"
        if dh_opaque_only or dh_non_water:
            # Both bounded routes exclude legacy auxiliary work. Keep DH's
            # renderer mode DEFAULT so the real CPU render-list traversal can
            # run; `enableRendering=false` below prevents the legacy GL draw
            # without making the semantic collector reject the frame.
            for key, value in (
                ("enableSsao", "false"),
                ("enableGenericRendering", "false"),
                ("dhFadeFarClipPlane", "false"),
                ("enableDhFog", "false"),
                ("enableRendering", "false"),
                ("lodChunkRenderDistanceRadius", "4"),
            ):
                if upsert_toml_value(dh_file, key, value):
                    self.append_meta(f"forced_dh_non_water_{key}={value}")
            if dh_opaque_only and upsert_toml_value(dh_file, "transparency", '"DISABLED"'):
                self.append_meta('forced_dh_opaque_transparency="DISABLED"')
            if dh_non_water and upsert_toml_value(dh_file, "transparency", '"COMPLETE"'):
                # The non-water route is meaningful only when DH generates its
                # real transparent streams. Keep this scoped to the explicit
                # deterministic coverage row; normal captures retain the
                # copied configuration.
                self.append_meta('forced_dh_non_water_transparency="COMPLETE"')
        if dh_water and upsert_toml_value(dh_file, "numberOfThreads", "1"):
            self.append_meta("forced_dh_water_numberOfThreads=1")
        if dh_water and upsert_toml_value(dh_file, "lodChunkRenderDistanceRadius", "2"):
            self.append_meta("forced_dh_water_lodChunkRenderDistanceRadius=2")
        voxelmap_file = self.run_dir / "config" / "voxelmap.properties"
        if voxelmap_file.is_file():
            upsert_option(voxelmap_file, "Welcome Message", "false")
            self.append_meta("forced_voxelmap_welcome=false")
            # The vanilla migration gate must not compare a mod-owned minimap
            # whose asynchronous cache warm-up differs between isolated game
            # directories.  Keep it out of both paired clients instead of
            # masking pixels or letting it influence world-color parity.
            upsert_option(voxelmap_file, "Hide Minimap", "true")
            self.append_meta("forced_voxelmap_minimap_hidden=true")
        width, height = menu_capture_window(self.config.title_screen_capture)
        self.append_meta(f"forced_window_width={width}")
        self.append_meta(f"forced_window_height={height}")
        for key, value in forced_options.items():
            self.append_meta(f"forced_option_{key}={value}")
        if self.config.title_screen_capture:
            self.append_meta(f"title_static_fixture={title_static_fixture}")

        manifest = find_validation_layer_manifest(self.platform_name)
        if manifest:
            self.validation_layer_manifest = str(manifest)
            self.validation_layer_dir = str(manifest.parent)
            self.validation_layer_available = True
        is_vulkan_validation_backend = self.config.backend in {"vulkan", "rust-vulkan"}
        if (
            is_vulkan_validation_backend
            and self.config.validation_mode == "standard"
            and self.validation_layer_available
        ):
            self.validation_enabled = True

        if self.config.validation_mode != "off" and not is_vulkan_validation_backend:
            self.append_meta("validation_note=ignored_for_non_vulkan_backend")
        elif self.config.validation_mode != "off" and not self.validation_enabled:
            self.append_meta("validation_note=requested_but_khronos_layer_unavailable")

    def configure_client_args(self) -> None:
        shaders_enabled = "true" if self.config.shaders == "on" else "false"
        self.config.client_args = remove_client_arg_option(self.config.client_args, "--quickPlaySingleplayer")
        self.config.client_args = remove_client_arg_option(self.config.client_args, "--width")
        self.config.client_args = remove_client_arg_option(self.config.client_args, "--height")
        self.config.client_args = remove_client_arg_assignment(self.config.client_args, "enableShaders")
        if not self.config.title_screen_capture:
            self.config.client_args = append_client_arg(self.config.client_args, f"--quickPlaySingleplayer={shlex.quote(self.config.world)}")
        width, height = menu_capture_window(self.config.title_screen_capture)
        self.config.client_args = append_client_arg(self.config.client_args, f"--width {width}")
        self.config.client_args = append_client_arg(self.config.client_args, f"--height {height}")
        self.config.client_args = append_client_arg(self.config.client_args, f"enableShaders={shaders_enabled}")
        if self.config.title_screen_capture:
            self.append_meta("title_screen_capture=true")
            self.append_meta(
                "title_screen_transition_capture="
                + ("true" if self.config.title_screen_transition_capture else "false")
            )
            self.append_meta("forced_quick_play_singleplayer=disabled")
            if self.config.backend == "rust-vulkan":
                self.env["MATTMC_RUST_TITLE_FRAME_CAPTURE_DIR"] = str(self.title_presented_frame_dir)
                self.append_meta(f"rust_title_presented_frame_dir={self.title_presented_frame_dir}")
        else:
            self.append_meta(f"forced_quick_play_singleplayer={self.config.world}")
        self.append_meta(f"forced_enable_shaders={shaders_enabled}")

        if not self.config.deterministic_camera_capture:
            return
        if self.config.deterministic_static_camera_capture:
            parity_diagnostics_path = self.artifact_dir / f"static_terrain_parity_diagnostics_{self.run_id}.jsonl"
            self.append_java_tool_options([
                "-Dmattmc.dev.staticTerrainParityDiagnostics=true",
                f"-Dmattmc.dev.staticTerrainParityDiagnostics.path={parity_diagnostics_path}",
                "-Dmattmc.dev.staticTerrainParityDiagnostics.waitForStable=true",
                "-Dmattmc.dev.staticTerrainParityDiagnostics.readyFrames=3",
                f"-Dmattmc.dev.staticTerrainParityDiagnostics.maxSamples={static_terrain_parity_max_samples()}",
                f"-Dmattmc.dev.staticTerrainParityDiagnostics.maxEvents={static_terrain_parity_max_events()}",
                f"-Dmattmc.dev.staticTerrainParityDiagnostics.maxVisibleListEvents={static_terrain_parity_max_visible_list_events()}",
                f"-Dmattmc.dev.staticTerrainParityDiagnostics.maxCoverageSamples={static_terrain_parity_max_coverage_samples()}",
                f"-Dmattmc.dev.staticTerrainParityDiagnostics.maxCoverageEvents={static_terrain_parity_max_coverage_events()}",
                f"-Dmattmc.dev.staticTerrainParityDiagnostics.maxWholeFrameCoverageEvents={static_terrain_parity_max_whole_frame_coverage_events()}",
                f"-Dmattmc.dev.staticTerrainParityDiagnostics.maxPortalTraceEvents={static_terrain_parity_max_portal_trace_events()}",
                f"-Dmattmc.dev.staticTerrainParityDiagnostics.maxFaceCullTraceEvents={static_terrain_parity_max_face_cull_trace_events()}",
            ])
            self.append_meta(f"static_terrain_parity_diagnostics={parity_diagnostics_path}")
            if self.config.world_static_terrain_fault:
                self.append_java_tool_options([
                    f"-Dmattmc.dev.rustGalStaticTerrain.fault={self.config.world_static_terrain_fault}",
                ])
                self.append_meta(f"world_static_terrain_fault={self.config.world_static_terrain_fault}")
            if self.config.world_static_terrain_water_animation_capture:
                self.append_java_tool_options([
                    "-Dmattmc.dev.rustGalStaticTerrain.waterAnimationDenseCapture=true",
                    "-Dmattmc.dev.rustGalStaticTerrain.waterAnimationDenseFrames=24",
                ])
                self.append_meta("static_terrain_water_animation_dense_capture=true")
            moving_mesh_sequence = self.has_active_moving_mesh_capture_sequence()
            if static_terrain_requires_translucent_camera_sort(
                self.config.world_static_terrain_scenario
            ) and not moving_mesh_sequence:
                self.append_java_tool_options([
                    "-Dmattmc.dev.deterministicCameraCapture.poseCount=7",
                    "-Dmattmc.dev.deterministicCameraCapture.framesPerPose=1",
                    "-Dmattmc.dev.deterministicCameraCapture.yawDelta=18.0",
                ])
                self.append_meta("deterministic_static_camera_capture=translucent_camera_sequence")
            elif not moving_mesh_sequence:
                self.append_java_tool_options([
                    "-Dmattmc.dev.deterministicCameraCapture.poseCount=1",
                    "-Dmattmc.dev.deterministicCameraCapture.yawDelta=0.0",
                ])
                self.append_meta("deterministic_static_camera_capture=true")
            else:
                # Moving-mesh scenarios own their multi-pose sequence. Static
                # terrain diagnostics remain enabled, but must not append a
                # later poseCount that collapses falling/piston motion proof.
                self.append_meta("deterministic_static_camera_capture=readiness_only_moving_sequence")
        configured_shader_pack = extract_property_value(self.run_dir / "config" / "iris.properties", "shaderPack")
        if not client_args_contains_assignment(self.config.client_args, "shaderPack"):
            if self.config.shaders == "on" and not configured_shader_pack:
                raise SystemExit(
                    "--deterministic-camera-capture requires a shaderPack in CLIENT_ARGS "
                    "or run/config/iris.properties"
                )
            if configured_shader_pack:
                self.config.client_args = append_client_arg(
                    self.config.client_args, f"shaderPack={configured_shader_pack}"
                )

    def prepare_snapshots(self) -> None:
        copy_config_snapshot(self.run_dir, self.config_before_dir)
        self.client_arg_shader_pack = extract_client_arg_assignment(self.config.client_args, "shaderPack")
        self.client_arg_enable_shaders = extract_client_arg_assignment(self.config.client_args, "enableShaders")
        iris_snapshot = self.config_before_dir / "iris.properties"
        self.iris_property_shader_pack = extract_property_value(iris_snapshot, "shaderPack")
        self.iris_property_enable_shaders = extract_property_value(iris_snapshot, "enableShaders")
        self.iris_property_color_space = extract_property_value(iris_snapshot, "colorSpace")
        self.effective_shader_pack = self.client_arg_shader_pack or self.iris_property_shader_pack
        self.effective_enable_shaders = self.client_arg_enable_shaders or self.iris_property_enable_shaders

        self.apply_iris_overrides()
        self.collect_system_snapshot()
        self.collect_git_snapshot()
        self.write_shaderpack_info()

        for line in [
            f"validation_enabled={str(self.validation_enabled).lower()}",
            f"validation_layer_available={str(self.validation_layer_available).lower()}",
            f"validation_layer_manifest={self.validation_layer_manifest or 'unavailable'}",
            f"config_before={self.config_before_dir}",
            f"system_snapshot={self.system_snapshot}",
            f"git_snapshot={self.git_snapshot}",
            f"shaderpack_info={self.shaderpack_info}",
            f"effective_enable_shaders={self.effective_enable_shaders or 'unset'}",
            f"effective_shader_pack={self.effective_shader_pack or 'unset'}",
            f"client_args_effective={self.config.client_args}",
            f"deterministic_metadata={self.deterministic_metadata}",
            f"deterministic_screenshot_dir={self.deterministic_screenshot_dir}",
        ]:
            self.append_meta(line)

    def apply_iris_overrides(self) -> None:
        iris_file = self.run_dir / "config" / "iris.properties"
        if not iris_file.is_file():
            self.append_meta("iris_override_note=iris.properties_missing")
            return

        if self.client_arg_enable_shaders:
            normalized = self.client_arg_enable_shaders.lower()
            if normalized in {"true", "false"}:
                upsert_property(iris_file, "enableShaders", normalized)
                self.append_meta(f"iris_override_enable_shaders={normalized}")
            else:
                self.append_meta(f"iris_override_enable_shaders_ignored={self.client_arg_enable_shaders}")

        if self.client_arg_shader_pack:
            upsert_property(iris_file, "shaderPack", self.client_arg_shader_pack)
            self.append_meta(f"iris_override_shader_pack={self.client_arg_shader_pack}")

    def collect_system_snapshot(self) -> None:
        lines = [
            f"backend={self.config.backend}",
            f"shaders={self.config.shaders}",
            f"validation_mode={self.config.validation_mode}",
            f"validation_enabled={str(self.validation_enabled).lower()}",
            f"validation_layer_available={str(self.validation_layer_available).lower()}",
            f"validation_layer_manifest={self.validation_layer_manifest or 'unavailable'}",
            f"shader_input_parity={self.config.shader_input_parity}",
            f"shader_input_parity_max_logs={self.config.shader_input_parity_max_logs}",
            f"display={os.environ.get('DISPLAY', 'unset')}",
            f"wayland_display={os.environ.get('WAYLAND_DISPLAY', 'unset')}",
            f"xdg_session_type={os.environ.get('XDG_SESSION_TYPE', 'unset')}",
            "",
            "===== platform =====",
            py_platform.platform(),
            f"python={sys.version.split()[0]}",
            f"detected_platform={self.platform_info.platform}",
            f"detected_arch={self.platform_info.arch}",
        ]
        for label, command in [
            ("uname -a", ["uname", "-a"]),
            ("java -version", ["java", "-version"]),
            ("vulkaninfo --summary", ["vulkaninfo", "--summary"]),
            ("glxinfo -B", ["glxinfo", "-B"]),
            ("nvidia-smi", ["nvidia-smi", "--query-gpu=name,driver_version,vbios_version", "--format=csv,noheader"]),
            ("systeminfo", ["systeminfo"]),
        ]:
            if not shutil.which(command[0]):
                continue
            if label.startswith("glxinfo") and self.platform_name != "linux":
                continue
            if label == "systeminfo" and self.platform_name != "windows":
                continue
            lines.extend(["", f"===== {label} =====", command_text(command, cwd=self.root)])
        self.system_snapshot.write_text("\n".join(lines), encoding="utf-8", errors="replace")

    def collect_git_snapshot(self) -> None:
        sections = [
            ("git branch", ["git", "-C", str(self.root), "branch", "--show-current"]),
            ("git rev-parse HEAD", ["git", "-C", str(self.root), "rev-parse", "HEAD"]),
            ("git status --short", ["git", "-C", str(self.root), "status", "--short"]),
        ]
        write_sections(self.git_snapshot, sections, cwd=self.root)

    def write_shaderpack_info(self) -> None:
        iris_file = self.config_before_dir / "iris.properties"
        lines = [
            f"client_args={self.config.client_args}",
            f"client_arg_enable_shaders={self.client_arg_enable_shaders or 'unset'}",
            f"client_arg_shader_pack={self.client_arg_shader_pack or 'unset'}",
            f"iris_property_enable_shaders={self.iris_property_enable_shaders or 'unset'}",
            f"iris_property_shader_pack={self.iris_property_shader_pack or 'unset'}",
            f"iris_property_color_space={self.iris_property_color_space or 'unset'}",
            f"effective_enable_shaders={self.effective_enable_shaders or 'unset'}",
            f"effective_shader_pack={self.effective_shader_pack or 'unset'}",
            f"config_snapshot={iris_file}",
        ]
        shader_pack_path = resolve_shader_pack_path(self.run_dir, self.root, self.effective_shader_pack)
        if shader_pack_path:
            lines.extend(["", f"shader_pack_path={shader_pack_path}", "", "===== shader pack stat ====="])
            lines.append(stat_text(shader_pack_path))
            lines.extend(["", "===== shader pack sha256 ====="])
            lines.append(file_sha256_text(shader_pack_path))
            if shader_pack_path.is_file() and shader_pack_path.suffix == ".zip" and shutil.which("unzip"):
                lines.extend(["", "===== shader pack zip listing (first 200 lines) ====="])
                unzip_output = command_text(["unzip", "-l", str(shader_pack_path)], cwd=self.root)
                lines.extend(unzip_output.splitlines()[:200])
            elif shader_pack_path.is_dir():
                lines.extend(["", "===== shader pack file listing ====="])
                lines.extend(str(path) for path in sorted(shader_pack_path.rglob("*")) if path.is_file())
        else:
            lines.append("shader_pack_path=unresolved")
        self.shaderpack_info.write_text("\n".join(lines) + "\n", encoding="utf-8", errors="replace")

    def start_gradle(self) -> None:
        print(f"Starting bounded runClient capture (run_id={self.run_id}, backend={self.config.backend})")
        gradle_cmd = [*self.gradle]
        gradle_cmd.append(f"-PmattmcRunGameDir={self.run_dir}")
        if self.config.skip_tests:
            gradle_cmd.extend(["-x", "test"])
        if self.config.capture_meshing_corpus:
            output = self.meshing_corpus_output_path()
            gradle_cmd.extend(
                [
                    f"-PmattmcReplayOutput={output}",
                    f"-PmattmcReplayWarmup={self.config.meshing_corpus_warmup}",
                    "-PmattmcReplayWarmupSeconds=0",
                    f"-PmattmcReplayMeasure={self.config.meshing_corpus_measure}",
                    "-PmattmcReplayMeasureSeconds=0",
                    "-PmattmcReplayRebuildsPerSample=1",
                    "-PmattmcReplayValidateEachSample=true",
                    "-PmattmcReplayCaptureInputs=true",
                ]
            )
            if self.config.meshing_corpus_fixture:
                gradle_cmd.append(f"-PmattmcReplayFixture={self.config.meshing_corpus_fixture}")
            gradle_cmd.append("runRealChunkMeshingReplay")
        else:
            gradle_cmd.append("runClient")
        if self.config.client_args and not self.config.capture_meshing_corpus:
            gradle_cmd.append(f"--args={self.config.client_args}")

        self.configure_validation_environment()
        self.configure_java_tool_options()
        gradle_cmd = self.fresh_environment_launch_command(gradle_cmd)
        gradle_cmd = self.renderdoc_wrapped_command(gradle_cmd)

        log_handle = self.run_log.open("wb")
        try:
            if self.platform_name == "windows":
                creationflags = getattr(subprocess, "CREATE_NEW_PROCESS_GROUP", 0)
                self.gradle_process = subprocess.Popen(
                    gradle_cmd,
                    cwd=self.root,
                    stdout=log_handle,
                    stderr=subprocess.STDOUT,
                    env=self.env,
                    creationflags=creationflags,
                )
            else:
                self.gradle_process = subprocess.Popen(
                    gradle_cmd,
                    cwd=self.root,
                    stdout=log_handle,
                    stderr=subprocess.STDOUT,
                    env=self.env,
                    start_new_session=True,
                )
        except Exception:
            log_handle.close()
            raise
        log_handle.close()
        self.run_client_active = True
        self.append_meta(f"gradle_pid={self.gradle_process.pid}")

    def renderdoc_wrapped_command(self, command: list[str]) -> list[str]:
        if self.env.get("MATTMC_RENDERDOC_CAPTURE", "").lower() not in {"1", "true", "yes"}:
            return command
        renderdoc = self.env.get("MATTMC_RENDERDOC_CMD") or shutil.which("renderdoccmd")
        if not renderdoc:
            self.append_meta("renderdoc_launch_error=renderdoccmd unavailable in capture runner")
            return command
        command = self.renderdoc_launch_command(command)
        capture_template = self.env.get("MATTMC_RENDERDOC_CAPTURE_TEMPLATE", "").strip()
        capture_path = self.env.get("MATTMC_RENDERDOC_CAPTURE_PATH", "").strip()
        if capture_template:
            requested_template = Path(capture_template)
            if not requested_template.is_absolute():
                requested_template = self.artifact_dir / requested_template
            elif not self._is_inside_artifact_dir(requested_template):
                requested_template = self.artifact_dir / requested_template.name
            capture_template = str(requested_template)
        if not capture_template and capture_path:
            requested_path = Path(capture_path)
            # Relative paths belong to the ignored artifact directory.  Also
            # repair the historical ``<repo-root>/.capture`` form so a stale
            # environment cannot recreate root contamination.
            if not requested_path.is_absolute():
                requested_path = self.artifact_dir / requested_path
            elif not self._is_inside_artifact_dir(requested_path):
                requested_path = self.artifact_dir / requested_path.name
            capture_template = str(requested_path.with_suffix(""))
        if not capture_template:
            capture_template = str(self.artifact_dir / f"renderdoc_{self.run_id}")
        wrapped = [
            renderdoc,
            "capture",
            "-d",
            str(self.root),
            "-w",
            "--opt-hook-children",
            "--opt-api-validation",
            "-c",
            capture_template,
            *command,
        ]
        self.append_meta(f"renderdoc_wrapped_actual_game_command={' '.join(shlex.quote(part) for part in wrapped)}")
        self.append_meta(f"renderdoc_capture_template={capture_template}")
        return wrapped

    def _is_inside_artifact_dir(self, path: Path) -> bool:
        """Return whether a capture path is contained by the ignored artifact root."""
        try:
            path.resolve().relative_to(self.artifact_dir.resolve())
        except ValueError:
            return False
        return True

    def renderdoc_launch_command(self, command: list[str]) -> list[str]:
        if not command:
            return command
        launcher = Path(command[0]).name
        if launcher in {"gradlew", "gradlew.bat"} and "--no-daemon" not in command:
            self.append_meta("renderdoc_forced_gradle_no_daemon=true")
            return [command[0], "--no-daemon", *command[1:]]
        return command

    def fresh_environment_launch_command(self, command: list[str]) -> list[str]:
        """Avoid a stale Gradle daemon for native diagnostic environment inputs."""
        if not command:
            return command
        diagnostic_inputs = {
            name: value.strip()
            for name, value in self.env.items()
            if value.strip()
            and (
                name.startswith("MATTMC_RUST_SELECTED_SOURCE_")
                or name.startswith("MATTMC_RUST_SELECTED_SOURCE_FULLSCREEN_")
                or name.startswith("MATTMC_STATIC_TERRAIN_APPEARANCE_TRACE_")
                or name.startswith("MATTMC_STATIC_TERRAIN_BATCH_TRACE_")
            )
        }
        launcher = Path(command[0]).name
        if (
            (diagnostic_inputs or getattr(getattr(self, "config", None), "deterministic_camera_capture", False))
            and launcher in {"gradlew", "gradlew.bat"}
            and "--no-daemon" not in command
        ):
            if diagnostic_inputs:
                self.append_meta("native_diagnostic_forced_gradle_no_daemon=true")
                self.append_meta(
                    "native_diagnostic_inputs="
                    + ",".join(sorted(diagnostic_inputs))
                )
            if getattr(getattr(self, "config", None), "deterministic_camera_capture", False):
                self.append_meta("deterministic_capture_forced_gradle_no_daemon=true")
            return [command[0], "--no-daemon", *command[1:]]
        return command

    def configure_validation_environment(self) -> None:
        if not self.validation_enabled:
            return
        current_layers = self.env.get("VK_INSTANCE_LAYERS", "")
        layer = "VK_LAYER_KHRONOS_validation"
        if current_layers:
            layers = current_layers.split(":")
            if layer not in layers:
                self.env["VK_INSTANCE_LAYERS"] = f"{layer}:{current_layers}"
        else:
            self.env["VK_INSTANCE_LAYERS"] = layer

        if self.validation_layer_dir:
            current_paths = self.env.get("VK_ADD_LAYER_PATH", "")
            sep = os.pathsep
            if current_paths:
                paths = current_paths.split(sep)
                if self.validation_layer_dir not in paths:
                    self.env["VK_ADD_LAYER_PATH"] = f"{self.validation_layer_dir}{sep}{current_paths}"
            else:
                self.env["VK_ADD_LAYER_PATH"] = self.validation_layer_dir
        self.env.setdefault("VK_LOADER_DEBUG", "error,warn,layer")
        validation_profile = os.environ.get("MATTMC_GRAPHICS_VALIDATION_PROFILE", self.config.validation_mode)
        if validation_profile in {"standard", "routine"}:
            self.env.setdefault("VK_LAYER_SETTINGS", "validate_sync=true,validate_best_practices=true")
        elif validation_profile == "deep":
            self.env.setdefault(
                "VK_LAYER_SETTINGS",
                "validate_sync=true,validate_best_practices=true,gpuav_enable=true,printf_enable=true",
            )
        self.append_meta(f"vk_instance_layers={self.env.get('VK_INSTANCE_LAYERS', '')}")
        self.append_meta(f"vk_add_layer_path={self.env.get('VK_ADD_LAYER_PATH', 'unset')}")
        self.append_meta(f"vk_loader_debug={self.env.get('VK_LOADER_DEBUG', '')}")
        self.append_meta(f"vk_layer_settings={self.env.get('VK_LAYER_SETTINGS', '')}")

    def configure_java_tool_options(self) -> None:
        if self.config.backend == "rust-vulkan":
            # build.gradle's development default is intentionally generous for
            # interactive play (8G/4G), but a graphics capture must remain below
            # the harness RSS budget.  Own the launch bounds here rather than
            # accepting an over-budget client and treating its late termination
            # as valid parity evidence.
            # Leave headroom for native Vulkan allocations, driver mappings, and
            # the JVM's non-heap regions.  With the normal 12288 MiB guard this
            # preserves the native-memory headroom needed by Vulkan and the
            # driver without imposing an arbitrary 4 GiB Java-heap ceiling.
            rss_limit_mb = self.config.client_rss_limit_mb
            # Vulkan command recording, shader compilation, driver mappings,
            # and the loader can consume substantial native RSS during startup.
            # Keep the client at its normal 8 GiB maximum whenever the RSS
            # budget permits it. The RSS guard, rather than a Java-heap target,
            # is the authoritative workstation-safety limit.
            heap_xmx_mb = capture_client_heap_max_mb(rss_limit_mb)
            heap_xms_mb = capture_client_heap_initial_mb(heap_xmx_mb)
            self.env["MATTMC_CLIENT_XMX"] = f"{heap_xmx_mb}m"
            self.env["MATTMC_CLIENT_XMS"] = f"{heap_xms_mb}m"
            # ZGC is useful for interactive runs but carries substantial native
            # bookkeeping during shader/resource startup. Capture runs use G1
            # explicitly so the RSS guard measures the client rather than a
            # collector's largely-reserved address space.
            self.env["MATTMC_CLIENT_GC"] = "G1"
            self.append_meta(
                f"bounded_client_heap=initial:{heap_xms_mb}m,max:{heap_xmx_mb}m,gc:G1"
            )
        options = [f"-Dmattmc.dev.runCaptureId={self.run_id}"]
        if self.config.backend == "rust-vulkan":
            options.append("-Dmattmc.dev.rustGalVulkanWholeFrame=true")
        if self.config.jvm_args:
            self.append_java_tool_options(self.config.jvm_args)
            self.append_meta(f"user_java_options={' '.join(self.config.jvm_args)}")
        self.append_java_tool_options(options)
        self.append_meta(f"run_capture_java_options={' '.join(options)}")
        self.append_meta(f"java_tool_options={self.env.get('JAVA_TOOL_OPTIONS', '')}")

        if self.config.shader_input_parity != "off":
            parity_options = [
                "-Dmattmc.vulkan.traceShaderInputParity=true",
                f"-Dmattmc.vulkan.traceShaderInputParity.maxLogs={self.config.shader_input_parity_max_logs}",
                "-Dmattmc.vulkan.deterministicLightmapParity=true",
                "-Dmattmc.vulkan.traceLightmapInfoParity=true",
                f"-Dmattmc.vulkan.traceLightmapInfoParity.maxLogs={self.config.lightmap_info_parity_max_logs}",
            ]
            if self.config.shader_input_parity == "full":
                parity_options.append("-Dmattmc.vulkan.traceStandaloneUniformBlockMembers=true")
                parity_options.append("-Dmattmc.vulkan.traceShaderInputParity.fullUniforms=true")
            self.append_java_tool_options(parity_options)
            self.append_meta(f"shader_input_parity_java_options={' '.join(parity_options)}")
            self.append_meta(f"java_tool_options={self.env.get('JAVA_TOOL_OPTIONS', '')}")
            self.append_meta("deterministic_lightmap_parity=true")

        # Every paired capture needs fixed temporal and camera inputs. Live
        # partial ticks change interpolated FOV independently after startup,
        # producing a different finite sky fan before renderer comparison.
        if self.config.deterministic_camera_capture:
            temporal_options = [
                # The dynamic block-light flicker is a temporal gameplay input to the
                # vanilla lightmap. A paired fixed-time capture must pin it even when
                # the optional shader-input trace is disabled; otherwise the two
                # processes can render different lightmaps solely from startup frame
                # phase. This activates existing fixture behavior, not a renderer path.
                "-Dmattmc.vulkan.deterministicLightmapParity=true",
                "-Dmattmc.vulkan.deterministicTemporalParity=true",
                "-Dmattmc.vulkan.deterministicTemporalParity.frameCounter=0",
                "-Dmattmc.vulkan.deterministicTemporalParity.frameTime=0.016666668",
                "-Dmattmc.vulkan.deterministicTemporalParity.frameTimeCounter=0.0",
                "-Dmattmc.vulkan.deterministicTemporalParity.partialTick=1.0",
                "-Dmattmc.vulkan.deterministicTemporalParity.fovModifier=1.0",
                f"-Dmattmc.vulkan.deterministicTemporalParity.worldTime={int(self.env.get('MATTMC_CAPTURE_WORLD_TIME', '6000'))}",
            ]
            self.append_java_tool_options(temporal_options)
            self.append_meta(f"deterministic_temporal_java_options={' '.join(temporal_options)}")
            self.append_meta(f"java_tool_options={self.env.get('JAVA_TOOL_OPTIONS', '')}")
            self.append_meta("deterministic_temporal_parity=true")

        if self.config.deterministic_camera_capture:
            deterministic_options = [
                "-Dmattmc.dev.deterministicCameraCapture=true",
                f"-Dmattmc.dev.deterministicCameraCapture.metadata={self.deterministic_metadata}",
                f"-Dmattmc.dev.deterministicCameraCapture.screenshotDir={self.deterministic_screenshot_dir}",
                f"-Dmattmc.dev.deterministicCameraCapture.shaderEnabled={self.effective_enable_shaders or 'unknown'}",
                f"-Dmattmc.dev.deterministicCameraCapture.shaderPack={self.effective_shader_pack or 'unknown'}",
                f"-Dmattmc.dev.deterministicCameraCapture.gitCommit={self.git_commit}",
                f"-Dmattmc.dev.deterministicCameraCapture.world={self.config.world}",
                "-Dmattmc.dev.deterministicCameraCapture.stopAfterComplete=true",
                "-Dmattmc.dev.deterministicCameraCapture.ackTimeoutFrames=12000",
                "-Dmattmc.vulkan.traceShaderInputParity.poseOnly=true",
            ]
            self.append_java_tool_options(deterministic_options)
            self.append_meta(f"deterministic_camera_capture_java_options={' '.join(deterministic_options)}")
            self.append_meta(f"java_tool_options={self.env.get('JAVA_TOOL_OPTIONS', '')}")

        if self.config.audio_validation:
            audio_options = [
                "-Dmattmc.dev.audioValidation=true",
                f"-Dmattmc.dev.audioValidation.status={self.audio_validation_status}",
            ]
            self.append_java_tool_options(audio_options)
            self.append_meta(f"audio_validation_java_options={' '.join(audio_options)}")
            self.append_meta(f"audio_validation_status={self.audio_validation_status}")
            self.append_meta(f"java_tool_options={self.env.get('JAVA_TOOL_OPTIONS', '')}")

        if self.config.region_validation:
            region_options = [
                "-Dmattmc.dev.nbtShadowCompare=true",
                f"-Dmattmc.dev.regionFilesValidationStatus={self.region_validation_status}",
            ]
            self.append_java_tool_options(region_options)
            self.append_meta(f"region_validation_java_options={' '.join(region_options)}")
            self.append_meta(f"region_validation_status={self.region_validation_status}")
            self.append_meta(f"java_tool_options={self.env.get('JAVA_TOOL_OPTIONS', '')}")

        if self.config.poi_validation:
            poi_options = [
                "-Dmattmc.dev.poiValidation=true",
                f"-Dmattmc.dev.poiValidation.status={self.poi_validation_status}",
            ]
            self.append_java_tool_options(poi_options)
            self.append_meta(f"poi_validation_java_options={' '.join(poi_options)}")
            self.append_meta(f"poi_validation_status={self.poi_validation_status}")
            self.append_meta(f"java_tool_options={self.env.get('JAVA_TOOL_OPTIONS', '')}")

        if self.config.capture_meshing_corpus:
            output = self.meshing_corpus_output_path()
            meshing_options = [
                "-Dmattmc.realMeshingReplay=true",
                f"-Dmattmc.realMeshingReplay.output={output}",
                f"-Dmattmc.realMeshingReplay.warmup={self.config.meshing_corpus_warmup}",
                f"-Dmattmc.realMeshingReplay.measure={self.config.meshing_corpus_measure}",
                "-Dmattmc.realMeshingReplay.warmupSeconds=0",
                "-Dmattmc.realMeshingReplay.measureSeconds=0",
                "-Dmattmc.realMeshingReplay.rebuildsPerSample=1",
                "-Dmattmc.realMeshingReplay.validateEachSample=true",
                "-Dmattmc.realMeshingReplay.captureInputs=true",
            ]
            if self.config.meshing_corpus_fixture:
                meshing_options.append(f"-Dmattmc.realMeshingReplay.fixture={self.config.meshing_corpus_fixture}")
            self.append_meta(f"meshing_corpus_java_options={' '.join(meshing_options)}")
            gradle_properties = [
                f"-PmattmcReplayOutput={output}",
                f"-PmattmcReplayWarmup={self.config.meshing_corpus_warmup}",
                "-PmattmcReplayWarmupSeconds=0",
                f"-PmattmcReplayMeasure={self.config.meshing_corpus_measure}",
                "-PmattmcReplayMeasureSeconds=0",
                "-PmattmcReplayRebuildsPerSample=1",
                "-PmattmcReplayValidateEachSample=true",
                "-PmattmcReplayCaptureInputs=true",
            ]
            if self.config.meshing_corpus_fixture:
                gradle_properties.append(f"-PmattmcReplayFixture={self.config.meshing_corpus_fixture}")
            self.append_meta(f"meshing_corpus_gradle_properties={' '.join(gradle_properties)}")
            self.append_meta(f"meshing_corpus_replay={output}")
            self.append_meta(f"java_tool_options={self.env.get('JAVA_TOOL_OPTIONS', '')}")

    def meshing_corpus_output_path(self) -> Path:
        output = Path(self.config.meshing_corpus_output) if self.config.meshing_corpus_output else self.meshing_corpus_replay
        return output if output.is_absolute() else (self.root / output)

    def append_java_tool_options(self, options: list[str]) -> None:
        current = shlex.split(self.env.get("JAVA_TOOL_OPTIONS", ""))
        joined = current + list(options)
        joined_text = " ".join(shlex.quote(option) for option in joined if option)
        self.env["JAVA_TOOL_OPTIONS"] = joined_text

    def has_active_moving_mesh_capture_sequence(self) -> bool:
        """Whether an upstream scenario owns a multi-frame moving-mesh capture."""
        inactive = {"", "false", "disabled", "hidden", "none", "off"}
        prefixes = (
            "-Dmattmc.dev.rustGalWorldMesh.fallingBlockScenario=",
            "-Dmattmc.dev.rustGalWorldMesh.pistonScenario=",
            "-Dmattmc.dev.rustGalWorldMesh.primedTntScenario=",
            "-Dmattmc.dev.rustGalWorldMesh.arrowScenario=",
            "-Dmattmc.dev.rustGalWorldMesh.modelScenario=",
            "-Dmattmc.dev.rustGalWorldEntityFlame.scenario=",
        )
        try:
            options = shlex.split(self.env.get("JAVA_TOOL_OPTIONS", ""))
        except ValueError:
            options = self.env.get("JAVA_TOOL_OPTIONS", "").split()
        for option in options:
            for prefix in prefixes:
                if option.startswith(prefix):
                    return option.removeprefix(prefix).strip().lower() not in inactive
        return False

    def monitor_gradle(self) -> int:
        assert self.gradle_process is not None
        elapsed = 0
        while self.gradle_process.poll() is None:
            time.sleep(1)
            elapsed += 1
            client_pid = self.find_client_pid()
            self.capture_deterministic_requests(client_pid)
            if self.config.deterministic_camera_capture and self.deterministic_capture_status() == "complete":
                self.deterministic_completed = True
                frame_benchmark_status = self.frame_benchmark_status()
                if frame_benchmark_status not in {None, "complete", "failed"}:
                    if not self.awaiting_frame_benchmark_after_capture:
                        self.append_meta("deterministic_capture_waiting_for_frame_benchmark=true")
                        self.awaiting_frame_benchmark_after_capture = True
                    continue
                if frame_benchmark_status is None and self.frame_benchmark_status_path is not None:
                    if not self.awaiting_frame_benchmark_after_capture:
                        self.append_meta("deterministic_capture_waiting_for_frame_benchmark=true")
                        self.awaiting_frame_benchmark_after_capture = True
                    continue
                self.intentional_deterministic_shutdown = True
                self.append_meta(f"deterministic_capture_complete_elapsed={elapsed}")
                force_capture_stop = os.environ.get("MATTMC_CAPTURE_KILL_AFTER_DETERMINISTIC", "false").lower() == "true"
                if force_capture_stop:
                    self.append_meta("deterministic_shutdown_mode=immediate-process-termination")
                    self.terminate_run_processes("deterministic_complete_immediate")
                elif not self.wait_for_deterministic_shutdown():
                    self.terminate_run_processes("deterministic_complete")
                break
            subsystem_status = self.subsystem_benchmark_status()
            if subsystem_status in {"complete", "failed", "partial"}:
                self.subsystem_benchmark_completed = subsystem_status == "complete"
                self.intentional_subsystem_shutdown = True
                self.append_meta(f"subsystem_benchmark_status={subsystem_status}")
                self.append_meta(f"subsystem_benchmark_terminal_elapsed={elapsed}")
                if not self.wait_for_process_shutdown(
                    "subsystem_shutdown",
                    self.config.deterministic_shutdown_grace_secs,
                ):
                    self.terminate_run_processes(f"subsystem_{subsystem_status}")
                break

            if not self.check_client_memory_guard(client_pid, elapsed):
                break

            if self.capture_rust_title_presented_frame(client_pid):
                # A normal title capture ends as soon as Rust has acknowledged
                # its presented image. A transition capture must instead keep
                # its already-owned final-frame receipt and continue sampling
                # the bounded launch timeline; otherwise the receipt arriving
                # at the title screen truncates the very Mojang-to-title gap
                # that the diagnostic was requested to observe.
                if not self.config.title_screen_transition_capture:
                    self.title_screen_capture_completed = True
                    self.intentional_title_screen_shutdown = True
                    self.append_meta(f"title_screen_capture_complete_elapsed={elapsed}")
                    self.terminate_run_processes("rust_title_presented_frame_capture_complete")
                    break

            if (
                not self.config.deterministic_camera_capture
                and self.screenshot_enabled
                and self.config.screenshot_interval_secs > 0
                and elapsed >= self.config.screenshot_start_delay_secs
                and (elapsed - self.config.screenshot_start_delay_secs) % self.config.screenshot_interval_secs == 0
            ):
                if not self.title_screen_receipt_ready(elapsed):
                    continue
                self.capture_root_screenshot("tick", elapsed, client_pid)
                # A transition timeline may fill before a cold launch has
                # reached TitleScreen.  Its requested samples are evidence of
                # the handoff, not proof that the destination rendered.  Keep
                # observing (within the existing run timeout) until the
                # log-only title receipt arrives; Rust then takes the stricter
                # acknowledged-presented-frame path above.
                if self.title_screen_timeline_complete():
                    self.title_screen_capture_completed = True
                    self.intentional_title_screen_shutdown = True
                    self.append_meta(f"title_screen_capture_complete_elapsed={elapsed}")
                    self.terminate_run_processes("title_screen_capture_complete")
                    break

            if not self.dump_taken and elapsed >= self.config.dump_secs:
                self.take_dump(elapsed, client_pid)

            if elapsed >= self.config.max_secs:
                self.timed_out = True
                break

        if self.timed_out:
            self.append_meta("timeout=true")
            if not self.config.deterministic_camera_capture:
                self.capture_root_screenshot("timeout", elapsed, None)
            client_pid = self.find_client_pid()
            if client_pid:
                append_text(self.thread_dump, f"\n===== final jcmd Thread.print before termination (pid={client_pid}) =====\n")
                append_text(
                    self.thread_dump,
                    command_text(["jcmd", str(client_pid), "Thread.print"], cwd=self.root, timeout_secs=8),
                )
            self.terminate_run_processes("timeout")

        if self.gradle_process.poll() is None:
            # The client/Gradle wrapper are launched in one process group, but
            # a JVM or wrapper child can occasionally keep the wrapper pipe
            # alive after the group termination above.  Never turn that into
            # an unbounded harness hang: bounded capture termination is part
            # of the artifact contract just like bounded resource retention.
            try:
                return self.gradle_process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.append_meta("cleanup_gradle_wait_timeout=true")
                self.terminate_run_processes("gradle_wait_timeout")
                try:
                    return self.gradle_process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    self.append_meta("cleanup_gradle_wait_escalation_timeout=true")
                    return self.gradle_process.poll() or 1
        return self.gradle_process.returncode or 0

    def wait_for_deterministic_shutdown(self, grace_secs: int | None = None) -> bool:
        if grace_secs is None:
            grace_secs = self.config.deterministic_shutdown_grace_secs
        return self.wait_for_process_shutdown("deterministic_shutdown", grace_secs)

    def wait_for_process_shutdown(self, prefix: str, grace_secs: int) -> bool:
        assert self.gradle_process is not None
        self.append_meta(f"{prefix}_grace_secs={grace_secs}")
        client_exit_elapsed: int | None = None
        for waited in range(grace_secs):
            client_pid = self.find_client_pid()
            if client_pid is None and client_exit_elapsed is None:
                client_exit_elapsed = waited
                self.append_meta(f"{prefix}_client_exit_elapsed={waited}")
            if self.gradle_process.poll() is not None:
                self.append_meta(f"{prefix}_grace_elapsed={waited}")
                if client_exit_elapsed is None:
                    self.append_meta(f"{prefix}_client_exit_elapsed=with_gradle")
                return True
            time.sleep(1)
        if self.gradle_process.poll() is not None:
            self.append_meta(f"{prefix}_grace_elapsed={grace_secs}")
            if client_exit_elapsed is None:
                self.append_meta(f"{prefix}_client_exit_elapsed=with_gradle")
            return True
        if client_exit_elapsed is None:
            self.append_meta(f"{prefix}_client_still_running=true")
        else:
            self.append_meta(f"{prefix}_gradle_wrapper_still_running=true")
        self.append_meta(f"{prefix}_grace_expired=true")
        return False

    def find_client_pid(self) -> int | None:
        marker = f"-Dmattmc.dev.runCaptureId={self.run_id}"
        for pid, command in list_java_processes():
            if CLIENT_PROCESS_PATTERN.search(command) and marker in command:
                return pid
        for pid, command in list_java_processes():
            if CLIENT_PROCESS_PATTERN.search(command):
                return pid
        if self.platform_name != "windows" and self.gradle_process is not None:
            pgid = self.gradle_process.pid
            output = command_text(["ps", "-eo", "pid=,pgid=,comm=,cmd="], cwd=self.root)
            for line in output.splitlines():
                parts = line.strip().split(None, 3)
                if len(parts) < 4:
                    continue
                try:
                    pid = int(parts[0])
                    line_pgid = int(parts[1])
                except ValueError:
                    continue
                if line_pgid == pgid and parts[2] == "java":
                    if re.search(r"devlaunchinjector|minecraft|fabric|MattMC-1\.21", parts[3], re.IGNORECASE):
                        return pid
            for line in output.splitlines():
                parts = line.strip().split(None, 3)
                if len(parts) >= 3 and parts[2] == "java":
                    try:
                        if int(parts[1]) == pgid:
                            return int(parts[0])
                    except ValueError:
                        continue
        return None

    def capture_root_screenshot(self, label: str, elapsed_secs: int, client_pid: int | None) -> None:
        if not self.screenshot_enabled or self.config.screenshot_max_count == 0:
            return
        if self.screenshot_count >= self.config.screenshot_max_count:
            return
        self.screenshot_count += 1
        screenshot_file = self.artifact_dir / (
            f"screenshot_{self.run_id}_{self.screenshot_count}_{label}_{elapsed_secs}s.png"
        )
        target = capture_screenshot(
            self.platform_name,
            screenshot_file,
            client_pid,
            require_client_window=self.config.title_screen_transition_capture,
        )
        # A startup-transition frame is evidence only when it came from an
        # X11 window whose PID is the launched client.  In particular, before
        # that PID is known, a title/class heuristic can select an IDE window
        # that happens to mention Minecraft.
        if self.config.title_screen_transition_capture and not target:
            safe_unlink(screenshot_file)
            self.screenshot_count -= 1
            self.append_meta(f"transition_screenshot_rejected_nonclient_target={elapsed_secs}")
            return
        provenance = window_capture_provenance(self.platform_name, target, client_pid)
        # Timeline pixels can be retained for operator investigation only
        # after their window identity is tied to the launched JVM.  They are
        # deliberately *not* renderer/parity evidence: X11 may expose stale
        # backing-store pixels for a newly mapped GLFW drawable (including an
        # unrelated compositor surface). A title-presenter acknowledgement is
        # required for a frame to be accepted as an actual game presentation.
        if self.config.title_screen_transition_capture and provenance.get("status") != "verified":
            safe_unlink(screenshot_file)
            self.screenshot_count -= 1
            self.append_meta(f"transition_screenshot_rejected_unverified_window={elapsed_secs}")
            return
        if target:
            if self.config.title_screen_transition_capture:
                provenance["status"] = "x11-identity-only"
                provenance["pixelAttribution"] = "unverified-without-renderer-frame-correlation"
                screenshot_file.with_suffix(screenshot_file.suffix + ".provenance.json").write_text(
                    json.dumps(provenance, indent=2, sort_keys=True) + "\n", encoding="utf-8"
                )
            self.append_meta(f"screenshot_{self.screenshot_count}={screenshot_file}")
            self.append_meta(f"screenshot_{self.screenshot_count}_target={target}")
        else:
            safe_unlink(screenshot_file)
            self.append_meta(f"screenshot_{self.screenshot_count}=failed:{label}:{elapsed_secs}")

    def title_screen_receipt_ready(self, elapsed_secs: int) -> bool:
        """Require an observed title producer before a diagnostic desktop capture.

        The receipt is logging only. It intentionally neither freezes panorama
        time nor converts a title image into a strict visual-parity verdict.
        """
        if not self.config.title_screen_capture:
            return True
        # A transition capture deliberately begins before TitleScreen exists;
        # it is how a transient blank handoff can be observed. Still record the
        # same log-only title receipt once it arrives, so the artifact says
        # whether its bounded timeline actually reached the destination.
        receipt = (
            "[MattMC graphics audit] rust-title-frame-presented"
            if self.config.backend == "rust-vulkan"
            else "[MattMC graphics audit] title-screen semantic receipt"
        )
        if self.config.title_screen_transition_capture:
            if not self.title_screen_receipt_observed:
                try:
                    observed = receipt in self.run_log.read_text(
                        encoding="utf-8", errors="replace"
                    )
                except OSError:
                    observed = False
                if observed:
                    self.title_screen_receipt_observed = True
                    self.append_meta("title_screen_receipt_observed=true")
                    self.append_meta(f"title_screen_receipt_elapsed={elapsed_secs}")
            return True
        if os.environ.get("MATTMC_TITLE_SCREEN_REQUIRE_RECEIPT", "").lower() not in {"1", "true", "yes"}:
            return True
        if self.title_screen_receipt_observed:
            return True
        try:
            observed = receipt in self.run_log.read_text(
                encoding="utf-8", errors="replace"
            )
        except OSError:
            observed = False
        if observed:
            self.title_screen_receipt_observed = True
            self.append_meta("title_screen_receipt_observed=true")
            self.append_meta(f"title_screen_receipt_elapsed={elapsed_secs}")
            return True
        if not self.title_screen_receipt_wait_reported:
            self.title_screen_receipt_wait_reported = True
            self.append_meta("title_screen_capture_waiting_for_receipt=true")
        return False

    def title_screen_timeline_complete(self) -> bool:
        """Whether retained title samples prove the requested title destination.

        A Rust title/menu capture needs the held presented-frame acknowledgment,
        not merely the first TitleScreen semantic receipt (which may still be
        fading). A transition also needs its complete finite timeline.
        """
        if not self.config.title_screen_capture:
            return False
        if self.screenshot_count < max(1, self.config.screenshot_max_count):
            return False
        if self.config.backend == "rust-vulkan" and not (
            self.title_presented_frame_dir / "title_frame_capture.ack.json"
        ).is_file():
            return False
        return (
            not self.config.title_screen_transition_capture
            or self.title_screen_receipt_observed
        )

    def capture_deterministic_requests(self, client_pid: int | None) -> None:
        if not self.config.deterministic_camera_capture or not self.deterministic_screenshot_dir.is_dir():
            return
        for request in sorted(self.deterministic_screenshot_dir.glob("capture_request_*.json")):
            if request.name.endswith(".ack.json"):
                continue
            ack = request.with_name(request.name.removesuffix(".json") + ".ack.json")
            if ack.is_file():
                continue
            try:
                data = json.loads(request.read_text(encoding="utf-8"))
                screenshot = Path(data.get("screenshot", ""))
            except Exception:
                self.append_meta(f"deterministic_capture_request_invalid={request}")
                continue
            if not str(screenshot):
                self.append_meta(f"deterministic_capture_request_invalid={request}")
                continue
            target = capture_screenshot(self.platform_name, screenshot, client_pid)
            if target:
                data["status"] = "captured"
                data["screenshot"] = str(screenshot)
                data["targetWindow"] = target
                data["capturedAtEpoch"] = int(time.time())
                ack.write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")
                self.append_meta(f"deterministic_capture_ack={ack}")
                self.append_meta(f"deterministic_capture_screenshot={screenshot}")
                self.append_meta(f"deterministic_capture_target={target}")
            else:
                self.append_meta(f"deterministic_capture_failed={request}")

    def capture_rust_title_presented_frame(self, client_pid: int | None) -> bool:
        """Acknowledge the held Rust-presented title image; never read Java's target."""
        if not self.config.title_screen_capture or self.config.backend != "rust-vulkan":
            return False
        request = self.title_presented_frame_dir / "title_frame_capture.json"
        ack = self.title_presented_frame_dir / "title_frame_capture.ack.json"
        if ack.is_file() or not request.is_file():
            return ack.is_file()
        try:
            data = json.loads(request.read_text(encoding="utf-8"))
            screenshot = Path(data["screenshot"])
        except (OSError, KeyError, TypeError, json.JSONDecodeError) as error:
            self.append_meta(f"rust_title_presented_frame_request_invalid={error}")
            return False
        target = capture_screenshot(
            self.platform_name,
            screenshot,
            client_pid,
            require_client_window=True,
        )
        # The held title request proves the Rust route named an exact frame,
        # but a root-desktop screenshot cannot prove that desktop pixels came
        # from that frame. Keep the title capture incomplete rather than
        # acknowledging another application as Rust Vulkan output.
        if not target or target == "root":
            self.append_meta("rust_title_presented_frame_capture_failed=true")
            return False
        provenance = window_capture_provenance(self.platform_name, target, client_pid)
        if provenance.get("status") != "verified":
            safe_unlink(screenshot)
            self.append_meta("rust_title_presented_frame_capture_unverified_window=true")
            return False
        data.update({
            "status": "captured", "targetWindow": target,
            "capturedAtEpoch": int(time.time()), "windowProvenance": provenance,
        })
        ack.write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")
        self.append_meta(f"rust_title_presented_frame_screenshot={screenshot}")
        self.append_meta(f"rust_title_presented_frame_ack={ack}")
        return True

    def deterministic_capture_status(self) -> str | None:
        if not self.config.deterministic_camera_capture or not self.deterministic_metadata.is_file():
            return None
        try:
            data = json.loads(self.deterministic_metadata.read_text(encoding="utf-8"))
        except Exception:
            return None
        status = data.get("status")
        return status if isinstance(status, str) else None

    def subsystem_benchmark_status(self) -> str | None:
        if os.environ.get("MATTMC_GRAPHICS_SUBSYSTEM_BENCHMARK", "false").lower() != "true":
            return None
        if self.subsystem_status_path is None or not self.subsystem_status_path.is_file():
            return None
        try:
            data = json.loads(self.subsystem_status_path.read_text(encoding="utf-8"))
        except Exception:
            return None
        status = data.get("status")
        return status if isinstance(status, str) else None

    def frame_benchmark_status(self) -> str | None:
        if self.frame_benchmark_status_path is None or not self.frame_benchmark_status_path.is_file():
            return None
        try:
            data = json.loads(self.frame_benchmark_status_path.read_text(encoding="utf-8"))
        except Exception:
            return None
        status = data.get("status")
        return status if isinstance(status, str) else None

    def check_client_memory_guard(self, client_pid: int | None, elapsed: int) -> bool:
        if self.config.client_rss_limit_mb == 0 or not client_pid:
            return True
        rss_kb = process_rss_kb(client_pid, self.platform_name)
        if rss_kb <= 0:
            return True
        self.memory_guard_peak_rss_kb = max(self.memory_guard_peak_rss_kb, rss_kb)
        limit_kb = self.config.client_rss_limit_mb * 1024
        if rss_kb <= limit_kb:
            return True
        if self.config.deterministic_camera_capture and self.deterministic_capture_status() == "complete":
            self.deterministic_completed = True
            self.intentional_deterministic_shutdown = True
            self.append_meta(f"deterministic_capture_complete_elapsed={elapsed}")
            self.append_meta(f"deterministic_capture_complete_rss_kb={rss_kb}")
            self.collect_client_memory_snapshot("deterministic_complete", client_pid)
            self.terminate_run_processes("deterministic_complete")
            return False
        self.memory_guard_triggered = True
        self.memory_guard_rss_kb = rss_kb
        message = (
            f"client_rss_limit_exceeded elapsed={elapsed}s pid={client_pid} "
            f"rss_kb={rss_kb} limit_kb={limit_kb}"
        )
        self.append_meta(message)
        append_text(self.run_log, message + "\n")
        self.collect_client_memory_snapshot("rss_guard", client_pid)
        print(message, file=sys.stderr)
        self.terminate_run_processes("client_rss_limit")
        return False

    def take_dump(self, elapsed: int, client_pid: int | None) -> None:
        lines = [f"===== process snapshot at dump point (elapsed={elapsed}s, gradle_pid={self.gradle_process.pid}) ====="]
        if self.platform_name == "windows":
            lines.append(command_text(["tasklist", "/v"], cwd=self.root))
        else:
            process_table = command_text(["ps", "-eo", "pid,ppid,pgid,cmd"], cwd=self.root)
            lines.extend(
                line for line in process_table.splitlines()
                if line.strip().startswith("PID") or f" {self.gradle_process.pid} " in f" {line} "
            )
        lines.extend(["", "===== jcmd -l =====", command_text(["jcmd", "-l"], cwd=self.root, timeout_secs=8)])
        self.process_snapshot.write_text("\n".join(lines) + "\n", encoding="utf-8", errors="replace")

        if self.screenshot_enabled and self.platform_name == "linux" and shutil.which("xwininfo"):
            write_command(self.window_tree_dump, ["xwininfo", "-root", "-tree"], cwd=self.root)
            self.append_meta(f"window_tree_dump={self.window_tree_dump}")

        self.append_meta(f"dump_epoch={int(time.time())}")
        self.append_meta(f"dump_elapsed_secs={elapsed}")
        self.append_meta(f"client_pid={client_pid or 'none'}")
        if client_pid:
            text = f"===== jcmd Thread.print (pid={client_pid}) =====\n"
            text += command_text(["jcmd", str(client_pid), "Thread.print"], cwd=self.root, timeout_secs=8)
            self.thread_dump.write_text(text, encoding="utf-8", errors="replace")
            self.collect_client_memory_snapshot(f"dump_elapsed_{elapsed}", client_pid)
        else:
            self.thread_dump.write_text("No Minecraft Java PID found at dump point.\n", encoding="utf-8")
        if not self.config.deterministic_camera_capture:
            self.capture_root_screenshot("dump", elapsed, client_pid)
        self.dump_taken = True

    def collect_client_memory_snapshot(self, label: str, client_pid: int) -> None:
        lines = [
            "",
            f"===== client memory snapshot: {label} pid={client_pid} =====",
            f"rss_kb={process_rss_kb(client_pid, self.platform_name)}",
        ]
        if shutil.which("jcmd"):
            lines.extend([
                "",
                "===== jcmd GC.heap_info =====",
                command_text(["jcmd", str(client_pid), "GC.heap_info"], cwd=self.root, timeout_secs=8),
                "",
                "===== jcmd GC.class_histogram (bounded diagnostic) =====",
                # This is intentionally a non-live histogram: it identifies
                # retained heap owners without forcing a full collection or
                # changing the measured game's allocation behaviour.
                command_text(["jcmd", str(client_pid), "GC.class_histogram"], cwd=self.root, timeout_secs=8),
                "",
                "===== jcmd VM.native_memory summary =====",
                command_text(
                    ["jcmd", str(client_pid), "VM.native_memory", "summary"],
                    cwd=self.root,
                    timeout_secs=8,
                ),
            ])
        if self.platform_name == "linux":
            lines.extend(read_proc_memory_summary(client_pid))
            if shutil.which("pmap"):
                lines.extend([
                    "",
                    "===== pmap -x (native mapping attribution) =====",
                    command_text(["pmap", "-x", str(client_pid)], cwd=self.root, timeout_secs=8),
                ])
        append_text(self.process_snapshot, "\n".join(lines) + "\n")
        self.append_meta(f"client_memory_snapshot_{label}={self.process_snapshot}")

    def terminate_run_processes(self, reason: str) -> None:
        if not self.gradle_process:
            return
        self.append_meta(f"cleanup_reason={reason}")
        client_pid = self.find_client_pid()
        if client_pid:
            self.append_meta(f"cleanup_client_pid={client_pid}")
        if self.platform_name == "windows":
            if client_pid:
                subprocess.run(["taskkill", "/PID", str(client_pid), "/T"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            subprocess.run(["taskkill", "/PID", str(self.gradle_process.pid), "/T"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            time.sleep(5)
            if client_pid and process_exists(client_pid, self.platform_name):
                self.append_meta("cleanup_client_kill=true")
                subprocess.run(["taskkill", "/PID", str(client_pid), "/T", "/F"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            if process_exists(self.gradle_process.pid, self.platform_name):
                self.append_meta("cleanup_gradle_kill=true")
                subprocess.run(["taskkill", "/PID", str(self.gradle_process.pid), "/T", "/F"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        else:
            immediate_capture_kill = (
                os.environ.get("MATTMC_CAPTURE_KILL_AFTER_DETERMINISTIC", "false").lower() == "true"
                and reason == "deterministic_complete_immediate"
            )
            if immediate_capture_kill:
                # Deterministic capture has already acknowledged its frame.
                # Avoid Minecraft/DH graceful world-save teardown, which can
                # allocate an unbounded lighting/NBT graph and invalidate the
                # artifact after the required frame is safely on disk.
                self.append_meta("cleanup_mode=immediate_sigkill")
                if client_pid:
                    safe_kill(client_pid, signal.SIGKILL)
                try:
                    os.killpg(self.gradle_process.pid, signal.SIGKILL)
                except OSError:
                    pass
                safe_kill(self.gradle_process.pid, signal.SIGKILL)
                self.run_client_active = False
                return
            if client_pid:
                safe_kill(client_pid, signal.SIGTERM)
            try:
                os.killpg(self.gradle_process.pid, signal.SIGTERM)
            except OSError:
                pass
            safe_kill(self.gradle_process.pid, signal.SIGTERM)
            time.sleep(5)
            if client_pid and process_exists(client_pid, self.platform_name):
                self.append_meta("cleanup_client_kill=true")
                safe_kill(client_pid, signal.SIGKILL)
            if process_exists(self.gradle_process.pid, self.platform_name):
                self.append_meta("cleanup_gradle_kill=true")
                try:
                    os.killpg(self.gradle_process.pid, signal.SIGKILL)
                except OSError:
                    pass
                safe_kill(self.gradle_process.pid, signal.SIGKILL)
        self.run_client_active = False

    def append_final_meta(self, exit_code: int) -> None:
        self.append_meta(f"exit_code={exit_code}")
        self.append_meta(f"deterministic_completed={str(self.deterministic_completed).lower()}")
        self.append_meta(f"title_screen_capture_completed={str(self.title_screen_capture_completed).lower()}")
        self.append_meta(
            f"intentional_deterministic_shutdown={str(self.intentional_deterministic_shutdown).lower()}"
        )
        self.append_meta(f"subsystem_benchmark_completed={str(self.subsystem_benchmark_completed).lower()}")
        self.append_meta(f"intentional_subsystem_shutdown={str(self.intentional_subsystem_shutdown).lower()}")
        if self.memory_guard_triggered and self.deterministic_completed:
            self.append_meta("memory_guard_reclassified_after_complete=true")
            self.memory_guard_triggered = False
        self.append_meta(f"memory_guard_triggered={str(self.memory_guard_triggered).lower()}")
        self.append_meta(f"memory_guard_peak_rss_kb={self.memory_guard_peak_rss_kb}")
        if self.memory_guard_triggered:
            self.append_meta(f"memory_guard_rss_kb={self.memory_guard_rss_kb}")
        self.append_meta(f"end_epoch={int(time.time())}")

    def collect_final_artifacts(self) -> None:
        copy_config_snapshot(self.run_dir, self.config_after_dir)
        self.append_meta(f"config_after={self.config_after_dir}")

        latest = self.run_dir / "logs" / "latest.log"
        if latest.is_file():
            shutil.copy2(latest, self.latest_log_copy)
            self.latest_tail.write_text("\n".join(tail_lines(self.latest_log_copy, 240)) + "\n", encoding="utf-8", errors="replace")

        hs_err_files = [
            path for path in self.run_dir.glob("hs_err_pid*.log")
            if path.is_file() and path.stat().st_mtime >= self.start_epoch
        ]
        self.hserr_list.write_text("\n".join(str(path) for path in sorted(hs_err_files)) + "\n", encoding="utf-8")
        for path in hs_err_files:
            shutil.copy2(path, self.artifact_dir / f"{path.stem}_{self.run_id}.log")

        copy_recent_files_since_start(self.run_dir / "crash-reports", self.crash_report_dir, self.crash_report_list, self.start_epoch)
        copy_recent_files_since_start(self.run_dir / "debug", self.debug_snapshot_dir, self.debug_file_list, self.start_epoch)

        patched = self.run_dir / "patched_shaders"
        if patched.is_dir():
            if self.patched_shaders_snapshot.exists():
                shutil.rmtree(self.patched_shaders_snapshot)
            shutil.copytree(patched, self.patched_shaders_snapshot)
            with self.patched_shaders_manifest.open("w", encoding="utf-8") as handle:
                handle.write(f"source={patched}\n")
                for path in sorted(patched.rglob("*")):
                    if path.is_file():
                        handle.write(f"{file_sha256(path)} {path.relative_to(patched)}\n")

        self.shader_event_log.write_text("", encoding="utf-8")
        append_filtered_section("runClient log", self.run_log, SHADER_EVENT_PATTERN, self.shader_event_log)
        append_filtered_section("latest.log", self.latest_log_copy, SHADER_EVENT_PATTERN, self.shader_event_log)
        for path in read_list_paths(self.crash_report_list):
            append_filtered_section("crash report", path, SHADER_EVENT_PATTERN, self.shader_event_log)
        for path in read_list_paths(self.hserr_list):
            append_filtered_section("hs_err", path, SHADER_EVENT_PATTERN, self.shader_event_log)

        self.validation_event_log.write_text("", encoding="utf-8")
        append_filtered_section("runClient log", self.run_log, VALIDATION_EVENT_PATTERN, self.validation_event_log)
        append_filtered_section("latest.log", self.latest_log_copy, VALIDATION_EVENT_PATTERN, self.validation_event_log)
        for path in read_list_paths(self.crash_report_list):
            append_filtered_section("crash report", path, VALIDATION_EVENT_PATTERN, self.validation_event_log)

        lines = [
            f"backend={self.config.backend}",
            f"validation_mode={self.config.validation_mode}",
            f"validation_enabled={str(self.validation_enabled).lower()}",
            f"validation_layer_available={str(self.validation_layer_available).lower()}",
            f"validation_layer_manifest={self.validation_layer_manifest or 'unavailable'}",
            f"shader_input_parity={self.config.shader_input_parity}",
            f"shader_input_parity_max_logs={self.config.shader_input_parity_max_logs}",
            f"lightmap_info_parity_max_logs={self.config.lightmap_info_parity_max_logs}",
            f"effective_enable_shaders={self.effective_enable_shaders or 'unset'}",
            f"effective_shader_pack={self.effective_shader_pack or 'unset'}",
            f"run_log={self.run_log}",
            f"latest_log={self.latest_log_copy if self.latest_log_copy.is_file() else 'missing'}",
            f"shader_events={self.shader_event_log}",
            f"validation_events={self.validation_event_log}",
            f"patched_shaders_snapshot={self.patched_shaders_snapshot if self.patched_shaders_snapshot.is_dir() else 'absent'}",
            f"patched_shaders_manifest={self.patched_shaders_manifest if self.patched_shaders_manifest.is_file() else 'absent'}",
            f"crash_report_count={count_list_entries(self.crash_report_list)}",
            f"hs_err_count={count_list_entries(self.hserr_list)}",
            f"debug_artifact_count={count_list_entries(self.debug_file_list)}",
            "",
            "===== key events from runClient log =====",
        ]
        lines.extend(filtered_lines(self.run_log, KEY_SUMMARY_PATTERN, limit=200))
        self.shader_summary.write_text("\n".join(lines) + "\n", encoding="utf-8", errors="replace")

    def validate_deterministic_capture(self) -> None:
        if not self.config.deterministic_camera_capture or self.memory_guard_triggered:
            return
        try:
            validate_deterministic_metadata(
                self.deterministic_metadata,
                self.deterministic_screenshot_dir,
                self.config.deterministic_pose_tolerance,
                celestial_body=self.env.get("MATTMC_CAPTURE_CELESTIAL", ""),
            )
            self.deterministic_validation_status = "ok"
        except Exception as exc:
            self.deterministic_validation_status = "failed"
            print(str(exc), file=sys.stderr)
        self.append_meta(f"deterministic_validation={self.deterministic_validation_status}")

    def validate_audio_status(self) -> None:
        if not self.config.audio_validation:
            return
        try:
            data = json.loads(self.audio_validation_status.read_text(encoding="utf-8"))
            required_true = [
                "openAlOnSoundThread",
                "listenerUpdated",
                "staticUpdatesSucceeded",
                "streamingUpdatesSucceeded",
                "reloadSucceeded",
            ]
            for key in required_true:
                if not data.get(key):
                    raise RuntimeError(f"audio validation {key} was not true")
            if data.get("status") != "complete":
                raise RuntimeError(f"audio validation status was {data.get('status')!r}: {data.get('error')!r}")
            if int(data.get("streamingChunksSubmitted", 0)) <= 0:
                raise RuntimeError("audio validation submitted no streaming chunks")
            if int(data.get("streamingChunksProcessed", 0)) <= 0:
                raise RuntimeError("audio validation consumed no streaming chunks")
            for name in ("afterReleaseCounts", "shutdownCounts"):
                counts = data.get(name) or {}
                if any(int(counts.get(key, -1)) != 0 for key in ("sounds", "buffers", "queuedStreamBuffers")):
                    raise RuntimeError(f"audio validation {name} retained native resources: {counts}")
            shutdown = data.get("shutdownCounts") or {}
            if int(shutdown.get("devices", -1)) != 0:
                raise RuntimeError(f"audio validation shutdown retained device: {shutdown}")
            self.audio_validation_result = "ok"
            print(f"audio validation OK: {self.audio_validation_status}")
        except Exception as exc:
            self.audio_validation_result = "failed"
            print(str(exc), file=sys.stderr)
        self.append_meta(f"audio_validation={self.audio_validation_result}")

    def validate_region_status(self) -> None:
        if not self.config.region_validation:
            return
        try:
            data = json.loads(self.region_validation_status.read_text(encoding="utf-8"))
            counters = data.get("counters") or {}
            for key in ("rustErrors", "unreadableChunks", "malformedNbt"):
                if int(counters.get(key, -1)) != 0:
                    raise RuntimeError(f"region validation {key} was {counters.get(key)}")
            if int(counters.get("regionsOpened", 0)) <= 0:
                raise RuntimeError("region validation opened no regions")
            if int(counters.get("chunksRead", 0)) <= 0:
                raise RuntimeError("region validation read no chunks")
            if int(counters.get("chunksWritten", 0)) <= 0:
                raise RuntimeError("region validation wrote no chunks")
            if int(counters.get("internalPayloads", 0)) + int(counters.get("externalPayloads", 0)) <= 0:
                raise RuntimeError("region validation recorded no payloads")
            leftover_tmp = list(self.run_dir.rglob("tmp-mattmc-region-*.mcc")) + list(self.run_dir.rglob("*.tmp"))
            if leftover_tmp:
                raise RuntimeError(f"region validation found leftover temp files: {leftover_tmp[:5]}")
            self.region_validation_result = "ok"
            print(f"region validation OK: {self.region_validation_status}")
        except Exception as exc:
            self.region_validation_result = "failed"
            print(str(exc), file=sys.stderr)
        self.append_meta(f"region_validation_result={self.region_validation_result}")

    def validate_poi_status(self) -> None:
        if not self.config.poi_validation:
            return
        try:
            data = json.loads(self.poi_validation_status.read_text(encoding="utf-8"))
            if data.get("status") != "complete":
                raise RuntimeError(f"POI validation status was {data.get('status')!r}: {data.get('error')!r}")
            for key in ("worldReady", "saveRequested", "shutdownRequested", "stopped", "currentVersionRustAuthoritative"):
                if not data.get(key):
                    raise RuntimeError(f"POI validation {key} was not true")
            if int(data.get("rustDecodedChunks", 0)) <= 0:
                raise RuntimeError("POI validation decoded no chunks through Rust")
            if int(data.get("rustWrittenChunks", 0)) <= 0:
                raise RuntimeError("POI validation wrote no chunks through Rust")
            for key in ("unknownTypes", "malformedInputs", "writeFailures"):
                if int(data.get(key, -1)) != 0:
                    raise RuntimeError(f"POI validation {key} was {data.get(key)}")
            self.poi_validation_result = "ok"
            print(f"POI validation OK: {self.poi_validation_status}")
        except Exception as exc:
            self.poi_validation_result = "failed"
            print(str(exc), file=sys.stderr)
        self.append_meta(f"poi_validation_result={self.poi_validation_result}")

    def print_summary(self, exit_code: int) -> None:
        print("Capture complete.")
        print(f"- Meta:        {self.meta_log}")
        print(f"- System:      {self.system_snapshot}")
        print(f"- Git:         {self.git_snapshot}")
        print(f"- Shader info: {self.shaderpack_info}")
        print(f"- Shader sum:  {self.shader_summary}")
        print(f"- Shader log:  {self.shader_event_log}")
        print(f"- Validation:  {self.validation_event_log}")
        print(f"- Run log:     {self.run_log}")
        if self.latest_log_copy.is_file():
            print(f"- Latest log:  {self.latest_log_copy}")
        print(f"- Thread dump: {self.thread_dump}")
        print(f"- Proc snap:   {self.process_snapshot}")
        print(f"- Latest tail: {self.latest_tail}")
        print(f"- hs_err list: {self.hserr_list}")
        print(f"- Crash list:  {self.crash_report_list}")
        print(f"- Debug list:  {self.debug_file_list}")
        if self.patched_shaders_snapshot.is_dir():
            print(f"- Patched shaders: {self.patched_shaders_snapshot}")
        if self.window_tree.is_file():
            print(f"- Window tree: {self.window_tree}")
        if self.config.deterministic_camera_capture:
            print(f"- Deterministic metadata: {self.deterministic_metadata}")
            print(f"- Deterministic screenshots: {self.deterministic_screenshot_dir}")
            print(f"- Deterministic validation: {self.deterministic_validation_status}")
        if self.config.audio_validation:
            print(f"- Audio validation: {self.audio_validation_status}")
            print(f"- Audio result:     {self.audio_validation_result}")
        if self.config.region_validation:
            print(f"- Region validation: {self.region_validation_status}")
            print(f"- Region result:     {self.region_validation_result}")
        if self.config.poi_validation:
            print(f"- POI validation:    {self.poi_validation_status}")
            print(f"- POI result:        {self.poi_validation_result}")
            print(f"- Game dir:          {self.run_dir}")
        if self.config.capture_meshing_corpus:
            output = Path(self.config.meshing_corpus_output) if self.config.meshing_corpus_output else self.meshing_corpus_replay
            output = output if output.is_absolute() else (self.root / output)
            print(f"- Meshing replay:   {output}")

        if self.timed_out:
            print(f"Result: timed out after {self.config.max_secs}s and was terminated automatically.")
        elif self.memory_guard_triggered:
            print(f"Result: terminated because client RSS exceeded {self.config.client_rss_limit_mb} MiB.")
        elif exit_code != 0:
            print(f"Result: exited with non-zero code {exit_code}.")
        else:
            print("Result: exited cleanly.")


def write_poi_validation_fixture(world_dir: Path) -> Path:
    """Write a copied-world-only current-version POI region at spawn."""
    import zlib

    poi_dir = world_dir / "poi"
    poi_dir.mkdir(parents=True, exist_ok=True)
    region_path = poi_dir / "r.0.0.mca"
    nbt = nbt_root_compound(
        [
            nbt_int("DataVersion", 4295),
            nbt_compound(
                "Sections",
                [
                    nbt_compound(
                        "0",
                        [
                            nbt_byte("Valid", 1),
                            nbt_list(
                                "Records",
                                10,
                                [
                                    nbt_anonymous_compound(
                                        [
                                            nbt_int_array("pos", [0, 64, 0]),
                                            nbt_string("type", "minecraft:home"),
                                            nbt_int("free_tickets", 0),
                                        ]
                                    ),
                                    nbt_anonymous_compound(
                                        [
                                            nbt_int_array("pos", [1, 64, 0]),
                                            nbt_string("type", "minecraft:meeting"),
                                            nbt_int("free_tickets", 1),
                                        ]
                                    ),
                                ],
                            ),
                        ],
                    ),
                    nbt_compound(
                        "4",
                        [
                            nbt_byte("Valid", 1),
                            nbt_list(
                                "Records",
                                10,
                                [
                                    nbt_anonymous_compound(
                                        [
                                            nbt_int_array("pos", [2, 80, 2]),
                                            nbt_string("type", "minecraft:armorer"),
                                            nbt_int("free_tickets", 2),
                                        ]
                                    )
                                ],
                            ),
                        ],
                    ),
                ],
            ),
        ]
    )
    compressed = zlib.compress(nbt)
    length = len(compressed) + 1
    chunk = length.to_bytes(4, "big") + bytes([2]) + compressed
    sector_count = max(1, (len(chunk) + 4095) // 4096)
    chunk = chunk + bytes(sector_count * 4096 - len(chunk))
    header = bytearray(8192)
    header[0:4] = bytes([0, 0, 2, sector_count])
    header[4096:4100] = int(time.time()).to_bytes(4, "big")
    region_path.write_bytes(bytes(header) + chunk)
    return region_path


def nbt_root_compound(children: list[bytes]) -> bytes:
    return bytes([10]) + modified_utf8_name("") + b"".join(children) + bytes([0])


def nbt_anonymous_compound(children: list[bytes]) -> bytes:
    return b"".join(children) + bytes([0])


def nbt_compound(name: str, children: list[bytes]) -> bytes:
    return bytes([10]) + modified_utf8_name(name) + b"".join(children) + bytes([0])


def nbt_list(name: str, element_type: int, values: list[bytes]) -> bytes:
    return bytes([9]) + modified_utf8_name(name) + bytes([element_type]) + len(values).to_bytes(4, "big") + b"".join(values)


def nbt_byte(name: str, value: int) -> bytes:
    return bytes([1]) + modified_utf8_name(name) + int(value).to_bytes(1, "big", signed=True)


def nbt_int(name: str, value: int) -> bytes:
    return bytes([3]) + modified_utf8_name(name) + int(value).to_bytes(4, "big", signed=True)


def nbt_string(name: str, value: str) -> bytes:
    encoded = modified_utf8_bytes(value)
    return bytes([8]) + modified_utf8_name(name) + len(encoded).to_bytes(2, "big") + encoded


def nbt_int_array(name: str, values: list[int]) -> bytes:
    payload = len(values).to_bytes(4, "big") + b"".join(int(value).to_bytes(4, "big", signed=True) for value in values)
    return bytes([11]) + modified_utf8_name(name) + payload


def modified_utf8_name(value: str) -> bytes:
    encoded = modified_utf8_bytes(value)
    return len(encoded).to_bytes(2, "big") + encoded


def modified_utf8_bytes(value: str) -> bytes:
    output = bytearray()
    units = value.encode("utf-16-be")
    for index in range(0, len(units), 2):
        unit = units[index] << 8 | units[index + 1]
        if unit == 0:
            output.extend((0xC0, 0x80))
        elif unit <= 0x7F:
            output.append(unit)
        elif unit <= 0x7FF:
            output.extend((0xC0 | (unit >> 6), 0x80 | (unit & 0x3F)))
        else:
            output.extend((0xE0 | (unit >> 12), 0x80 | ((unit >> 6) & 0x3F), 0x80 | (unit & 0x3F)))
    if len(output) > 65535:
        raise ValueError("modified UTF-8 string too long")
    return bytes(output)


def script_dir() -> Path:
    return Path(__file__).resolve().parent


def common_platform_dir() -> Path:
    return repo_root() / "DevUtils" / "Common" / "platform"


def load_platform_detection():
    detection_dir = common_platform_dir() / "detection"
    sys.path.insert(0, str(detection_dir))
    from platform_detection import detect_platform_info, normalize_platform

    return detect_platform_info(), normalize_platform


def repo_root() -> Path:
    current = script_dir()
    while current != current.parent:
        if (current / "gradlew").is_file() or (current / "gradlew.bat").is_file():
            return current
        current = current.parent
    raise SystemExit("ERROR: Could not find gradlew from DevUtils")


def gradle_command(root: Path, platform_name: str) -> list[str]:
    if platform_name == "windows":
        wrapper = root / "gradlew.bat"
        if wrapper.is_file():
            return [str(wrapper)]
    wrapper = root / "gradlew"
    if wrapper.is_file():
        return [str(wrapper)]
    fallback = root / "gradlew.bat"
    if fallback.is_file():
        return [str(fallback)]
    raise SystemExit("ERROR: Could not find gradlew. Are you in the MattMC project?")


def lock_refusal_message() -> str:
    return (
        "Refusing to start the graphics capture runner because another capture instance is already running.\n"
        "Concurrent captures can lock Origin and corrupt capture results."
    )


def command_text(command: list[str], *, cwd: Path, timeout_secs: int | None = None) -> str:
    if not shutil.which(command[0]) and not Path(command[0]).exists():
        return ""
    try:
        result = subprocess.run(
            command,
            cwd=cwd,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            timeout=timeout_secs,
        )
    except subprocess.TimeoutExpired:
        return f"command timed out after {timeout_secs}s: {' '.join(command)}\n"
    except OSError as exc:
        return str(exc)
    return result.stdout or ""


def write_command(path: Path, command: list[str], *, cwd: Path) -> None:
    path.write_text(command_text(command, cwd=cwd), encoding="utf-8", errors="replace")


def write_sections(path: Path, sections: list[tuple[str, list[str]]], *, cwd: Path) -> None:
    lines: list[str] = []
    for label, command in sections:
        lines.extend([f"===== {label} =====", command_text(command, cwd=cwd), ""])
    path.write_text("\n".join(lines), encoding="utf-8", errors="replace")


def safe_unlink(path: Path) -> None:
    try:
        path.unlink()
    except FileNotFoundError:
        pass


def append_text(path: Path, text: str) -> None:
    with path.open("a", encoding="utf-8", errors="replace") as handle:
        handle.write(text)


def find_existing_mattmc_clients() -> list[str]:
    lines = command_text(["jcmd", "-l"], cwd=Path.cwd()).splitlines()
    return [
        line for line in lines
        if CLIENT_PROCESS_PATTERN.search(line)
        and not re.search(r"jcmd|GradleWrapperMain|GradleDaemon", line)
    ]


def list_java_processes() -> list[tuple[int, str]]:
    processes: list[tuple[int, str]] = []
    jcmd_output = command_text(["jcmd", "-l"], cwd=Path.cwd())
    for line in jcmd_output.splitlines():
        parts = line.strip().split(maxsplit=1)
        if not parts:
            continue
        try:
            pid = int(parts[0])
        except ValueError:
            continue
        command = parts[1] if len(parts) > 1 else ""
        if not re.search(r"jcmd|GradleWrapperMain|GradleDaemon", command):
            processes.append((pid, command))
    return processes


def screenshot_backend_available(platform_name: str) -> bool:
    if platform_name == "linux":
        return bool(shutil.which("import") and os.environ.get("DISPLAY"))
    if platform_name == "macos":
        return bool(shutil.which("screencapture"))
    if platform_name == "windows":
        return bool(shutil.which("powershell") or shutil.which("pwsh"))
    return False


def capture_screenshot(
    platform_name: str,
    screenshot_file: Path,
    client_pid: int | None,
    *,
    require_client_window: bool = False,
) -> str | None:
    screenshot_file.parent.mkdir(parents=True, exist_ok=True)
    if platform_name == "linux":
        target = find_linux_client_window_id(client_pid)
        if not target:
            # Deterministic world captures predate PID discovery and retain
            # their ordinary desktop fallback.  Startup/title evidence is
            # stricter: it remains unavailable until an exact client window
            # exists, rather than risking an unrelated application capture.
            if require_client_window:
                return None
            target = "root"
        if target != "root" and not linux_x11_window_is_viewable(target):
            return None
        if capture_linux_x11_window(target, screenshot_file):
            return target
        return None
    if platform_name == "macos":
        if subprocess.run(
            ["screencapture", "-x", str(screenshot_file)],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        ).returncode == 0:
            return "screen"
        return None
    if platform_name == "windows":
        shell = shutil.which("powershell") or shutil.which("pwsh")
        if not shell:
            return None
        escaped_screenshot = str(screenshot_file).replace("'", "''")
        script = (
            "Add-Type -AssemblyName System.Windows.Forms;"
            "Add-Type -AssemblyName System.Drawing;"
            "$b=[System.Windows.Forms.Screen]::PrimaryScreen.Bounds;"
            "$bmp=New-Object System.Drawing.Bitmap $b.Width,$b.Height;"
            "$g=[System.Drawing.Graphics]::FromImage($bmp);"
            "$g.CopyFromScreen($b.Location,[System.Drawing.Point]::Empty,$b.Size);"
            f"$bmp.Save('{escaped_screenshot}',[System.Drawing.Imaging.ImageFormat]::Png);"
            "$g.Dispose();$bmp.Dispose();"
        )
        if subprocess.run(
            [shell, "-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", script],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        ).returncode == 0:
            return "screen"
    return None


def capture_linux_x11_window(target: str, screenshot_file: Path) -> bool:
    """Capture exactly one X11 window, without ImageMagick's window fallback.

    `import -window` can report success while returning another application's
    desktop pixels in this environment.  xwd reads the requested drawable by
    id and fails when it cannot.  Convert its temporary XWD only after that
    direct read succeeds, so a screenshot acknowledgement cannot be based on
    ImageMagick's implicit root-window fallback.
    """
    if target == "root" or not shutil.which("xwd"):
        return False
    raw_capture = screenshot_file.with_suffix(screenshot_file.suffix + ".xwd")
    try:
        captured = subprocess.run(
            ["xwd", "-silent", "-id", target, "-out", str(raw_capture)],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        ).returncode == 0
        if not captured or not raw_capture.is_file() or raw_capture.stat().st_size == 0:
            return False
        if __package__:
            from .xwd_pixels import write_png
        else:
            from xwd_pixels import write_png
        try:
            write_png(raw_capture, screenshot_file)
        except (ValueError, OSError):
            return False
        return screenshot_file.is_file()
    finally:
        if os.environ.get("MATTMC_CAPTURE_RETAIN_XWD", "").lower() != "true":
            safe_unlink(raw_capture)


def linux_x11_window_is_viewable(target: str) -> bool:
    """Reject an unmapped window whose backing store can contain stale pixels."""
    if not shutil.which("xwininfo"):
        return False
    info = command_text(["xwininfo", "-id", target], cwd=Path.cwd())
    return bool(re.search(r"Map State:\s*IsViewable", info))


def find_linux_client_window_id(client_pid: int | None) -> str | None:
    if not shutil.which("xprop"):
        return None
    client_list = command_text(["xprop", "-root", "_NET_CLIENT_LIST_STACKING"], cwd=Path.cwd())
    raw_ids = re.findall(r"0x[0-9a-fA-F]+", client_list)
    for window_id in reversed(raw_ids):
        props = command_text(
            ["xprop", "-id", window_id, "_NET_WM_PID", "WM_NAME", "_NET_WM_NAME", "WM_CLASS"],
            cwd=Path.cwd(),
        )
        pid_match = re.search(r"_NET_WM_PID\(CARDINAL\)\s*=\s*(\d+)", props)
        if client_pid and pid_match and pid_match.group(1) == str(client_pid):
            return window_id
        # A launched client PID is authoritative. Do not use a title/class
        # heuristic in that case: unrelated IDE, terminal, or Codex windows
        # can contain a project path or Minecraft text and would turn a
        # transition diagnostic into a false renderer frame.
        if client_pid is not None:
            continue
        if re.search(r"Minecraft|LWJGL|GLFW|KnotClient|devlaunchinjector", props, re.IGNORECASE):
            return window_id
    return None


def window_capture_provenance(
    platform_name: str,
    target: str | None,
    client_pid: int | None,
) -> dict[str, object]:
    """Record enough immutable capture provenance to reject a foreign window.

    This is harness-only evidence.  It neither reads renderer pixels through a
    backend API nor affects the client; the screenshot remains an external
    window capture.  Linux transition/title routes require an exact PID match.
    """
    evidence: dict[str, object] = {
        "schema": "mattmc-window-capture-provenance-v1",
        "platform": platform_name,
        "targetWindow": target,
        "expectedClientPid": client_pid,
        "status": "unverified",
    }
    if platform_name != "linux":
        evidence["status"] = "unsupported-platform"
        return evidence
    if not target or target == "root" or client_pid is None or not shutil.which("xprop"):
        return evidence
    props = command_text(
        ["xprop", "-id", target, "_NET_WM_PID", "WM_NAME", "_NET_WM_NAME", "WM_CLASS"],
        cwd=Path.cwd(),
    )
    pid_match = re.search(r"_NET_WM_PID\(CARDINAL\)\s*=\s*(\d+)", props)
    observed_pid = int(pid_match.group(1)) if pid_match else None
    evidence["observedWindowPid"] = observed_pid
    evidence["windowProperties"] = props
    viewable = linux_x11_window_is_viewable(target)
    evidence["windowViewable"] = viewable
    if observed_pid == client_pid and viewable:
        evidence["status"] = "verified"
    return evidence


def find_validation_layer_manifest(platform_name: str) -> Path | None:
    candidates: list[Path] = []
    vulkan_sdk = os.environ.get("VULKAN_SDK")
    if vulkan_sdk:
        root = Path(vulkan_sdk)
        candidates.extend(
            [
                root / "share" / "vulkan" / "explicit_layer.d" / "VkLayer_khronos_validation.json",
                root / "Bin" / "VkLayer_khronos_validation.json",
                root / "Bin32" / "VkLayer_khronos_validation.json",
            ]
        )
    if platform_name == "linux":
        candidates.extend(
            Path(path) for path in [
                "/usr/share/vulkan/explicit_layer.d/VkLayer_khronos_validation.json",
                "/usr/local/share/vulkan/explicit_layer.d/VkLayer_khronos_validation.json",
                "/etc/vulkan/explicit_layer.d/VkLayer_khronos_validation.json",
            ]
        )
    elif platform_name == "macos":
        candidates.extend(
            Path(path) for path in [
                "/usr/local/share/vulkan/explicit_layer.d/VkLayer_khronos_validation.json",
                "/opt/homebrew/share/vulkan/explicit_layer.d/VkLayer_khronos_validation.json",
            ]
        )
    for candidate in candidates:
        if candidate.is_file():
            return candidate
    return None


def copy_config_snapshot(run_dir: Path, target_dir: Path) -> None:
    target_dir.mkdir(parents=True, exist_ok=True)
    for rel_path in [
        "options.txt",
        "config/iris.properties",
        "config/iris-excluded.json",
        "config/DistantHorizons.toml",
        "config/sodium-options.json",
        "config/sodium-mixins.properties",
        "config/voxelmap.properties",
    ]:
        source = run_dir / rel_path
        if source.is_file():
            shutil.copy2(source, target_dir / source.name)


def extract_property_value(file_path: Path, key: str) -> str:
    if not file_path.is_file():
        return ""
    for line in file_path.read_text(encoding="utf-8", errors="replace").splitlines():
        if line.startswith(f"{key}="):
            return line.split("=", 1)[1]
    return ""


def extract_client_arg_assignment(client_args: str, key: str) -> str:
    if not client_args:
        return ""
    try:
        for token in shlex.split(client_args):
            if token.startswith(f"{key}="):
                return token.split("=", 1)[1]
    except ValueError:
        pass
    match = re.search(rf"(^|\s){re.escape(key)}=((?:'[^']*')|(?:\"[^\"]*\")|[^\s]+)", client_args)
    return match.group(2).strip("'\"") if match else ""


def append_client_arg(client_args: str, arg: str) -> str:
    return f"{client_args} {arg}".strip() if client_args else arg


def remove_client_arg_option(client_args: str, option: str) -> str:
    if not client_args:
        return ""
    try:
        tokens = shlex.split(client_args)
        filtered: list[str] = []
        skip_next = False
        for token in tokens:
            if skip_next:
                skip_next = False
                continue
            if token == option:
                skip_next = True
                continue
            if token.startswith(f"{option}="):
                continue
            filtered.append(token)
        return shlex.join(filtered)
    except ValueError:
        pass
    pattern = rf"(^|\s){re.escape(option)}(=[^\s]+|\s+[^\s]+)?"
    return re.sub(pattern, " ", client_args).strip()


def remove_client_arg_assignment(client_args: str, key: str) -> str:
    if not client_args:
        return ""
    try:
        return shlex.join(token for token in shlex.split(client_args) if not token.startswith(f"{key}="))
    except ValueError:
        pass
    pattern = rf"(^|\s){re.escape(key)}=[^\s]+"
    return re.sub(pattern, " ", client_args).strip()


def client_args_contains_option(client_args: str, option: str) -> bool:
    try:
        tokens = shlex.split(client_args)
        return option in tokens or any(token.startswith(f"{option}=") for token in tokens)
    except ValueError:
        return f" {option}" in f" {client_args} "


def client_args_contains_assignment(client_args: str, key: str) -> bool:
    try:
        return any(token.startswith(f"{key}=") for token in shlex.split(client_args))
    except ValueError:
        return f" {key}=" in f" {client_args} "


def upsert_property(file_path: Path, key: str, value: str) -> bool:
    if not file_path.is_file():
        return False
    lines = file_path.read_text(encoding="utf-8", errors="replace").splitlines()
    changed = False
    for index, line in enumerate(lines):
        if line.startswith(f"{key}="):
            lines[index] = f"{key}={value}"
            changed = True
            break
    if not changed:
        lines.append(f"{key}={value}")
    file_path.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return True


def upsert_option(file_path: Path, key: str, value: str) -> bool:
    if not file_path.is_file():
        return False
    lines = file_path.read_text(encoding="utf-8", errors="replace").splitlines()
    changed = False
    for index, line in enumerate(lines):
        if line.startswith(f"{key}:"):
            lines[index] = f"{key}:{value}"
            changed = True
            break
    if not changed:
        lines.append(f"{key}:{value}")
    file_path.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return True


def upsert_toml_value(file_path: Path, key: str, value: str) -> bool:
    if not file_path.is_file():
        return False
    lines = file_path.read_text(encoding="utf-8", errors="replace").splitlines()
    changed = False
    for index, line in enumerate(lines):
        stripped = line.strip()
        if stripped.startswith(f"{key} ") or stripped.startswith(f"{key}="):
            indent = line[: len(line) - len(line.lstrip())]
            lines[index] = f"{indent}{key} = {value}"
            changed = True
            break
    if not changed:
        lines.append(f"{key} = {value}")
    file_path.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return True


GUI_RESOURCE_PACK_SPRITES = (
    ("assets/minecraft/textures/gui/sprites/hud/crosshair.png", 15, 15),
    ("assets/minecraft/textures/gui/sprites/hud/hotbar.png", 182, 22),
    ("assets/minecraft/textures/gui/sprites/hud/hotbar_selection.png", 24, 23),
    ("assets/minecraft/textures/gui/sprites/hud/armor_empty.png", 9, 9),
    ("assets/minecraft/textures/gui/sprites/hud/armor_half.png", 9, 9),
    ("assets/minecraft/textures/gui/sprites/hud/armor_full.png", 9, 9),
    ("assets/minecraft/textures/gui/sprites/hud/heart/container.png", 9, 9),
    ("assets/minecraft/textures/gui/sprites/hud/heart/full.png", 9, 9),
    ("assets/minecraft/textures/gui/sprites/hud/heart/half.png", 9, 9),
    ("assets/minecraft/textures/gui/sprites/hud/heart/absorbing_full.png", 9, 9),
    ("assets/minecraft/textures/gui/sprites/hud/heart/absorbing_half.png", 9, 9),
    ("assets/minecraft/textures/gui/sprites/hud/experience_bar_background.png", 182, 5),
    ("assets/minecraft/textures/gui/sprites/hud/experience_bar_progress.png", 182, 5),
    ("assets/minecraft/textures/gui/sprites/boss_bar/pink_background.png", 182, 5),
    ("assets/minecraft/textures/gui/sprites/boss_bar/pink_progress.png", 182, 5),
)

WORLD_BORDER_RESOURCE_PACK_TEXTURE = "assets/minecraft/textures/misc/forcefield.png"
WORLD_CRACK_RESOURCE_PACK_TEXTURES = tuple(
    f"assets/minecraft/textures/block/destroy_stage_{stage}.png" for stage in range(10)
)
WORLD_MATERIAL_BLOCK_MARKER_TEXTURES = (
    "assets/minecraft/textures/item/barrier.png",
    *(f"assets/minecraft/textures/item/light_{level:02d}.png" for level in range(16)),
)
WORLD_MATERIAL_TERRAIN_PARTICLE_TEXTURES = (
    "assets/minecraft/textures/block/stone.png",
    "assets/minecraft/textures/block/dirt.png",
    "assets/minecraft/textures/block/oak_leaves.png",
    "assets/minecraft/textures/block/deepslate.png",
    "assets/minecraft/textures/block/white_wool.png",
    "assets/minecraft/textures/block/sand.png",
    "assets/minecraft/textures/block/gravel.png",
    "assets/minecraft/textures/block/white_concrete_powder.png",
)
WORLD_STATIC_TERRAIN_IDENTITY_TEXTURES = {
	"assets/minecraft/textures/block/grass_block_top.png": (42, 98, 51, 255),
	"assets/minecraft/textures/block/grass_block_side.png": (35, 87, 44, 255),
	"assets/minecraft/textures/block/grass_block_side_overlay.png": (26, 126, 39, 255),
	"assets/minecraft/textures/block/dirt.png": (92, 57, 36, 255),
	"assets/minecraft/textures/block/redstone_ore.png": (230, 27, 45, 255),
	"assets/minecraft/textures/block/yellow_terracotta.png": (244, 185, 28, 255),
	"assets/minecraft/textures/block/oak_leaves.png": (54, 112, 48, 255),
	"assets/minecraft/textures/block/oak_leaves2.png": (47, 104, 42, 255),
	"assets/minecraft/textures/block/oak_leaves_bushy.png": (61, 121, 51, 255),
	"assets/minecraft/textures/block/oak_leaves_bushy1.png": (43, 94, 37, 255),
}


GUI_PACK_COLORS = {
    "a": (238, 37, 67, 255),
    "b": (35, 212, 98, 255),
}


FLAT_ITEM_TINTS = (0xFF8040, 0x40FF80, 0x8040FF, 0xFFFFFF, 0x80FFFF,
                   0xFF80FF, 0xFFFF80, 0x808080, 0x4080C0)
FLAT_ITEM_RECTS = ((0,0,8,8), (8,0,16,8), (0,8,8,16), (8,8,16,16),
                   (4,4,12,12), (0,4,16,12), (4,0,12,16), (2,6,10,14), (0,0,16,16))
FLAT_ITEM_UV_COLORS = ((238,37,67), (35,212,98), (40,70,235), (235,200,30))
FLAT_ITEM_ORIENTATIONS = (((0,0,16,16),0), ((16,0,0,16),0), ((0,16,16,0),0),
    ((16,16,0,0),0), ((0,0,16,16),90), ((0,0,16,16),180), ((0,0,16,16),270),
    ((16,0,0,16),90), ((0,16,16,0),90))


def flat_item_orientation_colors(index):
    (u0,v0,u1,v1),rotation = FLAT_ITEM_ORIENTATIONS[index]
    colors = []
    for x,y in ((0.25,0.25),(0.75,0.25),(0.25,0.75),(0.75,0.75)):
        # BlockElementFace/Quadrant rotate the authored vertex UV indices.
        for _ in range(rotation//90): x,y = y,1-x
        u,v = u0+x*(u1-u0),v0+y*(v1-v0)
        colors.append(FLAT_ITEM_UV_COLORS[(int(v)//8)*2+int(u)//8])
    return colors


def gui_resource_pack_specs(scenario: str) -> list[dict[str, object]]:
    if scenario in ("shield-animation", "shield-animation-interpolated"):
        return [dict(name="mattmc-" + scenario, variant="a", shield_animation=True,
                     shield_interpolation=scenario.endswith("-interpolated"),
                     sprites=(("assets/minecraft/textures/entity/shield_base_nopattern.png",64,192),))]
    if scenario in ("shield-alpha", "shield-alpha-zero", "shield-alpha-occlusion"):
        return [dict(name="mattmc-" + scenario, variant="a", shield_alpha=True,
                     shield_alpha_zero=scenario != "shield-alpha",
                     shield_alpha_opaque_handle=scenario == "shield-alpha-occlusion",
                     sprites=(("assets/minecraft/textures/entity/shield_base_nopattern.png", 64, 64),))]
    if scenario == "experience-orb-occlusion":
        return [{"name":"mattmc-experience-orb-occlusion", "variant":"a", "solid_occluder":True,
                 "sprites":(("assets/minecraft/textures/block/stone.png",16,16),)}]
    if scenario == "experience-orb-replacement":
        return gui_resource_pack_specs("experience-orb-b") + gui_resource_pack_specs("experience-orb-a")
    if scenario in ("experience-orb-a", "experience-orb-b"):
        return [{"name": "mattmc-" + scenario, "variant": scenario[-1],
                 "experience_orb_sheet": True,
                 "sprites": (("assets/minecraft/textures/entity/experience_orb.png", 64, 64),)}]
    if scenario == "block-marker-replacement":
        return gui_resource_pack_specs("block-marker-b") + gui_resource_pack_specs("block-marker-a")
    if scenario in ("block-marker-a", "block-marker-b"):
        return [{"name": "mattmc-" + scenario, "variant": scenario[-1],
                 "sprites": tuple((f"assets/minecraft/textures/item/{name}.png",16,16)
                     for name in ("barrier", *(f"light_{level:02d}" for level in range(16))))}]
    if scenario in ("block-item-foil", "block-item-foil-moving"):
        return [dict(name="mattmc-block-item-foil",variant="a",sprites=(),item_foil_blend=True,item_foil_pattern=True)]
    if scenario in ("block-item-expanded", "block-item-oversized"):
        return [dict(name="mattmc-"+scenario,variant="a",sprites=(),block_item_expanded=True,
                     block_item_oversized=scenario=="block-item-oversized")]
    if scenario in ("special-item-foil-pattern", "special-item-foil-moving"):
        # Glint only: preserve vanilla clock/compass models, selectors and sprites.
        return [dict(name="mattmc-special-item-foil-pattern", variant="a", sprites=(),
                     item_foil_blend=True, item_foil_pattern=True)]
    if scenario == "flat-item-foil-nearest-wide":
        spec, = gui_resource_pack_specs("flat-item-foil-nearest")
        return [dict(spec, name="mattmc-"+scenario, item_foil_sprite_size=512)]
    if scenario in ("flat-item-foil-nearest", "flat-item-foil-clamp", "flat-item-foil-nearest-clamp"):
        spec, = gui_resource_pack_specs("flat-item-foil-pattern")
        return [dict(spec, name="mattmc-"+scenario, item_foil_sampler={
            "blur": "nearest" not in scenario, "clamp": "clamp" in scenario})]
    if scenario == "flat-item-foil-wide":
        spec, = gui_resource_pack_specs("flat-item-foil-pattern")
        # Ordinary high-resolution resource-pack sprite: the model geometry
        # is unchanged, but its atlas UV interval exercises foil repeat/filter.
        return [dict(spec, name="mattmc-flat-item-foil-wide", item_foil_sprite_size=512)]
    if scenario in ("flat-item-foil-pattern", "flat-item-foil-moving"):
        spec, = gui_resource_pack_specs("flat-item-foil-blend")
        return [dict(spec, name="mattmc-flat-item-foil-pattern", item_foil_pattern=True)]
    if scenario == "flat-item-foil-generated":
        spec, = gui_resource_pack_specs("flat-item-foil-blend")
        return [dict(spec, name="mattmc-flat-item-foil-generated", item_generated=True)]
    if scenario == "flat-item-foil-cutout":
        spec, = gui_resource_pack_specs("flat-item-foil-generated")
        return [dict(spec, name="mattmc-flat-item-foil-cutout", item_cutout=True, item_alpha_backdrop=True)]
    if scenario == "flat-item-foil-blend":
        spec, = gui_resource_pack_specs("flat-item-uv")
        return [dict(spec,name="mattmc-flat-item-foil-blend",item_foil_blend=True,item_uvs=((0,0,16,16),)*9)]
    if scenario == "flat-item-transform-replacement":
        after, = gui_resource_pack_specs("flat-item-child-transforms")
        before=dict(after,name="mattmc-flat-item-transform-original",item_child_transform_original=True)
        return [dict(after,name="mattmc-flat-item-transform-next"),before]
    if scenario == "flat-item-child-transforms":
        spec, = gui_resource_pack_specs("flat-item-transformed-asymmetric")
        return [dict(spec,name="mattmc-flat-item-child-transforms",item_child_transforms=True)]
    if scenario == "flat-item-transformed-asymmetric":
        spec, = gui_resource_pack_specs("flat-item-transformed")
        return [dict(spec,name="mattmc-flat-item-transformed-asymmetric",item_transform_asymmetric=True)]
    if scenario == "flat-item-transformed":
        spec, = gui_resource_pack_specs("flat-item-layers")
        return [dict(spec,name="mattmc-flat-item-transformed",item_transformed=True)]
    if scenario == "flat-item-rotated":
        spec, = gui_resource_pack_specs("flat-item-layers")
        return [dict(spec,name="mattmc-flat-item-rotated",item_rotated=True)]
    if scenario == "flat-item-layer-replacement":
        before, = gui_resource_pack_specs("flat-item-layers")
        after, = gui_resource_pack_specs("flat-item-layer-alpha")
        return [dict(after,name="mattmc-flat-item-layer-next",item_remove_last_layer=True),before]
    if scenario in ("flat-item-layers", "flat-item-layer-alpha"):
        spec, = gui_resource_pack_specs("flat-item-uv")
        return [dict(spec,name="mattmc-"+scenario,item_layers=True,item_alpha_backdrop=True,
                     item_layer_alpha=scenario == "flat-item-layer-alpha",
                     item_uvs=((0,0,16,16),)*9)]
    if scenario in ("flat-item-animation", "flat-item-animation-interpolated"):
        spec, = gui_resource_pack_specs("flat-item-uv")
        return [dict(spec,name="mattmc-"+scenario,item_animation=True,
                     item_interpolation=scenario.endswith("-interpolated"),
                     item_uvs=((0,0,16,16),)*9)]
    if scenario == "flat-item-orientation":
        spec, = gui_resource_pack_specs("flat-item-uv")
        return [dict(spec,name="mattmc-flat-item-orientation",
            item_uvs=tuple(uv for uv,_ in FLAT_ITEM_ORIENTATIONS),
            item_rotations=tuple(rotation for _,rotation in FLAT_ITEM_ORIENTATIONS))]
    if scenario == "flat-item-uv":
        spec, = gui_resource_pack_specs("flat-item-geometry")
        return [dict(spec,name="mattmc-flat-item-uv",item_uvs=FLAT_ITEM_RECTS,
                     item_rects=((0,0,16,16),)*9)]
    if scenario == "flat-item-geometry":
        spec, = gui_resource_pack_specs("flat-item-b")
        return [dict(spec, name="mattmc-flat-item-geometry", item_rects=FLAT_ITEM_RECTS)]
    if scenario == "flat-item-tint-alpha":
        spec, = gui_resource_pack_specs("flat-item-alpha")
        return [dict(spec, name="mattmc-flat-item-tint-alpha", item_tints=FLAT_ITEM_TINTS)]
    if scenario == "flat-item-alpha-backed":
        spec, = gui_resource_pack_specs("flat-item-alpha")
        return [dict(spec, name="mattmc-flat-item-alpha-backed", item_alpha_backdrop=True)]
    if scenario == "flat-item-alpha":
        specs = gui_resource_pack_specs("flat-item-b")
        return [dict(specs[0], name="mattmc-flat-item-alpha", item_alpha_steps=True)]
    if scenario == "flat-item-replacement":
        return gui_resource_pack_specs("flat-item-b") + gui_resource_pack_specs("flat-item-a")
    if scenario in ("flat-item-a", "flat-item-b"):
        size = 16 if scenario.endswith("a") else 32
        return [{"name": "mattmc-" + scenario, "variant": scenario[-1],
                 "sprites": tuple((f"assets/minecraft/textures/item/{name}.png", size, size)
                                  for name in ("apple", "feather", "paper", "diamond", "iron_ingot",
                                               "stick", "redstone", "arrow", "coal"))}]
    if scenario == "particle-atlas-static-replacement":
        return gui_resource_pack_specs("particle-atlas-static-b") + gui_resource_pack_specs("particle-atlas-static-a")
    if scenario in ("particle-atlas-static-a", "particle-atlas-static-b"):
        return [{"name": "mattmc-" + scenario, "variant": scenario[-1], "sprites": (),
                 "particle_atlas_static": True}]
    if scenario == "particle-atlas-animation":
        return [{"name": "mattmc-particle-atlas-animation", "variant": "a", "sprites": (),
                 "particle_atlas_animation": True}]
    if scenario == "lava-mip-minification":
        return [{"name": "mattmc-lava-mip-minification", "variant": "a", "sprites": (),
                 "water_mip_compatible": True, "lava_mip_minification": True}]
    if scenario in ("water-face-isolation", "water-bottom-isolation", "water-overlay-isolation", "water-overlay-hidden"):
        return [{"name": "mattmc-" + scenario, "variant": "a", "sprites": (),
                 "water_face_isolation": True, "water_bottom_isolation": scenario == "water-bottom-isolation",
                 "water_overlay_isolation": scenario in ("water-overlay-isolation", "water-overlay-hidden"),
                 "water_overlay_hidden": scenario == "water-overlay-hidden"}]
    if scenario == "water-mip-compatible":
        return [{"name": "mattmc-water-mip-compatible", "variant": "a", "sprites": (),
                 "water_mip_compatible": True}]
    if scenario in ("post-spider-multiline-import", "post-spider-multiline-version"):
        return [{"name": "mattmc-" + scenario, "variant": "a", "sprites": (),
                 "post_spider_namespaced": True, "multiline_import": True,
                 "multiline_version": scenario == "post-spider-multiline-version"}]
    if scenario == "post-spider-namespaced":
        return [{"name": "mattmc-post-spider-namespaced", "variant": "a", "sprites": (),
                 "post_spider_namespaced": True}]
    if scenario == "post-spider-dim":
        return [{"name": "mattmc-post-spider-dim", "variant": "a", "sprites": (),
                 "post_spider_red": 0.5}]
    if scenario == "post-creeper-eight":
        return [{"name": "mattmc-post-creeper-eight", "variant": "a", "sprites": (),
                 "post_creeper_resolution": 8.0}]
    if scenario == "post-invert-quarter":
        return [{"name": "mattmc-post-invert-quarter", "variant": "a", "sprites": (),
                 "post_invert_amount": 0.25}]
    if scenario == "sky-moon-sampler-replacement":
        # Identical texels: removing the higher-priority pack changes only sampling.
        return [{"name": f"mattmc-sky-moon-sampler-replacement-{name}", "variant": "a",
                 "sprites": (), "sky_moon_minification": True,
                 "sky_sampler_metadata": {"blur": blur, "clamp": True}}
                for name, blur in (("a", True), ("b", False))]
    if scenario == "sky-moon-filtered":
        return [{"name": "mattmc-sky-moon-filtered", "variant": "a", "sprites": (),
                 "sky_moon_minification": True, "sky_sampler_metadata": {"blur": True, "clamp": True}}]
    if scenario == "sky-moon-replacement":
        return [{"name": f"mattmc-sky-moon-replacement-{variant}", "variant": variant,
                 "sprites": (), "sky_moon_minification": True} for variant in ("a", "b")]
    if scenario == "sky-moon-minification":
        return [{"name": "mattmc-sky-moon-minification", "variant": "a",
                 "sprites": (), "sky_moon_minification": True}]
    if scenario == "sky-sun-minification":
        # Reuse shared isolated pack transport, but replace only the sky asset.
        return [{"name": "mattmc-sky-sun-minification", "variant": "a",
                 "sprites": (), "sky_sun_minification": True}]
    base = {
        "sprites": GUI_RESOURCE_PACK_SPRITES,
        "world_border_texture": WORLD_BORDER_RESOURCE_PACK_TEXTURE,
        "world_crack_textures": WORLD_CRACK_RESOURCE_PACK_TEXTURES,
        "world_material_textures": WORLD_MATERIAL_BLOCK_MARKER_TEXTURES + WORLD_MATERIAL_TERRAIN_PARTICLE_TEXTURES,
        "water_textures": WATER_TERRAIN_TEXTURES,
        "water_animation": "default",
        "malformed": (),
        "wrong_size": (),
    }
    if scenario == "pack-a":
        return [{**base, "name": "mattmc-rust-gui-pack-a", "variant": "a"}]
    if scenario == "pack-b":
        return [{**base, "name": "mattmc-rust-gui-pack-b", "variant": "b"}]
    if scenario == "priority-a-b":
        return [
            {**base, "name": "mattmc-rust-gui-pack-a", "variant": "a"},
            {**base, "name": "mattmc-rust-gui-pack-b", "variant": "b"},
        ]
    if scenario == "priority-b-a":
        return [
            {**base, "name": "mattmc-rust-gui-pack-b", "variant": "b"},
            {**base, "name": "mattmc-rust-gui-pack-a", "variant": "a"},
        ]
    if scenario == "missing":
        return [
            {
                **base,
                "name": "mattmc-rust-gui-pack-missing",
                "variant": "a",
                "sprites": GUI_RESOURCE_PACK_SPRITES[:6],
                "world_border_texture": "",
                "world_crack_textures": (),
                "world_material_textures": (),
                "water_textures": (),
            }
        ]
    if scenario == "malformed":
        return [
            {
                **base,
                "name": "mattmc-rust-gui-pack-malformed",
                "variant": "a",
                "malformed": (
                    "assets/minecraft/textures/gui/sprites/hud/crosshair.png",
                    WORLD_BORDER_RESOURCE_PACK_TEXTURE,
                    "assets/minecraft/textures/block/destroy_stage_4.png",
                    "assets/minecraft/textures/item/barrier.png",
                    "assets/minecraft/textures/block/stone.png",
                    "assets/minecraft/textures/block/sand.png",
                    "assets/minecraft/textures/block/water_still.png",
                ),
            }
        ]
    if scenario == "unsupported":
        return [
            {
                **base,
                "name": "mattmc-rust-gui-pack-unsupported",
                "variant": "a",
                "wrong_size": (
                    "assets/minecraft/textures/gui/sprites/hud/armor_full.png",
                    "assets/minecraft/textures/block/destroy_stage_4.png",
                    "assets/minecraft/textures/item/light_15.png",
                    "assets/minecraft/textures/block/oak_leaves.png",
                    "assets/minecraft/textures/block/gravel.png",
                    "assets/minecraft/textures/block/water_flow.png",
                ),
            }
        ]
    if scenario == "terrain-identity":
        return [
            {
                **base,
                "name": "mattmc-rust-terrain-identity",
                "variant": "a",
                "terrain_identity_textures": WORLD_STATIC_TERRAIN_IDENTITY_TEXTURES,
            }
        ]
    if scenario in WATER_ANIMATION_SCENARIOS:
        return [
            {
                **base,
                "name": f"mattmc-rust-terrain-{scenario}",
                "variant": "a",
                "water_animation": scenario.removeprefix("water-"),
            }
        ]
    raise SystemExit(
        "--gui-resource-pack-scenario must be one of: vanilla, pack-a, pack-b, "
        "priority-a-b, priority-b-a, missing, malformed, unsupported, terrain-identity, sky-sun-minification, sky-moon-minification, "
        + ", ".join(sorted(WATER_ANIMATION_SCENARIOS))
    )


def lava_minification_image(source):
    """Eightfold ordinary pack image with a zero-mean one-texel mip signal.

    Every 2x2 group preserves its original mean. Mip-off sampling retains the
    signal; minification can remove it. No renderer, clock, or UV override.
    """
    from PIL import Image
    source = source.convert("RGBA")
    scaled = source.resize((source.width * 8, source.height * 8), Image.Resampling.NEAREST)
    values = []
    for index, pixel in enumerate(scaled.getdata()):
        sign = 1 if ((index % scaled.width) + (index // scaled.width)) % 2 else -1
        values.append(tuple(value + sign * min(limit, value, 255 - value)
                            for value, limit in zip(pixel[:3], (24, 24, 8))) + (pixel[3],))
    scaled.putdata(values)
    return scaled


def shield_alpha_png(*, zero: bool = False, opaque_handle: bool = False) -> bytes:
    """Static plate bands at zero alpha and around the .1 cutout boundary.

    Nonzero RGB at zero alpha intentionally tests opaque semantics. No model,
    shader, pipeline, timing or UV override is included in this resource pack.
    """
    rows = bytearray()
    for y in range(64):
        alpha = 0 if zero else (0, 1, 25, 26, 128, 255)[(y // 4) % 6]
        rows.append(0)
        for x in range(64):
            # ShieldModel's handle occupies texOffs(26,0), extent 16x12.
            # Its opaque green surface must not leak through a zero-alpha
            # plate whose opaque material still writes color and depth.
            rows.extend((40, 240, 80, 255) if opaque_handle and 26 <= x < 42 and y < 12
                        else (224, 160, 80, alpha))
    return (b"\x89PNG\r\n\x1a\n"
            + png_chunk(b"IHDR", struct.pack(">IIBBBBB", 64, 64, 8, 6, 0, 0, 0))
            + png_chunk(b"IDAT", zlib.compress(bytes(rows))) + png_chunk(b"IEND", b""))


SHIELD_ANIMATION_COLORS = ((224,64,32,255), (32,208,64,255), (48,80,224,255))
SHIELD_ANIMATION_FRAMES = ((2,3), (0,5), (2,2), (1,7))


def shield_animation_png() -> bytes:
    rows = b"".join((b"\0" + bytes(color)*64)*64 for color in SHIELD_ANIMATION_COLORS)
    return (b"\x89PNG\r\n\x1a\n"
            + png_chunk(b"IHDR",struct.pack(">IIBBBBB",64,192,8,6,0,0,0))
            + png_chunk(b"IDAT",zlib.compress(rows)) + png_chunk(b"IEND",b""))


def write_gui_resource_pack(pack_dir: Path, spec: dict[str, object]) -> None:
    if pack_dir.exists():
        shutil.rmtree(pack_dir)
    pack_dir.mkdir(parents=True)
    (pack_dir / "pack.mcmeta").write_text(
        json.dumps(
            {
                "pack": {
                    "pack_format": 69,
                    "min_format": [69, 0],
                    "max_format": [69, 0],
                    "description": f"MattMC {spec['name']}",
                }
            }
        ),
        encoding="utf-8",
    )
    variant = str(spec["variant"])
    if spec.get("block_item_expanded"):
        # Replace only the middle GUI fixture item; world block models and
        # the stone/wool/log lighting controls remain vanilla.
        model = "item/mattmc_expanded_oak_slab"
        target = pack_dir / "assets/minecraft/models" / (model + ".json")
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(json.dumps({"gui_light":"side", "textures":{"all":"minecraft:block/oak_planks"},
            "display":{"gui":{"rotation":[30,225,0],"translation":[3,1,0],"scale":[0.9,0.7,0.8]}},
            "elements":[{"from":[-8,0,0],"to":[24,16,16],
                "faces":{face:{"texture":"#all","uv":[0,0,16,16]}
                         for face in ("down","up","north","south","west","east")}}]}),encoding="utf-8")
        target = pack_dir / "assets/minecraft/items/oak_slab.json"
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(json.dumps({"model":{"type":"minecraft:model","model":"minecraft:"+model},
                                     "oversized_in_gui":bool(spec.get("block_item_oversized"))}),encoding="utf-8")
    if spec.get("lava_mip_minification"):
        from PIL import Image
        resources = Path(__file__).resolve().parents[2] / "src/main/resources/assets/minecraft/textures/block"
        for name in ("lava_still", "lava_flow"):
            target = pack_dir / f"assets/minecraft/textures/block/{name}.png"
            target.parent.mkdir(parents=True, exist_ok=True)
            with Image.open(resources / (name + ".png")) as source:
                lava_minification_image(source).save(target)
            # Keep vanilla frame order and frame durations exactly unchanged.
            target.with_name(target.name + ".mcmeta").write_bytes(
                (resources / (name + ".png.mcmeta")).read_bytes())
    if spec.get("water_face_isolation"):
        # Ordinary resource-pack inputs only. Static, distinct sprite colors
        # isolate geometry/selection from animation phase. Invisible door
        # texels expose the water face without changing its block state/model.
        colors = {"water_still": (255, 255, 255, 128),
                  "water_flow": (255, 0, 0, 128),
                  "water_overlay": (0, 255, 0, 128),
                  "oak_door_top": (0, 0, 0, 0),
                  "oak_door_bottom": (0, 0, 0, 0)}
        if spec.get("water_bottom_isolation"):
            colors["blue_stained_glass"] = (0, 0, 0, 0)
        if spec.get("water_overlay_isolation"):
            # Keep the real ice block, collision and fluid-overlay eligibility.
            # Only remove its visual obstruction through normal pack loading.
            colors["ice"] = (0, 0, 0, 0)
        if spec.get("water_overlay_hidden"):
            colors["water_overlay"] = (0, 0, 0, 0)
        for name, color in colors.items():
            target = pack_dir / f"assets/minecraft/textures/block/{name}.png"
            target.parent.mkdir(parents=True, exist_ok=True)
            rows = (b"\0" + bytes(color) * 16) * 16
            target.write_bytes(b"\x89PNG\r\n\x1a\n"
                + png_chunk(b"IHDR", struct.pack(">IIBBBBB", 16, 16, 8, 6, 0, 0, 0))
                + png_chunk(b"IDAT", zlib.compress(rows)) + png_chunk(b"IEND", b""))
            # Override lower-priority animation metadata as well as pixels.
            target.with_name(target.name + ".mcmeta").write_text("{}", encoding="utf-8")
    if spec.get("water_mip_compatible"):
        # Isolated vanilla-water fixture: these eight unused item sprites are
        # the only non-16-aligned images in the bundled block/item directories.
        # Do not alter terrain/water pixels or the renderer's mip fallback rule.
        paths = ("item/tacz/attachments/sight_exp3", "item/tacz/attachments/scope_lpvo_1_6",
                 "item/tacz/guns/stg44", "item/tacz/guns/m1897", "item/tacz/guns/g43",
                 "item/tacz/guns/m1", "item/tacz/guns/mp40", "item/tacz/guns/m1a1")
        target = pack_dir / "assets/minecraft/atlases/blocks.json"
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(json.dumps({"sources": [
            {"type": "minecraft:filter", "pattern": {"namespace": "^minecraft$", "path": "^" + path + "$"}}
            for path in paths]}), encoding="utf-8")
    if spec.get("post_spider_namespaced"):
        resources = Path(__file__).resolve().parents[2] / "src/main/resources/assets/minecraft"
        definition = json.loads((resources / "post_effect/spider.json").read_text())
        definition["passes"][-1]["fragment_shader"] = "mattmc_audit:post/spider_tint"
        shader = (resources / "shaders/post/blit.fsh").read_text()
        expression = "texture(InSampler, texCoord) * ColorModulate"
        if shader.count(expression) != 1:
            raise ValueError("namespaced fixture requires the known vanilla blit expression")
        import_directive = "#moj_import <mattmc_tint:mattmc_namespace_tint.glsl>"
        if spec.get("multiline_import"):
            import_directive = "#/* first token gap\ncontinued */moj_import/* second token gap\ncontinued */<mattmc_tint:mattmc_namespace_tint.glsl>"
        shader = shader.replace("#version 330", "#version 330\n" + import_directive, 1)
        if spec.get("multiline_version"):
            shader = shader.replace("#version 330", "#/* version token gap\ncontinued */version/* version number gap\ncontinued */330", 1)
        shader = shader.replace(expression, "fixtureTint(" + expression + ")", 1)
        entries = {
            "assets/minecraft/post_effect/spider.json": json.dumps(definition),
            "assets/mattmc_audit/shaders/post/spider_tint.fsh": shader,
            "assets/mattmc_tint/shaders/include/mattmc_namespace_tint.glsl":
                "vec4 fixtureTint(vec4 value) { return value * vec4(0.5, 1.0, 1.0, 1.0); }\n",
            # Same path, wrong namespace and visibly different value. A
            # resolver that erases namespaces must not pass this fixture.
            "assets/minecraft/shaders/include/mattmc_namespace_tint.glsl":
                "vec4 fixtureTint(vec4 value) { return value * vec4(0.125, 1.0, 1.0, 1.0); }\n",
        }
        for relative, contents in entries.items():
            target = pack_dir / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(contents, encoding="utf-8")
    if "post_spider_red" in spec:
        definition = json.loads((Path(__file__).resolve().parents[2] /
            "src/main/resources/assets/minecraft/post_effect/spider.json").read_text())
        definition["passes"][-1]["uniforms"]["BlitConfig"][0]["value"][0] = spec["post_spider_red"]
        target = pack_dir / "assets/minecraft/post_effect/spider.json"
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(json.dumps(definition), encoding="utf-8")
    if "post_creeper_resolution" in spec:
        definition = json.loads((Path(__file__).resolve().parents[2] /
            "src/main/resources/assets/minecraft/post_effect/creeper.json").read_text())
        definition["passes"][1]["uniforms"]["BitsConfig"][0]["value"] = spec["post_creeper_resolution"]
        target = pack_dir / "assets/minecraft/post_effect/creeper.json"
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(json.dumps(definition), encoding="utf-8")
    if "post_invert_amount" in spec:
        definition = json.loads((Path(__file__).resolve().parents[2] /
            "src/main/resources/assets/minecraft/post_effect/invert.json").read_text())
        definition["passes"][0]["uniforms"]["InvertConfig"][0]["value"] = spec["post_invert_amount"]
        target = pack_dir / "assets/minecraft/post_effect/invert.json"
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(json.dumps(definition), encoding="utf-8")
    if spec.get("sky_moon_minification"):
        target = pack_dir / "assets/minecraft/textures/environment/moon_phases.png"
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(sky_moon_minification_png(variant))
        if spec.get("sky_sampler_metadata") is not None:
            target.with_suffix(".png.mcmeta").write_text(
                json.dumps({"texture": spec["sky_sampler_metadata"]}), encoding="utf-8")
    if spec.get("sky_sun_minification"):
        target = pack_dir / "assets/minecraft/textures/environment/sun.png"
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(sky_sun_minification_png())
    malformed = set(spec.get("malformed", ()))
    if spec.get("item_foil_blend"):
        # Constant authored glint isolates the blend contract. Animated and
        # repeating foil coordinates require separate, nonconstant fixtures.
        target=pack_dir/"assets/minecraft/textures/misc/enchanted_glint_item.png"
        target.parent.mkdir(parents=True,exist_ok=True)
        rows=(bytes([0])+bytes((32,64,96,255))*16)*16
        if spec.get("item_foil_pattern"):
            from gui_foil_reference import pattern_pixel
            rows=b"".join(bytes([0])+bytes(c for x in range(16) for c in pattern_pixel(x,y)) for y in range(16))
            sampler = spec.get("item_foil_sampler", {"blur": True, "clamp": False})
            if (not isinstance(sampler, dict) or set(sampler) != {"blur", "clamp"}
                    or any(type(value) is not bool for value in sampler.values())):
                raise ValueError("foil fixture requires explicit boolean blur/clamp")
            target.with_suffix(".png.mcmeta").write_text(json.dumps({"texture":sampler}))
        target.write_bytes(b"\x89PNG\r\n\x1a\n"+png_chunk(b"IHDR",struct.pack(">IIBBBBB",16,16,8,6,0,0,0))
                          +png_chunk(b"IDAT",zlib.compress(rows))+png_chunk(b"IEND",b""))
    wrong_size = set(spec.get("wrong_size", ()))
    if "item_rects" in spec:
        for index, ((resource_path, _, _), (left,top,right,bottom)) in enumerate(zip(spec["sprites"], spec["item_rects"], strict=True)):
            name = Path(resource_path).stem
            model_name = "item/mattmc_geometry/" + name
            target = pack_dir / "assets/minecraft/models" / (model_name + ".json")
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(json.dumps({"gui_light": "front", "ambientocclusion": False,
                "textures": {"layer0": "minecraft:item/feather" if spec.get("item_animation") and index else "minecraft:item/apple",
                             "particle": "minecraft:item/feather" if spec.get("item_animation") and index else "minecraft:item/apple"},
                "elements": [{"from": [left,16-bottom,7.5], "to": [right,16-top,8.5],
                    "faces": {"south": {"uv": spec.get("item_uvs", ((0,0,16,16),)*9)[index],
                                          "rotation": spec.get("item_rotations",(0,)*9)[index],
                                          "texture": "#layer0"}}}]}), encoding="utf-8")
            if spec.get("item_generated"):
                # Ordinary resource-pack generation: vanilla supplies front,
                # back and extrusion edges; no fixture-authored face omission.
                target.write_text(json.dumps({"parent":"minecraft:item/generated",
                    "textures":{"layer0":"minecraft:item/apple","particle":"minecraft:item/apple"}}),encoding="utf-8")
            target = pack_dir / "assets/minecraft/items" / (name + ".json")
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(json.dumps({"model": {"type": "minecraft:model",
                "model": "minecraft:" + model_name}}), encoding="utf-8")
    if "item_tints" in spec:
        # Ordinary resource-pack item definitions: all nine items deliberately
        # share one model/sprite, with distinct semantic tint values. This
        # catches raster-cache aliases without touching either renderer.
        for (resource_path, _, _), tint in zip(spec["sprites"], spec["item_tints"], strict=True):
            target = pack_dir / "assets/minecraft/items" / (Path(resource_path).stem + ".json")
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(json.dumps({"model": {"type": "minecraft:model",
                "model": "minecraft:item/apple",
                "tints": [{"type": "minecraft:constant", "value": tint}]}}), encoding="utf-8")
    if spec.get("item_alpha_backdrop"):
        # Separate diagnostic fixture: an authored opaque HUD texture makes
        # the destination of alpha blending independent of terrain pixels.
        # It is rendered normally by both backends, never an image mask.
        target = pack_dir / "assets/minecraft/textures/gui/sprites/hud/hotbar.png"
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(asymmetric_png(182, 22, (31,47,63,255), "b"))
    for resource_path, width, height in spec["sprites"]:  # type: ignore[index]
        target = pack_dir / resource_path
        target.parent.mkdir(parents=True, exist_ok=True)
        if resource_path in malformed:
            target.write_bytes(b"not a png")
            continue
        actual_width = width + 1 if resource_path in wrong_size else width
        if spec.get("shield_animation"):
            target.write_bytes(shield_animation_png())
            target.with_name(target.name + ".mcmeta").write_text(json.dumps({"animation": {
                "width":64,"height":64,"interpolate":bool(spec.get("shield_interpolation")),
                "frames":[{"index":index,"time":duration} for index,duration in SHIELD_ANIMATION_FRAMES]
            }}),encoding="utf-8")
            continue
        if spec.get("shield_alpha"):
            target.write_bytes(shield_alpha_png(zero=bool(spec.get("shield_alpha_zero")),
                opaque_handle=bool(spec.get("shield_alpha_opaque_handle"))))
            target.with_name(target.name + ".mcmeta").write_text("{}", encoding="utf-8")
            continue
        if spec.get("solid_occluder"):
            # An ordinary resource-pack texture isolates depth from texel-boundary
            # interpolation. No shader override, image mask or renderer change.
            rows = (b"\x00" + bytes((128,128,128,255))*16)*16
            target.write_bytes(b"\x89PNG\r\n\x1a\n"
                + png_chunk(b"IHDR", struct.pack(">IIBBBBB",16,16,8,6,0,0,0))
                + png_chunk(b"IDAT",zlib.compress(rows)) + png_chunk(b"IEND",b""))
            continue
        if spec.get("experience_orb_sheet"):
            target.write_bytes(experience_orb_sheet_png(variant))
            continue
        target.write_bytes(asymmetric_png(actual_width, height, GUI_PACK_COLORS[variant], variant,
                                         item_alpha_steps=bool(spec.get("item_alpha_steps"))))
    world_border_texture = str(spec.get("world_border_texture", ""))
    if "item_uvs" in spec:
        rows = bytearray()
        animation = bool(spec.get("item_animation"))
        width = int(spec.get("item_foil_sprite_size", 32))
        if width not in (32,512) or width != 32 and (animation or spec.get("item_cutout")):
            raise ValueError("wide foil sprite is a bounded static opaque fixture")
        height = 64 if animation else width
        for y in range(height):
            rows.append(0)
            for x in range(width):
                quadrant = ((y%width)//(width//2))*2+x//(width//2)
                if y >= width: quadrant = 3-quadrant
                alpha = 255
                if spec.get("item_cutout") and (x < 4 or x >= 28 or y < 4 or y >= 28
                                               or (12 <= x < 20 and 12 <= y < 20)):
                    alpha = 0
                rows.extend((*FLAT_ITEM_UV_COLORS[quadrant],alpha))
        target = pack_dir / ("assets/minecraft/textures/item/feather.png" if animation
                             else "assets/minecraft/textures/item/apple.png")
        target.write_bytes(b"\x89PNG\r\n\x1a\n"
            + png_chunk(b"IHDR",struct.pack(">IIBBBBB",width,height,8,6,0,0,0))
            + png_chunk(b"IDAT",zlib.compress(bytes(rows))) + png_chunk(b"IEND",b""))
        if animation:
            target.with_suffix(".png.mcmeta").write_text(json.dumps({"animation":{
                "width":32,"height":32,"frametime":4,"interpolate":bool(spec.get("item_interpolation")),"frames":[0,1]}}),encoding="utf-8")
            # The held first slot stays static. Only GUI-only slots use the
            # animated feather, so hand visibility cannot conceal a missing
            # GUI sprite-use event.
            (pack_dir / "assets/minecraft/textures/item/apple.png").write_bytes(b"\x89PNG\r\n\x1a\n"
                + png_chunk(b"IHDR",struct.pack(">IIBBBBB",32,32,8,6,0,0,0))
                + png_chunk(b"IDAT",zlib.compress(bytes(rows[:32*(1+32*4)]))) + png_chunk(b"IEND",b""))
    if spec.get("item_layers"):
        for label,color in (("red",(255,0,0,128)),("green",(0,255,0,128))):
            target=pack_dir/f"assets/minecraft/textures/item/mattmc_layer_{label}.png"
            target.parent.mkdir(parents=True,exist_ok=True)
            if spec.get("item_layer_alpha") and label == "green":
                # Ordinary resource pixels: holes, just below/above the 0.1
                # item alpha cutoff, and a translucent control.
                row=b"".join(bytes((*color[:3],(0,25,26,128)[x//8])) for x in range(32))
                rows=(bytes([0])+row)*32
            else:
                rows=(bytes([0])+bytes(color)*32)*32
            target.write_bytes(b"\x89PNG\r\n\x1a\n"+png_chunk(b"IHDR",struct.pack(">IIBBBBB",32,32,8,6,0,0,0))
                              +png_chunk(b"IDAT",zlib.compress(rows))+png_chunk(b"IEND",b""))
        for index,(resource_path,_,_) in enumerate(spec["sprites"]):
            if index == 0: continue  # opaque, static held-item control
            name=Path(resource_path).stem
            colors=("red","green") if index%2 or spec.get("item_layer_alpha") else ("green","red")
            tints=[{"type":"minecraft:constant","value":value} for value in (0xffffff,0x808080 if index>=5 else 0xffffff)]
            elements=[]
            children=[]
            for layer,color in enumerate(colors):
                if spec.get("item_remove_last_layer") and index == 8 and layer == 1: continue
                left,top,right,bottom=(4,4,12,12) if layer==1 and index>=3 and not spec.get("item_layer_alpha") else (0,0,16,16)
                if spec.get("item_rotated"): left,top,right,bottom=4,4,12,12
                if spec.get("item_transform_asymmetric"):
                    left,top,right,bottom=(0,0,16,8) if layer==0 else (4,4,12,12)
                element={"from":[left,16-bottom,7.5],"to":[right,16-top,8.5],
                         "faces":{"south":{"uv":[0,0,16,16],"texture":"#layer"+str(layer),"tintindex":layer}}}
                if spec.get("item_rotated"):
                    element["rotation"]={"origin":[8,8,8],"axis":"z","angle":45,"rescale":False}
                elements.append(element)
                child_name=f"item/mattmc_layers/{name}_{layer}"
                model={"gui_light":"front","ambientocclusion":False,
                       "textures":{"layer"+str(layer):"minecraft:item/mattmc_layer_"+color,
                                   "particle":"minecraft:item/apple"},"elements":[element]}
                if spec.get("item_transformed"):
                    model["display"]={"gui":{"rotation":[0,0,90],"translation":[2,0,0],"scale":[0.5,0.5,1]}}
                if spec.get("item_child_transforms") and not spec.get("item_child_transform_original") and layer == 1:
                    model["display"]={"gui":{"rotation":[0,0,0],"translation":[-2,0,0],"scale":[0.5,0.5,1]}}
                target=pack_dir/"assets/minecraft/models"/(child_name+".json")
                target.parent.mkdir(parents=True,exist_ok=True)
                target.write_text(json.dumps(model),encoding="utf-8")
                children.append({"type":"minecraft:model","model":"minecraft:"+child_name,"tints":tints})
            if index%2 and not spec.get("item_child_transforms"):
                model["elements"]=elements
                model["textures"].update({"layer0":"minecraft:item/mattmc_layer_"+colors[0]})
                target.write_text(json.dumps(model),encoding="utf-8")
                definition=children[-1]
            else:
                definition={"type":"minecraft:composite","models":children}
            (pack_dir/"assets/minecraft/items"/(name+".json")).write_text(json.dumps({"model":definition}),encoding="utf-8")
    if world_border_texture:
        target = pack_dir / world_border_texture
        target.parent.mkdir(parents=True, exist_ok=True)
        if world_border_texture in malformed:
            target.write_bytes(b"not a png")
        else:
            target.write_bytes(asymmetric_png(16, 16, GUI_PACK_COLORS[variant], variant))
    for crack_texture in spec.get("world_crack_textures", ()):  # type: ignore[assignment]
        resource_path = str(crack_texture)
        target = pack_dir / resource_path
        target.parent.mkdir(parents=True, exist_ok=True)
        if resource_path in malformed:
            target.write_bytes(b"not a png")
            continue
        actual_width = 17 if resource_path in wrong_size else 16
        target.write_bytes(asymmetric_png(actual_width, 16, GUI_PACK_COLORS[variant], variant))
    for material_texture in spec.get("world_material_textures", ()):  # type: ignore[assignment]
        resource_path = str(material_texture)
        target = pack_dir / resource_path
        target.parent.mkdir(parents=True, exist_ok=True)
        if resource_path in malformed:
            target.write_bytes(b"not a png")
            continue
        actual_width = 17 if resource_path in wrong_size else 16
        target.write_bytes(asymmetric_png(actual_width, 16, GUI_PACK_COLORS[variant], variant))
    for resource_path, color in dict(spec.get("terrain_identity_textures", {})).items():
        target = pack_dir / str(resource_path)
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(asymmetric_png(16, 16, tuple(color), "identity"))
    if spec.get("particle_atlas_animation"):
        target = pack_dir / "assets/minecraft/textures/particle/flame.png"
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(particle_atlas_animation_png())
        target.with_suffix(".png.mcmeta").write_text(json.dumps({"animation": {
            "width": 16, "height": 16, "frametime": 8, "frames": [0, 1], "interpolate": True
        }}), encoding="utf-8")
    if spec.get("particle_atlas_static"):
        target = pack_dir / "assets/minecraft/textures/particle/flame.png"
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(particle_atlas_animation_png(static_variant=str(spec["variant"])))
    water_animation = str(spec.get("water_animation", "default"))
    for water_texture in spec.get("water_textures", ()):  # type: ignore[assignment]
        resource_path = str(water_texture)
        target = pack_dir / resource_path
        target.parent.mkdir(parents=True, exist_ok=True)
        if resource_path in malformed:
            target.write_bytes(b"not a png")
            continue
        actual_width = 17 if resource_path in wrong_size else 16
        frames, frame_meta, interpolate = water_animation_fixture(water_animation, resource_path, variant)
        target.write_bytes(animated_water_png(actual_width, 16, frames, variant, resource_path))
        mcmeta = {"animation": {"frames": frame_meta, "interpolate": interpolate}}
        (target.parent / f"{target.name}.mcmeta").write_text(json.dumps(mcmeta, separators=(",", ":")), encoding="utf-8")


def particle_atlas_animation_png(static_variant: str | None = None) -> bytes:
    """Opaque two-frame pack input with asymmetric rows/columns; ordinary mcmeta drives it."""
    if static_variant not in (None, "a", "b"):
        raise ValueError("unsupported static particle variant")
    height = 32 if static_variant is None else 16
    rows = bytearray()
    for y in range(height):
        rows.append(0)
        for x in range(16):
            base = (190, 30, 20) if y < 16 and static_variant != "b" else (20, 150, 190)
            rows.extend((base[0] + x, base[1] + y % 16, base[2] + (x + y) % 8, 255))
    return (b"\x89PNG\r\n\x1a\n" + png_chunk(b"IHDR", struct.pack(">IIBBBBB", 16, height, 8, 6, 0, 0, 0))
            + png_chunk(b"IDAT", zlib.compress(bytes(rows))) + png_chunk(b"IEND", b""))


def sky_moon_minification_png(variant: str = "a") -> bytes:
    """Eight distinct phase cells: base-level detail and non-symmetric UV witnesses."""
    width, height = 1024, 512
    if variant not in ("a", "b"):
        raise ValueError("unsupported moon fixture variant")
    rows = bytearray()
    for y in range(height):
        rows.append(0)
        for x in range(width):
            phase = (y // 256) * 4 + x // 256
            u, v = x % 256, y % 256
            if 48 <= u < 80 and 48 <= v < 96:
                pixel = (255, 128, 96, 255)
            elif 80 <= u < 176 and 80 <= v < 176:
                pixel = (128 + phase * 12, 255 - phase * 9, 200 + phase * 5, 255)
                if variant == "b":
                    pixel = (pixel[2], pixel[0], pixel[1], pixel[3])
            elif 48 <= u < 208 and 48 <= v < 208:
                value = 255 if (u + v) % 2 == 0 else 0
                pixel = (value, value, value, 255)
            else:
                pixel = (0, 0, 0, 0)
            rows.extend(pixel)
    return (b"\x89PNG\r\n\x1a\n"
            + png_chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
            + png_chunk(b"IDAT", zlib.compress(bytes(rows)))
            + png_chunk(b"IEND", b""))


def sky_sun_minification_png() -> bytes:
    """A central sun witness surrounded by texel-scale minification evidence."""
    size = 1024
    rows = bytearray()
    for y in range(size):
        rows.append(0)
        for x in range(size):
            if 448 <= x < 576 and 448 <= y < 576:
                pixel = (255, 255, 255, 255)
            elif 384 <= x < 640 and 384 <= y < 640:
                value = 255 if (x + y) % 2 == 0 else 0
                pixel = (value, value, value, 255)
            else:
                pixel = (0, 0, 0, 0)
            rows.extend(pixel)
    return (b"\x89PNG\r\n\x1a\n"
            + png_chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
            + png_chunk(b"IDAT", zlib.compress(bytes(rows)))
            + png_chunk(b"IEND", b""))


def experience_orb_sheet_png(variant: str) -> bytes:
    """Ordinary resource override: distinct cells, orientation and cutout edges."""
    if variant not in ("a", "b"):
        raise ValueError("unknown orb sheet variant")
    rows = bytearray()
    for y in range(64):
        rows.append(0)
        for x in range(64):
            cell = (y // 16) * 4 + x // 16
            u, v = x % 16, y % 16
            color = ((43 + cell * 13) % 256, (211 - cell * 7) % 256, (71 + cell * 11) % 256)
            if variant == "b":
                color = tuple(255 - channel for channel in color)
            alpha = 0 if u < 2 or v < 2 or u > 13 or v > 13 else 255
            if u < 5 and v < 7:
                color = (255, 255, 255)
            rows.extend((*color, alpha))
    return (b"\x89PNG\r\n\x1a\n"
            + png_chunk(b"IHDR", struct.pack(">IIBBBBB", 64, 64, 8, 6, 0, 0, 0))
            + png_chunk(b"IDAT", zlib.compress(bytes(rows))) + png_chunk(b"IEND", b""))


def asymmetric_png(width: int, height: int, base: tuple[int, int, int, int], variant: str,
                   item_alpha_steps: bool = False) -> bytes:
    if item_alpha_steps and (width, height) != (32, 32):
        raise ValueError("item alpha steps require the fixed 32px fixture")
    marker = (255, 255, 255, 255) if variant == "a" else (0, 0, 0, 255)
    edge = (0, 70, 255, 255) if variant == "a" else (255, 212, 0, 255)
    rows = bytearray()
    for y in range(height):
        rows.append(0)
        for x in range(width):
            pixel = base
            if y == 0:
                pixel = marker
            elif x == 0:
                pixel = edge
            elif x == width - 1 or y == height - 1:
                pixel = (base[0] // 2, base[1] // 2, base[2] // 2, base[3])
            if item_alpha_steps and 4 <= y < 12:
                for left, alpha in ((2, 0), (6, 25), (10, 26), (18, 128), (24, 255)):
                    if left <= x < left + 4:
                        pixel = (255, 255, 255, alpha)
            rows.extend(pixel)
    raw = zlib.compress(bytes(rows))
    return (
        b"\x89PNG\r\n\x1a\n"
        + png_chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
        + png_chunk(b"IDAT", raw)
        + png_chunk(b"IEND", b"")
    )


def water_animation_fixture(
    scenario: str,
    resource_path: str,
    variant: str,
) -> tuple[list[int], list[object], bool]:
    if "water_overlay" in resource_path:
        return [0, 1], [{"index": 0, "time": 2}, {"index": 1, "time": 2}], False
    if scenario == "non-sequential":
        return [0, 1, 2, 3], [{"index": 0, "time": 1}, {"index": 2, "time": 1}, {"index": 1, "time": 1}, {"index": 3, "time": 1}], False
    if scenario == "unequal-duration":
        return [0, 1, 2, 3], [{"index": 0, "time": 1}, {"index": 1, "time": 3}, {"index": 2, "time": 2}, {"index": 3, "time": 4}], False
    if scenario == "interpolation-on":
        return [0, 1, 2, 3], [{"index": 0, "time": 2}, {"index": 1, "time": 2}, {"index": 2, "time": 2}, {"index": 3, "time": 2}], True
    if scenario == "pixel-replacement":
        return [0, 1, 2, 3], [{"index": 3, "time": 1}, {"index": 1, "time": 1}, {"index": 2, "time": 1}, {"index": 0, "time": 1}], True
    # default and interpolation-off
    return [0, 1, 2, 3], [{"index": 0, "time": 2}, {"index": 1, "time": 2}, {"index": 2, "time": 2}, {"index": 3, "time": 2}], False


def animated_water_png(width: int, frame_height: int, frames: list[int], variant: str, resource_path: str) -> bytes:
    palette_a = [
        (24, 80, 230, 170),
        (30, 150, 255, 170),
        (15, 210, 190, 170),
        (90, 60, 240, 170),
    ]
    palette_b = [
        (200, 44, 220, 170),
        (255, 110, 55, 170),
        (240, 210, 40, 170),
        (95, 230, 100, 170),
    ]
    palette = palette_a if variant == "a" else palette_b
    rows = bytearray()
    height = frame_height * max(1, len(frames))
    for y in range(height):
        rows.append(0)
        frame_row = min(len(frames) - 1, y // frame_height)
        frame_index = frames[frame_row] % len(palette)
        base = palette[frame_index]
        if "water_overlay" in resource_path:
            base = (base[0], base[1], base[2], 96)
        for x in range(width):
            pixel = base
            if x == 0 or y % frame_height == 0:
                pixel = (255, 255, 255, base[3])
            elif x == width - 1 or y % frame_height == frame_height - 1:
                pixel = (base[0] // 2, base[1] // 2, base[2] // 2, base[3])
            elif x == (frame_index + 3) % max(1, width):
                pixel = (255 - base[0], 255 - base[1], 255 - base[2], base[3])
            rows.extend(pixel)
    raw = zlib.compress(bytes(rows))
    return (
        b"\x89PNG\r\n\x1a\n"
        + png_chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
        + png_chunk(b"IDAT", raw)
        + png_chunk(b"IEND", b"")
    )


def png_chunk(kind: bytes, data: bytes) -> bytes:
    checksum = zlib.crc32(kind)
    checksum = zlib.crc32(data, checksum) & 0xFFFFFFFF
    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", checksum)


def directory_digest(path: Path) -> str:
    digest = hashlib.sha256()
    for item in sorted(path.rglob("*")):
        if item.is_file():
            digest.update(str(item.relative_to(path)).encode("utf-8"))
            digest.update(b"\0")
            digest.update(item.read_bytes())
            digest.update(b"\0")
    return digest.hexdigest()


def resolve_shader_pack_path(run_dir: Path, root: Path, shader_pack_name: str) -> Path | None:
    if not shader_pack_name:
        return None
    path = Path(shader_pack_name)
    if path.is_file() or path.is_dir():
        return path
    for candidate in [run_dir / "shaderpacks" / shader_pack_name, root / "shaderpacks" / shader_pack_name]:
        if candidate.is_file() or candidate.is_dir():
            return candidate
    return None


def stat_text(path: Path) -> str:
    try:
        info = path.stat()
    except OSError as exc:
        return str(exc)
    mode = stat.filemode(info.st_mode)
    return f"{mode} size={info.st_size} mtime={int(info.st_mtime)} path={path}"


def file_sha256(path: Path) -> str:
    import hashlib

    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def file_sha256_text(path: Path) -> str:
    if not path.is_file():
        return f"not a file: {path}"
    return f"{file_sha256(path)}  {path}"


def save_state_fingerprint(run_dir: Path, world: str) -> dict[str, object]:
    save_dir = run_dir / "saves" / world
    if not save_dir.is_dir():
        return {"hash": "missing", "file_count": 0, "total_bytes": 0, "truncated": False}
    digest = hashlib.sha256()
    file_count = 0
    total_bytes = 0
    truncated = False
    allowed_suffixes = {".mca"}
    for path in sorted(candidate for candidate in save_dir.rglob("*") if candidate.is_file()):
        rel = path.relative_to(save_dir).as_posix()
        if rel == "session.lock" or path.suffix not in allowed_suffixes:
            continue
        file_count += 1
        if file_count > 5000:
            truncated = True
            break
        try:
            data = path.read_bytes()
        except OSError:
            data = b""
        total_bytes += len(data)
        digest.update(rel.encode("utf-8"))
        digest.update(b"\0")
        digest.update(str(len(data)).encode("ascii"))
        digest.update(b"\0")
        digest.update(hashlib.sha256(data).hexdigest().encode("ascii"))
        digest.update(b"\n")
    return {
        "hash": digest.hexdigest(),
        "file_count": file_count,
        "total_bytes": total_bytes,
        "truncated": truncated,
    }


def copy_recent_files_since_start(source_dir: Path, dest_dir: Path, list_file: Path, start_epoch: int) -> None:
    if not source_dir.is_dir():
        list_file.write_text("", encoding="utf-8")
        return
    files = sorted(path for path in source_dir.rglob("*") if path.is_file() and path.stat().st_mtime >= start_epoch)
    list_file.write_text("\n".join(str(path) for path in files) + ("\n" if files else ""), encoding="utf-8")
    for path in files:
        rel_path = path.relative_to(source_dir)
        target = dest_dir / rel_path
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(path, target)


def append_filtered_section(label: str, file_path: Path, pattern: str, output_file: Path) -> None:
    if not file_path.is_file():
        return
    matches = filtered_lines(file_path, pattern)
    with output_file.open("a", encoding="utf-8", errors="replace") as handle:
        handle.write(f"===== {label}: {file_path} =====\n")
        if matches:
            handle.write("\n".join(matches) + "\n")
        else:
            handle.write("(no matches)\n")
        handle.write("\n")


def filtered_lines(file_path: Path, pattern: str, limit: int | None = None) -> list[str]:
    if not file_path.is_file():
        return []
    regex = re.compile(pattern, re.IGNORECASE)
    matches: list[str] = []
    with file_path.open("r", encoding="utf-8", errors="replace") as handle:
        for number, line in enumerate(handle, 1):
            if regex.search(line):
                matches.append(f"{number}:{line.rstrip()}")
                if limit and len(matches) >= limit:
                    break
    return matches


def count_list_entries(file_path: Path) -> int:
    if not file_path.is_file():
        return 0
    return sum(1 for line in file_path.read_text(encoding="utf-8", errors="replace").splitlines() if line.strip())


def read_list_paths(file_path: Path) -> list[Path]:
    if not file_path.is_file():
        return []
    return [Path(line.strip()) for line in file_path.read_text(encoding="utf-8", errors="replace").splitlines() if line.strip()]


def tail_lines(path: Path, count: int) -> list[str]:
    if not path.is_file():
        return []
    lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
    return lines[-count:]


def process_rss_kb(pid: int, platform_name: str) -> int:
    if platform_name == "windows":
        output = command_text(
            ["wmic", "process", "where", f"ProcessId={pid}", "get", "WorkingSetSize", "/value"],
            cwd=Path.cwd(),
        )
        match = re.search(r"WorkingSetSize=(\d+)", output)
        return int(match.group(1)) // 1024 if match else 0
    output = command_text(["ps", "-o", "rss=", "-p", str(pid)], cwd=Path.cwd()).strip()
    try:
        return int(output.split()[0])
    except (IndexError, ValueError):
        return 0


def read_proc_memory_summary(pid: int) -> list[str]:
    root = Path("/proc") / str(pid)
    lines: list[str] = []
    for name in ("status", "smaps_rollup"):
        path = root / name
        if not path.is_file():
            continue
        lines.extend(["", f"===== /proc/{pid}/{name} memory fields ====="])
        try:
            for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
                if name == "status":
                    if line.startswith(("Vm", "Rss", "HugetlbPages", "Threads", "voluntary_ctxt_switches", "nonvoluntary_ctxt_switches")):
                        lines.append(line)
                elif re.match(r"^(Rss|Pss|Shared_|Private_|Referenced|Anonymous|AnonHugePages|Swap|SwapPss|Locked):", line):
                    lines.append(line)
        except OSError as exc:
            lines.append(f"unavailable: {exc}")
    return lines


def process_exists(pid: int, platform_name: str) -> bool:
    if platform_name == "windows":
        output = command_text(["tasklist", "/FI", f"PID eq {pid}"], cwd=Path.cwd())
        return str(pid) in output
    try:
        os.kill(pid, 0)
        return True
    except OSError:
        return False


def safe_kill(pid: int, sig: signal.Signals) -> None:
    try:
        os.kill(pid, sig)
    except OSError:
        pass


def celestial_body_witness(image, body: str):
    """Positive evidence for the shared overhead fixture, excluding HUD and stars.

    The default moon's brightest texels are (218, 229, 255), not white.
    Require a substantial, bounded, central connected body instead of global
    frame brightness: isolated stars and a uniformly lit sky cannot qualify.
    """
    if body not in ("sun", "moon"):
        raise ValueError(f"unknown celestial body: {body!r}")
    width, height = image.size
    box = (width // 4, height // 5, width * 3 // 4, height * 4 // 5)
    crop = image.convert("RGB").crop(box)
    threshold = 224 if body == "sun" else 96
    mask = [min(pixel) >= threshold for pixel in crop.getdata()]
    pending = {index for index, present in enumerate(mask) if present}
    cw, ch = crop.size
    largest = []
    while pending:
        component = [pending.pop()]
        for index in component:
            x, y = index % cw, index // cw
            for dy in (-1, 0, 1):
                for dx in (-1, 0, 1):
                    nx, ny = x + dx, y + dy
                    neighbor = ny * cw + nx
                    if 0 <= nx < cw and 0 <= ny < ch and neighbor in pending:
                        pending.remove(neighbor)
                        component.append(neighbor)
        if len(component) > len(largest):
            largest = component
    count = len(largest)
    cx = sum(index % cw for index in largest) / max(1, count)
    cy = sum(index // cw for index in largest) / max(1, count)
    valid = (100 <= count < cw * ch / 4
             and cw / 3 <= cx <= cw * 2 / 3 and ch / 3 <= cy <= ch * 2 / 3)
    return {"passed": valid, "crop_box": box, "mask": mask,
            "body_pixels": count, "threshold": threshold}


def validate_deterministic_metadata(metadata_path: Path, screenshot_dir: Path, tolerance: float,
                                    celestial_body: str = "") -> None:
    if not metadata_path.is_file():
        raise RuntimeError(f"deterministic metadata was not written: {metadata_path}")
    data = json.loads(metadata_path.read_text(encoding="utf-8"))
    if data.get("status") != "complete":
        raise RuntimeError(
            f"deterministic capture did not complete: status={data.get('status')!r} "
            f"reason={data.get('reason')!r}"
        )

    backend = data.get("backend")
    shader_enabled = data.get("shaderEnabled")
    shader_pack = data.get("shaderPack")
    git_commit = data.get("gitCommit")
    dimension = data.get("dimension")
    yaw_delta = float(data.get("yawDelta", float("nan")))
    initial_pose = data.get("initialPose") or {}
    initial_yaw = float(initial_pose.get("yaw", float("nan")))
    initial_pitch = float(initial_pose.get("pitch", float("nan")))
    window = data.get("window") or {}
    if window.get("width") != 1280 or window.get("height") != 720:
        raise RuntimeError(f"deterministic capture window mismatch: {window}")

    captures = data.get("captures") or []
    expected_poses = data.get("poseSequence") or ["initial", "right", "left", "return"]
    actual_poses = [capture.get("poseName") for capture in captures]
    if actual_poses != expected_poses:
        raise RuntimeError(f"deterministic pose sequence mismatch: {actual_poses}")
    if not screenshot_dir.is_dir():
        raise RuntimeError(f"deterministic screenshot directory missing: {screenshot_dir}")

    expected_requested = {
        "initial": (initial_yaw, initial_pitch),
        "right": (initial_yaw + yaw_delta, initial_pitch),
        "left": (initial_yaw - yaw_delta, initial_pitch),
        "return": (initial_yaw, initial_pitch),
        "playing": (initial_yaw, initial_pitch),
        "paused": (initial_yaw, initial_pitch),
        "unpaused": (initial_yaw, initial_pitch),
        "crack-early": (initial_yaw, initial_pitch),
        "crack-middle": (initial_yaw, initial_pitch),
        "crack-late": (initial_yaw, initial_pitch),
    }
    initial_position = data.get("initialPosition") or {}
    previous_frame = -1
    for capture in captures:
        screenshot = Path(capture.get("screenshot", ""))
        if not screenshot.is_file():
            raise RuntimeError(f"deterministic screenshot missing for {capture.get('poseName')}: {screenshot}")
        try:
            from PIL import Image

            with Image.open(screenshot) as image:
                rgb = image.convert("RGB")
                celestial = celestial_body_witness(rgb, celestial_body) if celestial_body else None
                rgb.thumbnail((64, 36))
                pixels = list(rgb.getdata())
        except Exception as exc:
            raise RuntimeError(
                f"deterministic screenshot is not a readable rendered image for {capture.get('poseName')}: {screenshot}: {exc}"
            ) from exc
        visible_pixels = sum(1 for red, green, blue in pixels if max(red, green, blue) > 24)
        if celestial is not None and not celestial["passed"]:
            raise RuntimeError(f"deterministic screenshot lacks central {celestial_body} body: {screenshot}")
        if not pixels or (visible_pixels / len(pixels) < 0.05
                          and not (celestial is not None and celestial["passed"])):
            raise RuntimeError(
                f"deterministic screenshot is blank for {capture.get('poseName')}: {screenshot}"
            )
        if not screenshot.name.startswith(f"{capture.get('index'):02d}_{capture.get('poseName')}"):
            raise RuntimeError(f"deterministic screenshot name does not match pose: {screenshot}")
        for key, expected in [
            ("backend", backend),
            ("shaderEnabled", shader_enabled),
            ("shaderPack", shader_pack),
            ("gitCommit", git_commit),
            ("window", window),
            ("dimension", dimension),
        ]:
            if capture.get(key) != expected:
                raise RuntimeError(f"deterministic capture {key} mismatch: {capture}")
        position = capture.get("position") or {}
        expected_position = capture.get("requestedPosition") if isinstance(capture.get("requestedPosition"), dict) else initial_position
        for axis in ("x", "y", "z"):
            if not math.isclose(
                float(position.get(axis, float("nan"))),
                float(expected_position.get(axis, float("nan"))),
                rel_tol=0.0,
                abs_tol=0.0001,
            ):
                raise RuntimeError(
                    f"deterministic player position changed on {capture.get('poseName')} axis {axis}: "
                    f"expected={expected_position} capture={position}"
                )
        pose_name = capture.get("poseName")
        requested_yaw = float(capture.get("requestedYaw", float("nan")))
        requested_pitch = float(capture.get("requestedPitch", float("nan")))
        observed_yaw = float(capture.get("observedYaw", float("nan")))
        observed_pitch = float(capture.get("observedPitch", float("nan")))
        if pose_name in expected_requested:
            expected_yaw, expected_pitch = expected_requested[pose_name]
            if not math.isclose(requested_yaw, expected_yaw, rel_tol=0.0, abs_tol=tolerance):
                raise RuntimeError(
                    f"deterministic requested yaw mismatch for {pose_name}: "
                    f"expected={expected_yaw} actual={requested_yaw} tolerance={tolerance}"
                )
            if not math.isclose(requested_pitch, expected_pitch, rel_tol=0.0, abs_tol=tolerance):
                raise RuntimeError(
                    f"deterministic requested pitch mismatch for {pose_name}: "
                    f"expected={expected_pitch} actual={requested_pitch} tolerance={tolerance}"
                )
        if not math.isclose(observed_yaw, requested_yaw, rel_tol=0.0, abs_tol=tolerance):
            raise RuntimeError(
                f"deterministic observed yaw mismatch for {pose_name}: "
                f"requested={requested_yaw} observed={observed_yaw} tolerance={tolerance}"
            )
        if not math.isclose(observed_pitch, requested_pitch, rel_tol=0.0, abs_tol=tolerance):
            raise RuntimeError(
                f"deterministic observed pitch mismatch for {pose_name}: "
                f"requested={requested_pitch} observed={observed_pitch} tolerance={tolerance}"
            )
        frame = int(capture.get("renderedFrameIndex", -1))
        if frame <= previous_frame:
            raise RuntimeError(f"deterministic frame index did not increase at {pose_name}: previous={previous_frame} actual={frame}")
        previous_frame = frame
    if "return" in expected_poses and len(captures) > expected_poses.index("return"):
        return_capture = captures[expected_poses.index("return")]
        if captures[0]["requestedYaw"] != return_capture["requestedYaw"] or captures[0]["requestedPitch"] != return_capture["requestedPitch"]:
            raise RuntimeError("deterministic return pose does not exactly match initial requested pose")
    print(f"deterministic capture OK: {metadata_path} tolerance={tolerance}")


def int_env(name: str, default: int, *, signed: bool = False) -> int:
    value = os.environ.get(name, str(default))
    pattern = r"^-?[0-9]+$" if signed else r"^[0-9]+$"
    if not re.match(pattern, value):
        raise SystemExit(f"{name} must be an integer" if signed else f"{name} must be an integer number")
    return int(value)


def parse_args() -> CaptureConfig:
    parser = argparse.ArgumentParser(
        description=(
            "Runs Gradle runClient in a bounded session, captures diagnostics, "
            "and self-terminates so no manual kill is required."
        )
    )
    parser.add_argument("--backend", choices=("vulkan", "opengl", "rust-vulkan", "rust-opengl"), required=True)
    parser.add_argument("--shaders", choices=("on", "off"), required=True)
    parser.add_argument("--max-secs", type=int, default=int_env("MAX_SECS", 120))
    parser.add_argument("--dump-secs", type=int, default=int_env("DUMP_SECS", 45))
    parser.add_argument("--validation", choices=("off", "standard"), default=os.environ.get("VALIDATION_MODE", "off"))
    parser.add_argument(
        "--shader-input-parity",
        choices=("off", "standard", "full"),
        default=os.environ.get("SHADER_INPUT_PARITY", "off"),
    )
    parser.add_argument("--deterministic-camera-capture", action="store_true")
    parser.add_argument(
        "--deterministic-static-camera-capture",
        action="store_true",
        help=(
            "Run deterministic capture at only the initial Origin pose. This implies "
            "--deterministic-camera-capture and avoids the normal camera sweep."
        ),
    )
    parser.add_argument("--audio-validation", action="store_true")
    parser.add_argument("--capture-meshing-corpus", action="store_true")
    parser.add_argument("--meshing-corpus-output", default=os.environ.get("MATTMC_MESHING_CORPUS_OUTPUT", ""))
    parser.add_argument("--meshing-corpus-fixture", default=os.environ.get("MATTMC_MESHING_CORPUS_FIXTURE", ""))
    parser.add_argument("--meshing-corpus-warmup", type=int, default=int_env("MATTMC_MESHING_CORPUS_WARMUP", 2))
    parser.add_argument("--meshing-corpus-measure", type=int, default=int_env("MATTMC_MESHING_CORPUS_MEASURE", 3))
    parser.add_argument("--artifact-dir", default=os.environ.get("MATTMC_RUN_CAPTURE_ARTIFACT_DIR", ""))
    parser.add_argument(
        "--skip-tests",
        action="store_true",
        help=(
            "Exclude the Gradle test task from the launch command. runClient does not depend on the test suite; "
            "this option is retained for capture configurations that explicitly add verification tasks."
        ),
    )
    parser.add_argument("--client-args", default=os.environ.get("CLIENT_ARGS", ""))
    parser.add_argument(
        "--title-screen-capture",
        action="store_true",
        default=os.environ.get("MATTMC_TITLE_SCREEN_CAPTURE", "").lower() in {"1", "true", "yes"},
        help="Remain at the ordinary title screen instead of forcing quick-play; diagnostic capture only.",
    )
    parser.add_argument(
        "--title-screen-transition-capture",
        action="store_true",
        default=os.environ.get("MATTMC_TITLE_SCREEN_TRANSITION_CAPTURE", "").lower() in {"1", "true", "yes"},
        help="Capture a bounded reload-overlay-to-title desktop timeline; diagnostic only.",
    )
    parser.add_argument("--jvm-arg", action="append", default=[], help="Extra JVM option appended to JAVA_TOOL_OPTIONS.")
    parser.add_argument("--world", default=os.environ.get("MATTMC_CAPTURE_WORLD", "Origin"))
    parser.add_argument("--game-dir", default=os.environ.get("MATTMC_CAPTURE_GAME_DIR", ""))
    parser.add_argument(
        "--gui-resource-pack-scenario",
        default=os.environ.get("MATTMC_GUI_RESOURCE_PACK_SCENARIO", ""),
        help=(
            "Generate and select diagnostic GUI resource packs in the isolated game dir. "
            "Supported: vanilla, pack-a, pack-b, priority-a-b, priority-b-a, missing, malformed, unsupported, "
            "sky-sun-minification, sky-moon-minification (sky-only texture fixtures)."
        ),
    )
    parser.add_argument(
        "--world-static-terrain-resource-pack-scenario",
        default=os.environ.get("MATTMC_WORLD_STATIC_TERRAIN_RESOURCE_PACK_SCENARIO", ""),
        help=(
            "Generate and select diagnostic block-texture resource packs for Rust static-terrain atlas coverage. "
            "Supported: vanilla, pack-a, pack-b, priority-a-b, priority-b-a, missing, malformed, unsupported, terrain-identity, "
            "water-non-sequential, water-unequal-duration, water-interpolation-off, water-interpolation-on, "
            "water-pixel-replacement."
        ),
    )
    parser.add_argument(
        "--world-static-terrain-water-animation-capture",
        action="store_true",
        default=os.environ.get("MATTMC_WORLD_STATIC_TERRAIN_WATER_ANIMATION_CAPTURE", "").lower() in {"1", "true", "yes"},
        help="Retain a dense deterministic frame sequence for built-in water animation validation.",
    )
    parser.add_argument(
        "--world-static-terrain-scenario",
        default=os.environ.get("MATTMC_WORLD_STATIC_TERRAIN_SCENARIO", ""),
        help="Deterministic static-terrain lifecycle scenario name carried for isolated-world setup.",
    )
    parser.add_argument(
        "--world-static-terrain-second-world",
        default=os.environ.get("MATTMC_WORLD_STATIC_TERRAIN_SECOND_WORLD", ""),
        help="Explicit destination save name for the optional second static-terrain lifecycle world.",
    )
    parser.add_argument(
        "--world-static-terrain-fault",
        default=os.environ.get("MATTMC_WORLD_STATIC_TERRAIN_FAULT", ""),
        help="Static-terrain diagnostic fault injection to forward into copied deterministic runs.",
    )
    parser.add_argument("--region-validation", action="store_true")
    parser.add_argument("--region-validation-copy-world", action="store_true")
    parser.add_argument("--poi-validation", action="store_true")
    parser.add_argument("--platform", help="platform to pass to shared helpers: linux, windows, or macos")
    args = parser.parse_args()

    if args.max_secs < 0 or args.dump_secs < 0:
        raise SystemExit("--max-secs and --dump-secs must be integers")
    if args.meshing_corpus_warmup < 0 or args.meshing_corpus_measure < 0:
        raise SystemExit("--meshing-corpus-warmup and --meshing-corpus-measure must be non-negative integers")
    if args.deterministic_static_camera_capture:
        args.deterministic_camera_capture = True

    pose_tolerance = os.environ.get("DETERMINISTIC_POSE_TOLERANCE", "0.001")
    if not re.match(r"^[0-9]+([.][0-9]+)?$", pose_tolerance):
        raise SystemExit("DETERMINISTIC_POSE_TOLERANCE must be a non-negative number")

    return CaptureConfig(
        backend=args.backend,
        shaders=args.shaders,
        max_secs=args.max_secs,
        dump_secs=args.dump_secs,
        # The client routinely needs more than 4 GiB RSS during real startup
        # (Frozen OpenGL measured about 6.4 GiB in the migration fixture). Keep
        # a generous workstation-safety ceiling by default; callers can still
        # set CLIENT_RSS_LIMIT_MB explicitly for a tighter diagnostic run.
        client_rss_limit_mb=int_env("CLIENT_RSS_LIMIT_MB", 12288),
        screenshot_interval_secs=int_env("SCREENSHOT_INTERVAL_SECS", 5),
        screenshot_max_count=int_env("SCREENSHOT_MAX_COUNT", 6),
        screenshot_start_delay_secs=int_env("SCREENSHOT_START_DELAY_SECS", 0),
        validation_mode=args.validation,
        shader_input_parity=args.shader_input_parity,
        shader_input_parity_max_logs=int_env("SHADER_INPUT_PARITY_MAX_LOGS", 120000, signed=True),
        lightmap_info_parity_max_logs=int_env("LIGHTMAP_INFO_PARITY_MAX_LOGS", 512, signed=True),
        skip_tests=bool(args.skip_tests),
        client_args=args.client_args,
        deterministic_camera_capture=bool(args.deterministic_camera_capture),
        deterministic_static_camera_capture=bool(args.deterministic_static_camera_capture),
        deterministic_pose_tolerance=float(pose_tolerance),
        audio_validation=bool(args.audio_validation),
        capture_meshing_corpus=bool(args.capture_meshing_corpus),
        meshing_corpus_output=args.meshing_corpus_output,
        meshing_corpus_fixture=args.meshing_corpus_fixture,
        meshing_corpus_warmup=args.meshing_corpus_warmup,
        meshing_corpus_measure=args.meshing_corpus_measure,
        artifact_dir=args.artifact_dir,
        platform_name=args.platform,
        world=args.world,
        game_dir=args.game_dir,
        jvm_args=args.jvm_arg,
        gui_resource_pack_scenario=args.gui_resource_pack_scenario,
        world_static_terrain_scenario=args.world_static_terrain_scenario,
        world_static_terrain_second_world=args.world_static_terrain_second_world,
        world_static_terrain_fault=args.world_static_terrain_fault,
        world_static_terrain_resource_pack_scenario=args.world_static_terrain_resource_pack_scenario,
        world_static_terrain_water_animation_capture=bool(args.world_static_terrain_water_animation_capture),
        region_validation=bool(args.region_validation),
        region_validation_copy_world=bool(args.region_validation_copy_world),
        poi_validation=bool(args.poi_validation),
        title_screen_capture=bool(args.title_screen_capture),
        title_screen_transition_capture=bool(args.title_screen_transition_capture),
        deterministic_shutdown_grace_secs=int_env(
            "MATTMC_DETERMINISTIC_SHUTDOWN_GRACE_SECS",
            180 if args.region_validation else 20,
        ),
    )


def main() -> int:
    if os.environ.get("MATTMC_GRAPHICS_TOOL_INTERNAL") != "1":
        print(
            "capture_runner.py is an internal capture engine for graphics testing. "
            "Use DevUtils/Audit/Capture.py, DevUtils/PerfAudit/Gameplay.py, "
            "DevUtils/PerfAudit/Subsystem.py, or DevUtils/PerfAudit/Matrix.py.",
            file=sys.stderr,
        )
        return 2
    runner = CaptureRunner(parse_args())

    def handle_signal(signum, _frame) -> None:
        signal_name = signal.Signals(signum).name
        runner.terminate_run_processes(f"signal_{signal_name}")
        runner.cleanup_generated_game_dirs()
        runner.release_lock()
        raise SystemExit(128)

    for signal_name in ("SIGINT", "SIGTERM", "SIGHUP"):
        if hasattr(signal, signal_name):
            signal.signal(getattr(signal, signal_name), handle_signal)

    try:
        return runner.run()
    except KeyboardInterrupt:
        runner.terminate_run_processes("signal_INT")
        runner.cleanup_generated_game_dirs()
        runner.release_lock()
        return 128
    except SystemExit:
        runner.cleanup_generated_game_dirs()
        runner.release_lock()
        raise
    except Exception:
        if runner.run_client_active:
            runner.terminate_run_processes("script_exit_1")
        runner.cleanup_generated_game_dirs()
        runner.release_lock()
        raise


if __name__ == "__main__":
    raise SystemExit(main())
