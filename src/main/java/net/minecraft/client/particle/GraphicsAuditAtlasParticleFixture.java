package net.minecraft.client.particle;

import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import net.minecraft.client.Minecraft;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.renderer.texture.TextureAtlas;
import net.minecraft.resources.ResourceLocation;

/** Shared deterministic inputs for an ordinary flame particle; no rendering overrides. */
public final class GraphicsAuditAtlasParticleFixture {
    private static ClientLevel installedLevel;
    private static FlameParticle particle;
    public static final ResourceLocation SPRITE = ResourceLocation.withDefaultNamespace("flame");
    private GraphicsAuditAtlasParticleFixture() {}
    public static boolean requested() { return Boolean.getBoolean("mattmc.dev.graphicsAuditAtlasParticle"); }
    public static boolean staticRequested() {
        return requested() && Boolean.getBoolean("mattmc.dev.graphicsAuditAtlasParticleStatic");
    }

    public static boolean staticReady(Minecraft minecraft) {
        if (!staticRequested() || minecraft.level == null) return false;
        var atlas = minecraft.getAtlasManager().getAtlasOrThrow(net.minecraft.data.AtlasIds.PARTICLES);
        var contents = atlas.getSprite(SPRITE).contents();
        return contents.name().equals(SPRITE) && contents.animatedTexture == null
            && contents.originalImage.getWidth() == 16 && contents.originalImage.getHeight() == 16;
    }

    public static void install(Minecraft minecraft) {
        if (!requested() || minecraft.level == null || minecraft.player == null) return;
        var atlas = minecraft.getAtlasManager().getAtlasOrThrow(net.minecraft.data.AtlasIds.PARTICLES);
        var sprite = atlas.getSprite(SPRITE);
        if (installedLevel == minecraft.level && particle != null && particle.sprite == sprite
            && particle.isAlive() && minecraft.particleEngine.containsGraphicsAuditParticle(particle)) return;
        if (particle != null) particle.remove();
        var origin = minecraft.player.getEyePosition().add(minecraft.player.getLookAngle().normalize().scale(3));
        particle = new FlameParticle(minecraft.level, origin.x, origin.y, origin.z, 0, 0, 0, sprite);
        configure(particle, origin);
        installedLevel = minecraft.level;
        minecraft.particleEngine.installGraphicsAuditParticle(particle);
    }

    static void configure(FlameParticle particle, net.minecraft.world.phys.Vec3 origin) {
        if (!requested()) throw new IllegalStateException("Atlas particle fixture not requested");
        // Fixture inputs only. The ordinary particle still ticks and extracts
        // through the game's normal implementation in both repositories.
        particle.setPos(origin.x, origin.y, origin.z);
        particle.xo = origin.x; particle.yo = origin.y; particle.zo = origin.z;
        particle.xd = particle.yd = particle.zd = 0;
        particle.gravity = 0; particle.friction = 1; particle.hasPhysics = false;
        // Keep ordinary float lifetime scaling exactly unchanged throughout a
        // bounded capture; a million ticks still changes its last size bit.
        particle.quadSize = 0.35F; particle.lifetime = Integer.MAX_VALUE;
        particle.rCol = particle.gCol = particle.bCol = particle.alpha = 1;
    }

    public static String receipt(Minecraft minecraft) {
        if (!requested()) return "null";
        JsonObject result = new JsonObject();
        result.addProperty("fixture", staticRequested() ? "ordinary-static-flame-atlas-v1" : "ordinary-flame-atlas-v1");
        boolean present = installedLevel == minecraft.level && particle != null;
        result.addProperty("complete", present && particle.isAlive()
            && minecraft.particleEngine.containsGraphicsAuditParticle(particle));
        if (!present) return result.toString();
        result.addProperty("atlas", TextureAtlas.LOCATION_PARTICLES.toString());
        result.addProperty("sprite", particle.sprite.contents().name().toString());
        JsonArray position = new JsonArray();
        position.add(particle.x); position.add(particle.y); position.add(particle.z);
        result.add("position", position);
        result.addProperty("size", particle.getQuadSize(1));
        result.addProperty("light", particle.getLightColor(1));
        result.addProperty("color", 0xffffffff);
        if (staticRequested()) {
            result.addProperty("staticSourceComplete", staticReady(minecraft));
            var pixels = particle.sprite.contents().originalImage;
            long hash = 0xcbf29ce484222325L;
            for (int y = 0; y < pixels.getHeight(); y++) for (int x = 0; x < pixels.getWidth(); x++) {
                int argb = pixels.getPixel(x, y);
                for (int shift : new int[]{16, 8, 0, 24})
                    hash = (hash ^ ((argb >>> shift) & 255)) * 0x100000001b3L;
            }
            result.addProperty("sourceRgbaFnv64", String.format(java.util.Locale.ROOT, "%016x", hash));
        }
        return result.toString();
    }
}
