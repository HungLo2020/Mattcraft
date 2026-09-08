package net.minecraft.client.dev;

import org.junit.jupiter.api.Test;

import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertTrue;

class DeterministicCameraCaptureVignetteRegressionTest {
	@Test
	void finalOutputCaptureRequiresThePresentedTerrainSubmissionRatherThanThePreviousFrame() throws Exception {
		String coordinator = Files.readString(Path.of("src/main/java/net/vulkanic/gui/RustGalFrameCoordinator.java"));
		int present = coordinator.indexOf("bridge.presentFrame(frameId, correlationId, submissionId)");
		int execution = coordinator.indexOf("RustGalTerrainRenderer.recordExecutedStaticTerrainInstances(", present);
		int capture = coordinator.indexOf("writeWholeFrameAttachmentCorrelation(", present);
		assertTrue(present >= 0 && execution > present && capture > execution);
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/dev/DeterministicCameraCapture.java"));
		int method = source.indexOf("private static boolean captureWholeFrameFinalOutput()");
		int evidence = source.indexOf("recordRustFinalOutputCaptureCoverage(", method);
		assertTrue(evidence > method && evidence < source.indexOf("Files.copy(source, currentScreenshotPath", method));
		String diagnostics = Files.readString(Path.of("src/main/java/net/sodium/client/render/StaticTerrainParityDiagnostics.java"));
		assertTrue(diagnostics.contains("latestRustExecutionBackendFrameId != backendFrameId"));
	}

	@Test
	void captureCompletionDoesNotTruncateTheIndependentFrameSampleWindow() throws Exception {
		String capture = Files.readString(Path.of("src/main/java/net/minecraft/client/dev/DeterministicCameraCapture.java"));
		String benchmark = Files.readString(Path.of("src/main/java/net/minecraft/client/dev/GraphicsFrameBenchmark.java"));

		assertTrue(capture.contains("GraphicsFrameBenchmark.isAwaitingCompletion()"),
			"deterministic capture must wait for the configured frame benchmark rather than stopping at screenshot acknowledgement");
		assertTrue(benchmark.contains("return ENABLED && !complete && !failed;"),
			"the benchmark completion gate must remain independent of screenshot readiness");
		assertTrue(benchmark.contains("!DeterministicCameraCapture.isAwaitingCompletion()"),
			"a fast benchmark must not stop the client before its deterministic screenshot receipt arrives");
	}

	@Test
	void parityCaptureWaitsForVanillaVignetteAnimationWithoutMutatingIt() throws Exception {
		String capture = Files.readString(Path.of("src/main/java/net/minecraft/client/dev/DeterministicCameraCapture.java"));
		String gui = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/Gui.java"));

		assertTrue(gui.contains("vignetteBrightnessSettledForDeterministicCapture"),
			"the capture route must derive readiness from the vanilla vignette state");
		assertTrue(gui.contains("Math.abs(this.vignetteBrightness - target) <= 0.02F"),
			"readiness must wait for natural convergence rather than force a fixed vignette brightness");
		assertTrue(!gui.contains("deterministicCameraCapture.vignetteBrightness"),
			"the harness must not override vanilla GUI animation state");
		assertTrue(capture.contains("!minecraft.gui.vignetteBrightnessSettledForDeterministicCapture(minecraft.getCameraEntity())"),
			"the capture pose must not advance while the visual state is still animating");
		assertTrue(capture.contains("\\\"vignetteBrightness\\\"")
			&& capture.contains("vignetteBrightnessForDeterministicCapture()"),
			"the screenshot request must retain the exact vanilla fade value it was issued with");
	}

	@Test
	void rustWholeFrameFreshnessUsesTheGalFrameClockRatherThanHookCallbacks() throws Exception {
		String capture = Files.readString(Path.of("src/main/java/net/minecraft/client/dev/DeterministicCameraCapture.java"));
		String coordinator = Files.readString(Path.of("src/main/java/net/vulkanic/gui/RustGalFrameCoordinator.java"));

		assertTrue(capture.contains("lastAcquiredWholeFrameFrame()\n\t\t\t\t\t- net.vulkanic.gui.RustGalFrameCoordinator.lastRenderableWholeFrameWorldFrame() > 1"),
			"whole-frame freshness must compare the latest acquired GAL frame with the last world-admitted GAL frame");
		assertTrue(!capture.contains("renderedFrameIndex\n\t\t\t\t\t- net.vulkanic.gui.RustGalFrameCoordinator.lastRenderableWholeFrameWorldFrame() > 1"),
			"Java deterministic-hook callbacks are not on the GAL frame-id clock and cannot prove world-frame staleness");
		assertTrue(coordinator.contains("lastAcquiredWholeFrameFrame = frameId;")
			&& coordinator.contains("public static long lastAcquiredWholeFrameFrame()"),
			"the coordinator must publish a Rust-owned acquired-frame receipt for the freshness gate");
	}

	@Test
	void deterministicCaptureCanExerciseRustGalGuiInventoryRecovery() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/dev/DeterministicCameraCapture.java"));

		assertTrue(source.contains("mattmc.dev.deterministicCameraCapture.rustGalGuiScreenCycle"),
			"Deterministic capture must expose a bounded inventory open/close regression cycle for Rust GAL GUI recovery");
		assertTrue(source.contains("new InventoryScreen(minecraft.player)"),
			"The regression cycle must open the real inventory screen instead of simulating only metadata");
		assertTrue(source.contains("minecraft.setScreen(null)"),
			"The regression cycle must close inventory and return to gameplay HUD before the screenshot");
		assertTrue(source.contains("requestRustGalGuiScreenCycleInventoryCapture(minecraft)"),
			"The cycle must request a presented-frame capture before closing inventory");
		assertTrue(source.contains("capture_request_rust_gal_gui_screen_cycle_inventory.json"),
			"The inventory capture must use the shared external request/ack protocol, not Java GPU readback");
		assertTrue(source.contains("Java's main render target, which has no valid image on selected"),
			"The regression fixture must preserve the selected-Vulkan no-Java-readback boundary");
		assertTrue(source.contains("!rustGalGuiScreenCycleInventoryCaptureComplete"),
			"The cycle must keep inventory visible until the presented-frame capture is acknowledged");
		assertTrue(source.contains("cyclesCompleted"),
			"The deterministic artifact must record screen-cycle state for capture validation");
	}

	@Test
	void modelProducerReceiptSurvivesClientLevelTeardown() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/dev/DeterministicCameraCapture.java"));

		assertTrue(source.contains("!\"spawned\".equals(modelMeshSetupStatus)"),
			"teardown polling must not overwrite a successfully spawned model producer receipt");
		assertTrue(source.contains("!\"server-spawned\".equals(modelMeshSetupStatus)"),
			"teardown polling must preserve the server-spawned state until client evidence is recorded");
	}

	@Test
	void evokerFangsFixtureUsesVanillaAttackTimingAndRetriesAfterExpiry() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/dev/DeterministicCameraCapture.java"));
		assertTrue(source.contains("\"evoker-fangs\".equals(MODEL_MESH_SCENARIO)"),
			"the deterministic model fixture must keep an explicit Evoker Fangs branch");
		assertTrue(source.contains("modelMeshSetupServerEntityId >= 0")
			&& source.contains("serverLevel.getEntity(modelMeshSetupServerEntityId) == null")
			&& source.contains("modelMeshSetupStatus = \"server-fixture-expired\""),
			"short-lived vanilla fangs must retry server tracking after their real lifecycle expires");
		assertTrue(source.contains("Math.toRadians(player.getYRot()), 0, serverPlayer"),
			"the fixture must use vanilla immediate warmup so attack event 4 can reach the client during capture");
		assertTrue(source.contains("serverLevel.broadcastEntityEvent(fangs, (byte)4)"),
			"the fixture must replicate the vanilla fangs attack event through the server level rather than mutating client state");
	}

	@Test
	void rustWholeFrameBlockEntitiesDoNotDependOnCompiledTerrainReadiness() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/LevelRenderer.java"));

		assertTrue(source.contains("Set<Long> extractedBlockEntityPositions = Sets.newHashSet();"),
			"Rust whole-frame block-entity extraction must deduplicate semantic producers by block position");
		assertTrue(source.contains("getChunk(chunkX, chunkZ, ChunkStatus.FULL, false)"),
			"Rust whole-frame block entities must be discoverable from bounded loaded chunks before terrain compilation completes");
		assertTrue(source.contains("renderer shouldRender() and Rust route admission"),
			"the bounded block-entity scan must retain renderer visibility and Rust admission as semantic filters");
	}

	@Test
	void texturePaletteFixtureRebuildsOnlyItsEditedSections() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/dev/DeterministicCameraCapture.java"));
		int paletteCase = source.indexOf("case \"texture-palette\" -> {");
		int nextCase = source.indexOf("case \"resource-reload\"", paletteCase);
		String paletteFixture = source.substring(paletteCase, nextCase);

		assertTrue(paletteFixture.contains("applyStaticTerrainTexturePalette(minecraft, serverLevel, target);"),
			"The palette fixture must still place the real client/server block states");
		assertTrue(source.contains("minecraft.levelRenderer.setBlocksDirty("),
			"The palette helper must request a bounded Sodium rebuild for its changed blocks");
		assertTrue(!paletteFixture.contains("minecraft.levelRenderer.allChanged();"),
			"The palette fixture must not invalidate every terrain source snapshot after its local edits");
	}

	@Test
	void waterFixtureDoesNotDeadlockRouteSelectionOnItsPostExecutionProbe() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/dev/DeterministicCameraCapture.java"));
		int cacheGate = source.indexOf("boolean fixtureSourceCached = fixtureCoverage.cachedColumns() > 0");
		assertTrue(cacheGate >= 0,
			"DH water readiness must start from the published column cache without requiring a water probe first");
		int executedGate = source.indexOf("!DISTANT_HORIZONS_REQUIRE_WATER || (distantHorizonsTexturePaletteWaterSourceObserved", cacheGate);
		assertTrue(executedGate > cacheGate,
			"the water requirement must remain enforced at the final executed-route gate");
	}

	@Test
	void waterFixtureStaysInsideTheInvalidatedPaletteFootprint() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/dev/DeterministicCameraCapture.java"));
		// The initial readiness block also mentions this flag but only records
		// cached witnesses. Inspect the actual fixture-construction block, which
		// starts after the shared panel origin is established.
		int panelOrigin = source.indexOf("BlockPos panelOrigin");
		int water = source.indexOf("if (DISTANT_HORIZONS_REQUIRE_WATER)", panelOrigin);
		int nextBlock = source.indexOf("distantHorizonsWaterWitnesses =", water);
		assertTrue(panelOrigin >= 0 && water > panelOrigin && nextBlock > water);
		String fixture = source.substring(water, nextBlock);
		assertTrue(fixture.contains("localX = 28; localX < 32"));
		assertTrue(fixture.contains("localZ = 24; localZ < 28"));
		assertTrue(fixture.contains("localZ : new int[] { 23, 28 }"));
		assertTrue(fixture.contains("localX : new int[] { 23, 28 }"));
	}
}
