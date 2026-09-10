package net.minecraft.client.dev;

/** Capture scheduling only; never changes a game tick, texture, or renderer. */
public final class GraphicsAuditPhaseWait {
    private int attempts;
    private final java.util.function.LongSupplier clock;
    private long animationStart, lastPhase = -1, elapsedTicks;
    private int animationObservations;
    public GraphicsAuditPhaseWait() { this(System::nanoTime); }
    GraphicsAuditPhaseWait(java.util.function.LongSupplier clock) { this.clock = clock; }
    public boolean observe(boolean ready) {
        if (ready) {
            attempts = 0; lastPhase = -1; elapsedTicks = 0; animationObservations = 0;
            return true;
        }
        if (++attempts > 512) throw new IllegalStateException("animation capture phase was not reached within 512 rendered observations");
        return false;
    }
    public boolean observeAnimation(boolean ready, long phase, long duration) {
        if (duration <= 0 || duration > 512 || phase < 0 || phase >= duration)
            throw new IllegalStateException("invalid observed animation cycle");
        if (ready) return observe(true);
        long now = clock.getAsLong();
        if (lastPhase < 0) animationStart = now;
        else elapsedTicks += Math.floorMod(phase - lastPhase, duration);
        lastPhase = phase;
        // Count actual observed animation progress, not duplicate render frames.
        // Catch-up ticks can skip a requested phase between observations;
        // neither rendered-frame count nor cycle count measures opportunity.
        // A wall-clock deadline plus a hard frame bound still rejects stalls.
        if (++animationObservations > 16384 || now - animationStart >= 45_000_000_000L)
            throw new IllegalStateException("animation capture eligibility not reached: phase=" + phase
                + " duration=" + duration + " elapsedTicks=" + elapsedTicks
                + " renderedObservations=" + animationObservations);
        return false;
    }
    public static boolean cycleBoundary(long producedTick, long duration) {
        return producedTick > 0 && duration > 0 && producedTick % duration == 0;
    }
    public static long requestedPhase(long duration) {
        final long phase;
        try {
            phase = Long.parseLong(System.getProperty(
                GraphicsAuditBlockDisplayFixture.guiItemAnimationRequested() ? "mattmc.dev.graphicsAuditGuiItemCapturePhase" :
                net.minecraft.client.particle.GraphicsAuditVibrationParticleFixture.requested() ? "mattmc.dev.graphicsAuditVibrationCapturePhase" :
                net.minecraft.client.particle.GraphicsAuditAtlasParticleFixture.requested() ? "mattmc.dev.graphicsAuditAtlasParticleCapturePhase" :
                GraphicsAuditLavaFixture.requested() ? "mattmc.dev.graphicsAuditLavaCapturePhase" :
                GraphicsAuditBlockDisplayFixture.waterAnimationRequested()
                    ? "mattmc.dev.graphicsAuditWaterCapturePhase" : "mattmc.dev.graphicsAuditMagmaCapturePhase", "0"));
        } catch (NumberFormatException invalid) {
            throw new IllegalStateException("animation capture phase must be an integer", invalid);
        }
        if (duration <= 0 || phase < 0 || phase >= duration) {
            throw new IllegalStateException("animation capture phase must be inside the declared cycle");
        }
        return phase;
    }
    public static boolean phaseMatches(long producedTick, long duration, long phase) {
        return producedTick > 0 && duration > 0 && phase >= 0 && phase < duration
            && producedTick % duration == phase;
    }
}
