package net.minecraft.client.dev;

import net.minecraft.client.Minecraft;
import net.minecraft.resources.ResourceLocation;

/** Opt-in fixture setup only: selects the ordinary game's post-effect route. */
public final class GraphicsAuditPostEffectFixture {
    private static final String EFFECT = validate(System.getProperty(
        "mattmc.dev.deterministicCameraCapture.postEffect", ""));
    private static long appliedFrames;

    private GraphicsAuditPostEffectFixture() {}

    static String validate(String effect) {
        if (!effect.isEmpty() && !java.util.Set.of("invert", "creeper", "spider").contains(effect)) {
            throw new IllegalArgumentException("Unknown capture post effect: " + effect);
        }
        return effect;
    }

    public static void beforeRender(Minecraft minecraft) {
        if (EFFECT.isEmpty() || minecraft.level == null || minecraft.player == null) return;
        minecraft.gameRenderer.loadPostEffect(ResourceLocation.withDefaultNamespace(EFFECT));
        appliedFrames = Math.min(appliedFrames + 1, 1_000_000L);
    }

    public static String receipt(Minecraft minecraft) {
        if (EFFECT.isEmpty()) return "null";
        boolean selected = minecraft.gameRenderer != null
            && ResourceLocation.withDefaultNamespace(EFFECT).equals(minecraft.gameRenderer.currentPostEffect());
        return "{\"schema\":\"normal-post-effect-fixture-v1\",\"effect\":\"" + EFFECT
            + "\",\"selected\":" + selected + ",\"appliedFrames\":" + appliedFrames + "}";
    }
}

